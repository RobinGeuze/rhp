//! DOMNode's methods (php-src ext/dom/node.c) and the clone handlers.

use std::ffi::c_int;
use std::ptr;

use rhp_values::{conv, ArrayKey, Value};

use super::util::*;
use super::{
    arg, boolean, bytes, create_object, get_obj, intern_of, opt_bytes, throw, throw_msg, xs, Cx,
    DomObject, Kind, HIERARCHY_REQUEST_ERR, INVALID_STATE_ERR, NOT_FOUND_ERR,
    NO_MODIFICATION_ALLOWED_ERR, WRONG_DOCUMENT_ERR,
};
use crate::ext::libxml::node as lxn;
use crate::ext::libxml::sys::{self, XmlAttr, XmlNode};
use crate::Thrown;

/// dom_insert_fragment: move a fragment's children between `prevsib` and
/// `nextsib` under `nodep`; returns the first one.
pub fn insert_fragment(
    nodep: *mut XmlNode,
    prevsib: *mut XmlNode,
    nextsib: *mut XmlNode,
    fragment: *mut XmlNode,
) -> *mut XmlNode {
    unsafe {
        let newchild = (*fragment).children;
        if !newchild.is_null() {
            if prevsib.is_null() {
                (*nodep).children = newchild;
            } else {
                (*prevsib).next = newchild;
            }
            (*newchild).prev = prevsib;
            if nextsib.is_null() {
                (*nodep).last = (*fragment).last;
            } else {
                (*(*fragment).last).next = nextsib;
                (*nextsib).prev = (*fragment).last;
            }
            let mut n = newchild;
            while !n.is_null() {
                (*n).parent = nodep;
                if n == (*fragment).last {
                    break;
                }
                n = (*n).next;
            }
            (*fragment).children = ptr::null_mut();
            (*fragment).last = ptr::null_mut();
        }
        newchild
    }
}

/// dom_node_check_legacy_insertion_validity
fn check_legacy_insertion(
    cx: &mut Cx,
    parentp: *mut XmlNode,
    child: *mut XmlNode,
    strict: bool,
    warn_empty_fragment: bool,
) -> Result<bool, Thrown> {
    unsafe {
        if node_is_read_only(parentp)
            || (!(*child).parent.is_null() && node_is_read_only((*child).parent))
        {
            throw(cx, NO_MODIFICATION_ALLOWED_ERR, strict)?;
            return Ok(false);
        }
        if !hierarchy_ok(parentp, child) {
            throw(cx, HIERARCHY_REQUEST_ERR, strict)?;
            return Ok(false);
        }
        if (*child).doc != (*parentp).doc && !(*child).doc.is_null() {
            throw(cx, WRONG_DOCUMENT_ERR, strict)?;
            return Ok(false);
        }
        let ct = (*child).type_;
        if warn_empty_fragment && ct == sys::XML_DOCUMENT_FRAG_NODE && (*child).children.is_null() {
            cx.warn("Document Fragment is empty")?;
            return Ok(false);
        }
        if (*parentp).type_ == sys::XML_ATTRIBUTE_NODE
            && ct != sys::XML_TEXT_NODE
            && ct != sys::XML_ENTITY_REF_NODE
        {
            throw(cx, HIERARCHY_REQUEST_ERR, strict)?;
            return Ok(false);
        }
        if ct == sys::XML_ATTRIBUTE_NODE && (*parentp).type_ != sys::XML_ELEMENT_NODE {
            throw(cx, HIERARCHY_REQUEST_ERR, strict)?;
            return Ok(false);
        }
        if ct == sys::XML_DOCUMENT_NODE || ct == sys::XML_HTML_DOCUMENT_NODE {
            throw(cx, HIERARCHY_REQUEST_ERR, strict)?;
            return Ok(false);
        }
    }
    Ok(true)
}

/// The existing attribute an attribute node would replace on `parent`.
unsafe fn existing_attr(parent: *mut XmlNode, child: *mut XmlNode) -> *mut XmlAttr {
    unsafe {
        let ns = if (*child).ns.is_null() {
            ptr::null()
        } else {
            (*(*child).ns).href
        };
        sys::xmlHasNsProp(parent, (*child).name, ns)
    }
}

