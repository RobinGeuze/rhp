//! Tree and namespace helpers shared by the DOM classes: ports of
//! php-src ext/dom/php_dom.c and the legacy parts of namespace_compat.c.

use std::ffi::c_int;
use std::ptr;

use rhp_values::Value;

use crate::Thrown;

use crate::ext::libxml::node::{self as lxn, RefObj};
use crate::ext::libxml::sys::{self, XmlAttr, XmlDoc, XmlNode, XmlNs};

pub const XML_NS_URI: &[u8] = b"http://www.w3.org/XML/1998/namespace";
pub const XMLNS_NS_URI: &[u8] = b"http://www.w3.org/2000/xmlns/";

/// A NUL-terminated copy for C.
pub fn c(s: &[u8]) -> Vec<u8> {
    sys::cbytes(s)
}

/// `s` with a NUL after it, NUL bytes inside kept: for calls that also
/// take the length (an empty slice's pointer is not one C may read).
pub fn nul(s: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s);
    v.push(0);
    v
}

/// The bytes of a C string.
///
/// # Safety
/// `p` is NULL or NUL-terminated and outlives the slice.
pub unsafe fn b<'a>(p: *const u8) -> &'a [u8] {
    unsafe { sys::cstr(p) }
}

/// `xmlStrEqual` (NULL equals only NULL).
pub fn str_eq(a: *const u8, b: *const u8) -> bool {
    unsafe { sys::xmlStrEqual(a, b) != 0 }
}

/// `xmlStrEqual` against Rust bytes.
pub fn str_eq_b(a: *const u8, bytes: &[u8]) -> bool {
    !a.is_null() && unsafe { sys::cstr(a) } == bytes
}

/// dom_node_is_read_only
pub fn node_is_read_only(node: *const XmlNode) -> bool {
    unsafe {
        match (*node).type_ {
            sys::XML_ENTITY_REF_NODE
            | sys::XML_ENTITY_NODE
            | sys::XML_DOCUMENT_TYPE_NODE
            | sys::XML_NOTATION_NODE
            | sys::XML_DTD_NODE
            | sys::XML_ELEMENT_DECL
            | sys::XML_ATTRIBUTE_DECL
            | sys::XML_ENTITY_DECL
            | sys::XML_NAMESPACE_DECL => true,
            _ => (*node).doc.is_null(),
        }
    }
}

/// dom_node_children_valid
pub fn node_children_valid(node: *const XmlNode) -> bool {
    !matches!(
        unsafe { (*node).type_ },
        sys::XML_DOCUMENT_TYPE_NODE
            | sys::XML_DTD_NODE
            | sys::XML_PI_NODE
            | sys::XML_COMMENT_NODE
            | sys::XML_TEXT_NODE
            | sys::XML_CDATA_SECTION_NODE
            | sys::XML_NOTATION_NODE
    )
}

/// dom_hierarchy: false when `child` may not go under `parent`.
pub fn hierarchy_ok(parent: *mut XmlNode, child: *mut XmlNode) -> bool {
    unsafe {
        if parent.is_null() || child.is_null() || (*child).doc != (*parent).doc {
            return true;
        }
        if (*child).type_ == sys::XML_DOCUMENT_NODE {
            return false;
        }
        let mut n = parent;
        while !n.is_null() {
            if n == child {
                return false;
            }
            n = (*n).parent;
        }
    }
    true
}

/// dom_has_feature
pub fn has_feature(feature: &[u8], version: &[u8]) -> bool {
    matches!(version, b"1.0" | b"2.0" | b"")
        && (feature.eq_ignore_ascii_case(b"XML")
            || (feature.eq_ignore_ascii_case(b"Core") && version == b"1.0"))
}

/// php_dom_next_in_tree_order
pub fn next_in_tree_order(nodep: *const XmlNode, basep: *const XmlNode) -> *mut XmlNode {
    unsafe {
        if (*nodep).type_ == sys::XML_ELEMENT_NODE && !(*nodep).children.is_null() {
            return (*nodep).children;
        }
        if !(*nodep).next.is_null() {
            return (*nodep).next;
        }
        let mut n = nodep;
        loop {
            n = (*n).parent;
            if n == basep || n.is_null() {
                return ptr::null_mut();
            }
            if !(*n).next.is_null() {
                return (*n).next;
            }
        }
    }
}

