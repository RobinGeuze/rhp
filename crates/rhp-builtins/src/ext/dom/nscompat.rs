//! The modern DOM's per-document state and namespace model, ported from
//! php-src ext/dom/private_data.c and namespace_compat.c (PHP License
//! 3.01).
//!
//! A `Dom\` document keeps its namespaces in a mapper (one `xmlNs` per
//! prefix and URI) instead of `nsDef` lists: elements point at the
//! mapper's namespaces, and `xmlns` declarations are ordinary attributes
//! in the XMLNS namespace. A namespace's `_private` holds a "magic token"
//! once it is known to be one of the well-known namespaces.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;

use super::util::{next_in_tree_order, str_eq_b};
use crate::ext::libxml::node::{self as lxn, PrivateHeader, RefObj};
use crate::ext::libxml::sys::{self, XmlAttr, XmlDoc, XmlNode, XmlNs};

pub const XHTML_NS_URI: &[u8] = b"http://www.w3.org/1999/xhtml";
pub const MATHML_NS_URI: &[u8] = b"http://www.w3.org/1998/Math/MathML";
pub const SVG_NS_URI: &[u8] = b"http://www.w3.org/2000/svg";
pub const XLINK_NS_URI: &[u8] = b"http://www.w3.org/1999/xlink";
pub const XML_NS_URI: &[u8] = b"http://www.w3.org/XML/1998/namespace";
pub const XMLNS_NS_URI: &[u8] = b"http://www.w3.org/2000/xmlns/";