pub fn insert_before(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, parentp) = get_obj(cx, this)?;
    let (_, child) = get_obj(cx, &arg(args, 0))?;
    let refv = arg(args, 1);
    if !node_children_valid(parentp) {
        return Ok(Value::Bool(false));
    }
    let strict = intern.strict();
    if !check_legacy_insertion(cx, parentp, child, strict, true)? {
        return Ok(Value::Bool(false));
    }
    let mut refp: *mut XmlNode = ptr::null_mut();
    if !matches!(refv, Value::Null) {
        let (_, r) = get_obj(cx, &refv)?;
        refp = r;
        unsafe {
            if (*refp).parent != parentp {
                throw(cx, NOT_FOUND_ERR, strict)?;
                return Ok(Value::Bool(false));
            }
            if refp == child {
                refp = (*child).next;
            }
        }
    }
    unsafe {
        if (*child).doc.is_null() && !(*parentp).doc.is_null() {
            sys::xmlSetTreeDoc(child, (*parentp).doc);
            set_document_ref_pointers(child, intern.document());
        }
    }
    lxn::invalidate_node_list_cache(intern.document());
    let new_child;
    unsafe {
        if !refp.is_null() {
            if !(*child).parent.is_null() {
                sys::xmlUnlinkNode(child);
            }
            if (*child).type_ == sys::XML_TEXT_NODE
                && ((*refp).type_ == sys::XML_TEXT_NODE
                    || (!(*refp).prev.is_null() && (*(*refp).prev).type_ == sys::XML_TEXT_NODE))
            {
                new_child = child;
                (*new_child).parent = (*refp).parent;
                (*new_child).next = refp;
                (*new_child).prev = (*refp).prev;
                (*refp).prev = new_child;
                if !(*new_child).prev.is_null() {
                    (*(*new_child).prev).next = new_child;
                }
                if !(*new_child).parent.is_null() && (*(*new_child).parent).children == refp {
                    (*(*new_child).parent).children = new_child;
                }
            } else if (*child).type_ == sys::XML_ATTRIBUTE_NODE {
                let lastattr = existing_attr((*refp).parent, child);
                if !lastattr.is_null() && (*lastattr).type_ != sys::XML_ATTRIBUTE_DECL {
                    if lastattr != child.cast() {
                        sys::xmlUnlinkNode(lastattr.cast());
                        lxn::node_free_resource(lastattr.cast());
                    } else {
                        return create_object(cx, child, Some(&intern));
                    }
                }
                new_child = sys::xmlAddPrevSibling(refp, child);
                if new_child.is_null() {
                    return Err(Thrown::error(
                        "Error",
                        "Cannot add newnode as the previous sibling of refnode",
                    ));
                }
            } else if (*child).type_ == sys::XML_DOCUMENT_FRAG_NODE {
                let last = (*child).last;
                new_child = insert_fragment(parentp, (*refp).prev, refp, child);
                reconcile_ns_list((*parentp).doc, new_child, last);
            } else {
                new_child = sys::xmlAddPrevSibling(refp, child);
                if new_child.is_null() {
                    return Err(Thrown::error(
                        "Error",
                        "Cannot add newnode as the previous sibling of refnode",
                    ));
                }
                reconcile_ns((*parentp).doc, new_child);
            }
        } else {
            if !(*child).parent.is_null() {
                sys::xmlUnlinkNode(child);
            }
            if (*child).type_ == sys::XML_TEXT_NODE
                && !(*parentp).last.is_null()
                && (*(*parentp).last).type_ == sys::XML_TEXT_NODE
            {
                (*child).parent = parentp;
                new_child = child;
                if (*parentp).children.is_null() {
                    (*parentp).children = child;
                    (*parentp).last = child;
                } else {
                    let last = (*parentp).last;
                    (*last).next = new_child;
                    (*new_child).prev = last;
                    (*parentp).last = new_child;
                }
            } else if (*child).type_ == sys::XML_ATTRIBUTE_NODE {
                let lastattr = existing_attr(parentp, child);
                if !lastattr.is_null() && (*lastattr).type_ != sys::XML_ATTRIBUTE_DECL {
                    if lastattr != child.cast() {
                        sys::xmlUnlinkNode(lastattr.cast());
                        lxn::node_free_resource(lastattr.cast());
                    } else {
                        return create_object(cx, child, Some(&intern));
                    }
                }
                new_child = sys::xmlAddChild(parentp, child);
                if new_child.is_null() {
                    return Err(Thrown::error(
                        "Error",
                        "Cannot add newnode as the previous sibling of refnode",
                    ));
                }
            } else if (*child).type_ == sys::XML_DOCUMENT_FRAG_NODE {
                let last = (*child).last;
                new_child = insert_fragment(parentp, (*parentp).last, ptr::null_mut(), child);
                reconcile_ns_list((*parentp).doc, new_child, last);
            } else {
                new_child = sys::xmlAddChild(parentp, child);
                if new_child.is_null() {
                    return Err(Thrown::error(
                        "Error",
                        "Cannot add newnode as the previous sibling of refnode",
                    ));
                }
                reconcile_ns((*parentp).doc, new_child);
            }
        }
    }
    create_object(cx, new_child, Some(&intern))
}

pub fn replace_child(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let (_, mut newchild) = get_obj(cx, &arg(args, 0))?;
    let (_, oldchild) = get_obj(cx, &arg(args, 1))?;
    let strict = intern.strict();
    unsafe {
        if (*newchild).doc != (*nodep).doc && !(*newchild).doc.is_null() {
            throw(cx, WRONG_DOCUMENT_ERR, strict)?;
            return Ok(Value::Bool(false));
        }
        if !node_children_valid(nodep) || (*nodep).children.is_null() {
            return Ok(Value::Bool(false));
        }
        if !check_legacy_insertion(cx, nodep, newchild, strict, false)? {
            return Ok(Value::Bool(false));
        }
        if ((*oldchild).type_ == sys::XML_ATTRIBUTE_NODE)
            != ((*newchild).type_ == sys::XML_ATTRIBUTE_NODE)
        {
            throw(cx, HIERARCHY_REQUEST_ERR, strict)?;
            return Ok(Value::Bool(false));
        }
        if (*oldchild).parent != nodep {
            throw(cx, NOT_FOUND_ERR, strict)?;
            return Ok(Value::Bool(false));
        }
        if (*newchild).doc.is_null() && !(*nodep).doc.is_null() {
            sys::xmlSetTreeDoc(newchild, (*nodep).doc);
            set_document_ref_pointers(newchild, intern.document());
        }
        if (*newchild).type_ == sys::XML_DOCUMENT_FRAG_NODE {
            let prevsib = (*oldchild).prev;
            let nextsib = (*oldchild).next;
            sys::xmlUnlinkNode(oldchild);
            let last = (*newchild).last;
            newchild = insert_fragment(nodep, prevsib, nextsib, newchild);
            if !newchild.is_null() {
                reconcile_ns_list((*nodep).doc, newchild, last);
            }
        } else if oldchild != newchild {
            let int_subset = sys::xmlGetIntSubset((*nodep).doc);
            let replacedoctype = int_subset.cast::<XmlNode>() == oldchild;
            sys::xmlReplaceNode(oldchild, newchild);
            reconcile_ns((*nodep).doc, newchild);
            if replacedoctype {
                (*(*nodep).doc).intSubset = newchild.cast();
            }
        }
    }
    lxn::invalidate_node_list_cache(intern.document());
    create_object(cx, oldchild, Some(&intern))
}

