//! The `dom` extension's legacy API (DOMDocument, the DOMNode family,
//! DOMNodeList, DOMNamedNodeMap, DOMXPath, DOMImplementation) over real
//! libxml2 trees, ported from php-src ext/dom (PHP License 3.01).
//!
//! The classes are PHP source (`prelude.php`); every method calls
//! `__dom("Class::method", $this, ...)`, whose name is also the function
//! php names in its warnings. Property reads and writes go through the
//! prelude's magic accessors to `__dom_prop`, which runs php's property
//! handlers ([`props`]). An object's native state is its payload, a
//! [`DomObject`]: for a node php's `dom_object` (node pointer and document
//! reference, see [`crate::ext::libxml::node`]); for a list its map
//! ([`lists`]); for DOMXPath its context ([`xpath`]).
//!
//! The modern `Dom\` API (prelude_modern.php) shares these handlers,
//! branching where php's `php_dom_follow_spec_intern` does; its own parts
//! are in [`modern`], [`collections`], [`nscompat`] (the namespace mapper),
//! the serializers and [`html5`] (lexbor's HTML5 parser and selector
//! engine, through the C glue in `html5c/`).

mod chardata;
mod collections;
mod document;
mod element;
mod html5;
mod html5_serializer;
mod lists;
mod modern;
mod node;
mod nscompat;
mod props;
mod util;
mod xml_serializer;
mod xpath;

pub(crate) use document::valid_file_path;
pub(crate) use props::has_declared_property;

use std::cell::{Cell, RefCell};
use std::ptr;
use std::rc::Rc;

use rhp_values::{conv, ObjectHandle, PhpArray, Value};

use crate::ext::libxml::node::{self as lxn, NodeObject};
use crate::ext::libxml::sys::{self, XmlNode};
use crate::zend::object_handlers;
use crate::zend::object_handlers::{ClassHandlers, CloneObj, ObjectHandlers, PropPurpose};
use crate::{Host, Thrown};

/// php's `dom_object`, as an object's payload.
pub(crate) struct DomObject {
    pub base: NodeObject,
    pub kind: Kind,
    /// The DOM class whose property table applies (`prop_handler`), found
    /// on first property access.
    pub prop_class: Cell<Option<&'static str>>,
}

/// What else a `dom_object` carries, by class.
pub(crate) enum Kind {
    /// The DOMNode family.
    Node,
    /// DOMNameSpaceNode: the element it was read from, kept alive
    /// (`parent_intern`).
    NsNode(RefCell<Option<Value>>),
    /// DOMNodeList, DOMNamedNodeMap.
    Map(RefCell<lists::Map>),
    /// DOMXPath.
    XPath(RefCell<xpath::XPathState>),
    /// DOMImplementation.
    Plain,
    /// Dom\TokenList.
    TokenList(RefCell<collections::TokenListState>),
}

impl DomObject {
    pub fn new(kind: Kind) -> DomObject {
        DomObject {
            base: NodeObject::default(),
            kind,
            prop_class: Cell::new(None),
        }
    }

    /// The wrapped node (`dom_object_get_node`).
    pub fn node(&self) -> *mut XmlNode {
        self.base.xml_node()
    }

    pub fn document(&self) -> *mut lxn::RefObj {
        self.base.document.get()
    }

    /// `dom_get_strict_error`
    pub fn strict(&self) -> bool {
        lxn::doc_props_read_only(self.document()).stricterror
    }
}

impl Drop for DomObject {
    fn drop(&mut self) {
        match &self.kind {
            Kind::Node => free_node_storage(&self.base),
            Kind::NsNode(parent) => {
                let p = parent.borrow_mut().take();
                drop(p);
                free_node_storage(&self.base);
            }
            Kind::Map(m) => {
                m.borrow_mut().release();
                lxn::decrement_doc_ref(&self.base);
            }
            Kind::XPath(x) => {
                x.borrow_mut().free_context();
                lxn::decrement_doc_ref(&self.base);
            }
            Kind::Plain | Kind::TokenList(_) => {}
        }
    }
}

/// dom_objects_free_storage
fn free_node_storage(base: &NodeObject) {
    let p = base.node.get();
    if p.is_null() {
        return;
    }
    unsafe {
        let node = (*p).node;
        if node.is_null() {
            // php leaves such a node pointer alone; it must at least stop
            // naming this object, which is going away.
            if ptr::eq((*p)._private, base) {
                (*p)._private = ptr::null();
            }
            return;
        }
        let t = (*node).type_;
        if t != sys::XML_DOCUMENT_NODE && t != sys::XML_HTML_DOCUMENT_NODE {
            lxn::node_decrement_resource(base);
        } else {
            lxn::decrement_node_ptr(base);
            lxn::decrement_doc_ref(base);
        }
    }
    base.node.set(ptr::null_mut());
}

/// A builtin's view of one call: the host, and the function php names in
/// its warnings.
pub(crate) struct Cx<'h> {
    pub host: &'h mut dyn Host,
    pub fname: String,
    /// `DOM_G(suppress_warnings)`: set while building the debug view.
    pub quiet: bool,
    /// `fname` is the active user function, looked up when first needed.
    pub lazy_fname: bool,
}

impl Cx<'_> {
    /// The function php's warnings name. A property handler's is the user
    /// function running (found on first use: it walks the stack).
    pub fn fname(&mut self) -> String {
        if self.fname.is_empty() && self.lazy_fname {
            self.fname = crate::ext::libxml::active_user_function(self.host);
            self.lazy_fname = false;
        }
        self.fname.clone()
    }

    /// `php_error_docref(NULL, E_WARNING, msg)`.
    pub fn warn(&mut self, msg: &str) -> Result<(), Thrown> {
        let fname = self.fname();
        crate::warn(self.host, &fname, msg)
    }

    /// `php_libxml_issue_error(E_WARNING, msg)`: into the libxml error list
    /// while internal errors are on.
    pub fn issue_error(&mut self, msg: &str) -> Result<(), Thrown> {
        if crate::ext::libxml::push_internal_error(msg) {
            return Ok(());
        }
        self.warn(msg)
    }

    /// Run a libxml2 call whose callbacks may reach PHP.
    pub fn with_libxml<R>(&mut self, f: impl FnOnce() -> R) -> Result<R, Thrown> {
        let fname = self.fname();
        crate::ext::libxml::ffi::with_host(self.host, &fname, f)
    }

    /// [`Self::with_libxml`] for a call whose result owns memory: when a
    /// callback threw, `free` releases the result before the exception
    /// goes on.
    pub fn with_libxml_owned<R>(
        &mut self,
        f: impl FnOnce() -> R,
        free: impl FnOnce(R),
    ) -> Result<R, Thrown> {
        let fname = self.fname();
        match crate::ext::libxml::ffi::with_host_keep(self.host, &fname, f) {
            (r, Some(t)) => {
                free(r);
                Err(t)
            }
            (r, None) => Ok(r),
        }
    }
}