/// php_dom_first_child_of_container_node
pub fn first_child_of_container(parent: *mut XmlNode) -> *mut XmlNode {
    unsafe {
        let t = (*parent).type_;
        if t == sys::XML_DOCUMENT_NODE || t == sys::XML_HTML_DOCUMENT_NODE {
            sys::xmlDocGetRootElement(parent.cast())
        } else {
            (*parent).children
        }
    }
}

/// dom_get_elements_by_tag_name_ns_raw: the `index`th element at or after
/// `nodep` (counting in `cur`) whose names match.
pub fn elements_by_tag_name_ns_raw(
    basep: *mut XmlNode,
    mut nodep: *mut XmlNode,
    ns: Option<&[u8]>,
    local: &[u8],
    cur: &mut i64,
    index: i64,
) -> *mut XmlNode {
    if nodep.is_null() {
        return ptr::null_mut();
    }
    let local_any = local == b"*";
    let ns_any = ns.is_none_or(|n| n == b"*");
    let match_qname = ns.is_none() && super::nscompat::follow_spec_node(basep);
    let local_lower = if match_qname {
        local.to_ascii_lowercase()
    } else {
        Vec::new()
    };
    // dom_match_qualified_name_for_tag_name_equality
    let name_matches = |n: *mut XmlNode| unsafe {
        if !match_qname {
            return b((*n).name) == local;
        }
        let l = if (*(*n).doc).type_ == sys::XML_HTML_DOCUMENT_NODE
            && super::nscompat::ns_is_fast(n, &super::nscompat::HTML)
        {
            &local_lower[..]
        } else {
            local
        };
        super::modern::match_qualified_name(l, n)
    };
    unsafe {
        while *cur <= index {
            if (*nodep).type_ == sys::XML_ELEMENT_NODE && (local_any || name_matches(nodep)) {
                let nsp = (*nodep).ns;
                let matches = ns_any
                    || (ns == Some(b"") && nsp.is_null())
                    || (!nsp.is_null() && Some(b((*nsp).href)) == ns);
                if matches {
                    if *cur == index {
                        return nodep;
                    }
                    *cur += 1;
                }
            }
            nodep = next_in_tree_order(nodep, basep);
            if nodep.is_null() {
                return ptr::null_mut();
            }
        }
    }
    ptr::null_mut()
}

/// php_dom_get_content_into_zval: the node's text (null or "" when there
/// is none, as `null_on_failure` says).
pub fn get_content(nodep: *const XmlNode, null_on_failure: bool) -> rhp_values::Value {
    use rhp_values::Value;
    unsafe {
        match (*nodep).type_ {
            sys::XML_TEXT_NODE
            | sys::XML_CDATA_SECTION_NODE
            | sys::XML_PI_NODE
            | sys::XML_COMMENT_NODE => {
                if !(*nodep).content.is_null() {
                    return Value::str(b((*nodep).content));
                }
            }
            sys::XML_ATTRIBUTE_NODE => {
                return Value::str(lxn::attr_value(nodep.cast()));
            }
            _ => {
                if let Some(s) = sys::take_string(sys::xmlNodeGetContent(nodep)) {
                    return Value::str(s);
                }
            }
        }
    }
    if null_on_failure {
        Value::Null
    } else {
        Value::str("")
    }
}

/// node_list_unlink: unlink the nodes an object wraps out of a list that
/// is about to be freed.
pub fn node_list_unlink(mut node: *mut XmlNode) {
    unsafe {
        while !node.is_null() {
            let next = (*node).next;
            if lxn::object_of_node(node).is_some() {
                sys::xmlUnlinkNode(node);
            } else if (*node).type_ != sys::XML_ENTITY_REF_NODE {
                node_list_unlink((*node).children);
                match (*node).type_ {
                    sys::XML_ATTRIBUTE_DECL
                    | sys::XML_DTD_NODE
                    | sys::XML_DOCUMENT_TYPE_NODE
                    | sys::XML_ENTITY_DECL
                    | sys::XML_ATTRIBUTE_NODE
                    | sys::XML_TEXT_NODE => {}
                    _ => node_list_unlink((*node).properties.cast()),
                }
            }
            node = next;
        }
    }
}

