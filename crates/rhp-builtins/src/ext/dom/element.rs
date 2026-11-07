//! DOMElement's methods (php-src ext/dom/element.c) and the ParentNode /
//! ChildNode algorithms of ext/dom/parentnode/tree.c.

use std::ffi::c_int;
use std::ptr;

use rhp_values::{PhpArray, Value};

use super::util::*;
use super::{
    arg, boolean, bytes, create_object, ensure_intern, get_intern, get_obj, lists, opt_bytes,
    throw, throw_msg, xs, Cx, DomObject, Kind, HIERARCHY_REQUEST_ERR, INVALID_CHARACTER_ERR,
    INVALID_MODIFICATION_ERR, INVALID_STATE_ERR, NAMESPACE_ERR, NOT_FOUND_ERR,
    NO_MODIFICATION_ALLOWED_ERR, SYNTAX_ERR, WRONG_DOCUMENT_ERR,
};
use crate::ext::libxml::node as lxn;
use crate::ext::libxml::sys::{self, XmlAttr, XmlDoc, XmlNode, XmlNs};
use crate::Thrown;

fn must_not_be_empty(cx: &Cx, n: usize, pname: &str) -> Thrown {
    crate::zend::zpp::value_error(&cx.fname, n, pname, "must not be empty")
}

pub fn construct(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let value = opt_bytes(args, 1);
    let uri = bytes(args, 2);
    if !valid_name(&name) {
        throw(cx, INVALID_CHARACTER_ERR, true)?;
    }
    let nodep;
    unsafe {
        if !uri.is_empty() {
            let (localname, prefix) = match check_qname(&name, uri.len()) {
                Ok(p) => p,
                Err(code) => {
                    throw(cx, code, true)?;
                    return Ok(Value::Null);
                }
            };
            let cl = c(&localname);
            nodep = sys::xmlNewDocNode(ptr::null_mut(), ptr::null_mut(), cl.as_ptr(), ptr::null());
            if !nodep.is_null() {
                match get_ns(nodep, &uri, prefix.as_deref()) {
                    Ok(ns) => sys::xmlSetNs(nodep, ns),
                    Err(code) => {
                        sys::xmlFreeNode(nodep);
                        throw(cx, code, true)?;
                        return Ok(Value::Null);
                    }
                }
            }
        } else {
            if split_qname2(&name).is_some() {
                throw(cx, NAMESPACE_ERR, true)?;
            }
            let cn = c(&name);
            nodep = sys::xmlNewDocNode(ptr::null_mut(), ptr::null_mut(), cn.as_ptr(), ptr::null());
        }
        if nodep.is_null() {
            throw(cx, INVALID_STATE_ERR, true)?;
        }
        if let Some(v) = &value {
            if !v.is_empty() {
                cx.with_libxml(|| {
                    sys::xmlNodeSetContentLen(nodep, nul(v).as_ptr(), v.len() as c_int)
                })?;
            }
        }
    }
    set_constructed_node(this, nodep);
    Ok(Value::Null)
}

/// The end of every node constructor: drop the old node, wrap the new one.
pub fn set_constructed_node(this: &Value, nodep: *mut XmlNode) {
    let Some(intern) = ensure_intern(this, || Kind::Node) else {
        return;
    };
    if !intern.node().is_null() {
        lxn::node_decrement_resource(&intern.base);
    }
    lxn::increment_node_ptr(&intern.base, nodep, &intern.base);
}

/// dom_get_attribute_or_nsdecl: an attribute, or a namespace declaration
/// (as an `xmlNs`) for `xmlns` / `xmlns:p`.
fn get_attribute_or_nsdecl(elem: *mut XmlNode, name: &[u8]) -> *mut XmlNode {
    unsafe {
        if let Some(colon) = name.iter().position(|&b| b == b':') {
            let nqname = &name[colon + 1..];
            // xmlSplitQName3: a non-empty prefix.
            if colon > 0 {
                if &name[..colon] == b"xmlns" {
                    let mut ns = (*elem).nsDef;
                    while !ns.is_null() {
                        if str_eq_b((*ns).prefix, nqname) {
                            break;
                        }
                        ns = (*ns).next;
                    }
                    return ns.cast();
                }
                let cp = c(&name[..colon]);
                let ns = sys::xmlSearchNs((*elem).doc, elem, cp.as_ptr());
                if !ns.is_null() {
                    let cl = c(nqname);
                    return sys::xmlHasNsProp(elem, cl.as_ptr(), (*ns).href).cast();
                }
            }
        } else if name == b"xmlns" {
            let mut ns = (*elem).nsDef;
            while !ns.is_null() {
                if (*ns).prefix.is_null() {
                    return ns.cast();
                }
                ns = (*ns).next;
            }
            return ptr::null_mut();
        }
        let cn = c(name);
        sys::xmlHasNsProp(elem, cn.as_ptr(), ptr::null()).cast()
    }
}

/// The node type of what get_attribute_or_nsdecl found (an `xmlNs` has
/// its type in its second field too).
fn found_type(n: *mut XmlNode) -> c_int {
    unsafe {
        let ns = n as *mut XmlNs;
        if (*ns).type_ == sys::XML_NAMESPACE_DECL {
            return sys::XML_NAMESPACE_DECL;
        }
        (*n).type_
    }
}

pub fn get_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (_, nodep) = get_obj(cx, this)?;
    let attr = get_attribute_or_nsdecl(nodep, &bytes(args, 0));
    if attr.is_null() {
        return Ok(Value::str(""));
    }
    unsafe {
        Ok(match found_type(attr) {
            sys::XML_ATTRIBUTE_NODE => Value::str(
                sys::take_string(sys::xmlNodeListGetString((*attr).doc, (*attr).children, 1))
                    .unwrap_or_default(),
            ),
            sys::XML_NAMESPACE_DECL => xs((*(attr as *mut XmlNs)).href),
            // xmlAttribute: an attribute default from the DTD.
            _ => Value::str(sys::cstr(attribute_decl_default(attr))),
        })
    }
}