/// The payload of a DOM object.
pub(crate) fn intern_of(v: &Value) -> Option<Rc<DomObject>> {
    object_handlers::payload::<DomObject>(v)
}

/// The class name of `v` (for messages).
pub(crate) fn class_of(host: &dyn Host, v: &Value) -> String {
    String::from_utf8_lossy(&host.object_class_name(v).unwrap_or_default()).into_owned()
}

/// DOM_GET_INTERN: the payload, or "Couldn't fetch".
pub(crate) fn get_intern(cx: &mut Cx, v: &Value) -> Result<Rc<DomObject>, Thrown> {
    let ok = intern_of(v).filter(|d| match &d.kind {
        Kind::Node | Kind::NsNode(_) => !d.base.node.get().is_null(),
        Kind::XPath(x) => !x.borrow().ctx.is_null(),
        Kind::Map(_) | Kind::TokenList(_) => true,
        Kind::Plain => false,
    });
    ok.ok_or_else(|| Thrown::error("Error", format!("Couldn't fetch {}", class_of(cx.host, v))))
}

/// DOM_GET_OBJ: the payload and its node.
pub(crate) fn get_obj(cx: &mut Cx, v: &Value) -> Result<(Rc<DomObject>, *mut XmlNode), Thrown> {
    let d = get_intern(cx, v)?;
    let n = d.node();
    if n.is_null() {
        return Err(Thrown::error(
            "Error",
            format!("Couldn't fetch {}", class_of(cx.host, v)),
        ));
    }
    Ok((d, n))
}

/// The payload of `this`, created if the object has none yet (a
/// constructor), or an error for an object of no DOM class.
pub(crate) fn ensure_intern(v: &Value, kind: impl FnOnce() -> Kind) -> Option<Rc<DomObject>> {
    if let Some(d) = intern_of(v) {
        return Some(d);
    }
    let h = object_handlers::object_of(v)?;
    let d = Rc::new(DomObject::new(kind()));
    *d.base.this.borrow_mut() = Rc::downgrade(&h);
    object_handlers::set_payload(&h, d.clone());
    Some(d)
}

// DOMException codes (dom_exception_code)
pub const INDEX_SIZE_ERR: i64 = 1;
pub const HIERARCHY_REQUEST_ERR: i64 = 3;
pub const WRONG_DOCUMENT_ERR: i64 = 4;
pub const INVALID_CHARACTER_ERR: i64 = 5;
pub const NO_MODIFICATION_ALLOWED_ERR: i64 = 7;
pub const NOT_FOUND_ERR: i64 = 8;
pub const NOT_SUPPORTED_ERR: i64 = 9;
pub const INVALID_STATE_ERR: i64 = 11;
pub const SYNTAX_ERR: i64 = 12;
pub const INVALID_MODIFICATION_ERR: i64 = 13;
pub const NAMESPACE_ERR: i64 = 14;

fn err_message(code: i64) -> &'static str {
    match code {
        1 => "Index Size Error",
        2 => "DOM String Size Error",
        3 => "Hierarchy Request Error",
        4 => "Wrong Document Error",
        5 => "Invalid Character Error",
        6 => "No Data Allowed Error",
        7 => "No Modification Allowed Error",
        8 => "Not Found Error",
        9 => "Not Supported Error",
        10 => "Inuse Attribute Error",
        11 => "Invalid State Error",
        12 => "Syntax Error",
        13 => "Invalid Modification Error",
        14 => "Namespace Error",
        15 => "Invalid Access Error",
        16 => "Validation Error",
        _ => "Unhandled Error",
    }
}

/// php_dom_throw_error_with_message: a DOMException, or a warning when
/// strictErrorChecking is off. Returns php's `false` for the latter.
pub(crate) fn throw_msg(cx: &mut Cx, code: i64, msg: &str, strict: bool) -> Result<Value, Thrown> {
    if strict {
        let args = vec![Value::str(msg), Value::Int(code)];
        return Err(
            match cx.host.reflect_new_instance(b"DOMException", args, true) {
                Ok(Value::Object(h)) => Thrown::Object(h),
                Ok(_) => Thrown::error("DOMException", msg),
                Err(t) => t,
            },
        );
    }
    cx.issue_error(msg)?;
    Ok(Value::Bool(false))
}

/// php_dom_throw_error
pub(crate) fn throw(cx: &mut Cx, code: i64, strict: bool) -> Result<Value, Thrown> {
    throw_msg(cx, code, err_message(code), strict)
}

/// The modern class php gives a node (php_dom_create_object).
fn modern_class_for(node: *mut XmlNode) -> Option<&'static str> {
    let t = unsafe { (*node).type_ };
    Some(match t {
        sys::XML_DOCUMENT_NODE => "Dom\\XMLDocument",
        sys::XML_HTML_DOCUMENT_NODE => "Dom\\HTMLDocument",
        sys::XML_DTD_NODE | sys::XML_DOCUMENT_TYPE_NODE => "Dom\\DocumentType",
        sys::XML_ELEMENT_NODE => {
            if nscompat::ns_is_fast(node, &nscompat::HTML) {
                "Dom\\HTMLElement"
            } else {
                "Dom\\Element"
            }
        }
        sys::XML_ATTRIBUTE_NODE => "Dom\\Attr",
        sys::XML_TEXT_NODE => "Dom\\Text",
        sys::XML_COMMENT_NODE => "Dom\\Comment",
        sys::XML_PI_NODE => "Dom\\ProcessingInstruction",
        sys::XML_ENTITY_REF_NODE => "Dom\\EntityReference",
        sys::XML_ENTITY_DECL | sys::XML_ELEMENT_DECL => "Dom\\Entity",
        sys::XML_CDATA_SECTION_NODE => "Dom\\CDATASection",
        sys::XML_DOCUMENT_FRAG_NODE => "Dom\\DocumentFragment",
        sys::XML_NOTATION_NODE => "Dom\\Notation",
        sys::XML_NAMESPACE_DECL => "DOMNameSpaceNode",
        _ => return None,
    })
}