/// dom_remove_all_children
pub fn remove_all_children(nodep: *mut XmlNode) {
    unsafe {
        if !(*nodep).children.is_null() {
            node_list_unlink((*nodep).children);
            lxn::node_free_list((*nodep).children);
            (*nodep).children = ptr::null_mut();
            (*nodep).last = ptr::null_mut();
        }
    }
}

fn is_empty_node(n: *const XmlNode) -> bool {
    unsafe { (*n).content.is_null() || *(*n).content == 0 }
}

/// free_node in php_dom.c: free a node no object wraps.
fn free_unwrapped(node: *mut XmlNode) {
    unsafe {
        if (*node)._private.is_null() {
            sys::xmlFreeNode(node);
        }
    }
}

fn merge_adjacent_text_nodes(node: *mut XmlNode) {
    unsafe {
        let mut nextp = (*node).next;
        while !nextp.is_null() && (*nextp).type_ == sys::XML_TEXT_NODE {
            let newnext = (*nextp).next;
            if !(*nextp).content.is_null() {
                sys::xmlNodeAddContent(node, (*nextp).content);
            }
            sys::xmlUnlinkNode(nextp);
            free_unwrapped(nextp);
            nextp = newnext;
        }
    }
}

/// php_dom_normalize_legacy, with dom_normalize_check_stack_limit's
/// Error once the stack limit is passed.
pub fn normalize_legacy(nodep: *mut XmlNode) -> Result<Value, Thrown> {
    let mut deep = false;
    normalize_legacy_rec(nodep, &mut deep);
    if deep {
        return Err(Thrown::error(
            "Error",
            "Maximum call stack size reached. Infinite recursion?",
        ));
    }
    Ok(Value::Null)
}

/// One level of php_dom_normalize_legacy: past the stack limit it returns
/// at once and sets `deep` (the levels above carry on, as in php).
fn normalize_legacy_rec(nodep: *mut XmlNode, deep: &mut bool) {
    if crate::zend::call_stack::overflowed() {
        *deep = true;
        return;
    }
    unsafe {
        let mut child = (*nodep).children;
        while !child.is_null() {
            match (*child).type_ {
                sys::XML_TEXT_NODE => {
                    merge_adjacent_text_nodes(child);
                    if is_empty_node(child) {
                        let next = (*child).next;
                        sys::xmlUnlinkNode(child);
                        free_unwrapped(child);
                        child = next;
                        continue;
                    }
                }
                sys::XML_ELEMENT_NODE => {
                    normalize_legacy_rec(child, deep);
                    let mut attr = (*child).properties;
                    while !attr.is_null() {
                        normalize_legacy_rec(attr.cast(), deep);
                        attr = (*attr).next;
                    }
                }
                _ => {}
            }
            child = (*child).next;
        }
    }
}

fn reconcile_ns_internal(doc: *mut XmlDoc, nodep: *mut XmlNode, search_parent: *mut XmlNode) {
    unsafe {
        if (*nodep).nsDef.is_null() {
            return;
        }
        let mut prevns: *mut XmlNs = ptr::null_mut();
        let mut curns = (*nodep).nsDef;
        while !curns.is_null() {
            let nsdftptr = (*curns).next;
            if !(*curns).href.is_null() {
                let nsptr = sys::xmlSearchNsByHref(doc, search_parent, (*curns).href);
                if !nsptr.is_null()
                    && ((*curns).prefix.is_null() || str_eq((*nsptr).prefix, (*curns).prefix))
                {
                    (*curns).next = ptr::null_mut();
                    if prevns.is_null() {
                        (*nodep).nsDef = nsdftptr;
                    } else {
                        (*prevns).next = nsdftptr;
                    }
                    lxn::set_old_ns(doc, curns);
                    curns = prevns;
                }
            }
            prevns = curns;
            curns = nsdftptr;
        }
    }
}