/// `((xmlAttributePtr) attr)->defaultValue`.
unsafe fn attribute_decl_default(attr: *mut XmlNode) -> *const u8 {
    unsafe { sys::rhp_xml_attribute_decl_default(attr.cast()) }
}

pub fn get_attribute_names(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, nodep) = get_obj(cx, this)?;
    let mut out = PhpArray::new();
    unsafe {
        let mut ns = (*nodep).nsDef;
        while !ns.is_null() {
            if (*ns).prefix.is_null() {
                out.push(Value::str("xmlns"));
            } else {
                let mut s = b"xmlns:".to_vec();
                s.extend_from_slice(b((*ns).prefix));
                out.push(Value::str(s));
            }
            ns = (*ns).next;
        }
        let mut a = (*nodep).properties;
        while !a.is_null() {
            out.push(Value::str(qualified_name(a.cast())));
            a = (*a).next;
        }
    }
    Ok(Value::Array(out))
}

/// dom_create_attribute
fn create_attribute(nodep: *mut XmlNode, name: &[u8], value: &[u8]) -> *mut XmlNode {
    let cn = c(name);
    let cv = c(value);
    unsafe {
        if name == b"xmlns" {
            sys::xmlNewNs(nodep, cv.as_ptr(), ptr::null()).cast()
        } else {
            sys::xmlSetProp(nodep, cn.as_ptr(), cv.as_ptr()).cast()
        }
    }
}

pub fn set_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let value = bytes(args, 1);
    if name.is_empty() {
        return Err(must_not_be_empty(cx, 1, "qualifiedName"));
    }
    if !valid_name(&name) {
        throw(cx, INVALID_CHARACTER_ERR, true)?;
    }
    let (intern, nodep) = get_obj(cx, this)?;
    let attr = get_attribute_or_nsdecl(nodep, &name);
    if !attr.is_null() {
        match found_type(attr) {
            sys::XML_ATTRIBUTE_NODE => {
                attr_value_will_change(intern.document(), attr.cast());
                node_list_unlink(unsafe { (*attr).children });
            }
            sys::XML_NAMESPACE_DECL => return Ok(Value::Bool(false)),
            _ => {}
        }
    }
    let attr = cx.with_libxml(|| create_attribute(nodep, &name, &value))?;
    if attr.is_null() {
        return Err(crate::zend::zpp::value_error(
            &cx.fname,
            1,
            "qualifiedName",
            "must be a valid XML attribute",
        ));
    }
    if found_type(attr) == sys::XML_NAMESPACE_DECL {
        return Ok(Value::Bool(true));
    }
    create_object(cx, attr, Some(&intern))
}

/// dom_deep_ns_redef: redeclare `ns` closest to the nodes still using it.
fn deep_ns_redef(node: *mut XmlNode, ns: *mut XmlNs) {
    let mut worklist: Vec<(*mut XmlNode, *mut XmlNs)> = vec![(node, ptr::null_mut())];
    unsafe {
        while let Some((cur, mut defined)) = worklist.pop() {
            if (*cur).ns == ns {
                if defined.is_null() {
                    defined = sys::xmlNewNs(cur, (*ns).href, (*ns).prefix);
                }
                (*cur).ns = defined;
            }
            let mut a = (*cur).properties;
            while !a.is_null() {
                if (*a).ns == ns {
                    if defined.is_null() {
                        defined = sys::xmlNewNs(cur, (*ns).href, (*ns).prefix);
                    }
                    (*a).ns = defined;
                }
                a = (*a).next;
            }
            let mut ch = (*cur).children;
            while !ch.is_null() {
                if (*ch).type_ == sys::XML_ELEMENT_NODE {
                    worklist.push((ch, defined));
                }
                ch = (*ch).next;
            }
        }
    }
}

/// dom_remove_attribute
fn remove_attribute_node_impl(
    thisp: *mut XmlNode,
    attrp: *mut XmlNode,
    document: *mut lxn::RefObj,
) -> bool {
    unsafe {
        match found_type(attrp) {
            sys::XML_ATTRIBUTE_NODE => {
                if lxn::object_of_node(attrp).is_none() {
                    node_list_unlink((*attrp).children);
                    sys::xmlUnlinkNode(attrp);
                    sys::xmlFreeProp(attrp.cast());
                } else {
                    sys::xmlUnlinkNode(attrp);
                }
            }
            sys::XML_NAMESPACE_DECL => {
                let ns = attrp as *mut XmlNs;
                if (*thisp).nsDef == ns {
                    (*thisp).nsDef = (*ns).next;
                } else if !(*thisp).nsDef.is_null() {
                    let mut prev = (*thisp).nsDef;
                    let mut cur = (*prev).next;
                    while !cur.is_null() {
                        if cur == ns {
                            (*prev).next = (*cur).next;
                            break;
                        }
                        prev = cur;
                        cur = (*cur).next;
                    }
                } else {
                    lxn::invalidate_node_list_cache(document);
                    return true;
                }
                (*ns).next = ptr::null_mut();
                lxn::set_old_ns((*thisp).doc, ns);
                deep_ns_redef(thisp, ns);
            }
            _ => return false,
        }
    }
    lxn::invalidate_node_list_cache(document);
    true
}

pub fn remove_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let attrp = get_attribute_or_nsdecl(nodep, &bytes(args, 0));
    if attrp.is_null() {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(remove_attribute_node_impl(
        nodep,
        attrp,
        intern.document(),
    )))
}

pub fn get_attribute_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let attrp = get_attribute_or_nsdecl(nodep, &bytes(args, 0));
    if attrp.is_null() {
        return Ok(Value::Bool(false));
    }
    if found_type(attrp) == sys::XML_NAMESPACE_DECL {
        return super::node::create_fake_namespace_decl(cx, nodep, attrp.cast(), &intern);
    }
    create_object(cx, attrp, Some(&intern))
}

