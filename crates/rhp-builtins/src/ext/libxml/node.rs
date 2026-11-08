//! Who owns a libxml2 tree: php's `php_libxml_ref_obj` (one per document,
//! counted by the objects that use it) and `php_libxml_node_ptr` (one per
//! node an object wraps, hung off `node->_private`), and the functions of
//! ext/libxml/libxml.c that free a node once no object needs it. DOM,
//! SimpleXML and XMLReader's `expand()` share these.
//!
//! The counts are php's, kept by hand on raw pointers, because php-src's
//! tests observe exactly when a node or document goes away
//! (ext/dom/tests/delayed_freeing). An object keeps its [`NodeObject`] in
//! its payload; the payload's `Drop` is php's `free_obj`.
//!
//! Ported from php-src ext/libxml/libxml.c (PHP License 3.01).

use std::cell::{Cell, RefCell};
use std::ptr;
use std::rc::Weak;

use rhp_values::{ObjectData, Value};

use super::sys::{self, XmlAttr, XmlDoc, XmlEntity, XmlNode, XmlNs};

/// `libxml_doc_props`: DOMDocument's parser and serializer switches, and
/// `registerNodeClass`'s class map.
#[derive(Clone)]
pub struct DocProps {
    /// Base class (lowercase name) to the user class registered for it.
    pub classmap: Vec<(Vec<u8>, Vec<u8>)>,
    pub formatoutput: bool,
    pub validateonparse: bool,
    pub resolveexternals: bool,
    pub preservewhitespace: bool,
    pub substituteentities: bool,
    pub stricterror: bool,
    pub recover: bool,
}

impl DocProps {
    pub const DEFAULT: DocProps = DocProps {
        classmap: Vec::new(),
        formatoutput: false,
        validateonparse: false,
        resolveexternals: false,
        preservewhitespace: true,
        substituteentities: false,
        stricterror: true,
        recover: false,
    };
}

/// `php_libxml_class_type`: which API a document's nodes were exposed
/// through.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ClassType {
    Unset = 0,
    Legacy = 1,
    Modern = 2,
}

/// `php_libxml_private_data_header`: what a document's API keeps with it
/// (the modern DOM's namespace mapper and template contents). The owner's
/// struct starts with this header.
#[repr(C)]
pub struct PrivateHeader {
    pub dtor: unsafe fn(*mut PrivateHeader),
    /// Called when an element whose namespace is tagged with
    /// [`NS_TAG_HOOK`] is freed.
    pub ns_hook: unsafe fn(*mut PrivateHeader, *mut XmlNode),
}

/// `LIBXML_NS_TAG_HOOK`: the tag on `ns->_private` that points at a
/// [`PrivateHeader`].
pub const NS_TAG_HOOK: usize = 1;

/// `php_libxml_quirks_mode`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuirksMode {
    NoQuirks,
    LimitedQuirks,
    Quirks,
}

/// `php_libxml_ref_obj`.
pub struct RefObj {
    pub ptr: *mut XmlDoc,
    pub doc_props: Option<Box<DocProps>>,
    /// `cache_tag.modification_nr`.
    pub cache_tag: usize,
    pub refcount: u32,
    pub class_type: ClassType,
    pub private_data: *mut PrivateHeader,
    pub quirks_mode: QuirksMode,
}

/// `php_libxml_node_ptr`: `node->_private` points here.
pub struct NodePtr {
    pub node: *mut XmlNode,
    pub refcount: u32,
    /// The object php's DOM registered for the node (NULL for SimpleXML).
    pub _private: *const NodeObject,
}

/// `php_libxml_node_object`: what every libxml-backed object carries.
pub struct NodeObject {
    pub node: Cell<*mut NodePtr>,
    pub document: Cell<*mut RefObj>,
    /// The object this state belongs to, to hand it out again.
    pub this: RefCell<Weak<RefCell<ObjectData>>>,
}