pub fn remove_child(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let (_, child) = get_obj(cx, &arg(args, 0))?;
    let strict = intern.strict();
    unsafe {
        if (*nodep).children.is_null() || (*child).parent != nodep {
            throw(cx, NOT_FOUND_ERR, strict)?;
            return Ok(Value::Bool(false));
        }
        if node_is_read_only(nodep)
            || (!(*child).parent.is_null() && node_is_read_only((*child).parent))
        {
            throw(cx, NO_MODIFICATION_ALLOWED_ERR, strict)?;
            return Ok(Value::Bool(false));
        }
        sys::xmlUnlinkNode(child);
    }
    lxn::invalidate_node_list_cache(intern.document());
    create_object(cx, child, Some(&intern))
}

pub fn append_child(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let (_, mut child) = get_obj(cx, &arg(args, 0))?;
    if !node_children_valid(nodep) {
        return Ok(Value::Bool(false));
    }
    let strict = intern.strict();
    if !check_legacy_insertion(cx, nodep, child, strict, true)? {
        return Ok(Value::Bool(false));
    }
    let new_child;
    unsafe {
        if (*child).doc.is_null() && !(*nodep).doc.is_null() {
            sys::xmlSetTreeDoc(child, (*nodep).doc);
            set_document_ref_pointers(child, intern.document());
        }
        if !(*child).parent.is_null() {
            sys::xmlUnlinkNode(child);
        }
        let ct = (*child).type_;
        if ct == sys::XML_TEXT_NODE
            && !(*nodep).last.is_null()
            && (*(*nodep).last).type_ == sys::XML_TEXT_NODE
        {
            (*child).parent = nodep;
            new_child = child;
            if (*nodep).children.is_null() {
                (*nodep).children = child;
                (*nodep).last = child;
            } else {
                child = (*nodep).last;
                (*child).next = new_child;
                (*new_child).prev = child;
                (*nodep).last = new_child;
            }
        } else if ct == sys::XML_ATTRIBUTE_NODE {
            let lastattr = existing_attr(nodep, child);
            if !lastattr.is_null()
                && (*lastattr).type_ != sys::XML_ATTRIBUTE_DECL
                && lastattr != child.cast()
            {
                sys::xmlUnlinkNode(lastattr.cast());
                lxn::node_free_resource(lastattr.cast());
            }
            new_child = sys::xmlAddChild(nodep, child);
            if new_child.is_null() {
                throw(cx, INVALID_STATE_ERR, strict)?;
                return Ok(Value::Bool(false));
            }
            reconcile_attribute_namespace_after_insertion(new_child.cast());
        } else if ct == sys::XML_DOCUMENT_FRAG_NODE {
            let last = (*child).last;
            new_child = insert_fragment(nodep, (*nodep).last, ptr::null_mut(), child);
            reconcile_ns_list((*nodep).doc, new_child, last);
        } else if ct == sys::XML_DTD_NODE {
            if !(*(*nodep).doc).intSubset.is_null() {
                throw_msg(
                    cx,
                    HIERARCHY_REQUEST_ERR,
                    "A document may only contain one document type",
                    strict,
                )?;
                return Ok(Value::Bool(false));
            }
            new_child = sys::xmlAddChild(nodep, child);
            if new_child.is_null() {
                throw(cx, INVALID_STATE_ERR, strict)?;
                return Ok(Value::Bool(false));
            }
            (*(*nodep).doc).intSubset = new_child.cast();
        } else {
            new_child = sys::xmlAddChild(nodep, child);
            if new_child.is_null() {
                throw(cx, INVALID_STATE_ERR, strict)?;
                return Ok(Value::Bool(false));
            }
            reconcile_ns((*nodep).doc, new_child);
        }
    }
    lxn::invalidate_node_list_cache(intern.document());
    create_object(cx, new_child, Some(&intern))
}

pub fn has_child_nodes(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, n) = get_obj(cx, this)?;
    Ok(Value::Bool(
        node_children_valid(n) && !unsafe { (*n).children }.is_null(),
    ))
}

pub fn clone_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, n) = get_obj(cx, this)?;
    let recursive = boolean(args, 0);
    let clone_document = matches!(
        unsafe { (*n).type_ },
        sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE
    );
    let node = super::util::clone_node(n, unsafe { (*n).doc }, recursive);
    if node.is_null() {
        return Ok(Value::Bool(false));
    }
    if clone_document {
        let v = create_object(cx, node, None)?;
        if let Some(ni) = intern_of(&v) {
            update_document_after_clone(&intern, &ni);
        }
        Ok(v)
    } else {
        unsafe {
            if (*node).type_ == sys::XML_ATTRIBUTE_NODE
                && !(*n).ns.is_null()
                && (*node).ns.is_null()
            {
                (*node).ns = (*n).ns;
            }
        }
        create_object(cx, node, Some(&intern))
    }
}