fn set_attribute_node_common(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
    use_ns: bool,
) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let (_, attrn) = get_obj(cx, &arg(args, 0))?;
    let attrp = attrn as *mut XmlAttr;
    unsafe {
        if !((*attrp).doc.is_null() || (*attrp).doc == (*nodep).doc) {
            throw(cx, WRONG_DOCUMENT_ERR, intern.strict())?;
            return Ok(Value::Bool(false));
        }
        let nsp = (*attrp).ns;
        let mut existattrp = if use_ns && !nsp.is_null() {
            sys::xmlHasNsProp(nodep, (*attrp).name, (*nsp).href)
        } else if nsp.is_null() {
            sys::xmlHasNsProp(nodep, (*attrp).name, ptr::null())
        } else {
            sys::xmlHasProp(nodep, (*attrp).name)
        };
        if !existattrp.is_null() && (*existattrp).type_ == sys::XML_ATTRIBUTE_DECL {
            existattrp = ptr::null_mut();
        }
        if !existattrp.is_null() {
            if let Some(old) = lxn::object_of_node(existattrp.cast()) {
                if old.xml_node() == attrn {
                    return Ok(Value::Null);
                }
            }
            sys::xmlUnlinkNode(existattrp.cast());
        }
        if !(*attrp).parent.is_null() {
            sys::xmlUnlinkNode(attrn);
        }
        if (*attrp).doc.is_null() && !(*nodep).doc.is_null() && !intern.document().is_null() {
            set_document_ref_pointers_attr(attrp, intern.document());
        }
        sys::xmlAddChild(nodep, attrn);
        mark_ids_modified(intern.document());
        reconcile_attribute_namespace_after_insertion(attrp);
        if !existattrp.is_null() {
            create_object(cx, existattrp.cast(), Some(&intern))
        } else {
            Ok(Value::Null)
        }
    }
}

pub fn set_attribute_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    set_attribute_node_common(cx, this, args, false)
}

pub fn set_attribute_node_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    set_attribute_node_common(cx, this, args, true)
}

pub fn remove_attribute_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let (_, attrp) = get_obj(cx, &arg(args, 0))?;
    unsafe {
        if (*attrp).parent != nodep {
            throw(cx, NOT_FOUND_ERR, intern.strict())?;
            return Ok(Value::Bool(false));
        }
        lxn::invalidate_node_list_cache(intern.document());
        sys::xmlUnlinkNode(attrp);
    }
    create_object(cx, attrp, Some(&intern))
}

pub fn get_elements_by_tag_name(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    if name.contains(&0) {
        return Err(crate::zend::zpp::value_error(
            &cx.fname,
            1,
            "qualifiedName",
            "must not contain any null bytes",
        ));
    }
    let intern = get_intern(cx, this)?;
    lists::new_map_named(
        cx,
        &intern,
        lists::Handler::ByTagName,
        b"DOMNodeList",
        Some(&name),
        None,
    )
}

pub fn get_elements_by_tag_name_ns(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0).unwrap_or_default();
    let name = bytes(args, 1);
    for (i, (s, p)) in [(&uri, "namespace"), (&name, "localName")]
        .iter()
        .enumerate()
    {
        if s.contains(&0) {
            return Err(crate::zend::zpp::value_error(
                &cx.fname,
                i + 1,
                p,
                "must not contain any null bytes",
            ));
        }
    }
    let intern = get_intern(cx, this)?;
    lists::new_map_named(
        cx,
        &intern,
        lists::Handler::ByTagName,
        b"DOMNodeList",
        Some(&name),
        Some(&uri),
    )
}

/// dom_get_attribute_ns (legacy)
fn get_attribute_ns_value(elemp: *mut XmlNode, uri: Option<&[u8]>, name: &[u8]) -> Option<Vec<u8>> {
    let cn = c(name);
    let cu = uri.map(c);
    unsafe {
        if let Some(s) = sys::take_string(sys::xmlGetNsProp(
            elemp,
            cn.as_ptr(),
            cu.as_ref().map_or(ptr::null(), |u| u.as_ptr()),
        )) {
            return Some(s);
        }
        if uri == Some(XMLNS_NS_URI) {
            let ns = get_nsdecl(elemp, Some(name));
            if !ns.is_null() {
                return Some(b((*ns).href).to_vec());
            }
        }
    }
    None
}

pub fn get_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (_, elemp) = get_obj(cx, this)?;
    let uri = opt_bytes(args, 0);
    Ok(Value::str(
        get_attribute_ns_value(elemp, uri.as_deref(), &bytes(args, 1)).unwrap_or_default(),
    ))
}

pub fn has_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (_, elemp) = get_obj(cx, this)?;
    let uri = opt_bytes(args, 0);
    Ok(Value::Bool(
        get_attribute_ns_value(elemp, uri.as_deref(), &bytes(args, 1)).is_some(),
    ))
}