/// dom_reconcile_ns
pub fn reconcile_ns(doc: *mut XmlDoc, nodep: *mut XmlNode) {
    unsafe {
        if (*nodep).type_ == sys::XML_ELEMENT_NODE {
            reconcile_ns_internal(doc, nodep, (*nodep).parent);
            sys::xmlReconciliateNs(doc, nodep);
        }
    }
}

fn reconcile_ns_list_internal(
    doc: *mut XmlDoc,
    mut nodep: *mut XmlNode,
    last: *mut XmlNode,
    search_parent: *mut XmlNode,
) {
    unsafe {
        loop {
            if (*nodep).type_ == sys::XML_ELEMENT_NODE {
                reconcile_ns_internal(doc, nodep, search_parent);
                if !(*nodep).children.is_null() {
                    reconcile_ns_list_internal(
                        doc,
                        (*nodep).children,
                        (*nodep).last,
                        search_parent,
                    );
                }
            }
            if nodep == last {
                break;
            }
            nodep = (*nodep).next;
        }
    }
}

/// dom_reconcile_ns_list
pub fn reconcile_ns_list(doc: *mut XmlDoc, nodep: *mut XmlNode, last: *mut XmlNode) {
    unsafe {
        reconcile_ns_list_internal(doc, nodep, last, (*nodep).parent);
        let mut n = nodep;
        loop {
            sys::xmlReconciliateNs(doc, n);
            if n == last {
                break;
            }
            n = (*n).next;
        }
    }
}

/// php_dom_reconcile_attribute_namespace_after_insertion
pub fn reconcile_attribute_namespace_after_insertion(attrp: *mut XmlAttr) {
    unsafe {
        if (*attrp).ns.is_null() {
            return;
        }
        let nodep = (*attrp).parent;
        let matching = sys::xmlSearchNs((*nodep).doc, nodep, (*(*attrp).ns).prefix);
        if !matching.is_null() && str_eq((*matching).href, (*(*attrp).ns).href) {
            (*attrp).ns = matching;
        } else if !(*(*attrp).ns).prefix.is_null() {
            sys::xmlReconciliateNs((*nodep).doc, nodep);
        }
    }
}

/// `xmlSplitQName2`: (local, prefix) for "p:l", or None for an unprefixed
/// name.
pub fn split_qname2(name: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let cn = c(name);
    let mut prefix: *mut u8 = ptr::null_mut();
    unsafe {
        let local = sys::xmlSplitQName2(cn.as_ptr(), &mut prefix);
        if local.is_null() {
            sys::xml_free(prefix);
            return None;
        }
        let l = sys::take_string(local).unwrap_or_default();
        let p = sys::take_string(prefix).unwrap_or_default();
        Some((l, p))
    }
}

/// `xmlValidateName(name, 0) == 0`
pub fn valid_name(name: &[u8]) -> bool {
    let cn = c(name);
    unsafe { sys::xmlValidateName(cn.as_ptr(), 0) == 0 }
}

/// `xmlValidateQName(name, 0) == 0`
pub fn valid_qname(name: &[u8]) -> bool {
    let cn = c(name);
    unsafe { sys::xmlValidateQName(cn.as_ptr(), 0) == 0 }
}

/// dom_check_qname: (localname, prefix) or a DOMException code.
pub fn check_qname(qname: &[u8], uri_len: usize) -> Result<(Vec<u8>, Option<Vec<u8>>), i64> {
    if qname.is_empty() {
        return Err(super::NAMESPACE_ERR);
    }
    let (local, prefix) = match split_qname2(qname) {
        Some((l, p)) => (l, Some(p)),
        None => {
            if uri_len == 0 {
                return Ok((qname.to_vec(), None));
            }
            (qname.to_vec(), None)
        }
    };
    if !valid_qname(qname) {
        return Err(super::NAMESPACE_ERR);
    }
    if prefix.is_some() && uri_len == 0 {
        return Err(super::NAMESPACE_ERR);
    }
    Ok((local, prefix))
}