/// php_dom_update_document_after_clone / dom_copy_document_ref: a cloned
/// document takes its original's settings and class map.
pub fn update_document_after_clone(original: &DomObject, clone: &DomObject) {
    let src = original.document();
    let dst = clone.document();
    if src.is_null() || dst.is_null() {
        return;
    }
    let props = lxn::doc_props_read_only(src);
    unsafe {
        if let Some(p) = lxn::doc_props_mut(dst) {
            *p = props;
        }
        (*dst).class_type = (*src).class_type;
    }
}

pub fn normalize(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, n) = get_obj(cx, this)?;
    normalize_legacy(n);
    Ok(Value::Null)
}

pub fn is_supported(_: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(has_feature(&bytes(args, 0), &bytes(args, 1))))
}

pub fn has_attributes(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, n) = get_obj(cx, this)?;
    Ok(Value::Bool(unsafe {
        (*n).type_ == sys::XML_ELEMENT_NODE && !(*n).properties.is_null()
    }))
}

pub fn is_same_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (_, n) = get_obj(cx, this)?;
    let (_, o) = get_obj(cx, &arg(args, 0))?;
    Ok(Value::Bool(n == o))
}

fn content_equal(a: *const XmlNode, b: *const XmlNode) -> bool {
    unsafe {
        let x = sys::xmlNodeGetContent(a);
        let y = sys::xmlNodeGetContent(b);
        let r = str_eq(x, y);
        sys::xml_free(x);
        sys::xml_free(y);
        r
    }
}

fn ns_href(n: *const XmlNode) -> *const u8 {
    unsafe {
        if (*n).ns.is_null() {
            ptr::null()
        } else {
            (*(*n).ns).href
        }
    }
}

fn ns_prefix(n: *const XmlNode) -> *const u8 {
    unsafe {
        if (*n).ns.is_null() {
            ptr::null()
        } else {
            (*(*n).ns).prefix
        }
    }
}

fn is_equal_attr(a: *const XmlAttr, b: *const XmlAttr) -> bool {
    unsafe {
        str_eq((*a).name, (*b).name)
            && str_eq(ns_href(a.cast()), ns_href(b.cast()))
            && content_equal(a.cast(), b.cast())
    }
}

fn list_len(mut n: *const XmlNode) -> usize {
    let mut c = 0;
    while !n.is_null() {
        c += 1;
        n = unsafe { (*n).next };
    }
    c
}

fn ordered_equal(mut a: *const XmlNode, mut b: *const XmlNode) -> bool {
    if list_len(a) != list_len(b) {
        return false;
    }
    while !a.is_null() {
        if !nodes_equal(a, b) {
            return false;
        }
        unsafe {
            a = (*a).next;
            b = (*b).next;
        }
    }
    true
}

fn unordered_equal(a: *const XmlNode, b: *const XmlNode) -> bool {
    if list_len(a) != list_len(b) {
        return false;
    }
    let mut n1 = a;
    while !n1.is_null() {
        let mut found = false;
        let mut n2 = b;
        while !n2.is_null() && !found {
            if nodes_equal(n1, n2) {
                found = true;
            }
            n2 = unsafe { (*n2).next };
        }
        if !found {
            return false;
        }
        n1 = unsafe { (*n1).next };
    }
    true
}

fn unordered_equal_ns(a: *const sys::XmlNs, b: *const sys::XmlNs) -> bool {
    let len = |mut n: *const sys::XmlNs| {
        let mut c = 0;
        while !n.is_null() {
            c += 1;
            n = unsafe { (*n).next };
        }
        c
    };
    if len(a) != len(b) {
        return false;
    }
    let mut n1 = a;
    while !n1.is_null() {
        let mut found = false;
        let mut n2 = b;
        while !n2.is_null() && !found {
            unsafe {
                if str_eq((*n1).prefix, (*n2).prefix) && str_eq((*n1).href, (*n2).href) {
                    found = true;
                }
                n2 = (*n2).next;
            }
        }
        if !found {
            return false;
        }
        n1 = unsafe { (*n1).next };
    }
    true
}

/// php_dom_node_is_equal_node (legacy: namespace declarations count).
fn nodes_equal(a: *const XmlNode, b: *const XmlNode) -> bool {
    unsafe {
        let t = (*a).type_;
        if t != (*b).type_ {
            return false;
        }
        match t {
            sys::XML_ELEMENT_NODE => {
                str_eq((*a).name, (*b).name)
                    && str_eq(ns_prefix(a), ns_prefix(b))
                    && str_eq(ns_href(a), ns_href(b))
                    && unordered_equal((*a).properties.cast(), (*b).properties.cast())
                    && unordered_equal_ns((*a).nsDef, (*b).nsDef)
                    && ordered_equal((*a).children, (*b).children)
            }
            sys::XML_DTD_NODE => {
                let (x, y) = (a as *const sys::XmlDtd, b as *const sys::XmlDtd);
                str_eq((*x).name, (*y).name)
                    && str_eq((*x).ExternalID, (*y).ExternalID)
                    && str_eq((*x).SystemID, (*y).SystemID)
            }
            sys::XML_PI_NODE => str_eq((*a).name, (*b).name) && str_eq((*a).content, (*b).content),
            sys::XML_TEXT_NODE | sys::XML_COMMENT_NODE | sys::XML_CDATA_SECTION_NODE => {
                str_eq((*a).content, (*b).content)
            }
            sys::XML_ATTRIBUTE_NODE => is_equal_attr(a.cast(), b.cast()),
            sys::XML_ENTITY_REF_NODE => str_eq((*a).name, (*b).name),
            sys::XML_ENTITY_DECL | sys::XML_NOTATION_NODE | sys::XML_ENTITY_NODE => {
                let (x, y) = (a as *const sys::XmlEntity, b as *const sys::XmlEntity);
                (*x).etype == (*y).etype
                    && str_eq((*x).name, (*y).name)
                    && str_eq((*x).ExternalID, (*y).ExternalID)
                    && str_eq((*x).SystemID, (*y).SystemID)
                    && content_equal(a, b)
            }
            sys::XML_NAMESPACE_DECL => {
                let (x, y) = (a as *const sys::XmlNs, b as *const sys::XmlNs);
                str_eq((*x).prefix, (*y).prefix) && str_eq((*x).href, (*y).href)
            }
            sys::XML_DOCUMENT_FRAG_NODE | sys::XML_HTML_DOCUMENT_NODE | sys::XML_DOCUMENT_NODE => {
                ordered_equal((*a).children, (*b).children)
            }
            _ => false,
        }
    }
}