pub fn set_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, elemp) = get_obj(cx, this)?;
    let uri = opt_bytes(args, 0);
    let name = bytes(args, 1);
    let value = bytes(args, 2);
    if name.is_empty() {
        return Err(must_not_be_empty(cx, 2, "qualifiedName"));
    }
    let uri_len = uri.as_ref().map_or(0, |u| u.len());
    let mut strict = intern.strict();
    let mut errorcode = 0;
    let cv = c(&value);
    match check_qname(&name, uri_len) {
        Err(code) => errorcode = code,
        Ok((localname, prefix)) => unsafe {
            mark_ids_modified(intern.document());
            let cl = c(&localname);
            if uri_len > 0 {
                let u = uri.as_deref().unwrap_or_default();
                let cu = c(u);
                let nodep: *mut XmlNode = sys::xmlHasNsProp(elemp, cl.as_ptr(), cu.as_ptr()).cast();
                if !nodep.is_null() && (*nodep).type_ != sys::XML_ATTRIBUTE_DECL {
                    node_list_unlink((*nodep).children);
                }
                let mut is_xmlns = false;
                let mut nsptr: *mut XmlNs;
                if (prefix.as_deref() == Some(b"xmlns")
                    || (prefix.is_none() && localname == b"xmlns"))
                    && u == XMLNS_NS_URI
                {
                    is_xmlns = true;
                    nsptr = if prefix.is_none() {
                        get_nsdecl(elemp, None)
                    } else {
                        get_nsdecl(elemp, Some(&localname))
                    };
                } else {
                    nsptr = sys::xmlSearchNsByHref((*elemp).doc, elemp, cu.as_ptr());
                    if !nsptr.is_null() && (*nsptr).prefix.is_null() {
                        let mut tmp = (*nsptr).next;
                        while !tmp.is_null() {
                            if !(*tmp).prefix.is_null()
                                && !(*tmp).href.is_null()
                                && str_eq_b((*tmp).href, u)
                            {
                                nsptr = tmp;
                                break;
                            }
                            tmp = (*tmp).next;
                        }
                        if tmp.is_null() {
                            nsptr = get_ns_resolve_prefix_conflict(elemp, (*nsptr).href);
                        }
                    }
                }
                if nsptr.is_null() {
                    if is_xmlns {
                        let p = if prefix.is_none() {
                            None
                        } else {
                            Some(c(&localname))
                        };
                        sys::xmlNewNs(
                            elemp,
                            cv.as_ptr(),
                            p.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
                        );
                    } else {
                        match get_ns(elemp, u, prefix.as_deref()) {
                            Ok(ns) => nsptr = ns,
                            Err(code) => errorcode = code,
                        }
                    }
                    sys::xmlReconciliateNs((*elemp).doc, elemp);
                } else if is_xmlns {
                    sys::xml_free((*nsptr).href as *mut u8);
                    (*nsptr).href = sys::xmlStrdup(cv.as_ptr());
                }
                if errorcode == 0 && !is_xmlns {
                    sys::xmlSetNsProp(elemp, nsptr, cl.as_ptr(), cv.as_ptr());
                }
            } else if !valid_name(&localname) {
                errorcode = INVALID_CHARACTER_ERR;
                strict = true;
            } else {
                let attr = sys::xmlHasProp(elemp, cl.as_ptr());
                if !attr.is_null() && (*attr).type_ != sys::XML_ATTRIBUTE_DECL {
                    node_list_unlink((*attr).children);
                }
                sys::xmlSetProp(elemp, cl.as_ptr(), cv.as_ptr());
            }
        },
    }
    if errorcode != 0 {
        throw(cx, errorcode, strict)?;
    }
    Ok(Value::Null)
}

fn remove_eliminated_ns_single(node: *mut XmlNode, ns: *mut XmlNs) {
    unsafe {
        if (*node).ns == ns {
            (*node).ns = ptr::null_mut();
        }
        let mut a = (*node).properties;
        while !a.is_null() {
            if (*a).ns == ns {
                (*a).ns = ptr::null_mut();
            }
            a = (*a).next;
        }
    }
}

/// dom_eliminate_ns
fn eliminate_ns(nodep: *mut XmlNode, nsptr: *mut XmlNs) {
    unsafe {
        if !(*nsptr).href.is_null() {
            sys::xml_free((*nsptr).href as *mut u8);
            (*nsptr).href = ptr::null();
        }
        if !(*nsptr).prefix.is_null() {
            sys::xml_free((*nsptr).prefix as *mut u8);
            (*nsptr).prefix = ptr::null();
        }
        let mut cur = (*nodep).nsDef;
        if cur == nsptr {
            (*nodep).nsDef = (*nsptr).next;
        } else {
            while !cur.is_null() {
                if (*cur).next == nsptr {
                    (*cur).next = (*nsptr).next;
                    break;
                }
                cur = (*cur).next;
            }
        }
        (*nsptr).next = ptr::null_mut();
        lxn::set_old_ns((*nodep).doc, nsptr);
        remove_eliminated_ns_single(nodep, nsptr);
        let base = nodep;
        let mut n = (*nodep).children;
        while !n.is_null() {
            if (*n).type_ == sys::XML_ELEMENT_NODE {
                remove_eliminated_ns_single(n, nsptr);
            }
            n = next_in_tree_order(n, base);
        }
    }
}

pub fn remove_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let uri = opt_bytes(args, 0);
    let name = bytes(args, 1);
    let cn = c(&name);
    let cu = uri.as_deref().map(c);
    unsafe {
        let attrp = sys::xmlHasNsProp(
            nodep,
            cn.as_ptr(),
            cu.as_ref().map_or(ptr::null(), |u| u.as_ptr()),
        );
        let nsptr = get_nsdecl(nodep, Some(&name));
        if !nsptr.is_null() {
            if str_eq(
                cu.as_ref().map_or(ptr::null(), |u| u.as_ptr()),
                (*nsptr).href,
            ) {
                eliminate_ns(nodep, nsptr);
                lxn::invalidate_node_list_cache(intern.document());
            } else {
                return Ok(Value::Null);
            }
        }
        if !attrp.is_null() && (*attrp).type_ != sys::XML_ATTRIBUTE_DECL {
            if lxn::object_of_node(attrp.cast()).is_none() {
                node_list_unlink((*attrp).children);
                sys::xmlUnlinkNode(attrp.cast());
                sys::xmlFreeProp(attrp);
            } else {
                sys::xmlUnlinkNode(attrp.cast());
            }
            lxn::invalidate_node_list_cache(intern.document());
        }
    }
    Ok(Value::Null)
}