/// dom_validate_and_extract: (localname, prefix) or a DOMException code.
pub fn validate_and_extract(
    namespace: Option<&[u8]>,
    qname: &[u8],
) -> Result<(Vec<u8>, Option<Vec<u8>>), i64> {
    let ns = namespace.unwrap_or(b"");
    if !valid_qname(qname) {
        return Err(super::INVALID_CHARACTER_ERR);
    }
    let (local, prefix) = match split_qname2(qname) {
        Some((l, p)) => (Some(l), Some(p)),
        None => (None, None),
    };
    if prefix.is_some() && ns.is_empty() {
        return Err(super::NAMESPACE_ERR);
    }
    if ns != XML_NS_URI && prefix.as_deref() == Some(b"xml") {
        return Err(super::NAMESPACE_ERR);
    }
    if (qname == b"xmlns" || prefix.as_deref() == Some(b"xmlns")) && ns != XMLNS_NS_URI {
        return Err(super::NAMESPACE_ERR);
    }
    if ns == XMLNS_NS_URI && qname != b"xmlns" && prefix.as_deref() != Some(b"xmlns") {
        return Err(super::NAMESPACE_ERR);
    }
    Ok((local.unwrap_or_else(|| qname.to_vec()), prefix))
}

/// dom_get_ns_resolve_prefix_conflict: declare `uri` on `tree` under the
/// first free "defaultN" prefix.
pub fn get_ns_resolve_prefix_conflict(tree: *mut XmlNode, uri: *const u8) -> *mut XmlNs {
    unsafe {
        let doc = (*tree).doc;
        if doc.is_null() {
            return ptr::null_mut();
        }
        let mut prefix = b"default\0".to_vec();
        let mut counter = 1;
        while !sys::xmlSearchNs(doc, tree, prefix.as_ptr()).is_null() {
            if counter > 1000 {
                return ptr::null_mut();
            }
            prefix = format!("default{counter}\0").into_bytes();
            counter += 1;
        }
        sys::xmlNewNs(tree, uri, prefix.as_ptr())
    }
}

/// dom_get_ns_unchecked
pub fn get_ns_unchecked(nodep: *mut XmlNode, uri: &[u8], prefix: Option<&[u8]>) -> *mut XmlNs {
    let cu = c(uri);
    let cp = prefix.map(c);
    unsafe {
        let ns = sys::xmlNewNs(
            nodep,
            cu.as_ptr(),
            cp.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
        );
        if ns.is_null() {
            return get_ns_resolve_prefix_conflict(nodep, cu.as_ptr());
        }
        ns
    }
}

/// dom_get_ns: declare `uri` under `prefix`, or NAMESPACE_ERR.
pub fn get_ns(nodep: *mut XmlNode, uri: &[u8], prefix: Option<&[u8]>) -> Result<*mut XmlNs, i64> {
    let bad = match prefix {
        Some(b"xml") => uri != XML_NS_URI,
        Some(b"xmlns") => uri != XMLNS_NS_URI,
        Some(_) => uri == XMLNS_NS_URI,
        None => false,
    };
    if bad {
        return Err(super::NAMESPACE_ERR);
    }
    let ns = get_ns_unchecked(nodep, uri, prefix);
    if ns.is_null() {
        Err(super::NAMESPACE_ERR)
    } else {
        Ok(ns)
    }
}

/// dom_get_nsdecl: the declaration of `local` (a prefix, or the default
/// namespace for None/"") on `node` itself.
pub fn get_nsdecl(node: *mut XmlNode, local: Option<&[u8]>) -> *mut XmlNs {
    if node.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        let mut cur = (*node).nsDef;
        while !cur.is_null() {
            match local {
                None | Some(b"") => {
                    if (*cur).prefix.is_null() && !(*cur).href.is_null() {
                        return cur;
                    }
                }
                Some(l) => {
                    if !(*cur).prefix.is_null() && b((*cur).prefix) == l {
                        return cur;
                    }
                }
            }
            cur = (*cur).next;
        }
    }
    ptr::null_mut()
}