pub fn is_equal_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let other = arg(args, 0);
    if matches!(other, Value::Null) {
        return Ok(Value::Bool(false));
    }
    let (_, otherp) = get_obj(cx, &other)?;
    let (_, nodep) = get_obj(cx, this)?;
    if nodep == otherp {
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(nodes_equal(nodep, otherp)))
}

pub fn lookup_prefix(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (_, nodep) = get_obj(cx, this)?;
    let uri = bytes(args, 0);
    if uri.is_empty() {
        return Ok(Value::Null);
    }
    unsafe {
        let lookupp = match (*nodep).type_ {
            sys::XML_ELEMENT_NODE => nodep,
            sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE => {
                sys::xmlDocGetRootElement(nodep.cast())
            }
            sys::XML_ENTITY_NODE
            | sys::XML_NOTATION_NODE
            | sys::XML_DOCUMENT_FRAG_NODE
            | sys::XML_DOCUMENT_TYPE_NODE
            | sys::XML_DTD_NODE => return Ok(Value::Null),
            _ => (*nodep).parent,
        };
        if !lookupp.is_null() {
            let cu = c(&uri);
            let ns = sys::xmlSearchNsByHref((*lookupp).doc, lookupp, cu.as_ptr());
            if !ns.is_null() && !(*ns).prefix.is_null() {
                return Ok(xs((*ns).prefix));
            }
        }
    }
    Ok(Value::Null)
}

pub fn is_default_namespace(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (_, mut nodep) = get_obj(cx, this)?;
    let uri = bytes(args, 0);
    if uri.is_empty() {
        return Ok(Value::Bool(false));
    }
    unsafe {
        let t = (*nodep).type_;
        if t == sys::XML_DOCUMENT_NODE || t == sys::XML_HTML_DOCUMENT_NODE {
            nodep = sys::xmlDocGetRootElement(nodep.cast());
            if nodep.is_null() {
                return Ok(Value::Bool(false));
            }
        }
        let ns = sys::xmlSearchNs((*nodep).doc, nodep, ptr::null());
        Ok(Value::Bool(!ns.is_null() && str_eq_b((*ns).href, &uri)))
    }
}

pub fn lookup_namespace_uri(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (_, mut nodep) = get_obj(cx, this)?;
    let prefix = opt_bytes(args, 0);
    unsafe {
        let t = (*nodep).type_;
        if t == sys::XML_DOCUMENT_NODE || t == sys::XML_HTML_DOCUMENT_NODE {
            nodep = sys::xmlDocGetRootElement(nodep.cast());
            if nodep.is_null() {
                return Ok(Value::Null);
            }
        }
        let cp = prefix.as_deref().map(c);
        let ns = sys::xmlSearchNs(
            (*nodep).doc,
            nodep,
            cp.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
        );
        if !ns.is_null() && !(*ns).href.is_null() {
            return Ok(xs((*ns).href));
        }
    }
    Ok(Value::Null)
}

unsafe extern "C" fn c14n_parent_lookup(
    user_data: *mut std::ffi::c_void,
    node: *mut XmlNode,
    parent: *mut XmlNode,
) -> c_int {
    let root = user_data as *mut XmlNode;
    if node == root {
        return 1;
    }
    let mut n = parent;
    while !n.is_null() {
        if n == root {
            return 1;
        }
        n = unsafe { (*n).parent };
    }
    0
}