/// The DOM class php gives a node of this type (php_dom_create_object).
fn class_for_type(t: i32) -> Option<&'static str> {
    Some(match t {
        sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE => "DOMDocument",
        sys::XML_DTD_NODE | sys::XML_DOCUMENT_TYPE_NODE => "DOMDocumentType",
        sys::XML_ELEMENT_NODE => "DOMElement",
        sys::XML_ATTRIBUTE_NODE => "DOMAttr",
        sys::XML_TEXT_NODE => "DOMText",
        sys::XML_COMMENT_NODE => "DOMComment",
        sys::XML_PI_NODE => "DOMProcessingInstruction",
        sys::XML_ENTITY_REF_NODE => "DOMEntityReference",
        sys::XML_ENTITY_DECL | sys::XML_ELEMENT_DECL => "DOMEntity",
        sys::XML_CDATA_SECTION_NODE => "DOMCdataSection",
        sys::XML_DOCUMENT_FRAG_NODE => "DOMDocumentFragment",
        sys::XML_NOTATION_NODE => "DOMNotation",
        sys::XML_NAMESPACE_DECL => "DOMNameSpaceNode",
        _ => return None,
    })
}

/// php_dom_create_object: the object for `node`, the one already wrapping
/// it if any. A new object shares `domobj`'s document reference and class
/// map.
pub(crate) fn create_object(
    cx: &mut Cx,
    node: *mut XmlNode,
    domobj: Option<&DomObject>,
) -> Result<Value, Thrown> {
    create_object_in(cx, node, domobj.map_or(ptr::null_mut(), |d| d.document()))
}

/// [`create_object`] with the document reference of any libxml-backed
/// object (`DOM_RET_OBJ` with a SimpleXML or XMLReader object).
pub(crate) fn create_object_in(
    cx: &mut Cx,
    node: *mut XmlNode,
    document: *mut lxn::RefObj,
) -> Result<Value, Thrown> {
    if let Some(obj) = unsafe { lxn::object_of_node(node) } {
        if let Some(v) = obj.object() {
            return Ok(v);
        }
    }
    let t = unsafe { (*node).type_ };
    let base = if nscompat::follow_spec(document) {
        modern_class_for(node)
    } else {
        class_for_type(t)
    };
    let Some(base) = base else {
        return Err(Thrown::error(
            "Error",
            format!("Unsupported node type: {t}"),
        ));
    };
    let mut class = base.as_bytes().to_vec();
    if let Some(c) = classmap_lookup(document, base) {
        class = c;
    }
    instantiate(cx, &class, node, document)
}

/// php_dom_create_nullable_object
pub(crate) fn create_nullable(
    cx: &mut Cx,
    node: *mut XmlNode,
    domobj: Option<&DomObject>,
) -> Result<Value, Thrown> {
    if node.is_null() {
        Ok(Value::Null)
    } else {
        create_object(cx, node, domobj)
    }
}

/// php_dom_instantiate_object_helper
pub(crate) fn instantiate(
    cx: &mut Cx,
    class: &[u8],
    node: *mut XmlNode,
    document: *mut lxn::RefObj,
) -> Result<Value, Thrown> {
    let internal = class_for_type_name(class);
    let v = if internal {
        cx.host.instantiate(class, Vec::new())
    } else {
        Some(cx.host.reflect_new_instance(class, Vec::new(), false)?)
    };
    let Some(v) = v else {
        return Err(cx
            .host
            .take_pending_throw()
            .unwrap_or_else(|| Thrown::error("Error", "Cannot instantiate DOM node")));
    };
    let is_ns = unsafe { (*node).type_ } == sys::XML_NAMESPACE_DECL;
    let d = ensure_intern(&v, || {
        if is_ns {
            Kind::NsNode(RefCell::new(None))
        } else {
            Kind::Node
        }
    })
    .expect("a new DOM object");
    unsafe {
        if !(*node).doc.is_null() {
            if !document.is_null() {
                d.base.document.set(document);
            }
            lxn::increment_doc_ref(&d.base, (*node).doc);
        }
    }
    let me: *const NodeObject = &d.base;
    lxn::increment_node_ptr(&d.base, node, me);
    // An object whose last reference went while a builtin held its state
    // still names itself in the node; this one takes over.
    unsafe {
        let p = (*node)._private as *mut lxn::NodePtr;
        if !p.is_null() {
            if let Some(old) = (*p)._private.as_ref() {
                if !ptr::eq(old, me) && old.object().is_none() {
                    (*p)._private = me;
                }
            }
        }
    }
    Ok(v)
}

fn class_for_type_name(class: &[u8]) -> bool {
    matches!(
        class,
        b"DOMDocument"
            | b"DOMDocumentType"
            | b"DOMElement"
            | b"DOMAttr"
            | b"DOMText"
            | b"DOMComment"
            | b"DOMProcessingInstruction"
            | b"DOMEntityReference"
            | b"DOMEntity"
            | b"DOMCdataSection"
            | b"DOMDocumentFragment"
            | b"DOMNotation"
            | b"DOMNameSpaceNode"
            | b"DOMNodeList"
            | b"DOMNamedNodeMap"
            | b"DOMImplementation"
    ) || class.starts_with(b"Dom\\")
}

/// dom_get_doc_classmap
fn classmap_lookup(document: *mut lxn::RefObj, base: &str) -> Option<Vec<u8>> {
    let d = unsafe { document.as_ref() }?;
    let props = d.doc_props.as_ref()?;
    let lc = base.to_ascii_lowercase();
    props
        .classmap
        .iter()
        .find(|(b, _)| b.as_slice() == lc.as_bytes())
        .map(|(_, c)| c.clone())
}

// ---- arguments ----

pub(crate) use rhp_values::ops::deref;

pub(crate) fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(deref).unwrap_or(Value::Null)
}

pub(crate) fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&arg(args, i))
}

pub(crate) fn opt_bytes(args: &[Value], i: usize) -> Option<Vec<u8>> {
    match arg(args, i) {
        Value::Null => None,
        v => Some(conv::to_string_bytes(&v)),
    }
}

pub(crate) fn int(args: &[Value], i: usize) -> i64 {
    conv::to_int(&arg(args, i))
}

pub(crate) fn boolean(args: &[Value], i: usize) -> bool {
    conv::to_bool(&arg(args, i))
}

/// A libxml2 string as a PHP string ("" for NULL).
pub(crate) fn xs(p: *const u8) -> Value {
    Value::str(unsafe { sys::cstr(p) })
}

/// A libxml2 string, or null.
pub(crate) fn xs_opt(p: *const u8) -> Value {
    if p.is_null() {
        Value::Null
    } else {
        xs(p)
    }
}

// ---- the dispatchers ----

type Method = fn(&mut Cx, &Value, &[Value]) -> Result<Value, Thrown>;