/// php_dom_create_fake_namespace_decl_node_ptr: a DOMNameSpaceNode's node,
/// a copy of `original` that names `nodep` as its parent.
pub fn fake_namespace_decl_node(
    nodep: *mut XmlNode,
    original: *mut XmlNs,
    fallback_doc: *mut XmlDoc,
) -> *mut XmlNode {
    unsafe {
        let doc = if nodep.is_null() {
            fallback_doc
        } else {
            (*nodep).doc
        };
        let curns = sys::xmlNewNs(ptr::null_mut(), (*original).href, ptr::null());
        let attrp = if !(*original).prefix.is_null() {
            (*curns).prefix = sys::xmlStrdup((*original).prefix);
            sys::xmlNewDocNode(doc, ptr::null_mut(), (*original).prefix, (*original).href)
        } else {
            sys::xmlNewDocNode(
                doc,
                ptr::null_mut(),
                c"xmlns".as_ptr().cast(),
                (*original).href,
            )
        };
        (*attrp).type_ = sys::XML_NAMESPACE_DECL;
        (*attrp).parent = nodep;
        (*attrp).ns = curns;
        attrp
    }
}

/// dom_set_document_ref_obj_single: false if the node's object already had
/// a document.
fn set_document_ref_single(node: *mut XmlNode, document: *mut RefObj) -> bool {
    let Some(obj) = (unsafe { lxn::object_of_node(node) }) else {
        return true;
    };
    if obj.document.get().is_null() {
        obj.document.set(document);
        unsafe { (*document).refcount += 1 };
        return true;
    }
    false
}

/// dom_set_document_ref_pointers_attr
pub fn set_document_ref_pointers_attr(attr: *mut XmlAttr, document: *mut RefObj) {
    set_document_ref_single(attr.cast(), document);
    unsafe {
        let mut ch = (*attr).children;
        while !ch.is_null() {
            set_document_ref_single(ch, document);
            ch = (*ch).next;
        }
    }
}

fn set_document_ref_pointers_node(node: *mut XmlNode, document: *mut RefObj) -> bool {
    if !set_document_ref_single(node, document) {
        return false;
    }
    unsafe {
        if (*node).type_ == sys::XML_ELEMENT_NODE {
            let mut attr = (*node).properties;
            while !attr.is_null() {
                set_document_ref_pointers_attr(attr, document);
                attr = (*attr).next;
            }
        }
    }
    true
}

/// dom_set_document_ref_pointers: objects in a subtree that just entered a
/// document take a reference on it.
pub fn set_document_ref_pointers(node: *mut XmlNode, document: *mut RefObj) {
    if document.is_null() || !set_document_ref_pointers_node(node, document) {
        return;
    }
    let base = node;
    let mut n = unsafe { (*node).children };
    while !n.is_null() && set_document_ref_pointers_node(n, document) {
        n = next_in_tree_order(n, base);
    }
}

/// dom_clone_node (legacy): `xmlDocCopyNode`, a DTD with `xmlCopyDtd`.
pub fn clone_node(node: *mut XmlNode, doc: *mut XmlDoc, recursive: bool) -> *mut XmlNode {
    unsafe {
        if (*node).type_ == sys::XML_DTD_NODE {
            let dtd = sys::xmlCopyDtd(node.cast());
            sys::xmlSetTreeDoc(dtd.cast(), doc);
            return dtd.cast();
        }
        let extended: c_int = if !recursive && (*node).type_ == sys::XML_ELEMENT_NODE {
            2
        } else {
            recursive as c_int
        };
        sys::xmlDocCopyNode(node, doc, extended)
    }
}