/// The magic tokens: their addresses identify a namespace (aligned, so the
/// low bit stays free for [`lxn::NS_TAG_HOOK`]).
#[repr(C, align(8))]
pub struct Token(pub &'static [u8]);

pub static HTML: Token = Token(XHTML_NS_URI);
pub static MATHML: Token = Token(MATHML_NS_URI);
pub static SVG: Token = Token(SVG_NS_URI);
pub static XLINK: Token = Token(XLINK_NS_URI);
pub static XML: Token = Token(XML_NS_URI);
pub static XMLNS: Token = Token(XMLNS_NS_URI);

fn token_ptr(t: &'static Token) -> *mut c_void {
    t as *const Token as *mut c_void
}

/// php_dom_ns_is_fast_ex
pub fn ns_is_fast_ex(ns: *mut XmlNs, token: &'static Token) -> bool {
    unsafe {
        let p = (*ns)._private;
        if p == token_ptr(token) {
            return true;
        }
        if !p.is_null() && (p as usize) & 1 == 0 {
            return false;
        }
        if str_eq_b((*ns).href, token.0) {
            if p.is_null() {
                (*ns)._private = token_ptr(token);
            }
            return true;
        }
        false
    }
}

/// php_dom_ns_is_fast
pub fn ns_is_fast(node: *const XmlNode, token: &'static Token) -> bool {
    let ns = unsafe { (*node).ns };
    !ns.is_null() && ns_is_fast_ex(ns, token)
}

/// php_dom_ns_is_html_and_document_is_html
pub fn ns_is_html_and_document_is_html(node: *const XmlNode) -> bool {
    unsafe {
        !(*node).doc.is_null()
            && (*(*node).doc).type_ == sys::XML_HTML_DOCUMENT_NODE
            && ns_is_fast(node, &HTML)
    }
}

/// An owned namespace's prefix and URI (NUL-terminated).
type OwnedStrings = (Box<[u8]>, Box<[u8]>);

/// One namespace of the mapper. An owned one's strings live here.
struct Entry {
    ns: *mut XmlNs,
    owned: Option<OwnedStrings>,
}

/// `php_dom_libxml_ns_mapper`
pub struct NsMapper {
    html_ns: *mut XmlNs,
    prefixless_xmlns_ns: *mut XmlNs,
    /// URI to prefix to namespace.
    map: HashMap<Vec<u8>, HashMap<Vec<u8>, Entry>>,
}

impl Default for NsMapper {
    fn default() -> Self {
        NsMapper {
            html_ns: ptr::null_mut(),
            prefixless_xmlns_ns: ptr::null_mut(),
            map: HashMap::new(),
        }
    }
}

impl Drop for NsMapper {
    fn drop(&mut self) {
        for m in self.map.values() {
            for e in m.values() {
                if e.owned.is_some() {
                    drop(unsafe { Box::from_raw(e.ns) });
                }
            }
        }
    }
}

impl NsMapper {
    /// php_dom_libxml_ns_mapper_get_ns: NULL for no prefix and no URI.
    pub fn get_ns(&mut self, prefix: &[u8], uri: &[u8]) -> *mut XmlNs {
        if prefix.is_empty() && uri.is_empty() {
            return ptr::null_mut();
        }
        let m = self.map.entry(uri.to_vec()).or_default();
        if let Some(e) = m.get(prefix) {
            return e.ns;
        }
        let mut p = prefix.to_vec();
        p.push(0);
        let mut u = uri.to_vec();
        u.push(0);
        let (p, u) = (p.into_boxed_slice(), u.into_boxed_slice());
        let ns = Box::into_raw(Box::new(XmlNs {
            next: ptr::null_mut(),
            type_: sys::XML_LOCAL_NAMESPACE,
            href: u.as_ptr(),
            prefix: if prefix.is_empty() {
                ptr::null()
            } else {
                p.as_ptr()
            },
            _private: ptr::null_mut(),
            context: ptr::null_mut(),
        }));
        m.insert(
            prefix.to_vec(),
            Entry {
                ns,
                owned: Some((p, u)),
            },
        );
        ns
    }

    /// php_dom_libxml_ns_mapper_get_ns_raw_strings_nullsafe
    pub fn get_ns_nullsafe(&mut self, prefix: *const u8, uri: *const u8) -> *mut XmlNs {
        let p = unsafe { sys::cstr(prefix) };
        let u = unsafe { sys::cstr(uri) };
        self.get_ns(p, u)
    }

    fn cached(&mut self, which: bool, uri: &[u8], token: &'static Token) -> *mut XmlNs {
        let slot = if which {
            self.html_ns
        } else {
            self.prefixless_xmlns_ns
        };
        if !slot.is_null() {
            return slot;
        }
        let ns = self.get_ns(b"", uri);
        unsafe { (*ns)._private = token_ptr(token) };
        if which {
            self.html_ns = ns;
        } else {
            self.prefixless_xmlns_ns = ns;
        }
        ns
    }

    /// php_dom_libxml_ns_mapper_ensure_html_ns
    pub fn ensure_html_ns(&mut self) -> *mut XmlNs {
        self.cached(true, XHTML_NS_URI, &HTML)
    }

    /// php_dom_libxml_ns_mapper_ensure_prefixless_xmlns_ns
    pub fn ensure_prefixless_xmlns_ns(&mut self) -> *mut XmlNs {
        self.cached(false, XMLNS_NS_URI, &XMLNS)
    }

    /// php_dom_libxml_ns_mapper_store_and_normalize_parsed_ns
    fn store_parsed_ns(&mut self, ns: *mut XmlNs) {
        let href = unsafe { sys::cstr((*ns).href) }.to_vec();
        let prefix = unsafe { sys::cstr((*ns).prefix) }.to_vec();
        let m = self.map.entry(href).or_default();
        m.entry(prefix).or_insert(Entry { ns, owned: None });
    }
}

/// `php_dom_private_data`
#[repr(C)]
pub struct DomPrivate {
    header: PrivateHeader,
    pub ns_mapper: NsMapper,
    /// Template element to its content fragment.
    templates: Option<HashMap<usize, *mut XmlNode>>,
}

unsafe fn private_dtor(h: *mut PrivateHeader) {
    let mut p = unsafe { Box::from_raw(h as *mut DomPrivate) };
    if let Some(t) = p.templates.take() {
        for (_, n) in t {
            unsafe { sys::xmlFreeNode(n) };
        }
    }
}

unsafe fn private_ns_hook(h: *mut PrivateHeader, node: *mut XmlNode) {
    remove_templated_content(unsafe { &mut *(h as *mut DomPrivate) }, node);
}

impl DomPrivate {
    /// php_dom_private_data_create
    pub fn create() -> *mut PrivateHeader {
        Box::into_raw(Box::new(DomPrivate {
            header: PrivateHeader {
                dtor: private_dtor,
                ns_hook: private_ns_hook,
            },
            ns_mapper: NsMapper::default(),
            templates: None,
        }))
        .cast()
    }
}

/// php_dom_private_data_destroy for private data no document took.
pub fn destroy(h: *mut PrivateHeader) {
    unsafe { private_dtor(h) }
}

/// php_dom_get_private_data: the document's modern state.
pub fn of(document: *mut RefObj) -> Option<&'static mut DomPrivate> {
    unsafe {
        let d = document.as_ref()?;
        (d.private_data as *mut DomPrivate).as_mut()
    }
}

/// php_dom_get_ns_mapper
pub fn mapper(document: *mut RefObj) -> Option<&'static mut NsMapper> {
    of(document).map(|p| &mut p.ns_mapper)
}