pub fn get_attribute_node_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, elemp) = get_obj(cx, this)?;
    let uri = opt_bytes(args, 0);
    let name = bytes(args, 1);
    let cn = c(&name);
    let cu = uri.as_deref().map(c);
    let attrp = unsafe {
        sys::xmlHasNsProp(
            elemp,
            cn.as_ptr(),
            cu.as_ref().map_or(ptr::null(), |u| u.as_ptr()),
        )
    };
    if attrp.is_null() {
        if uri.as_deref() == Some(XMLNS_NS_URI) {
            let nsptr = get_nsdecl(elemp, Some(&name));
            if !nsptr.is_null() {
                return super::node::create_fake_namespace_decl(cx, elemp, nsptr, &intern);
            }
        }
        return Ok(Value::Null);
    }
    create_object(cx, attrp.cast(), Some(&intern))
}

pub fn has_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (_, nodep) = get_obj(cx, this)?;
    Ok(Value::Bool(
        !get_attribute_or_nsdecl(nodep, &bytes(args, 0)).is_null(),
    ))
}

/// php_set_attribute_id
pub fn set_attribute_id(attrp: *mut XmlAttr, is_id: bool, document: *mut lxn::RefObj) {
    unsafe {
        if is_id && (*attrp).atype != sys::XML_ATTRIBUTE_ID {
            (*attrp).atype = sys::XML_ATTRIBUTE_ID;
        } else if !is_id && (*attrp).atype == sys::XML_ATTRIBUTE_ID {
            sys::xmlRemoveID((*attrp).doc, attrp);
            (*attrp).atype = 0;
        }
    }
    mark_ids_modified(document);
}

pub fn set_id_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let cn = c(&bytes(args, 0));
    let attrp = unsafe { sys::xmlHasNsProp(nodep, cn.as_ptr(), ptr::null()) };
    if attrp.is_null() || unsafe { (*attrp).type_ } == sys::XML_ATTRIBUTE_DECL {
        throw(cx, NOT_FOUND_ERR, intern.strict())?;
    } else {
        set_attribute_id(attrp, boolean(args, 1), intern.document());
    }
    Ok(Value::Null)
}

pub fn set_id_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, elemp) = get_obj(cx, this)?;
    let cu = c(&bytes(args, 0));
    let cn = c(&bytes(args, 1));
    let attrp = unsafe { sys::xmlHasNsProp(elemp, cn.as_ptr(), cu.as_ptr()) };
    if attrp.is_null() || unsafe { (*attrp).type_ } == sys::XML_ATTRIBUTE_DECL {
        throw(cx, NOT_FOUND_ERR, intern.strict())?;
    } else {
        set_attribute_id(attrp, boolean(args, 2), intern.document());
    }
    Ok(Value::Null)
}

pub fn set_id_attribute_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let (_, attrp) = get_obj(cx, &arg(args, 0))?;
    if unsafe { (*attrp).parent } != nodep {
        throw(cx, NOT_FOUND_ERR, intern.strict())?;
    } else {
        set_attribute_id(attrp.cast(), boolean(args, 1), intern.document());
    }
    Ok(Value::Null)
}

pub fn toggle_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let mut qname = bytes(args, 0);
    let force = super::node::opt_bool(args, 1);
    let (intern, thisp) = get_obj(cx, this)?;
    if !valid_name(&qname) {
        throw(cx, INVALID_CHARACTER_ERR, true)?;
    }
    unsafe {
        let doc = (*thisp).doc;
        if !doc.is_null() && (*doc).type_ == sys::XML_HTML_DOCUMENT_NODE && (*thisp).ns.is_null() {
            qname = qname.to_ascii_lowercase();
        }
    }
    let attribute = get_attribute_or_nsdecl(thisp, &qname);
    if attribute.is_null() {
        if force != Some(false) {
            match qname.iter().position(|&b| b == b':') {
                Some(colon) if colon > 0 && &qname[..colon] == b"xmlns" => {
                    let cp = c(&qname[colon + 1..]);
                    unsafe { sys::xmlNewNs(thisp, c"".as_ptr().cast(), cp.as_ptr()) };
                }
                _ => {
                    create_attribute(thisp, &qname, b"");
                }
            }
            return Ok(Value::Bool(true));
        }
        return Ok(Value::Bool(false));
    }
    if force != Some(true) {
        return Ok(Value::Bool(!remove_attribute_node_impl(
            thisp,
            attribute,
            intern.document(),
        )));
    }
    Ok(Value::Bool(true))
}

/// What dom_insert_adjacent did with the node.
enum Adjacent {
    /// No parent to insert next to.
    Null,
    Done,
    /// A bad position or a failed adoption: the node is untouched.
    NotTaken,
    /// The insertion was refused: pre_insert freed a temporary node.
    Refused,
}

fn insert_adjacent(
    cx: &mut Cx,
    where_: &[u8],
    thisp: *mut XmlNode,
    this_intern: &DomObject,
    otherp: *mut XmlNode,
) -> (Adjacent, Option<Thrown>) {
    let w = where_.to_ascii_lowercase();
    unsafe {
        let doc = (*thisp).doc;
        let (parent, before) = match w.as_slice() {
            b"beforebegin" => {
                if (*thisp).parent.is_null() {
                    return (Adjacent::Null, None);
                }
                ((*thisp).parent, thisp)
            }
            b"afterbegin" => (thisp, (*thisp).children),
            b"beforeend" => (thisp, ptr::null_mut()),
            b"afterend" => {
                if (*thisp).parent.is_null() {
                    return (Adjacent::Null, None);
                }
                ((*thisp).parent, (*thisp).next)
            }
            _ => {
                let e = throw(cx, SYNTAX_ERR, this_intern.strict()).err();
                return (Adjacent::NotTaken, e);
            }
        };
        if !super::document::adopt_node_impl(otherp, this_intern, doc) {
            return (Adjacent::NotTaken, None);
        }
        match pre_insert(cx, this_intern.document(), otherp, parent, before) {
            Ok(true) => (Adjacent::Done, None),
            Ok(false) => (Adjacent::Refused, None),
            Err(e) => (Adjacent::Refused, Some(e)),
        }
    }
}