/// php_dom_libxml_hash_iter: the `index`th entry of a libxml2 hash table.
pub fn hash_iter(ht: *mut sys::xmlHashTable, index: i64) -> *mut XmlNode {
    struct It {
        cur: i64,
        index: i64,
        node: *mut XmlNode,
    }
    unsafe extern "C" fn scan(
        payload: *mut std::ffi::c_void,
        data: *mut std::ffi::c_void,
        _: *const u8,
    ) {
        let it = unsafe { &mut *(data as *mut It) };
        if it.cur < it.index {
            it.cur += 1;
        } else if it.node.is_null() {
            it.node = payload.cast();
        }
    }
    if ht.is_null() {
        return ptr::null_mut();
    }
    let size = unsafe { sys::xmlHashSize(ht) } as i64;
    if size <= 0 || index >= size {
        return ptr::null_mut();
    }
    let mut it = It {
        cur: 0,
        index,
        node: ptr::null_mut(),
    };
    unsafe { sys::xmlHashScan(ht, scan, (&mut it as *mut It).cast()) };
    it.node
}

/// create_notation: a notation as a node (an entity in disguise, freed by
/// php_libxml_node_free's XML_NOTATION_NODE case).
pub fn create_notation(
    name: *const u8,
    external_id: *const u8,
    system_id: *const u8,
) -> *mut XmlNode {
    unsafe {
        let size = std::mem::size_of::<sys::XmlEntity>().max(std::mem::size_of::<XmlNode>()) + 64;
        let ret = sys::rhp_xml_malloc(size) as *mut sys::XmlEntity;
        ptr::write_bytes(ret.cast::<u8>(), 0, size);
        (*ret).type_ = sys::XML_NOTATION_NODE;
        (*ret).name = sys::xmlStrdup(name);
        (*ret).ExternalID = sys::xmlStrdup(external_id);
        (*ret).SystemID = sys::xmlStrdup(system_id);
        ret.cast()
    }
}

/// dom_node_concatenated_name_helper / the qualified name of an element
/// or attribute.
pub fn qualified_name(nodep: *const XmlNode) -> Vec<u8> {
    unsafe {
        let name = b((*nodep).name);
        let ns = (*nodep).ns;
        if !ns.is_null() && !(*ns).prefix.is_null() {
            let mut v = b((*ns).prefix).to_vec();
            v.push(b':');
            v.extend_from_slice(name);
            v
        } else {
            name.to_vec()
        }
    }
}

/// php_dom_is_node_connected
pub fn is_connected(mut node: *const XmlNode) -> bool {
    unsafe {
        while !node.is_null() {
            let t = (*node).type_;
            if t == sys::XML_DOCUMENT_NODE || t == sys::XML_HTML_DOCUMENT_NODE {
                return true;
            }
            node = (*node).parent;
        }
    }
    false
}

/// dom_mark_document_cache_as_modified_since_parsing (legacy documents
/// start at 2: `new DOMDocument` plus a load).
pub fn mark_ids_modified(doc: *mut RefObj) {
    if let Some(d) = unsafe { doc.as_mut() } {
        let min = if d.class_type == lxn::ClassType::Modern {
            1
        } else {
            2
        };
        d.cache_tag = (min + 1).max(d.cache_tag);
    }
}

/// dom_is_document_cache_modified_since_parsing
pub fn ids_modified_since_parsing(doc: *mut RefObj) -> bool {
    match unsafe { doc.as_ref() } {
        None => true,
        Some(d) => {
            let min = if d.class_type == lxn::ClassType::Modern {
                1
            } else {
                2
            };
            d.cache_tag > min
        }
    }
}

/// dom_compare_value
pub fn attr_value_equals(attr: *const XmlAttr, value: &[u8]) -> bool {
    unsafe { lxn::attr_value(attr) == value }
}

/// dom_attr_value_will_change
pub fn attr_value_will_change(document: *mut RefObj, attrp: *mut XmlAttr) {
    unsafe {
        if (*attrp).atype == sys::XML_ATTRIBUTE_ID {
            sys::xmlRemoveID((*attrp).doc, attrp);
            (*attrp).atype = sys::XML_ATTRIBUTE_ID;
        }
    }
    mark_ids_modified(document);
}