impl Default for NodeObject {
    fn default() -> Self {
        NodeObject {
            node: Cell::new(ptr::null_mut()),
            document: Cell::new(ptr::null_mut()),
            this: RefCell::new(Weak::new()),
        }
    }
}

impl NodeObject {
    /// The wrapped node (`dom_object_get_node`), or NULL.
    pub fn xml_node(&self) -> *mut XmlNode {
        let p = self.node.get();
        if p.is_null() {
            ptr::null_mut()
        } else {
            unsafe { (*p).node }
        }
    }

    /// The object, while it is alive.
    pub fn object(&self) -> Option<Value> {
        self.this.borrow().upgrade().map(Value::Object)
    }
}

/// `dom_get_doc_props_read_only`
pub fn doc_props_read_only(document: *mut RefObj) -> DocProps {
    unsafe { document.as_ref() }
        .and_then(|d| d.doc_props.as_deref().cloned())
        .unwrap_or(DocProps::DEFAULT)
}

/// `dom_get_doc_props`: the document's props, created on first write.
///
/// # Safety
/// `document` is NULL or a live [`RefObj`].
pub unsafe fn doc_props_mut<'a>(document: *mut RefObj) -> Option<&'a mut DocProps> {
    let d = unsafe { document.as_mut() }?;
    Some(
        d.doc_props
            .get_or_insert_with(|| Box::new(DocProps::DEFAULT)),
    )
}

/// php_libxml_set_old_ns_list
unsafe fn set_old_ns_list(doc: *mut XmlDoc, first: *mut XmlNs, last: *mut XmlNs) {
    let Some(d) = (unsafe { doc.as_mut() }) else {
        return;
    };
    unsafe {
        if d.oldNs.is_null() {
            let ns = sys::rhp_xml_malloc(std::mem::size_of::<XmlNs>()) as *mut XmlNs;
            if ns.is_null() {
                return;
            }
            ptr::write_bytes(ns, 0, 1);
            (*ns).type_ = sys::XML_LOCAL_NAMESPACE;
            (*ns).href = sys::xmlStrdup(c"http://www.w3.org/XML/1998/namespace".as_ptr().cast());
            (*ns).prefix = sys::xmlStrdup(c"xml".as_ptr().cast());
            d.oldNs = ns;
        } else {
            (*last).next = (*d.oldNs).next;
        }
        (*d.oldNs).next = first;
    }
}

/// php_libxml_set_old_ns
///
/// # Safety
/// `doc` is NULL or a live document; `ns` is a namespace no list holds.
pub unsafe fn set_old_ns(doc: *mut XmlDoc, ns: *mut XmlNs) {
    unsafe { set_old_ns_list(doc, ns, ns) }
}

unsafe extern "C" fn unlink_entity(
    data: *mut std::ffi::c_void,
    table: *mut std::ffi::c_void,
    name: *const u8,
) {
    let entity = data as *mut XmlEntity;
    unsafe {
        if !(*entity)._private.is_null() {
            sys::xmlHashRemoveEntry(table.cast(), name, ptr::null_mut());
        }
    }
}

/// php_libxml_unregister_node
unsafe fn unregister_node(nodep: *mut XmlNode) {
    let nodeptr = unsafe { (*nodep)._private } as *mut NodePtr;
    if nodeptr.is_null() {
        return;
    }
    unsafe {
        let wrapper = (*nodeptr)._private;
        if let Some(w) = wrapper.as_ref() {
            decrement_node_ptr(w);
            decrement_doc_ref(w);
        } else {
            if (*nodep).type_ != sys::XML_DOCUMENT_NODE {
                (*nodep)._private = ptr::null_mut();
            }
            (*nodeptr).node = ptr::null_mut();
        }
    }
}