fn free_templated_content(p: &mut DomPrivate, base: *mut XmlNode) {
    if p.templates.as_ref().is_some_and(|t| !t.is_empty()) {
        let mut cur = unsafe { (*base).children };
        while !cur.is_null() {
            if unsafe { (*cur).type_ } == sys::XML_ELEMENT_NODE {
                remove_templated_content(p, cur);
            }
            cur = next_in_tree_order(cur, base);
        }
    }
    unsafe { sys::xmlFreeNode(base) };
}

/// php_dom_add_templated_content
pub fn add_templated_content(p: &mut DomPrivate, template: *const XmlNode, fragment: *mut XmlNode) {
    p.templates
        .get_or_insert_with(HashMap::new)
        .insert(template as usize, fragment);
}

/// php_dom_retrieve_templated_content
pub fn retrieve_templated_content(p: &DomPrivate, template: *const XmlNode) -> *mut XmlNode {
    p.templates
        .as_ref()
        .and_then(|t| t.get(&(template as usize)).copied())
        .unwrap_or(ptr::null_mut())
}

/// php_dom_remove_templated_content
pub fn remove_templated_content(p: &mut DomPrivate, template: *const XmlNode) {
    let n = p
        .templates
        .as_mut()
        .and_then(|t| t.remove(&(template as usize)));
    if let Some(n) = n {
        free_templated_content(p, n);
    }
}

/// dom_add_element_ns_hook: the element gets its own copy of its
/// namespace, tagged so freeing it frees its template content.
pub fn add_element_ns_hook(p: &mut DomPrivate, element: *mut XmlNode) {
    unsafe {
        let ns = sys::rhp_xml_malloc(std::mem::size_of::<XmlNs>()) as *mut XmlNs;
        ptr::write_bytes(ns, 0, 1);
        let old = (*element).ns;
        (*ns).prefix = sys::xmlStrdup((*old).prefix);
        (*ns).href = sys::xmlStrdup((*old).href);
        (*ns).type_ = sys::XML_LOCAL_NAMESPACE;
        (*ns)._private = ((p as *mut DomPrivate as usize) | lxn::NS_TAG_HOOK) as *mut c_void;
        (*element).ns = ns;
        lxn::set_old_ns((*element).doc, ns);
    }
}

/// php_dom_ns_compat_mark_attribute: the xmlns attribute for `ns`.
pub fn mark_attribute(mapper: &mut NsMapper, node: *mut XmlNode, ns: *mut XmlNs) -> *mut XmlAttr {
    unsafe {
        let (xmlns_ns, name) = if !(*ns).prefix.is_null() {
            (mapper.get_ns(b"xmlns", XMLNS_NS_URI), (*ns).prefix)
        } else {
            (
                mapper.ensure_prefixless_xmlns_ns(),
                c"xmlns".as_ptr().cast(),
            )
        };
        sys::xmlSetNsProp(node, xmlns_ns, name, (*ns).href)
    }
}

/// php_dom_ns_compat_mark_attribute_list: a parsed element's declarations
/// become attributes (in front of the others) and mapper namespaces.
pub fn mark_attribute_list(mapper: &mut NsMapper, node: *mut XmlNode) {
    unsafe {
        if (*node).nsDef.is_null() {
            return;
        }
        let attr = (*node).properties;
        (*node).properties = ptr::null_mut();
        let mut ns = (*node).nsDef;
        let mut last_added: *mut XmlAttr = ptr::null_mut();
        while !ns.is_null() {
            last_added = mark_attribute(mapper, node, ns);
            mapper.store_parsed_ns(ns);
            let next = (*ns).next;
            (*ns).next = ptr::null_mut();
            lxn::set_old_ns((*node).doc, ns);
            ns = next;
        }
        if !last_added.is_null() {
            if !attr.is_null() {
                (*last_added).next = attr;
                (*attr).prev = last_added;
            }
        } else {
            (*node).properties = attr;
        }
        (*node).nsDef = ptr::null_mut();
    }
}

/// dom_mark_namespaces_as_attributes_too
pub fn mark_namespaces_as_attributes_too(mapper: &mut NsMapper, doc: *mut XmlDoc) {
    let mut node = unsafe { (*doc).children };
    while !node.is_null() {
        if unsafe { (*node).type_ } == sys::XML_ELEMENT_NODE {
            mark_attribute_list(mapper, node);
        }
        node = next_in_tree_order(node, ptr::null());
    }
}

/// dom_document_convert_to_modern
pub fn convert_to_modern(document: *mut RefObj, doc: *mut XmlDoc) {
    let h = DomPrivate::create();
    unsafe { (*document).private_data = h };
    let m = &mut unsafe { &mut *(h as *mut DomPrivate) }.ns_mapper;
    mark_namespaces_as_attributes_too(m, doc);
}