fn method(name: &[u8]) -> Option<Method> {
    if let Some(m) = modern_method(name) {
        return Some(m);
    }
    use chardata as c;
    use document as d;
    use element as e;
    use lists as l;
    use node as n;
    use xpath as x;
    Some(match name {
        b"DOMNode::appendChild" => n::append_child,
        b"DOMNode::C14N" => n::c14n,
        b"DOMNode::C14NFile" => n::c14n_file,
        b"DOMNode::cloneNode" => n::clone_node,
        b"DOMNode::getLineNo" => n::get_line_no,
        b"DOMNode::getNodePath" => n::get_node_path,
        b"DOMNode::hasAttributes" => n::has_attributes,
        b"DOMNode::hasChildNodes" => n::has_child_nodes,
        b"DOMNode::insertBefore" => n::insert_before,
        b"DOMNode::isDefaultNamespace" => n::is_default_namespace,
        b"DOMNode::isSameNode" => n::is_same_node,
        b"DOMNode::isEqualNode" => n::is_equal_node,
        b"DOMNode::isSupported" => n::is_supported,
        b"DOMNode::lookupNamespaceURI" => n::lookup_namespace_uri,
        b"DOMNode::lookupPrefix" => n::lookup_prefix,
        b"DOMNode::normalize" => n::normalize,
        b"DOMNode::removeChild" => n::remove_child,
        b"DOMNode::replaceChild" => n::replace_child,
        b"DOMNode::contains" => n::contains,
        b"DOMNode::getRootNode" => n::get_root_node,
        b"DOMNode::compareDocumentPosition" => n::compare_document_position,
        b"DOMNode::__sleep" => n::sleep,
        b"DOMNode::__wakeup" => n::wakeup,
        b"DOMImplementation::hasFeature" => c::impl_has_feature,
        b"DOMImplementation::createDocumentType" => c::impl_create_document_type,
        b"DOMImplementation::createDocument" => c::impl_create_document,
        b"DOMDocumentFragment::__construct" => c::fragment_construct,
        b"DOMDocumentFragment::appendXML" => c::fragment_append_xml,
        b"DOMDocumentFragment::append" | b"DOMElement::append" | b"DOMDocument::append" => {
            e::append
        }
        b"DOMDocumentFragment::prepend" | b"DOMElement::prepend" | b"DOMDocument::prepend" => {
            e::prepend
        }
        b"DOMDocumentFragment::replaceChildren"
        | b"DOMElement::replaceChildren"
        | b"DOMDocument::replaceChildren" => e::replace_children,
        b"DOMNodeList::count" => l::nodelist_count,
        b"DOMNodeList::getIterator" | b"DOMNamedNodeMap::getIterator" => l::get_iterator_m,
        b"DOMNodeList::item" => l::nodelist_item,
        b"DOMNodeList::offsetGet" => l::nodelist_offset_get,
        b"DOMNodeList::offsetExists" => l::nodelist_offset_exists,
        b"DOMNamedNodeMap::getNamedItem" => l::map_get_named_item,
        b"DOMNamedNodeMap::getNamedItemNS" => l::map_get_named_item_ns,
        b"DOMNamedNodeMap::item" => l::map_item,
        b"DOMNamedNodeMap::count" => l::map_count,
        b"DOMNamedNodeMap::offsetGet" => l::map_offset_get,
        b"DOMNamedNodeMap::offsetExists" => l::map_offset_exists,
        b"DOMCharacterData::appendData" => c::append_data,
        b"DOMCharacterData::substringData" => c::substring_data,
        b"DOMCharacterData::insertData" => c::insert_data,
        b"DOMCharacterData::deleteData" => c::delete_data,
        b"DOMCharacterData::replaceData" => c::replace_data,
        b"DOMCharacterData::replaceWith" | b"DOMElement::replaceWith" => e::replace_with,
        b"DOMCharacterData::remove" | b"DOMElement::remove" => e::remove,
        b"DOMCharacterData::before" | b"DOMElement::before" => e::before,
        b"DOMCharacterData::after" | b"DOMElement::after" => e::after,
        b"DOMText::__construct" => c::text_construct,
        b"DOMText::isWhitespaceInElementContent" | b"DOMText::isElementContentWhitespace" => {
            c::text_is_whitespace
        }
        b"DOMText::splitText" => c::text_split,
        b"DOMCdataSection::__construct" => c::cdata_construct,
        b"DOMComment::__construct" => c::comment_construct,
        b"DOMAttr::__construct" => c::attr_construct,
        b"DOMAttr::isId" => c::attr_is_id,
        b"DOMEntityReference::__construct" => c::entityref_construct,
        b"DOMProcessingInstruction::__construct" => c::pi_construct,
        b"DOMElement::__construct" => e::construct,
        b"DOMElement::getAttribute" => e::get_attribute,
        b"DOMElement::getAttributeNames" => e::get_attribute_names,
        b"DOMElement::getAttributeNS" => e::get_attribute_ns,
        b"DOMElement::getAttributeNode" => e::get_attribute_node,
        b"DOMElement::getAttributeNodeNS" => e::get_attribute_node_ns,
        b"DOMElement::getElementsByTagName" | b"DOMDocument::getElementsByTagName" => {
            e::get_elements_by_tag_name
        }
        b"DOMElement::getElementsByTagNameNS" | b"DOMDocument::getElementsByTagNameNS" => {
            e::get_elements_by_tag_name_ns
        }
        b"DOMElement::hasAttribute" => e::has_attribute,
        b"DOMElement::hasAttributeNS" => e::has_attribute_ns,
        b"DOMElement::removeAttribute" => e::remove_attribute,
        b"DOMElement::removeAttributeNS" => e::remove_attribute_ns,
        b"DOMElement::removeAttributeNode" => e::remove_attribute_node,
        b"DOMElement::setAttribute" => e::set_attribute,
        b"DOMElement::setAttributeNS" => e::set_attribute_ns,
        b"DOMElement::setAttributeNode" => e::set_attribute_node,
        b"DOMElement::setAttributeNodeNS" => e::set_attribute_node_ns,
        b"DOMElement::setIdAttribute" => e::set_id_attribute,
        b"DOMElement::setIdAttributeNS" => e::set_id_attribute_ns,
        b"DOMElement::setIdAttributeNode" => e::set_id_attribute_node,
        b"DOMElement::toggleAttribute" => e::toggle_attribute,
        b"DOMElement::insertAdjacentElement" => e::insert_adjacent_element,
        b"DOMElement::insertAdjacentText" => e::insert_adjacent_text,
        b"DOMDocument::__construct" => d::construct,
        b"DOMDocument::createAttribute" => d::create_attribute,
        b"DOMDocument::createAttributeNS" => d::create_attribute_ns,
        b"DOMDocument::createCDATASection" => d::create_cdata_section,
        b"DOMDocument::createComment" => d::create_comment,
        b"DOMDocument::createDocumentFragment" => d::create_document_fragment,
        b"DOMDocument::createElement" => d::create_element,
        b"DOMDocument::createElementNS" => d::create_element_ns,
        b"DOMDocument::createEntityReference" => d::create_entity_reference,
        b"DOMDocument::createProcessingInstruction" => d::create_processing_instruction,
        b"DOMDocument::createTextNode" => d::create_text_node,
        b"DOMDocument::getElementById" => d::get_element_by_id,
        b"DOMDocument::importNode" => d::import_node,
        b"DOMDocument::load" => d::load,
        b"DOMDocument::loadXML" => d::load_xml,
        b"DOMDocument::normalizeDocument" => d::normalize_document,
        b"DOMDocument::registerNodeClass" => d::register_node_class,
        b"DOMDocument::save" => d::save,
        b"DOMDocument::loadHTML" => d::load_html,
        b"DOMDocument::loadHTMLFile" => d::load_html_file,
        b"DOMDocument::saveHTML" => d::save_html,
        b"DOMDocument::saveHTMLFile" => d::save_html_file,
        b"DOMDocument::saveXML" => d::save_xml,
        b"DOMDocument::schemaValidate" => d::schema_validate,
        b"DOMDocument::schemaValidateSource" => d::schema_validate_source,
        b"DOMDocument::relaxNGValidate" => d::relaxng_validate,
        b"DOMDocument::relaxNGValidateSource" => d::relaxng_validate_source,
        b"DOMDocument::validate" => d::validate,
        b"DOMDocument::xinclude" => d::xinclude,
        b"DOMDocument::adoptNode" => d::adopt_node,
        b"DOMXPath::__construct" => x::construct,
        b"DOMXPath::evaluate" => x::evaluate,
        b"DOMXPath::query" => x::query,
        b"DOMXPath::registerNamespace" => x::register_namespace,
        b"DOMXPath::registerPhpFunctions" => x::register_php_functions,
        b"DOMXPath::registerPhpFunctionNS" => x::register_php_function_ns,
        b"DOMXPath::quote" => x::quote,
        _ => return None,
    })
}