/// php_libxml_unlink_entity_decl: libxml2 unlinks an entity from its DTD
/// only when the DTD is in a document.
unsafe fn unlink_entity_decl(entity: *mut XmlEntity) {
    unsafe {
        let dtd = (*entity).parent;
        if dtd.is_null() {
            return;
        }
        let name = (*entity).name;
        if sys::xmlHashLookup((*dtd).entities, name) == entity.cast() {
            sys::xmlHashRemoveEntry((*dtd).entities, name, ptr::null_mut());
        }
        if sys::xmlHashLookup((*dtd).pentities, name) == entity.cast() {
            sys::xmlHashRemoveEntry((*dtd).pentities, name, ptr::null_mut());
        }
    }
}

/// php_libxml_node_free
unsafe fn node_free(node: *mut XmlNode) {
    unsafe {
        if !(*node)._private.is_null() {
            (*((*node)._private as *mut NodePtr)).node = ptr::null_mut();
        }
        match (*node).type_ {
            sys::XML_ATTRIBUTE_NODE => sys::xmlFreeProp(node.cast()),
            sys::XML_ENTITY_DECL => {
                let entity = node as *mut XmlEntity;
                if (*entity).etype != sys::XML_INTERNAL_PREDEFINED_ENTITY {
                    unlink_entity_decl(entity);
                    sys::rhp_xml_free_entity(entity);
                }
            }
            sys::XML_NOTATION_NODE => {
                // create_notation()'s entities in disguise
                let entity = node as *mut XmlEntity;
                sys::xml_free((*node).name as *mut u8);
                sys::xml_free((*entity).ExternalID as *mut u8);
                sys::xml_free((*entity).SystemID as *mut u8);
                sys::xml_free(node);
            }
            sys::XML_ELEMENT_DECL | sys::XML_ATTRIBUTE_DECL => {}
            sys::XML_NAMESPACE_DECL => {
                if !(*node).ns.is_null() {
                    sys::xmlFreeNs((*node).ns);
                    (*node).ns = ptr::null_mut();
                }
                (*node).type_ = sys::XML_ELEMENT_NODE;
                sys::xmlFreeNode(node);
            }
            sys::XML_DTD_NODE => {
                let dtd = node as *mut sys::XmlDtd;
                if (*dtd)._private.is_null() {
                    // No object holds the DTD, but some may hold its entities.
                    sys::xmlHashScan((*dtd).entities, unlink_entity, (*dtd).entities.cast());
                    sys::xmlHashScan((*dtd).pentities, unlink_entity, (*dtd).pentities.cast());
                }
                sys::xmlFreeDtd(dtd);
            }
            sys::XML_ELEMENT_NODE => {
                let ns = (*node).ns;
                if !ns.is_null() && ((*ns)._private as usize) & 1 == NS_TAG_HOOK {
                    // An element of a "special" namespace (a template).
                    let header = ((*ns)._private as usize & !1) as *mut PrivateHeader;
                    ((*header).ns_hook)(header, node);
                }
                if !(*node).nsDef.is_null() && !(*node).doc.is_null() {
                    // Keep the declarations alive past their element: a
                    // node elsewhere may still use them.
                    let ns = (*node).nsDef;
                    let mut last = ns;
                    while !(*last).next.is_null() {
                        last = (*last).next;
                    }
                    set_old_ns_list((*node).doc, ns, last);
                    (*node).nsDef = ptr::null_mut();
                }
                sys::xmlFreeNode(node);
            }
            _ => sys::xmlFreeNode(node),
        }
    }
}