pub fn insert_adjacent_element(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let where_ = bytes(args, 0);
    let elem = arg(args, 1);
    let (this_intern, thisp) = get_obj(cx, this)?;
    let (other_intern, otherp) = get_obj(cx, &elem)?;
    match insert_adjacent(cx, &where_, thisp, &this_intern, otherp) {
        (_, Some(e)) => Err(e),
        (Adjacent::Done, None) => create_object(cx, otherp, Some(&other_intern)),
        _ => Ok(Value::Null),
    }
}

pub fn insert_adjacent_text(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let where_ = bytes(args, 0);
    let data = bytes(args, 1);
    let (this_intern, thisp) = get_obj(cx, this)?;
    if data.len() > i32::MAX as usize {
        return Err(crate::zend::zpp::value_error(
            &cx.fname,
            2,
            "data",
            "is too long",
        ));
    }
    let otherp =
        unsafe { sys::xmlNewDocTextLen((*thisp).doc, nul(&data).as_ptr(), data.len() as c_int) };
    let (r, e) = insert_adjacent(cx, &where_, thisp, &this_intern, otherp);
    if matches!(r, Adjacent::Null | Adjacent::NotTaken) {
        unsafe { sys::xmlFreeNode(otherp) };
    }
    match e {
        Some(e) => Err(e),
        None => Ok(Value::Null),
    }
}

// ---- ParentNode / ChildNode (parentnode/tree.c) ----

/// dom_add_child_without_merging
fn add_child_without_merging(parent: *mut XmlNode, child: *mut XmlNode) {
    unsafe {
        if (*parent).children.is_null() {
            (*parent).children = child;
        } else {
            let last = (*parent).last;
            (*last).next = child;
            (*child).prev = last;
        }
        (*parent).last = child;
        (*child).parent = parent;
    }
}

fn fragment_assign_parent(parent: *mut XmlNode, fragment: *mut XmlNode) {
    unsafe {
        let mut n = (*fragment).children;
        while !n.is_null() {
            (*n).parent = parent;
            if n == (*fragment).last {
                break;
            }
            n = (*n).next;
        }
    }
}

fn is_parent_invalid(parent: *mut XmlNode) -> bool {
    !matches!(
        unsafe { (*parent).type_ },
        sys::XML_DOCUMENT_NODE
            | sys::XML_HTML_DOCUMENT_NODE
            | sys::XML_ELEMENT_NODE
            | sys::XML_DOCUMENT_FRAG_NODE
    )
}

/// dom_is_pre_insert_valid_without_step_1 (legacy documents)
fn pre_insert_valid(
    cx: &mut Cx,
    document: *mut lxn::RefObj,
    parent: *mut XmlNode,
    node: *mut XmlNode,
    child: *mut XmlNode,
    document_node: *mut XmlDoc,
) -> Result<bool, Thrown> {
    let strict = lxn::doc_props_read_only(document).stricterror;
    unsafe {
        if is_parent_invalid(parent) {
            throw(cx, HIERARCHY_REQUEST_ERR, strict)?;
            return Ok(false);
        }
        if (*node).doc != document_node {
            throw(cx, WRONG_DOCUMENT_ERR, strict)?;
            return Ok(false);
        }
        if !child.is_null() && (*child).parent != parent {
            throw(cx, NOT_FOUND_ERR, strict)?;
            return Ok(false);
        }
        if !hierarchy_ok(parent, node) || (*node).type_ == sys::XML_ATTRIBUTE_NODE {
            throw(cx, HIERARCHY_REQUEST_ERR, strict)?;
            return Ok(false);
        }
    }
    Ok(true)
}

/// dom_free_node_after_zval_single_node_creation
fn free_temp_fragment_children(node: *mut XmlNode) {
    unsafe {
        let mut ch = (*node).children;
        while !ch.is_null() {
            let next = (*ch).next;
            sys::xmlUnlinkNode(ch);
            if (*ch)._private.is_null() {
                sys::xmlFreeNode(ch);
            }
            ch = next;
        }
    }
}

/// The arguments of append() & co: nodes (objects) and strings.
enum Item {
    Node(*mut XmlNode),
    Text(Vec<u8>),
}

/// dom_sanity_check_node_list_types
fn check_node_list(cx: &mut Cx, nodes: &[Value]) -> Result<Vec<Item>, Thrown> {
    let mut out = Vec::new();
    for (i, v) in nodes.iter().enumerate() {
        let v = super::deref(v);
        match &v {
            Value::Object(_) if is_dom_node(cx, &v) => {
                out.push(Item::Node(
                    super::intern_of(&v).map_or(ptr::null_mut(), |d| d.node()),
                ));
            }
            Value::Str(s) => {
                if s.len() > i32::MAX as usize {
                    return Err(crate::zend::zpp::value_error(
                        &cx.fname,
                        i + 1,
                        "nodes",
                        &format!("must be less than or equal to {} bytes long", i32::MAX),
                    ));
                }
                out.push(Item::Text(s.as_bytes().to_vec()));
            }
            _ => {
                return Err(Thrown::type_error(format!(
                    "{}(): Argument #{} must be of type DOMNode|string, {} given",
                    cx.fname,
                    i + 1,
                    crate::zend::zpp::zval_type_name(cx.host, &v)
                )));
            }
        }
    }
    Ok(out)
}

/// `instanceof DOMNode`
fn is_dom_node(cx: &Cx, v: &Value) -> bool {
    cx.host
        .object_class_name(v)
        .is_some_and(|c| cx.host.reflect_is_a(&c, b"DOMNode"))
}