/// The `Dom\` methods: their own ([`modern`], [`collections`]) or the
/// handler php shares with the legacy class.
fn modern_method(name: &[u8]) -> Option<Method> {
    if !name.starts_with(b"Dom\\") {
        return None;
    }
    use chardata as c;
    use collections as co;
    use element as e;
    use lists as l;
    use modern as m;
    use node as n;
    use xpath as x;
    let name = &name[4..];
    Some(match name {
        b"Attr::isId" => c::attr_is_id,
        b"Attr::rename" | b"Element::rename" => m::rename,
        b"CharacterData::after" | b"Element::after" | b"DocumentType::after" => e::after,
        b"CharacterData::before" | b"Element::before" | b"DocumentType::before" => e::before,
        b"CharacterData::remove" | b"Element::remove" | b"DocumentType::remove" => e::remove,
        b"CharacterData::replaceWith" | b"Element::replaceWith" | b"DocumentType::replaceWith" => {
            e::replace_with
        }
        b"CharacterData::appendData" => c::append_data,
        b"CharacterData::deleteData" => c::delete_data,
        b"CharacterData::insertData" => c::insert_data,
        b"CharacterData::replaceData" => c::replace_data,
        b"CharacterData::substringData" => c::substring_data,
        b"Document::adoptNode" => m::doc_adopt_node,
        b"Document::append" | b"DocumentFragment::append" | b"Element::append" => e::append,
        b"Document::prepend" | b"DocumentFragment::prepend" | b"Element::prepend" => e::prepend,
        b"Document::replaceChildren"
        | b"DocumentFragment::replaceChildren"
        | b"Element::replaceChildren" => e::replace_children,
        b"Document::createAttribute" => m::doc_create_attribute,
        b"Document::createAttributeNS" => m::doc_create_attribute_ns,
        b"Document::createCDATASection" => m::doc_create_cdata_section,
        b"Document::createComment" => document::create_comment,
        b"Document::createDocumentFragment" => document::create_document_fragment,
        b"Document::createElement" => m::doc_create_element,
        b"Document::createElementNS" => m::doc_create_element_ns,
        b"Document::createProcessingInstruction" => m::doc_create_processing_instruction,
        b"Document::createTextNode" => document::create_text_node,
        b"Document::getElementById" => document::get_element_by_id,
        b"Document::getElementsByClassName" | b"Element::getElementsByClassName" => {
            m::get_elements_by_class_name
        }
        b"Document::getElementsByTagName" | b"Element::getElementsByTagName" => {
            m::get_elements_by_tag_name
        }
        b"Document::getElementsByTagNameNS" | b"Element::getElementsByTagNameNS" => {
            m::get_elements_by_tag_name_ns
        }
        b"Document::importLegacyNode" => m::doc_import_legacy_node,
        b"Document::importNode" => m::doc_import_node,
        b"Document::registerNodeClass" => document::register_node_class,
        b"Document::relaxNgValidate" => document::relaxng_validate,
        b"Document::relaxNgValidateSource" => document::relaxng_validate_source,
        b"Document::schemaValidate" => document::schema_validate,
        b"Document::schemaValidateSource" => document::schema_validate_source,
        b"DocumentFragment::appendXml" => c::fragment_append_xml,
        b"Document::querySelector"
        | b"DocumentFragment::querySelector"
        | b"Element::querySelector" => m::query_selector,
        b"Document::querySelectorAll"
        | b"DocumentFragment::querySelectorAll"
        | b"Element::querySelectorAll" => m::query_selector_all,
        b"Element::closest" => m::closest,
        b"Element::matches" => m::matches,
        b"Element::insertAdjacentHTML" => m::insert_adjacent_html,
        b"DtdNamedNodeMap::count" | b"NamedNodeMap::count" => l::map_count,
        b"DtdNamedNodeMap::getNamedItem" | b"NamedNodeMap::getNamedItem" => l::map_get_named_item,
        b"DtdNamedNodeMap::getNamedItemNS" | b"NamedNodeMap::getNamedItemNS" => {
            l::map_get_named_item_ns
        }
        b"DtdNamedNodeMap::item" | b"NamedNodeMap::item" => l::map_item,
        b"Element::getAttribute" => m::el_get_attribute,
        b"Element::getAttributeNames" => m::el_get_attribute_names,
        b"Element::getAttributeNode" => m::el_get_attribute_node,
        b"Element::getAttributeNodeNS" => m::el_get_attribute_node_ns,
        b"Element::getAttributeNS" => m::el_get_attribute_ns,
        b"Element::getDescendantNamespaces" => m::el_get_descendant_namespaces,
        b"Element::getInScopeNamespaces" => m::el_get_in_scope_namespaces,
        b"Element::hasAttribute" => m::el_has_attribute,
        b"Element::hasAttributeNS" => m::el_has_attribute_ns,
        b"Element::hasAttributes" => n::has_attributes,
        b"Element::insertAdjacentElement" => m::insert_adjacent_element,
        b"Element::insertAdjacentText" => m::insert_adjacent_text,
        b"Element::removeAttribute" => m::el_remove_attribute,
        b"Element::removeAttributeNode" => m::el_remove_attribute_node,
        b"Element::removeAttributeNS" => m::el_remove_attribute_ns,
        b"Element::setAttribute" => m::el_set_attribute,
        // php aliases setAttributeNode to setAttributeNodeNS.
        b"Element::setAttributeNode" | b"Element::setAttributeNodeNS" => {
            m::el_set_attribute_node_ns
        }
        b"Element::setAttributeNS" => m::el_set_attribute_ns,
        b"Element::setIdAttribute" => e::set_id_attribute,
        b"Element::setIdAttributeNode" => e::set_id_attribute_node,
        b"Element::setIdAttributeNS" => e::set_id_attribute_ns,
        b"Element::toggleAttribute" => m::el_toggle_attribute,
        b"HTMLCollection::count" | b"NodeList::count" => l::nodelist_count,
        b"HTMLCollection::getIterator"
        | b"NodeList::getIterator"
        | b"NamedNodeMap::getIterator"
        | b"DtdNamedNodeMap::getIterator" => l::get_iterator_m,
        b"HTMLCollection::item" | b"NodeList::item" => l::nodelist_item,
        b"HTMLCollection::namedItem" => co::html_collection_named_item,
        b"HTMLDocument::createEmpty" => m::html_create_empty,
        b"HTMLDocument::createFromFile" => m::html_create_from_file,
        b"HTMLDocument::createFromString" => m::html_create_from_string,
        b"HTMLDocument::saveHtml" => m::save_html,
        b"HTMLDocument::saveHtmlFile" => m::save_html_file,
        b"HTMLDocument::saveXml" | b"XMLDocument::saveXml" => m::save_xml,
        b"HTMLDocument::saveXmlFile" | b"XMLDocument::saveXmlFile" => m::save_xml_file,
        b"Implementation::createDocument" => m::impl_create_document,
        b"Implementation::createDocumentType" => m::impl_create_document_type,
        b"Implementation::createHTMLDocument" => m::impl_create_html_document,
        b"Node::__sleep" => n::sleep,
        b"Node::__wakeup" => n::wakeup,
        b"Node::appendChild" => m::append_child,
        b"Node::C14N" => n::c14n,
        b"Node::C14NFile" => n::c14n_file,
        b"Node::cloneNode" => m::clone_node,
        b"Node::compareDocumentPosition" => n::compare_document_position,
        b"Node::contains" => n::contains,
        b"Node::getLineNo" => n::get_line_no,
        b"Node::getNodePath" => m::get_node_path,
        b"Node::getRootNode" => n::get_root_node,
        b"Node::hasChildNodes" => n::has_child_nodes,
        b"Node::insertBefore" => m::insert_before,
        b"Node::isDefaultNamespace" => m::is_default_namespace,
        b"Node::isEqualNode" => n::is_equal_node,
        b"Node::isSameNode" => n::is_same_node,
        b"Node::lookupNamespaceURI" => m::lookup_namespace_uri,
        b"Node::lookupPrefix" => m::lookup_prefix,
        b"Node::normalize" => m::normalize,
        b"Node::removeChild" => n::remove_child,
        b"Node::replaceChild" => m::replace_child,
        b"Text::splitText" => c::text_split,
        b"TokenList::add" => co::tl_add,
        b"TokenList::contains" => co::tl_contains,
        b"TokenList::count" => co::tl_count,
        b"TokenList::getIterator" => co::tl_get_iterator_m,
        b"TokenList::item" => co::tl_item_m,
        b"TokenList::remove" => co::tl_remove,
        b"TokenList::replace" => co::tl_replace,
        b"TokenList::supports" => co::tl_supports,
        b"TokenList::toggle" => co::tl_toggle,
        b"XMLDocument::createEmpty" => m::xml_create_empty,
        b"XMLDocument::createEntityReference" => document::create_entity_reference,
        b"XMLDocument::createFromFile" => m::xml_create_from_file,
        b"XMLDocument::createFromString" => m::xml_create_from_string,
        b"XMLDocument::validate" => document::validate,
        b"XMLDocument::xinclude" => document::xinclude_modern,
        b"XPath::__construct" => x::construct,
        b"XPath::evaluate" => x::evaluate,
        b"XPath::query" => x::query,
        b"XPath::quote" => x::quote,
        b"XPath::registerNamespace" => x::register_namespace,
        b"XPath::registerPhpFunctionNS" => x::register_php_function_ns,
        b"XPath::registerPhpFunctions" => x::register_php_functions,
        _ => return None,
    })
}