/// php_libxml_node_free_list: free a sibling list, except the nodes an
/// object still wraps, which are unlinked and kept.
///
/// # Safety
/// `node` is NULL or the head of a list no object frees concurrently.
pub unsafe fn node_free_list(node: *mut XmlNode) {
    let mut curnode = node;
    unsafe {
        while !curnode.is_null() {
            if !(*curnode)._private.is_null() {
                let next = (*curnode).next;
                sys::xmlUnlinkNode(curnode);
                if (*curnode).type_ == sys::XML_ELEMENT_NODE {
                    // Namespaces the subtree uses must be declared in it,
                    // before their holder goes.
                    let p = (*curnode)._private as *const NodePtr;
                    if let Some(obj) = (*p)._private.as_ref() {
                        let doc = obj.document.get();
                        if doc.is_null() || (*doc).class_type < ClassType::Modern {
                            reconcile_detached(curnode);
                        }
                    }
                }
                curnode = next;
                continue;
            }
            let node = curnode;
            match (*node).type_ {
                sys::XML_ENTITY_REF_NODE | sys::XML_NOTATION_NODE => {}
                sys::XML_ENTITY_DECL => unlink_entity_decl(node.cast()),
                t @ (sys::XML_ATTRIBUTE_NODE
                | sys::XML_ATTRIBUTE_DECL
                | sys::XML_DTD_NODE
                | sys::XML_DOCUMENT_TYPE_NODE
                | sys::XML_NAMESPACE_DECL
                | sys::XML_TEXT_NODE) => {
                    if t == sys::XML_ATTRIBUTE_NODE
                        && !(*node).doc.is_null()
                        && (*(node as *mut XmlAttr)).atype == sys::XML_ATTRIBUTE_ID
                    {
                        sys::xmlRemoveID((*node).doc, node.cast());
                    }
                    node_free_list((*node).children);
                }
                _ => {
                    node_free_list((*node).children);
                    node_free_list((*node).properties.cast());
                }
            }
            curnode = (*node).next;
            sys::xmlUnlinkNode(node);
            unregister_node(node);
            node_free(node);
        }
    }
}

/// `xmlReconciliateNs` for a node that just left its tree. Before 2.13
/// it needs a document, which may create the xml namespace on it; a node
/// without one gets a stand-in and keeps what was created on it.
unsafe fn reconcile_detached(curnode: *mut XmlNode) {
    unsafe {
        if super::header_version() < 21300 && (*curnode).doc.is_null() {
            let mut dummy: XmlDoc = std::mem::zeroed();
            dummy.type_ = sys::XML_DOCUMENT_NODE;
            (*curnode).doc = &mut dummy;
            sys::xmlReconciliateNs((*curnode).doc, curnode);
            (*curnode).doc = ptr::null_mut();
            if !dummy.oldNs.is_null() {
                let old = (*curnode).nsDef;
                (*curnode).nsDef = dummy.oldNs;
                (*dummy.oldNs).next = old;
            }
        } else {
            sys::xmlReconciliateNs((*curnode).doc, curnode);
        }
    }
}

/// php_libxml_increment_node_ptr
pub fn increment_node_ptr(
    object: &NodeObject,
    node: *mut XmlNode,
    private_data: *const NodeObject,
) -> u32 {
    if node.is_null() {
        return 0;
    }
    unsafe {
        let cur = object.node.get();
        if !cur.is_null() {
            if (*cur).node == node {
                return (*cur).refcount;
            }
            decrement_node_ptr(object);
        }
        if !(*node)._private.is_null() {
            let p = (*node)._private as *mut NodePtr;
            object.node.set(p);
            (*p).refcount += 1;
            if (*p)._private.is_null() {
                (*p)._private = private_data;
            }
            (*p).refcount
        } else {
            let p = Box::into_raw(Box::new(NodePtr {
                node,
                refcount: 1,
                _private: private_data,
            }));
            object.node.set(p);
            (*node)._private = p.cast();
            1
        }
    }
}

/// php_libxml_decrement_node_ptr_ref
///
/// # Safety
/// `p` is a live node pointer.
pub unsafe fn decrement_node_ptr_ref(p: *mut NodePtr) -> u32 {
    unsafe {
        (*p).refcount -= 1;
        let rc = (*p).refcount;
        if rc == 0 {
            if !(*p).node.is_null() {
                (*(*p).node)._private = ptr::null_mut();
            }
            if let Some(obj) = (*p)._private.as_ref() {
                obj.node.set(ptr::null_mut());
            }
            drop(Box::from_raw(p));
        }
        rc
    }
}