/// dom_canonicalization: C14N (mode 0) and C14NFile (mode 1).
fn canonicalization(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
    mode: usize,
) -> Result<Value, Thrown> {
    let off = mode;
    let file = if mode == 1 {
        Some(bytes(args, 0))
    } else {
        None
    };
    let exclusive = boolean(args, off);
    let with_comments = boolean(args, off + 1);
    let xpath_array = arg(args, off + 2);
    let ns_prefixes = arg(args, off + 3);
    let (_, nodep) = get_obj(cx, this)?;
    let docp = unsafe { (*nodep).doc };
    if docp.is_null() {
        return Err(Thrown::error(
            "Error",
            "Node must be associated with a document",
        ));
    }
    let mut simple_lookup = false;
    let mut ctxp: *mut sys::xmlXPathContext = ptr::null_mut();
    let mut xpathobjp: *mut sys::XmlXPathObject = ptr::null_mut();
    let mut nodeset: *mut sys::XmlNodeSet = ptr::null_mut();
    match &xpath_array {
        Value::Array(ht) => {
            let Some(q) = ht.get(&ArrayKey::from_bytes(b"query")) else {
                return Err(crate::zend::zpp::value_error(
                    &cx.fname,
                    3 + mode,
                    "xpath",
                    "must have a \"query\" key",
                ));
            };
            let q = super::deref(q);
            let Value::Str(qs) = &q else {
                return Err(Thrown::type_error(format!(
                    "{}(): Argument #{} ($xpath) \"query\" option must be a string, {} given",
                    cx.fname,
                    3 + mode,
                    crate::zend::zpp::zval_value_name(cx.host, &q)
                )));
            };
            let cq = c(qs.as_bytes());
            unsafe {
                ctxp = sys::xmlXPathNewContext(docp);
                sys::rhp_xml_xpath_set_node(ctxp, nodep);
                if let Some(Value::Array(nss)) = ht
                    .get(&ArrayKey::from_bytes(b"namespaces"))
                    .map(super::deref)
                {
                    for (k, v) in nss.iter() {
                        if let (ArrayKey::Str(prefix), Value::Str(uri)) = (k, super::deref(v)) {
                            let cp = c(prefix.as_bytes());
                            let cu = c(uri.as_bytes());
                            sys::xmlXPathRegisterNs(ctxp, cp.as_ptr(), cu.as_ptr());
                        }
                    }
                }
                xpathobjp = match cx.with_libxml_owned(
                    || sys::xmlXPathEvalExpression(cq.as_ptr(), ctxp),
                    |o| {
                        if !o.is_null() {
                            sys::xmlXPathFreeObject(o)
                        }
                    },
                ) {
                    Ok(o) => o,
                    Err(t) => {
                        sys::xmlXPathFreeContext(ctxp);
                        return Err(t);
                    }
                };
                sys::rhp_xml_xpath_set_node(ctxp, ptr::null_mut());
                if !xpathobjp.is_null() && (*xpathobjp).type_ == sys::XPATH_NODESET {
                    nodeset = (*xpathobjp).nodesetval;
                } else {
                    if !xpathobjp.is_null() {
                        sys::xmlXPathFreeObject(xpathobjp);
                    }
                    sys::xmlXPathFreeContext(ctxp);
                    return Err(Thrown::error(
                        "Error",
                        "XPath query did not return a nodeset",
                    ));
                }
            }
        }
        _ => {
            let t = unsafe { (*nodep).type_ };
            if t != sys::XML_DOCUMENT_NODE && t != sys::XML_HTML_DOCUMENT_NODE {
                simple_lookup = true;
            }
        }
    }
    let mut prefixes: Vec<Vec<u8>> = Vec::new();
    let mut prefix_ptrs: Vec<*mut u8> = Vec::new();
    let mut have_prefixes = false;
    if let Value::Array(nsp) = &ns_prefixes {
        if exclusive {
            for v in nsp.values() {
                if let Value::Str(s) = super::deref(v) {
                    prefixes.push(c(s.as_bytes()));
                }
            }
            prefix_ptrs = prefixes.iter_mut().map(|p| p.as_mut_ptr()).collect();
            prefix_ptrs.push(ptr::null_mut());
            have_prefixes = true;
        } else {
            crate::notice(
                cx.host,
                &cx.fname,
                "Inclusive namespace prefixes only allowed in exclusive mode.",
            )?;
        }
    }
    let incl = if have_prefixes {
        prefix_ptrs.as_mut_ptr()
    } else {
        ptr::null_mut()
    };
    let cfile = file.as_deref().map(c);
    let r = cx.with_libxml(|| unsafe {
        let buf = match &cfile {
            Some(f) => sys::xmlOutputBufferCreateFilename(f.as_ptr().cast(), ptr::null_mut(), 0),
            None => sys::xmlAllocOutputBuffer(ptr::null_mut()),
        };
        let mut ret = -1;
        if !buf.is_null() {
            ret = if simple_lookup {
                sys::xmlC14NExecute(
                    docp,
                    Some(c14n_parent_lookup),
                    nodep.cast(),
                    exclusive as c_int,
                    incl,
                    with_comments as c_int,
                    buf,
                )
            } else {
                sys::xmlC14NDocSaveTo(
                    docp,
                    nodeset,
                    exclusive as c_int,
                    incl,
                    with_comments as c_int,
                    buf,
                )
            };
        }
        if !xpathobjp.is_null() {
            sys::xmlXPathFreeObject(xpathobjp);
        }
        if !ctxp.is_null() {
            sys::xmlXPathFreeContext(ctxp);
        }
        let mut out = Value::Bool(false);
        if !buf.is_null() && ret >= 0 && mode == 0 {
            let size = sys::xmlOutputBufferGetSize(buf);
            let p = sys::xmlOutputBufferGetContent(buf);
            out = if size > 0 && !p.is_null() {
                Value::str(std::slice::from_raw_parts(p, size))
            } else {
                Value::str("")
            };
        }
        if !buf.is_null() {
            let bytes = sys::xmlOutputBufferClose(buf);
            if mode == 1 && ret >= 0 {
                out = Value::Int(bytes as i64);
            }
        }
        out
    })?;
    Ok(r)
}

pub fn c14n(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    canonicalize(cx, this, args, 0)
}

/// dom_canonicalization's modern half: the tree must hang off a document,
/// and gets the nsDef lists C14N reads for the duration.
fn canonicalize(cx: &mut Cx, this: &Value, args: &[Value], mode: usize) -> Result<Value, Thrown> {
    let (_, nodep) = get_obj(cx, this)?;
    if !super::nscompat::follow_spec_node(nodep) {
        return canonicalization(cx, this, args, mode);
    }
    unsafe {
        let mut root = nodep;
        while !(*root).parent.is_null() {
            root = (*root).parent;
        }
        if !matches!(
            (*root).type_,
            sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE
        ) {
            throw_msg(
                cx,
                HIERARCHY_REQUEST_ERR,
                "Canonicalization can only happen on nodes attached to a document.",
                true,
            )?;
        }
        let mut links = super::nscompat::Links::default();
        let root_element = sys::xmlDocGetRootElement((*nodep).doc);
        if !root_element.is_null() {
            super::nscompat::relink_ns_decls(&mut links, root_element);
        }
        let r = canonicalization(cx, this, args, mode);
        super::nscompat::unlink_ns_decls(links);
        r
    }
}