/// `__dom("Class::method", $this, ...$args)`
fn fn_dom(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let Some(m) = method(&name) else {
        return Err(Thrown::error(
            "Error",
            format!(
                "Call to undefined method {}()",
                String::from_utf8_lossy(&name)
            ),
        ));
    };
    let this = arg(args, 1);
    let mut cx = Cx {
        host,
        fname: String::from_utf8_lossy(&name).into_owned(),
        quiet: false,
        lazy_fname: false,
    };
    m(&mut cx, &this, &args[2.min(args.len())..])
}

/// `__dom_prop($op, $this, $name [, $value])`: php's read_property (0),
/// write_property (1), has_property (2) and unset_property (3) for the
/// DOM properties. An empty array means "not a DOM property".
fn fn_dom_prop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = int(args, 0);
    let this = arg(args, 1);
    let name = bytes(args, 2);
    let mut cx = Cx {
        host,
        fname: String::new(),
        quiet: false,
        lazy_fname: true,
    };
    props::dispatch(&mut cx, op, &this, &name, args.get(3).map(deref))
}

/// php_dom_export_node: the node a DOMNode stands for (for
/// php_libxml_import_node), and its document reference.
pub(crate) fn export_node(v: &Value) -> Option<(*mut XmlNode, *mut lxn::RefObj)> {
    let d = intern_of(v)?;
    if !matches!(d.kind, Kind::Node) {
        return None;
    }
    let n = d.node();
    (!n.is_null()).then(|| (n, d.document()))
}

/// The document reference of a libxml-backed object (`nodeobj` in php).
fn node_object_document(v: &Value) -> *mut lxn::RefObj {
    if let Some(d) = intern_of(v) {
        return d.document();
    }
    crate::ext::simplexml::document_of(v)
}