/// php_libxml_decrement_node_ptr
pub fn decrement_node_ptr(object: &NodeObject) -> u32 {
    let p = object.node.get();
    if p.is_null() {
        return 0;
    }
    unsafe { decrement_node_ptr_ref(p) }
}

/// php_libxml_increment_doc_ref
pub fn increment_doc_ref(object: &NodeObject, docp: *mut XmlDoc) -> u32 {
    let cur = object.document.get();
    if !cur.is_null() {
        unsafe {
            (*cur).refcount += 1;
            (*cur).refcount
        }
    } else if !docp.is_null() {
        object.document.set(Box::into_raw(Box::new(RefObj {
            ptr: docp,
            doc_props: None,
            // Iterators start at 0, so they start out stale.
            cache_tag: 1,
            refcount: 1,
            class_type: ClassType::Unset,
            private_data: ptr::null_mut(),
            quirks_mode: QuirksMode::NoQuirks,
        })));
        1
    } else {
        0
    }
}

/// php_libxml_decrement_doc_ref_directly: the last reference frees the
/// document.
///
/// # Safety
/// `document` is a live [`RefObj`].
pub unsafe fn decrement_doc_ref_directly(document: *mut RefObj) -> u32 {
    unsafe {
        (*document).refcount -= 1;
        let rc = (*document).refcount;
        if rc == 0 {
            let d = Box::from_raw(document);
            if !d.private_data.is_null() {
                ((*d.private_data).dtor)(d.private_data);
            }
            if !d.ptr.is_null() {
                sys::xmlFreeDoc(d.ptr);
            }
        }
        rc
    }
}

/// php_libxml_decrement_doc_ref
pub fn decrement_doc_ref(object: &NodeObject) -> u32 {
    let d = object.document.get();
    if d.is_null() {
        return 0;
    }
    object.document.set(ptr::null_mut());
    unsafe { decrement_doc_ref_directly(d) }
}

/// php_libxml_node_free_resource: free `node` if it is not in a tree.
///
/// # Safety
/// `node` is NULL or a live node.
pub unsafe fn node_free_resource(node: *mut XmlNode) {
    if node.is_null() {
        return;
    }
    unsafe {
        match (*node).type_ {
            sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE => {}
            sys::XML_ENTITY_REF_NODE => {
                // Its children are the entity's, not its own.
                unregister_node(node);
                if (*node).parent.is_null() {
                    node_free(node);
                }
            }
            t => {
                if (*node).parent.is_null() || t == sys::XML_NAMESPACE_DECL {
                    node_free_list((*node).children);
                    if t == sys::XML_ELEMENT_NODE {
                        node_free_list((*node).properties.cast());
                    }
                    unregister_node(node);
                    node_free(node);
                } else {
                    unregister_node(node);
                }
            }
        }
    }
}

/// php_libxml_node_decrement_resource: an object lets go of its node and
/// document.
pub fn node_decrement_resource(object: &NodeObject) {
    let obj_node = object.node.get();
    if !obj_node.is_null() {
        unsafe {
            let nodep = (*obj_node).node;
            let rc = decrement_node_ptr(object);
            if rc == 0 {
                node_free_resource(nodep);
            } else if ptr::eq((*obj_node)._private, object) {
                (*obj_node)._private = ptr::null();
            }
        }
        object.node.set(ptr::null_mut());
    }
    if !object.document.get().is_null() {
        decrement_doc_ref(object);
    }
}

/// php_libxml_attr_value
///
/// # Safety
/// `attr` is a live attribute.
pub unsafe fn attr_value(attr: *const XmlAttr) -> Vec<u8> {
    unsafe {
        let children = (*attr).children;
        if children.is_null() {
            return Vec::new();
        }
        if (*children).type_ == sys::XML_TEXT_NODE && (*children).next.is_null() {
            return sys::cstr((*children).content).to_vec();
        }
        sys::take_string(sys::xmlNodeGetContent(attr.cast())).unwrap_or_default()
    }
}