pub fn c14n_file(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let f = bytes(args, 0);
    if f.contains(&0) {
        return Err(crate::zend::zpp::value_error(
            &cx.fname,
            1,
            "uri",
            "must not contain any null bytes",
        ));
    }
    canonicalize(cx, this, args, 1)
}

pub fn get_node_path(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, n) = get_obj(cx, this)?;
    Ok(match unsafe { sys::take_string(sys::xmlGetNodePath(n)) } {
        Some(s) => Value::str(s),
        None => Value::Null,
    })
}

pub fn get_line_no(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, n) = get_obj(cx, this)?;
    Ok(Value::Int(unsafe { sys::xmlGetLineNo(n) } as i64))
}

pub fn contains(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let other = arg(args, 0);
    if matches!(other, Value::Null) {
        return Ok(Value::Bool(false));
    }
    let (_, mut otherp) = get_obj(cx, &other)?;
    let (_, thisp) = get_obj(cx, this)?;
    while !otherp.is_null() {
        if otherp == thisp {
            return Ok(Value::Bool(true));
        }
        otherp = unsafe { (*otherp).parent };
    }
    Ok(Value::Bool(false))
}

pub fn get_root_node(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (intern, mut thisp) = get_obj(cx, this)?;
    unsafe {
        while !(*thisp).parent.is_null() {
            thisp = (*thisp).parent;
        }
    }
    create_object(cx, thisp, Some(&intern))
}

pub fn compare_document_position(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    const DISCONNECTED: i64 = 0x01;
    const PRECEDING: i64 = 0x02;
    const FOLLOWING: i64 = 0x04;
    const CONTAINS: i64 = 0x08;
    const CONTAINED_BY: i64 = 0x10;
    const IMPLEMENTATION_SPECIFIC: i64 = 0x20;
    let (this_intern, thisp) = get_obj(cx, this)?;
    let other = arg(args, 0);
    let (other_intern, otherp) = get_obj(cx, &other)?;
    if thisp == otherp {
        return Ok(Value::Int(0));
    }
    let mut node1 = otherp;
    let mut node2 = thisp;
    let mut attr1: *mut XmlNode = ptr::null_mut();
    let mut attr2: *mut XmlNode = ptr::null_mut();
    unsafe {
        if (*node1).type_ == sys::XML_ATTRIBUTE_NODE {
            attr1 = node1;
            node1 = (*attr1).parent;
        }
        if (*node2).type_ == sys::XML_ATTRIBUTE_NODE {
            attr2 = node2;
            node2 = (*attr2).parent;
            if !attr1.is_null() && !node1.is_null() && node2 == node1 {
                let mut a = (*node2).properties;
                while !a.is_null() {
                    if is_equal_attr(a, attr1.cast()) {
                        return Ok(Value::Int(IMPLEMENTATION_SPECIFIC | PRECEDING));
                    } else if is_equal_attr(a, attr2.cast()) {
                        return Ok(Value::Int(IMPLEMENTATION_SPECIFIC | FOLLOWING));
                    }
                    a = (*a).next;
                }
            }
        }
        let disconnected = |node1: *mut XmlNode, node2: *mut XmlNode| {
            let ordering = if node1 == node2 {
                let a = std::rc::Rc::as_ptr(&other_intern) as usize;
                let b = std::rc::Rc::as_ptr(&this_intern) as usize;
                if a < b {
                    PRECEDING
                } else {
                    FOLLOWING
                }
            } else if (node1 as usize) < (node2 as usize) {
                PRECEDING
            } else {
                FOLLOWING
            };
            Ok(Value::Int(
                DISCONNECTED | IMPLEMENTATION_SPECIFIC | ordering,
            ))
        };
        if node1.is_null() || node2.is_null() {
            return disconnected(node1, node2);
        }
        let mut node2_is_ancestor_of_node1 = false;
        let mut node1_depth = 0usize;
        let mut root1 = node1;
        while !(*root1).parent.is_null() {
            root1 = (*root1).parent;
            if root1 == node2 {
                node2_is_ancestor_of_node1 = true;
            }
            node1_depth += 1;
        }
        let mut node1_is_ancestor_of_node2 = false;
        let mut node2_depth = 0usize;
        let mut root2 = node2;
        while !(*root2).parent.is_null() {
            root2 = (*root2).parent;
            if root2 == node1 {
                node1_is_ancestor_of_node2 = true;
            }
            node2_depth += 1;
        }
        if root1 != root2 {
            return disconnected(node1, node2);
        }
        if (node1_is_ancestor_of_node2 && attr1.is_null()) || (node1 == node2 && !attr2.is_null()) {
            return Ok(Value::Int(CONTAINS | PRECEDING));
        }
        if (node2_is_ancestor_of_node1 && attr2.is_null()) || (node1 == node2 && !attr1.is_null()) {
            return Ok(Value::Int(CONTAINED_BY | FOLLOWING));
        }
        if node1_is_ancestor_of_node2 {
            return Ok(Value::Int(PRECEDING));
        } else if node2_is_ancestor_of_node1 {
            return Ok(Value::Int(FOLLOWING));
        }
        while node1_depth > node2_depth {
            node1 = (*node1).parent;
            node1_depth -= 1;
        }
        while node2_depth > node1_depth {
            node2 = (*node2).parent;
            node2_depth -= 1;
        }
        while (*node1).parent != (*node2).parent {
            node1 = (*node1).parent;
            node2 = (*node2).parent;
        }
        loop {
            node1 = (*node1).next;
            if node1 == node2 {
                return Ok(Value::Int(PRECEDING));
            }
            if node1.is_null() {
                break;
            }
        }
    }
    Ok(Value::Int(FOLLOWING))
}