/// dom_import_simplexml
fn fn_dom_import_simplexml(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = arg(args, 0);
    let bad = || {
        Thrown::type_error("dom_import_simplexml(): Argument #1 ($node) is not a valid node type")
    };
    let nodep = crate::ext::libxml::import_node(host, &v)?;
    let document = node_object_document(&v);
    let Some((nodep, _)) = nodep else {
        return Err(bad());
    };
    let t = unsafe { (*nodep).type_ };
    if document.is_null() || !(t == sys::XML_ELEMENT_NODE || t == sys::XML_ATTRIBUTE_NODE) {
        return Err(bad());
    }
    unsafe {
        let old = (*document).class_type;
        if old != lxn::ClassType::Unset && old != lxn::ClassType::Legacy {
            return Err(Thrown::type_error(
                "dom_import_simplexml(): Argument #1 ($node) must not be already imported as a Dom\\Node",
            ));
        }
        (*document).class_type = lxn::ClassType::Legacy;
    }
    let mut cx = Cx {
        host,
        fname: "dom_import_simplexml".into(),
        quiet: false,
        lazy_fname: false,
    };
    create_object_in(&mut cx, nodep, document)
}

/// Dom\\import_simplexml (dom_import_simplexml_common, modern)
fn fn_dom_import_simplexml_modern(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = arg(args, 0);
    let bad = || {
        Thrown::type_error("Dom\\import_simplexml(): Argument #1 ($node) is not a valid node type")
    };
    let nodep = crate::ext::libxml::import_node(host, &v)?;
    let document = node_object_document(&v);
    let Some((nodep, _)) = nodep else {
        return Err(bad());
    };
    let t = unsafe { (*nodep).type_ };
    if document.is_null() || !(t == sys::XML_ELEMENT_NODE || t == sys::XML_ATTRIBUTE_NODE) {
        return Err(bad());
    }
    unsafe {
        let old = (*document).class_type;
        if old != lxn::ClassType::Unset && old != lxn::ClassType::Modern {
            return Err(Thrown::type_error(
                "Dom\\import_simplexml(): Argument #1 ($node) must not be already imported as a DOMNode",
            ));
        }
        if old != lxn::ClassType::Modern {
            (*document).class_type = lxn::ClassType::Modern;
            nscompat::convert_to_modern(document, (*nodep).doc);
        }
    }
    let mut cx = Cx {
        host,
        fname: "Dom\\import_simplexml".into(),
        quiet: false,
        lazy_fname: false,
    };
    create_object_in(&mut cx, nodep, document)
}

// ---- object handlers ----

/// dom_get_debug_info: the properties, then every DOM property's value
/// (objects shown as "(object value omitted)").
fn get_properties_for(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose != PropPurpose::Debug {
        return Ok(None);
    }
    let class = h.borrow().class;
    if host.class_has_method(class, b"__debuginfo") {
        return Ok(None);
    }
    let v = Value::Object(h.clone());
    let mut cx = Cx {
        host,
        fname: String::new(),
        quiet: true,
        lazy_fname: false,
    };
    props::debug_view(&mut cx, &v).map(Some)
}

/// dom_objects_store_clone_obj: a node clones its (deep) libxml2 node.
fn clone_node_obj(
    host: &mut dyn Host,
    src: &ObjectHandle,
    dst: &ObjectHandle,
) -> Result<(), Thrown> {
    let sv = Value::Object(src.clone());
    let dv = Value::Object(dst.clone());
    let mut cx = Cx {
        host,
        fname: String::new(),
        quiet: false,
        lazy_fname: false,
    };
    node::clone_object(&mut cx, &sv, &dv)?;
    // dom_modern_element_clone_obj: the clone gets its own cached
    // $children and $classList.
    let mut d = dst.borrow_mut();
    for name in ["children", "classList"] {
        d.remove_prop(&rhp_values::PhpStr::from_bytes(name.as_bytes()));
    }
    Ok(())
}

/// dom_object_namespace_node_clone_obj
fn clone_ns_obj(host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let sv = Value::Object(src.clone());
    let dv = Value::Object(dst.clone());
    let mut cx = Cx {
        host,
        fname: String::new(),
        quiet: false,
        lazy_fname: false,
    };
    node::clone_ns_object(&mut cx, &sv, &dv)
}

static NODE_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    unset_property: Some(unset_property),
    clone_obj: CloneObj::Custom(clone_node_obj),
    ..ObjectHandlers::STD
};

static NS_NODE_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    unset_property: Some(unset_property),
    clone_obj: CloneObj::Custom(clone_ns_obj),
    ..ObjectHandlers::STD
};

static MAP_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    unset_property: Some(unset_property),
    clone_obj: CloneObj::None,
    ..ObjectHandlers::STD
};

static LIST_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    unset_property: Some(unset_property),
    get_iterator: Some(lists::get_iterator),
    clone_obj: CloneObj::None,
    ..ObjectHandlers::STD
};

static PLAIN_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    unset_property: Some(unset_property),
    ..ObjectHandlers::STD
};

/// "Cannot unset Class::$prop" for a declared DOM property (the prelude's
/// hooks would say "hooked property").
fn unset_property(host: &mut dyn Host, h: &ObjectHandle, name: &[u8]) -> Result<(), Thrown> {
    let class = h.borrow().class;
    if has_declared_property(host, class, name) {
        return Err(Thrown::error(
            "Error",
            format!(
                "Cannot unset {}::${}",
                class_of(host, &Value::Object(h.clone())),
                String::from_utf8_lossy(name)
            ),
        ));
    }
    Ok(())
}

static MODERN_NODE_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    clone_obj: CloneObj::Custom(clone_node_obj),
    unset_property: Some(unset_property),
    ..ObjectHandlers::STD
};

static MODERN_PLAIN_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    clone_obj: CloneObj::None,
    unset_property: Some(unset_property),
    ..ObjectHandlers::STD
};

static MODERN_LIST_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    get_iterator: Some(lists::get_iterator),
    clone_obj: CloneObj::None,
    unset_property: Some(unset_property),
    dimension: Some(&collections::LIST_DIMENSION),
    ..ObjectHandlers::STD
};

static MODERN_MAP_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    get_iterator: Some(lists::get_iterator),
    clone_obj: CloneObj::None,
    unset_property: Some(unset_property),
    dimension: Some(&collections::MAP_DIMENSION),
    ..ObjectHandlers::STD
};

static TOKEN_LIST_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    get_iterator: Some(collections::tl_get_iterator),
    clone_obj: CloneObj::None,
    unset_property: Some(unset_property),
    dimension: Some(&collections::TOKEN_LIST_DIMENSION),
    ..ObjectHandlers::STD
};

/// zend_register_class_alias of php_dom_arginfo.h: (alias, class).
pub(crate) const CLASS_ALIASES: &[(&str, &str)] = &[("Dom\\DOMException", "DOMException")];