struct Reconcile<'a> {
    map: HashMap<usize, *mut XmlNs>,
    last_src: *mut XmlNs,
    last_dst: *mut XmlNs,
    mapper: &'a mut NsMapper,
}

impl Reconcile<'_> {
    fn single(&mut self, node: *mut XmlNode) {
        unsafe {
            let ns = (*node).ns;
            if ns == self.last_src {
                (*node).ns = self.last_dst;
                return;
            }
            match self.map.get(&(ns as usize)) {
                None => {
                    let new_ns = self.mapper.get_ns_nullsafe((*ns).prefix, (*ns).href);
                    self.map.insert(ns as usize, new_ns);
                    self.last_src = ns;
                    self.last_dst = new_ns;
                    (*node).ns = new_ns;
                }
                Some(&n) => (*node).ns = n,
            }
        }
    }

    fn element(&mut self, node: *mut XmlNode) {
        unsafe {
            if !(*node).ns.is_null() {
                self.single(node);
            }
            let mut a = (*node).properties;
            while !a.is_null() {
                if !(*a).ns.is_null() {
                    self.single(a.cast());
                }
                a = (*a).next;
            }
        }
    }
}

/// php_dom_libxml_reconcile_modern: point a moved subtree's namespaces at
/// the mapper's.
pub fn reconcile_modern(mapper: &mut NsMapper, node: *mut XmlNode) {
    unsafe {
        if (*node).type_ == sys::XML_ATTRIBUTE_NODE {
            if !(*node).ns.is_null() {
                (*node).ns = mapper.get_ns_nullsafe((*(*node).ns).prefix, (*(*node).ns).href);
            }
            return;
        }
        if (*node).type_ != sys::XML_ELEMENT_NODE
            || ((*node).children.is_null()
                && (*node).properties.is_null()
                && (*node).ns == (*node).nsDef)
        {
            return;
        }
        let mut ctx = Reconcile {
            map: HashMap::new(),
            last_src: ptr::null_mut(),
            last_dst: ptr::null_mut(),
            mapper,
        };
        ctx.element(node);
        let base = node;
        let mut n = (*node).children;
        while !n.is_null() {
            if (*n).type_ == sys::XML_ELEMENT_NODE {
                ctx.element(n);
            }
            n = next_in_tree_order(n, base);
        }
    }
}

/// php_dom_get_in_scope_ns: the namespaces in scope at `node`, from the
/// element namespaces and xmlns attributes (innermost first per prefix).
pub fn in_scope_ns(
    mapper: &mut NsMapper,
    node: *const XmlNode,
    ignore_elements: bool,
) -> Vec<*mut XmlNs> {
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let mut out = Vec::new();
    let mut cur = node;
    unsafe {
        while !cur.is_null() {
            if (*cur).type_ == sys::XML_ELEMENT_NODE {
                if !ignore_elements && !(*cur).ns.is_null() && !(*(*cur).ns).prefix.is_null() {
                    let p = sys::cstr((*(*cur).ns).prefix).to_vec();
                    if !seen.contains(&p) {
                        seen.push(p);
                        out.push((*cur).ns);
                    }
                }
                let mut a = (*cur).properties;
                while !a.is_null() {
                    if !(*a).ns.is_null()
                        && !(*(*a).ns).prefix.is_null()
                        && ns_is_fast_ex((*a).ns, &XMLNS)
                        && !(*a).children.is_null()
                        && !(*(*a).children).content.is_null()
                    {
                        let p = sys::cstr((*a).name).to_vec();
                        let ns = mapper.get_ns(&p, sys::cstr((*(*a).children).content));
                        if !seen.contains(&p) {
                            seen.push(p);
                            out.push(ns);
                        }
                    }
                    a = (*a).next;
                }
            }
            cur = (*cur).parent;
        }
    }
    out
}

/// The document a modern node belongs to (php_dom_follow_spec_intern).
pub fn follow_spec(document: *mut RefObj) -> bool {
    unsafe { document.as_ref() }.is_some_and(|d| d.class_type == lxn::ClassType::Modern)
}

/// php_dom_follow_spec_node: by the node's document object.
pub fn follow_spec_node(node: *const XmlNode) -> bool {
    unsafe {
        let doc = (*node).doc;
        if doc.is_null() {
            return false;
        }
        let p = (*doc)._private as *mut lxn::NodePtr;
        if p.is_null() {
            return false;
        }
        let Some(obj) = (*p)._private.as_ref() else {
            return false;
        };
        follow_spec(obj.document.get())
    }
}