pub fn sleep(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let class = super::class_of(cx.host, this);
    Err(exception(
        cx,
        &format!("Serialization of '{class}' is not allowed, unless serialization methods are implemented in a subclass"),
    ))
}

pub fn wakeup(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let class = super::class_of(cx.host, this);
    Err(exception(
        cx,
        &format!("Unserialization of '{class}' is not allowed, unless unserialization methods are implemented in a subclass"),
    ))
}

/// zend_throw_exception_ex(NULL, ...): a plain Exception.
fn exception(cx: &mut Cx, msg: &str) -> Thrown {
    match cx
        .host
        .reflect_new_instance(b"Exception", vec![Value::str(msg)], true)
    {
        Ok(Value::Object(h)) => Thrown::Object(h),
        Ok(_) => Thrown::error("Exception", msg),
        Err(t) => t,
    }
}

/// dom_objects_store_clone_obj for a node: the copy wraps a deep clone.
pub fn clone_object(cx: &mut Cx, src: &Value, dst: &Value) -> Result<(), Thrown> {
    let Some(intern) = intern_of(src) else {
        return Ok(());
    };
    let clone = super::ensure_intern(dst, || Kind::Node).expect("a clone");
    let node = intern.node();
    if node.is_null() {
        return Ok(());
    }
    let _ = cx;
    if super::modern::is_modern(&intern) {
        let t = unsafe { (*node).type_ };
        let is_doc = t == sys::XML_DOCUMENT_NODE || t == sys::XML_HTML_DOCUMENT_NODE;
        let private = if is_doc {
            super::nscompat::DomPrivate::create()
        } else {
            unsafe { (*intern.document()).private_data }
        };
        let m = unsafe { &mut (*(private as *mut super::nscompat::DomPrivate)).ns_mapper };
        let cloned = super::modern::clone_node_modern(Some(m), node, unsafe { (*node).doc }, true);
        if !cloned.is_null() {
            update_refcount_after_clone(&intern, node, &clone, cloned);
        }
        if is_doc {
            unsafe {
                let d = clone.document();
                if d.is_null() || !(*d).private_data.is_null() {
                    super::nscompat::destroy(private);
                } else {
                    (*d).private_data = private;
                    (*d).quirks_mode = (*intern.document()).quirks_mode;
                }
            }
        }
        return Ok(());
    }
    let cloned = super::util::clone_node(node, unsafe { (*node).doc }, true);
    if !cloned.is_null() {
        update_refcount_after_clone(&intern, node, &clone, cloned);
    }
    Ok(())
}

/// dom_update_refcount_after_clone
fn update_refcount_after_clone(
    original: &DomObject,
    original_node: *mut XmlNode,
    clone: &DomObject,
    cloned_node: *mut XmlNode,
) {
    unsafe {
        if (*cloned_node).doc == (*original_node).doc {
            clone.base.document.set(original.document());
        }
        lxn::increment_doc_ref(&clone.base, (*cloned_node).doc);
        lxn::increment_node_ptr(&clone.base, cloned_node, &clone.base);
    }
    if original.document() != clone.document() {
        update_document_after_clone(original, clone);
    }
}

/// dom_object_namespace_node_clone_obj
pub fn clone_ns_object(cx: &mut Cx, src: &Value, dst: &Value) -> Result<(), Thrown> {
    let Some(intern) = intern_of(src) else {
        return Ok(());
    };
    let parent = match &intern.kind {
        Kind::NsNode(p) => p.borrow().clone(),
        _ => None,
    };
    let clone =
        super::ensure_intern(dst, || Kind::NsNode(std::cell::RefCell::new(None))).expect("a clone");
    if let Kind::NsNode(p) = &clone.kind {
        *p.borrow_mut() = parent.clone();
    }
    let original = intern.node();
    if !original.is_null() {
        let parent_node = parent
            .as_ref()
            .and_then(intern_of)
            .map_or(ptr::null_mut(), |p| p.node());
        let cloned =
            unsafe { fake_namespace_decl_node(parent_node, (*original).ns, (*original).doc) };
        update_refcount_after_clone(&intern, original, &clone, cloned);
    }
    let _ = cx;
    Ok(())
}

/// php_dom_create_fake_namespace_decl: a DOMNameSpaceNode for `original`,
/// declared on `nodep`, keeping the parent's object alive.
pub fn create_fake_namespace_decl(
    cx: &mut Cx,
    nodep: *mut XmlNode,
    original: *mut sys::XmlNs,
    parent_intern: &DomObject,
) -> Result<Value, Thrown> {
    let attrp = fake_namespace_decl_node(nodep, original, ptr::null_mut());
    let v = create_object(cx, attrp, Some(parent_intern))?;
    if let Some(d) = intern_of(&v) {
        if let Kind::NsNode(p) = &d.kind {
            *p.borrow_mut() = parent_intern.base.object();
        }
    }
    Ok(v)
}

/// `conv::to_bool` of an optional argument.
pub fn opt_bool(args: &[Value], i: usize) -> Option<bool> {
    match arg(args, i) {
        Value::Null => None,
        v => Some(conv::to_bool(&v)),
    }
}