/// php_libxml_invalidate_node_list_cache
pub fn invalidate_node_list_cache(document: *mut RefObj) {
    if let Some(d) = unsafe { document.as_mut() } {
        d.cache_tag += 1;
    }
}

/// The [`NodeObject`] registered for `node` (`php_dom_object_get_data`).
///
/// # Safety
/// `node` is NULL or a live node.
pub unsafe fn object_of_node<'a>(node: *const XmlNode) -> Option<&'a NodeObject> {
    unsafe {
        if node.is_null() || (*node)._private.is_null() {
            return None;
        }
        (*((*node)._private as *const NodePtr))._private.as_ref()
    }
}

/// The document of `docp`'s registered object
/// (php_libxml_invalidate_node_list_cache_from_doc).
pub fn invalidate_node_list_cache_from_doc(docp: *mut XmlDoc) {
    if let Some(obj) = unsafe { object_of_node(docp.cast()) } {
        invalidate_node_list_cache(obj.document.get());
    }
}

unsafe extern "C" fn write_vec(
    ctx: *mut std::ffi::c_void,
    buf: *const std::ffi::c_char,
    len: std::ffi::c_int,
) -> std::ffi::c_int {
    let v = unsafe { &mut *(ctx as *mut Vec<u8>) };
    if len > 0 {
        v.extend_from_slice(unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), len as usize) });
    }
    len
}

/// php_libxml_default_dump_node_to_str
pub fn dump_node_to_str(
    docp: *mut XmlDoc,
    node: *mut XmlNode,
    format: bool,
    encoding: *const u8,
) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    unsafe {
        let buf = sys::xmlOutputBufferCreateIO(
            Some(write_vec),
            None,
            (&mut out as *mut Vec<u8>).cast(),
            ptr::null_mut(),
        );
        if buf.is_null() {
            return None;
        }
        sys::xmlNodeDumpOutput(buf, docp, node, 0, format as i32, encoding.cast());
        if sys::xmlOutputBufferFlush(buf) < 0 {
            sys::xmlOutputBufferClose(buf);
            return None;
        }
        sys::xmlOutputBufferClose(buf);
    }
    Some(out)
}

/// php_libxml_default_dump_doc_to_str
pub fn dump_doc_to_str(docp: *mut XmlDoc, options: i32, encoding: *const u8) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    unsafe {
        let ctxt = sys::xmlSaveToIO(
            Some(write_vec),
            None,
            (&mut out as *mut Vec<u8>).cast(),
            encoding.cast(),
            options,
        );
        if ctxt.is_null() {
            return None;
        }
        let mut status = sys::xmlSaveDoc(ctxt, docp);
        status |= sys::xmlSaveClose(ctxt) as std::ffi::c_long;
        if status < 0 {
            return None;
        }
    }
    Some(out)
}

/// php_libxml_default_dump_doc_to_file: bytes written, or -1.
///
/// # Safety
/// `doc` is a live document; `filename` is NUL-terminated.
pub unsafe fn dump_doc_to_file(filename: &[u8], doc: *mut XmlDoc, format: bool) -> i64 {
    unsafe {
        sys::xmlSaveFormatFileEnc(
            filename.as_ptr().cast(),
            doc,
            (*doc).encoding.cast(),
            format as i32,
        ) as i64
    }
}

/// php_libxml_dump_node_to_file: bytes written, or -1.
///
/// # Safety
/// `doc` and `node` are live; `filename` is NUL-terminated.
pub unsafe fn dump_node_to_file(
    filename: &[u8],
    doc: *mut XmlDoc,
    node: *mut XmlNode,
    format: bool,
) -> i64 {
    unsafe {
        let outbuf =
            sys::xmlOutputBufferCreateFilename(filename.as_ptr().cast(), ptr::null_mut(), 0);
        if outbuf.is_null() {
            return -1;
        }
        sys::xmlNodeDumpOutput(outbuf, doc, node, 0, format as i32, ptr::null());
        sys::xmlOutputBufferClose(outbuf) as i64
    }
}