pub(crate) static CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "Dom\\Node",
        handlers: &MODERN_NODE_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Dom\\Implementation",
        handlers: &MODERN_PLAIN_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Dom\\NodeList",
        handlers: &MODERN_LIST_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Dom\\HTMLCollection",
        handlers: &MODERN_LIST_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Dom\\NamedNodeMap",
        handlers: &MODERN_MAP_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Dom\\DtdNamedNodeMap",
        handlers: &MODERN_MAP_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Dom\\XPath",
        handlers: &MAP_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Dom\\TokenList",
        handlers: &TOKEN_LIST_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DOMNode",
        handlers: &NODE_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DOMNameSpaceNode",
        handlers: &NS_NODE_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DOMImplementation",
        handlers: &PLAIN_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DOMNodeList",
        handlers: &LIST_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DOMNamedNodeMap",
        handlers: &LIST_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DOMXPath",
        handlers: &MAP_HANDLERS,
        create_object: true,
    },
];

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("dom_import_simplexml", fn_dom_import_simplexml),
            ("Dom\\import_simplexml", fn_dom_import_simplexml_modern),
        ],
    );
    // The registry is keyed by lowercase name.
    if let Some(b) = m.remove("Dom\\import_simplexml") {
        m.insert("dom\\import_simplexml", b);
    }
    crate::register_helpers(
        m,
        &[
            ("__dom", fn_dom, 2, None),
            ("__dom_prop", fn_dom_prop, 3, Some(4)),
        ],
    );
}

pub fn constants() -> Vec<(&'static str, Value)> {
    let i = Value::Int;
    vec![
        ("XML_ELEMENT_NODE", i(1)),
        ("XML_ATTRIBUTE_NODE", i(2)),
        ("XML_TEXT_NODE", i(3)),
        ("XML_CDATA_SECTION_NODE", i(4)),
        ("XML_ENTITY_REF_NODE", i(5)),
        ("XML_ENTITY_NODE", i(6)),
        ("XML_PI_NODE", i(7)),
        ("XML_COMMENT_NODE", i(8)),
        ("XML_DOCUMENT_NODE", i(9)),
        ("XML_DOCUMENT_TYPE_NODE", i(10)),
        ("XML_DOCUMENT_FRAG_NODE", i(11)),
        ("XML_NOTATION_NODE", i(12)),
        ("XML_HTML_DOCUMENT_NODE", i(13)),
        ("XML_DTD_NODE", i(14)),
        ("XML_ELEMENT_DECL_NODE", i(15)),
        ("XML_ATTRIBUTE_DECL_NODE", i(16)),
        ("XML_ENTITY_DECL_NODE", i(17)),
        ("XML_NAMESPACE_DECL_NODE", i(18)),
        ("XML_LOCAL_NAMESPACE", i(18)),
        ("XML_ATTRIBUTE_CDATA", i(1)),
        ("XML_ATTRIBUTE_ID", i(2)),
        ("XML_ATTRIBUTE_IDREF", i(3)),
        ("XML_ATTRIBUTE_IDREFS", i(4)),
        ("XML_ATTRIBUTE_ENTITY", i(6)),
        ("XML_ATTRIBUTE_NMTOKEN", i(7)),
        ("XML_ATTRIBUTE_NMTOKENS", i(8)),
        ("XML_ATTRIBUTE_ENUMERATION", i(9)),
        ("XML_ATTRIBUTE_NOTATION", i(10)),
        ("DOM_PHP_ERR", i(0)),
        ("DOM_INDEX_SIZE_ERR", i(1)),
        ("DOMSTRING_SIZE_ERR", i(2)),
        ("DOM_HIERARCHY_REQUEST_ERR", i(3)),
        ("DOM_WRONG_DOCUMENT_ERR", i(4)),
        ("DOM_INVALID_CHARACTER_ERR", i(5)),
        ("DOM_NO_DATA_ALLOWED_ERR", i(6)),
        ("DOM_NO_MODIFICATION_ALLOWED_ERR", i(7)),
        ("DOM_NOT_FOUND_ERR", i(8)),
        ("DOM_NOT_SUPPORTED_ERR", i(9)),
        ("DOM_INUSE_ATTRIBUTE_ERR", i(10)),
        ("DOM_INVALID_STATE_ERR", i(11)),
        ("DOM_SYNTAX_ERR", i(12)),
        ("DOM_INVALID_MODIFICATION_ERR", i(13)),
        ("DOM_NAMESPACE_ERR", i(14)),
        ("DOM_INVALID_ACCESS_ERR", i(15)),
        ("DOM_VALIDATION_ERR", i(16)),
        ("dom\\INDEX_SIZE_ERR", i(1)),
        ("dom\\STRING_SIZE_ERR", i(2)),
        ("dom\\HIERARCHY_REQUEST_ERR", i(3)),
        ("dom\\WRONG_DOCUMENT_ERR", i(4)),
        ("dom\\INVALID_CHARACTER_ERR", i(5)),
        ("dom\\NO_DATA_ALLOWED_ERR", i(6)),
        ("dom\\NO_MODIFICATION_ALLOWED_ERR", i(7)),
        ("dom\\NOT_FOUND_ERR", i(8)),
        ("dom\\NOT_SUPPORTED_ERR", i(9)),
        ("dom\\INUSE_ATTRIBUTE_ERR", i(10)),
        ("dom\\INVALID_STATE_ERR", i(11)),
        ("dom\\SYNTAX_ERR", i(12)),
        ("dom\\INVALID_MODIFICATION_ERR", i(13)),
        ("dom\\NAMESPACE_ERR", i(14)),
        ("dom\\VALIDATION_ERR", i(16)),
        ("dom\\HTML_NO_DEFAULT_NS", i(modern::HTML_NO_DEFAULT_NS)),
    ]
}

/// php_new_dom_dump_node_to_str: the serializer behind a modern
/// document's libxml handlers (SimpleXML's asXML uses it too).
pub fn modern_dump_to_str(
    document: *mut crate::ext::libxml::node::RefObj,
    node: *mut crate::ext::libxml::sys::XmlNode,
    options: std::ffi::c_int,
    format: bool,
    encoding: *const u8,
) -> Result<Option<Vec<u8>>, Thrown> {
    let p = unsafe { nscompat::of(document) }.map(|p| &*p);
    xml_serializer::dump_to_str(node, options, format, encoding, false, p)
}

pub const PRELUDE_PHP: &str = concat!(
    "namespace {\n",
    include_str!("prelude.php"),
    "}\n",
    include_str!("prelude_modern.php")
);

/// The `#[\Deprecated]` notice of a dom constant.
pub fn constant_deprecation(name: &[u8]) -> Option<String> {
    (name == b"DOM_PHP_ERR")
        .then(|| "Constant DOM_PHP_ERR is deprecated since 8.4, as it is no longer used".into())
}
