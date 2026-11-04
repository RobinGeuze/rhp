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
//! The modern `Dom\` API (HTML5 via lexbor, CSS selectors) is not here.

mod bridge;
mod chardata;
mod document;
mod element;
mod lists;
mod node;
mod props;
mod util;
mod xpath;

pub use bridge::{from_old_tree, to_old_tree};

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr;
use std::rc::Rc;

use rhp_values::{conv, ObjectHandle, PhpArray, Value};

use crate::ext::libxml::node::{self as lxn, NodeObject};
use crate::ext::libxml::sys::{self, XmlNode};
use crate::object::{self, ClassHandlers, CloneObj, ObjectHandlers, PropPurpose};
use crate::{Builtin, Host, Thrown};

/// php's `dom_object`, as an object's payload.
pub(crate) struct DomObject {
    pub base: NodeObject,
    pub kind: Kind,
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
}

impl DomObject {
    pub fn new(kind: Kind) -> DomObject {
        DomObject {
            base: NodeObject::default(),
            kind,
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
            Kind::Plain => {}
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
}

impl Cx<'_> {
    /// `php_error_docref(NULL, E_WARNING, msg)`.
    pub fn warn(&mut self, msg: &str) -> Result<(), Thrown> {
        crate::warn(self.host, &self.fname, msg)
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
        let fname = self.fname.clone();
        crate::ext::libxml::ffi::with_host(self.host, &fname, f)
    }
}

/// The payload of a DOM object.
pub(crate) fn intern_of(v: &Value) -> Option<Rc<DomObject>> {
    object::payload::<DomObject>(v)
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
        Kind::Map(_) => true,
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
    let h = object::object_of(v)?;
    let d = Rc::new(DomObject::new(kind()));
    *d.base.this.borrow_mut() = Rc::downgrade(&h);
    object::set_payload(&h, d.clone());
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
    if let Some(obj) = unsafe { lxn::object_of_node(node) } {
        if let Some(v) = obj.object() {
            return Ok(v);
        }
    }
    let t = unsafe { (*node).type_ };
    let Some(base) = class_for_type(t) else {
        return Err(Thrown::error(
            "Error",
            format!("Unsupported node type: {t}"),
        ));
    };
    let mut class = base.as_bytes().to_vec();
    if let Some(d) = domobj {
        if let Some(c) = classmap_lookup(d.document(), base) {
            class = c;
        }
    }
    instantiate(cx, &class, node, domobj)
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
    parent: Option<&DomObject>,
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
            if let Some(p) = parent {
                d.base.document.set(p.document());
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
    )
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

pub(crate) fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

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
    Value::str(unsafe { sys::cstr(p) }.to_vec())
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
    let fname = crate::ext::libxml::active_user_function(host);
    let mut cx = Cx {
        host,
        fname,
        quiet: false,
    };
    props::dispatch(&mut cx, op, &this, &name, args.get(3).map(deref))
}

/// `__dom_iter($list, $current, $index)`: php's DOMNodeList and
/// DOMNamedNodeMap iterator, one step: `[key, node]` for the item at
/// `$index` (the first item when `$current` is null), or null at the end.
fn fn_dom_iter(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let this = arg(args, 0);
    let cur = arg(args, 1);
    let index = int(args, 2);
    let mut cx = Cx {
        host,
        fname: String::new(),
        quiet: false,
    };
    lists::iterate(&mut cx, &this, &cur, index)
}

/// dom_import_simplexml
fn fn_dom_import_simplexml(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = arg(args, 0);
    bridge::import_simplexml(host, &v)
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
    };
    node::clone_object(&mut cx, &sv, &dv)
}

/// dom_object_namespace_node_clone_obj
fn clone_ns_obj(host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let sv = Value::Object(src.clone());
    let dv = Value::Object(dst.clone());
    let mut cx = Cx {
        host,
        fname: String::new(),
        quiet: false,
    };
    node::clone_ns_object(&mut cx, &sv, &dv)
}

static NODE_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    clone_obj: CloneObj::Custom(clone_node_obj),
    ..ObjectHandlers::STD
};

static NS_NODE_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    clone_obj: CloneObj::Custom(clone_ns_obj),
    ..ObjectHandlers::STD
};

static MAP_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    clone_obj: CloneObj::None,
    ..ObjectHandlers::STD
};

static PLAIN_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(get_properties_for),
    ..ObjectHandlers::STD
};

pub(crate) static CLASSES: &[ClassHandlers] = &[
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
        handlers: &MAP_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DOMNamedNodeMap",
        handlers: &MAP_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DOMXPath",
        handlers: &MAP_HANDLERS,
        create_object: true,
    },
];

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    for (name, func, min, max) in [
        ("__dom", fn_dom as crate::NativeFn, 2, None),
        ("__dom_prop", fn_dom_prop, 3, Some(4)),
        ("__dom_iter", fn_dom_iter, 3, Some(3)),
        ("dom_import_simplexml", fn_dom_import_simplexml, 1, Some(1)),
    ] {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
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
    ]
}

pub const PRELUDE_PHP: &str = include_str!("prelude.php");

/// The `#[\Deprecated]` notice of a dom constant.
pub fn constant_deprecation(name: &[u8]) -> Option<String> {
    (name == b"DOM_PHP_ERR")
        .then(|| "Constant DOM_PHP_ERR is deprecated since 8.4, as it is no longer used".into())
}

/// Read a document source (path or URL): (bytes, the URL libxml2 records).
/// XMLReader still reads through it.
pub fn read_source(host: &mut dyn Host, path: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let p = String::from_utf8_lossy(path).into_owned();
    if let Some(rest) = p.strip_prefix("file://") {
        let data = std::fs::read(rest).ok()?;
        return Some((data, rest.as_bytes().to_vec()));
    }
    if p.contains("://") {
        let data = host.stream_get_contents(path)?;
        return Some((data, path.to_vec()));
    }
    let full = expand_filepath(path);
    let data = std::fs::read(String::from_utf8_lossy(&full).as_ref()).ok()?;
    Some((data, full))
}

/// php's `expand_filepath`: the absolute path (symlinks in the existing
/// leading part resolved).
pub fn expand_filepath(path: &[u8]) -> Vec<u8> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    if path.windows(3).any(|w| w == b"://") {
        return path.to_vec();
    }
    let full = if path.starts_with(b"/") {
        path.to_vec()
    } else {
        let mut cwd = std::env::current_dir()
            .map(|d| d.into_os_string().into_vec())
            .unwrap_or_default();
        cwd.push(b'/');
        cwd.extend_from_slice(path);
        cwd
    };
    let mut parts: Vec<&[u8]> = Vec::new();
    for seg in full.split(|&b| b == b'/') {
        match seg {
            b"" | b"." => {}
            b".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let join = |segs: &[&[u8]]| {
        let mut out = Vec::new();
        for seg in segs {
            out.push(b'/');
            out.extend_from_slice(seg);
        }
        out
    };
    for keep in (1..=parts.len()).rev() {
        if let Ok(real) = std::fs::canonicalize(OsStr::from_bytes(&join(&parts[..keep]))) {
            let mut out = real.into_os_string().into_vec();
            out.extend_from_slice(&join(&parts[keep..]));
            return out;
        }
    }
    let out = join(&parts);
    if out.is_empty() {
        b"/".to_vec()
    } else {
        out
    }
}