fn is_node_in_list(items: &[Item], n: *const XmlNode) -> bool {
    items
        .iter()
        .any(|i| matches!(i, Item::Node(p) if std::ptr::eq(*p, n)))
}

/// dom_zvals_to_single_node: one node for the arguments (a fragment for
/// several).
fn zvals_to_single_node(
    cx: &mut Cx,
    document: *mut lxn::RefObj,
    context: *mut XmlNode,
    items: &[Item],
) -> Result<*mut XmlNode, Thrown> {
    unsafe {
        let t = (*context).type_;
        let document_node: *mut XmlDoc =
            if t == sys::XML_DOCUMENT_NODE || t == sys::XML_HTML_DOCUMENT_NODE {
                context.cast()
            } else {
                (*context).doc
            };
        let cannot = |cx: &mut Cx| {
            throw_msg(
                cx,
                INVALID_MODIFICATION_ERR,
                "Unable to allocate temporary nodes",
                true,
            )
        };
        if items.len() == 1 {
            return match &items[0] {
                Item::Node(n) => Ok(*n),
                Item::Text(s) => {
                    let n = sys::xmlNewDocTextLen(document_node, nul(s).as_ptr(), s.len() as c_int);
                    if n.is_null() {
                        cannot(cx)?;
                    }
                    Ok(n)
                }
            };
        }
        let node = sys::xmlNewDocFragment(document_node);
        if node.is_null() {
            cannot(cx)?;
            return Ok(ptr::null_mut());
        }
        for item in items {
            match item {
                Item::Node(new_node) => {
                    let mut new_node = *new_node;
                    if new_node.is_null() {
                        free_temp_fragment_children(node);
                        sys::xml_free(node);
                        throw(cx, INVALID_STATE_ERR, true)?;
                        return Ok(ptr::null_mut());
                    }
                    let ok = pre_insert_valid(
                        cx,
                        document,
                        node,
                        new_node,
                        ptr::null_mut(),
                        document_node,
                    );
                    match ok {
                        Ok(true) => {}
                        other => {
                            free_temp_fragment_children(node);
                            sys::xml_free(node);
                            other?;
                            return Ok(ptr::null_mut());
                        }
                    }
                    if !(*new_node).parent.is_null() {
                        sys::xmlUnlinkNode(new_node);
                    }
                    if (*new_node).type_ == sys::XML_DOCUMENT_FRAG_NODE {
                        new_node = (*new_node).children;
                        while !new_node.is_null() {
                            let next = (*new_node).next;
                            sys::xmlUnlinkNode(new_node);
                            add_child_without_merging(node, new_node);
                            new_node = next;
                        }
                    } else {
                        add_child_without_merging(node, new_node);
                    }
                }
                Item::Text(s) => {
                    let n = sys::xmlNewDocTextLen(document_node, nul(s).as_ptr(), s.len() as c_int);
                    if n.is_null() {
                        free_temp_fragment_children(node);
                        sys::xml_free(node);
                        cannot(cx)?;
                        return Ok(ptr::null_mut());
                    }
                    add_child_without_merging(node, n);
                }
            }
        }
        Ok(node)
    }
}

/// php_dom_pre_insert_helper
fn pre_insert_helper(
    insertion_point: *mut XmlNode,
    parent: *mut XmlNode,
    newchild: *mut XmlNode,
    last: *mut XmlNode,
) {
    unsafe {
        if insertion_point.is_null() {
            if !(*parent).children.is_null() {
                (*newchild).prev = (*parent).last;
                (*(*parent).last).next = newchild;
            } else {
                (*parent).children = newchild;
            }
            (*parent).last = last;
        } else {
            (*last).next = insertion_point;
            if !(*insertion_point).prev.is_null() {
                (*(*insertion_point).prev).next = newchild;
                (*newchild).prev = (*insertion_point).prev;
            }
            (*insertion_point).prev = last;
            if (*parent).children == insertion_point {
                (*parent).children = newchild;
            }
        }
    }
}

/// dom_insert_node_list_cleanup
fn insert_cleanup(node: *mut XmlNode) {
    unsafe {
        if !(*node)._private.is_null() {
            return;
        }
        if (*node).type_ == sys::XML_DOCUMENT_FRAG_NODE {
            free_temp_fragment_children(node);
            sys::xml_free(node);
        } else if (*node).type_ == sys::XML_TEXT_NODE {
            sys::xmlFreeNode(node);
        }
    }
}

/// dom_insert_node_list_unchecked
fn insert_unchecked(node: *mut XmlNode, parent: *mut XmlNode, mut insertion_point: *mut XmlNode) {
    unsafe {
        if (*node).type_ == sys::XML_DOCUMENT_FRAG_NODE {
            let newchild = (*node).children;
            if !newchild.is_null() {
                let last = (*node).last;
                pre_insert_helper(insertion_point, parent, newchild, last);
                fragment_assign_parent(parent, node);
                reconcile_ns_list((*parent).doc, newchild, last);
                if !(*parent).doc.is_null() && (*newchild).type_ == sys::XML_DTD_NODE {
                    (*(*parent).doc).intSubset = newchild.cast();
                    (*newchild).parent = (*parent).doc.cast();
                }
            }
            if (*node)._private.is_null() {
                sys::xml_free(node);
            } else {
                (*node).children = ptr::null_mut();
                (*node).last = ptr::null_mut();
            }
        } else {
            if insertion_point == node {
                insertion_point = (*node).next;
            }
            sys::xmlUnlinkNode(node);
            pre_insert_helper(insertion_point, parent, node, node);
            (*node).parent = parent;
            if !(*parent).doc.is_null() && (*node).type_ == sys::XML_DTD_NODE {
                (*(*parent).doc).intSubset = node.cast();
                (*node).parent = (*parent).doc.cast();
            } else {
                reconcile_ns((*parent).doc, node);
            }
        }
    }
}

/// php_dom_pre_insert
pub fn pre_insert(
    cx: &mut Cx,
    document: *mut lxn::RefObj,
    node: *mut XmlNode,
    parent: *mut XmlNode,
    insertion_point: *mut XmlNode,
) -> Result<bool, Thrown> {
    if node.is_null() {
        return Ok(false);
    }
    let doc = unsafe { (*parent).doc };
    match pre_insert_valid(cx, document, parent, node, insertion_point, doc) {
        Ok(true) => {
            insert_unchecked(node, parent, insertion_point);
            Ok(true)
        }
        other => {
            insert_cleanup(node);
            other
        }
    }
}

fn context(cx: &mut Cx, this: &Value) -> Result<std::rc::Rc<DomObject>, Thrown> {
    get_intern(cx, this)
}

pub fn append(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let intern = context(cx, this)?;
    let items = check_node_list(cx, args)?;
    let parent = intern.node();
    lxn::invalidate_node_list_cache(intern.document());
    let node = zvals_to_single_node(cx, intern.document(), parent, &items)?;
    if !node.is_null() {
        pre_insert(cx, intern.document(), node, parent, ptr::null_mut())?;
    }
    Ok(Value::Null)
}

pub fn prepend(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let intern = context(cx, this)?;
    let parent = intern.node();
    if unsafe { (*parent).children }.is_null() {
        return append(cx, this, args);
    }
    let items = check_node_list(cx, args)?;
    lxn::invalidate_node_list_cache(intern.document());
    let node = zvals_to_single_node(cx, intern.document(), parent, &items)?;
    if !node.is_null() {
        let first = unsafe { (*parent).children };
        pre_insert(cx, intern.document(), node, parent, first)?;
    }
    Ok(Value::Null)
}

pub fn after(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let intern = context(cx, this)?;
    let items = check_node_list(cx, args)?;
    let thisp = intern.node();
    let parent = unsafe { (*thisp).parent };
    if parent.is_null() {
        return Ok(Value::Null);
    }
    let mut viable = unsafe { (*thisp).next };
    while !viable.is_null() && is_node_in_list(&items, viable) {
        viable = unsafe { (*viable).next };
    }
    lxn::invalidate_node_list_cache(intern.document());
    let fragment = zvals_to_single_node(cx, intern.document(), parent, &items)?;
    pre_insert(cx, intern.document(), fragment, parent, viable)?;
    Ok(Value::Null)
}

pub fn before(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let intern = context(cx, this)?;
    let items = check_node_list(cx, args)?;
    let thisp = intern.node();
    let parent = unsafe { (*thisp).parent };
    if parent.is_null() {
        return Ok(Value::Null);
    }
    let mut viable = unsafe { (*thisp).prev };
    while !viable.is_null() && is_node_in_list(&items, viable) {
        viable = unsafe { (*viable).prev };
    }
    lxn::invalidate_node_list_cache(intern.document());
    let fragment = zvals_to_single_node(cx, intern.document(), parent, &items)?;
    let viable = if viable.is_null() {
        unsafe { (*parent).children }
    } else {
        unsafe { (*viable).next }
    };
    pre_insert(cx, intern.document(), fragment, parent, viable)?;
    Ok(Value::Null)
}

/// dom_child_removal_preconditions
fn removal_preconditions(cx: &mut Cx, child: *mut XmlNode, d: &DomObject) -> Result<bool, Thrown> {
    unsafe {
        if node_is_read_only(child)
            || (!(*child).parent.is_null() && node_is_read_only((*child).parent))
        {
            throw(cx, NO_MODIFICATION_ALLOWED_ERR, d.strict())?;
            return Ok(false);
        }
        if (*child).parent.is_null() {
            throw(cx, NOT_FOUND_ERR, d.strict())?;
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn remove(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let intern = context(cx, this)?;
    let child = intern.node();
    if !removal_preconditions(cx, child, &intern)? {
        return Ok(Value::Null);
    }
    lxn::invalidate_node_list_cache(intern.document());
    unsafe { sys::xmlUnlinkNode(child) };
    Ok(Value::Null)
}

pub fn replace_with(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let intern = context(cx, this)?;
    let items = check_node_list(cx, args)?;
    let child = intern.node();
    let parent = unsafe { (*child).parent };
    if parent.is_null() {
        return Ok(Value::Null);
    }
    let mut viable = unsafe { (*child).next };
    while !viable.is_null() && is_node_in_list(&items, viable) {
        viable = unsafe { (*viable).next };
    }
    if !removal_preconditions(cx, child, &intern)? {
        return Ok(Value::Null);
    }
    lxn::invalidate_node_list_cache(intern.document());
    let node = zvals_to_single_node(cx, intern.document(), parent, &items)?;
    if node.is_null() {
        return Ok(Value::Null);
    }
    let doc = unsafe { (*parent).doc };
    match pre_insert_valid(cx, intern.document(), parent, node, viable, doc) {
        Ok(true) => {
            unsafe {
                if (*child).parent != node {
                    sys::xmlUnlinkNode(child);
                }
            }
            insert_unchecked(node, parent, viable);
        }
        other => {
            insert_cleanup(node);
            other?;
        }
    }
    Ok(Value::Null)
}

pub fn replace_children(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let intern = context(cx, this)?;
    let items = check_node_list(cx, args)?;
    let thisp = intern.node();
    lxn::invalidate_node_list_cache(intern.document());
    let node = zvals_to_single_node(cx, intern.document(), thisp, &items)?;
    if node.is_null() {
        return Ok(Value::Null);
    }
    let doc = unsafe { (*thisp).doc };
    match pre_insert_valid(cx, intern.document(), thisp, node, ptr::null_mut(), doc) {
        Ok(true) => {
            remove_all_children(thisp);
            pre_insert(cx, intern.document(), node, thisp, ptr::null_mut())?;
        }
        other => {
            insert_cleanup(node);
            other?;
        }
    }
    Ok(Value::Null)
}
