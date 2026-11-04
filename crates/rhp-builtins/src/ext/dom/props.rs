//! php's DOM property handlers: one table per DOM class, in php's
//! registration order (the order `var_dump` shows them in), with the read
//! and write functions of ext/dom's *.c files.

use std::rc::Rc;

use rhp_values::{conv, PhpArray, PhpStr, Value};

use super::util::*;
use super::{
    create_nullable, create_object, lists, throw, xs, xs_opt, Cx, DomObject, Kind,
    INVALID_STATE_ERR, NAMESPACE_ERR,
};
use crate::ext::libxml::node as lxn;
use crate::ext::libxml::sys::{self, XmlDoc, XmlDtd, XmlEntity, XmlNode};
use crate::Thrown;

type Read = fn(&mut Cx, &Rc<DomObject>) -> Result<Option<Value>, Thrown>;
type Write = fn(&mut Cx, &Rc<DomObject>, Value) -> Result<(), Thrown>;

#[derive(Clone, Copy)]
enum Ty {
    Str,
    NStr,
    Bool,
}

struct Prop {
    name: &'static str,
    /// The class that declares the property (for type errors).
    decl: &'static str,
    read: Read,
    write: Option<(Write, Ty)>,
}

const fn r(name: &'static str, decl: &'static str, read: Read) -> Prop {
    Prop {
        name,
        decl,
        read,
        write: None,
    }
}

const fn w(name: &'static str, decl: &'static str, read: Read, write: Write, ty: Ty) -> Prop {
    Prop {
        name,
        decl,
        read,
        write: Some((write, ty)),
    }
}

const N: &str = "DOMNode";

static NODE: &[Prop] = &[
    r("nodeName", N, node_name),
    w("nodeValue", N, node_value, node_value_write, Ty::NStr),
    r("nodeType", N, node_type),
    r("parentNode", N, parent_node),
    r("parentElement", N, parent_element),
    r("childNodes", N, child_nodes),
    r("firstChild", N, first_child),
    r("lastChild", N, last_child),
    r("previousSibling", N, previous_sibling),
    r("nextSibling", N, next_sibling),
    r("attributes", N, attributes),
    r("isConnected", N, is_connected_read),
    r("ownerDocument", N, owner_document),
    r("namespaceURI", N, namespace_uri),
    w("prefix", N, prefix, prefix_write, Ty::Str),
    r("localName", N, local_name),
    r("baseURI", N, base_uri),
    w("textContent", N, text_content, text_content_write, Ty::Str),
];

const NS: &str = "DOMNameSpaceNode";
static NS_NODE: &[Prop] = &[
    r("nodeName", NS, node_name),
    r("nodeValue", NS, node_value),
    r("nodeType", NS, node_type),
    r("prefix", NS, prefix),
    r("localName", NS, local_name),
    r("namespaceURI", NS, namespace_uri),
    r("isConnected", NS, is_connected_read),
    r("ownerDocument", NS, owner_document),
    r("parentNode", NS, parent_node),
    r("parentElement", NS, parent_element),
];

const FR: &str = "DOMDocumentFragment";
static FRAGMENT: &[Prop] = &[
    r("firstElementChild", FR, first_element_child),
    r("lastElementChild", FR, last_element_child),
    r("childElementCount", FR, child_element_count),
];

const D: &str = "DOMDocument";
static DOCUMENT: &[Prop] = &[
    r("doctype", D, doc_doctype),
    r("implementation", D, doc_implementation),
    r("documentElement", D, doc_document_element),
    r("actualEncoding", D, doc_actual_encoding),
    w("encoding", D, doc_encoding, doc_encoding_write, Ty::NStr),
    r("xmlEncoding", D, doc_encoding),
    w(
        "standalone",
        D,
        doc_standalone,
        doc_standalone_write,
        Ty::Bool,
    ),
    w(
        "xmlStandalone",
        D,
        doc_standalone,
        doc_standalone_write,
        Ty::Bool,
    ),
    w("version", D, doc_version, doc_version_write, Ty::NStr),
    w("xmlVersion", D, doc_version, doc_version_write, Ty::NStr),
    w(
        "strictErrorChecking",
        D,
        doc_strict,
        doc_strict_write,
        Ty::Bool,
    ),
    w("documentURI", D, doc_uri, doc_uri_write, Ty::NStr),
    r("config", D, doc_config),
    w("formatOutput", D, doc_format, doc_format_write, Ty::Bool),
    w(
        "validateOnParse",
        D,
        doc_validate,
        doc_validate_write,
        Ty::Bool,
    ),
    w(
        "resolveExternals",
        D,
        doc_resolve,
        doc_resolve_write,
        Ty::Bool,
    ),
    w(
        "preserveWhiteSpace",
        D,
        doc_preserve,
        doc_preserve_write,
        Ty::Bool,
    ),
    w("recover", D, doc_recover, doc_recover_write, Ty::Bool),
    w(
        "substituteEntities",
        D,
        doc_substitute,
        doc_substitute_write,
        Ty::Bool,
    ),
    r("firstElementChild", D, first_element_child),
    r("lastElementChild", D, last_element_child),
    r("childElementCount", D, child_element_count),
];

const CD: &str = "DOMCharacterData";
static CHARDATA: &[Prop] = &[
    w("data", CD, chardata_data, chardata_data_write, Ty::Str),
    r("length", CD, chardata_length),
    r("previousElementSibling", CD, previous_element_sibling),
    r("nextElementSibling", CD, next_element_sibling),
];

const A: &str = "DOMAttr";
static ATTR: &[Prop] = &[
    r("name", A, attr_name),
    r("specified", A, attr_specified),
    w("value", A, attr_value, attr_value_write, Ty::Str),
    r("ownerElement", A, attr_owner_element),
    r("schemaTypeInfo", A, null_read),
];

const E: &str = "DOMElement";
static ELEMENT: &[Prop] = &[
    r("tagName", E, tag_name),
    w("className", E, class_name, class_name_write, Ty::Str),
    w("id", E, id_read, id_write, Ty::Str),
    r("schemaTypeInfo", E, null_read),
    r("firstElementChild", E, first_element_child),
    r("lastElementChild", E, last_element_child),
    r("childElementCount", E, child_element_count),
    r("previousElementSibling", E, previous_element_sibling),
    r("nextElementSibling", E, next_element_sibling),
];

static TEXT: &[Prop] = &[r("wholeText", "DOMText", whole_text)];

const DT: &str = "DOMDocumentType";
static DOCTYPE: &[Prop] = &[
    r("name", DT, doctype_name),
    r("entities", DT, doctype_entities),
    r("notations", DT, doctype_notations),
    r("publicId", DT, doctype_public_id),
    r("systemId", DT, doctype_system_id),
    r("internalSubset", DT, doctype_internal_subset),
];

const NO: &str = "DOMNotation";
static NOTATION: &[Prop] = &[
    r("publicId", NO, notation_public_id),
    r("systemId", NO, notation_system_id),
];

const EN: &str = "DOMEntity";
static ENTITY: &[Prop] = &[
    r("publicId", EN, entity_public_id),
    r("systemId", EN, entity_system_id),
    r("notationName", EN, entity_notation_name),
    r("actualEncoding", EN, entity_actual_encoding),
    r("encoding", EN, entity_encoding),
    r("version", EN, entity_version),
];

static ENTITY_REF_OVERRIDES: &[Prop] = &[
    r("firstChild", N, entityref_child),
    r("lastChild", N, entityref_child),
    r("textContent", N, entityref_text_content),
    r("childNodes", N, entityref_child_nodes),
];

const PI: &str = "DOMProcessingInstruction";
static PROC_INST: &[Prop] = &[
    r("target", PI, pi_target),
    w("data", PI, pi_data, pi_data_write, Ty::Str),
];

static NODELIST: &[Prop] = &[r("length", "DOMNodeList", nodelist_length)];
static NAMEDNODEMAP: &[Prop] = &[r("length", "DOMNamedNodeMap", map_length)];

const XP: &str = "DOMXPath";
static XPATH: &[Prop] = &[
    r("document", XP, xpath_document),
    w(
        "registerNodeNamespaces",
        XP,
        xpath_register_ns,
        xpath_register_ns_write,
        Ty::Bool,
    ),
];

/// A class's property table: its own properties, then the inherited ones
/// it does not redefine (php's zend_hash_merge without overwrite), with
/// DOMEntityReference's overrides in place.
fn table(base: &str) -> Vec<&'static Prop> {
    let parts: &[&[Prop]] = match base {
        "DOMNode" => &[NODE],
        "DOMNameSpaceNode" => &[NS_NODE],
        "DOMDocumentFragment" => &[FRAGMENT, NODE],
        "DOMDocument" => &[DOCUMENT, NODE],
        "DOMCharacterData" | "DOMComment" => &[CHARDATA, NODE],
        "DOMText" | "DOMCdataSection" => &[TEXT, CHARDATA, NODE],
        "DOMAttr" => &[ATTR, NODE],
        "DOMElement" => &[ELEMENT, NODE],
        "DOMDocumentType" => &[DOCTYPE, NODE],
        "DOMNotation" => &[NOTATION, NODE],
        "DOMEntity" => &[ENTITY, NODE],
        "DOMEntityReference" => &[NODE],
        "DOMProcessingInstruction" => &[PROC_INST, NODE],
        "DOMNodeList" => &[NODELIST],
        "DOMNamedNodeMap" => &[NAMEDNODEMAP],
        "DOMXPath" => &[XPATH],
        _ => &[],
    };
    let mut out: Vec<&'static Prop> = Vec::new();
    for part in parts {
        for p in part.iter() {
            if !out.iter().any(|q| q.name == p.name) {
                out.push(p);
            }
        }
    }
    if base == "DOMEntityReference" {
        for o in ENTITY_REF_OVERRIDES {
            if let Some(slot) = out.iter_mut().find(|q| q.name == o.name) {
                *slot = o;
            }
        }
    }
    out
}

const DOM_CLASSES: &[&str] = &[
    "DOMNode",
    "DOMNameSpaceNode",
    "DOMDocumentFragment",
    "DOMDocument",
    "DOMCharacterData",
    "DOMComment",
    "DOMText",
    "DOMCdataSection",
    "DOMAttr",
    "DOMElement",
    "DOMDocumentType",
    "DOMNotation",
    "DOMEntity",
    "DOMEntityReference",
    "DOMProcessingInstruction",
    "DOMNodeList",
    "DOMNamedNodeMap",
    "DOMXPath",
    "DOMImplementation",
];

/// The DOM class an object's class derives from (dom_objects_set_class_ex
/// picks the property table by it).
fn dom_base(cx: &Cx, v: &Value) -> Option<&'static str> {
    let mut name = cx.host.object_class_name(v)?;
    for _ in 0..64 {
        if let Some(c) = DOM_CLASSES
            .iter()
            .find(|c| c.as_bytes().eq_ignore_ascii_case(&name))
        {
            return Some(c);
        }
        let id = cx.host.resolve_class(&name)?;
        name = cx.host.class_parent_name(id)?;
    }
    None
}

fn find(base: &str, name: &[u8]) -> Option<&'static Prop> {
    table(base).into_iter().find(|p| p.name.as_bytes() == name)
}

/// The object's state, or an empty one (php's `dom_object` of an object
/// whose constructor never ran).
fn state_of(v: &Value) -> Rc<DomObject> {
    super::intern_of(v).unwrap_or_else(|| Rc::new(DomObject::new(Kind::Plain)))
}

/// `__dom_prop`: read (0), write (1), isset (2), unset (3).
pub fn dispatch(
    cx: &mut Cx,
    op: i64,
    this: &Value,
    name: &[u8],
    value: Option<Value>,
) -> Result<Value, Thrown> {
    let no_prop = || Ok(Value::Array(PhpArray::new()));
    let Some(base) = dom_base(cx, this) else {
        return no_prop();
    };
    let Some(prop) = find(base, name) else {
        if op == 1 {
            // Creating a dynamic property: php's deprecation, as the
            // prelude's __set then writes it.
            let class = super::class_of(cx.host, this);
            cx.host.emit_diagnostic(
                crate::E_DEPRECATED,
                &format!(
                    "Creation of dynamic property {class}::${} is deprecated",
                    String::from_utf8_lossy(name)
                ),
            )?;
        }
        return no_prop();
    };
    let st = state_of(this);
    match op {
        0 => Ok((prop.read)(cx, &st)?.unwrap_or(Value::Null)),
        1 => {
            let Some((write, ty)) = prop.write else {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot modify readonly property {}::${}",
                        super::class_of(cx.host, this),
                        prop.name
                    ),
                ));
            };
            let v = coerce(cx, ty, value.unwrap_or(Value::Null), prop)?;
            write(cx, &st, v)?;
            Ok(Value::Bool(true))
        }
        2 => Ok(Value::Bool(matches!(
            (prop.read)(cx, &st)?,
            Some(v) if !matches!(v, Value::Null)
        ))),
        _ => Err(Thrown::error(
            "Error",
            format!(
                "Cannot unset {}::${}",
                super::class_of(cx.host, this),
                prop.name
            ),
        )),
    }
}

/// zend_verify_property_type, coercive mode.
fn coerce(cx: &mut Cx, ty: Ty, v: Value, prop: &Prop) -> Result<Value, Thrown> {
    let tname = match ty {
        Ty::Str => "string",
        Ty::NStr => "?string",
        Ty::Bool => "bool",
    };
    let fail = |cx: &mut Cx, v: &Value| {
        Thrown::type_error(format!(
            "Cannot assign {} to property {}::${} of type {tname}",
            crate::ext::zpp::given_type(cx.host, v),
            prop.decl,
            prop.name
        ))
    };
    match (&v, ty) {
        (Value::Null, Ty::NStr) => Ok(Value::Null),
        (Value::Null, _) | (Value::Array(_), _) | (Value::Resource(_), _) => Err(fail(cx, &v)),
        (Value::Object(_) | Value::Closure(_), Ty::Bool) => Err(fail(cx, &v)),
        (Value::Object(_), _) => {
            let has = cx
                .host
                .object_class_name(&v)
                .and_then(|c| cx.host.resolve_class(&c))
                .is_some_and(|id| cx.host.class_has_method(id, b"__tostring"));
            if !has {
                return Err(fail(cx, &v));
            }
            Ok(Value::str(crate::ext::libxml::io::to_string(cx.host, &v)?))
        }
        (Value::Closure(_), _) => Err(fail(cx, &v)),
        (_, Ty::Bool) => Ok(Value::Bool(conv::to_bool(&v))),
        _ => Ok(Value::str(conv::to_string_bytes(&v))),
    }
}

/// dom_get_debug_info
pub fn debug_view(cx: &mut Cx, v: &Value) -> Result<PhpArray, Thrown> {
    let mut out = PhpArray::new();
    if let Value::Object(h) = v {
        let b = h.borrow();
        for (k, val) in b.properties.iter() {
            out.insert(rhp_values::ArrayKey::Str(k.clone()), val.clone());
        }
    }
    let Some(base) = dom_base(cx, v) else {
        return Ok(out);
    };
    let st = state_of(v);
    for p in table(base) {
        let val = match (p.read)(cx, &st) {
            Ok(Some(val)) => val,
            Ok(None) | Err(_) => continue,
        };
        let val = match val {
            Value::Object(_) => Value::str("(object value omitted)"),
            other => other,
        };
        out.insert(
            rhp_values::ArrayKey::Str(PhpStr::from_bytes(p.name.as_bytes().to_vec())),
            val,
        );
    }
    Ok(out)
}

// ---- DOM_PROP_NODE ----

fn node_of(cx: &mut Cx, d: &DomObject) -> Result<*mut XmlNode, Thrown> {
    let n = d.node();
    if n.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    Ok(n)
}

fn deprecated(cx: &mut Cx, msg: &str) -> Result<(), Thrown> {
    if cx.quiet {
        return Ok(());
    }
    cx.host.emit_diagnostic(crate::E_DEPRECATED, msg)
}

fn some(v: Value) -> Result<Option<Value>, Thrown> {
    Ok(Some(v))
}

// ---- DOMNode ----

fn node_name(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    unsafe {
        some(match (*n).type_ {
            sys::XML_ELEMENT_NODE | sys::XML_ATTRIBUTE_NODE => Value::str(qualified_name(n)),
            sys::XML_NAMESPACE_DECL => {
                let ns = (*n).ns;
                if !ns.is_null() && !(*ns).prefix.is_null() {
                    let mut s = b"xmlns:".to_vec();
                    s.extend_from_slice(b((*ns).prefix));
                    Value::str(s)
                } else {
                    xs((*n).name)
                }
            }
            sys::XML_DOCUMENT_TYPE_NODE | sys::XML_DTD_NODE => xs((*n).name),
            sys::XML_PI_NODE
            | sys::XML_ENTITY_DECL
            | sys::XML_ENTITY_REF_NODE
            | sys::XML_NOTATION_NODE => xs((*n).name),
            sys::XML_CDATA_SECTION_NODE => Value::str("#cdata-section"),
            sys::XML_COMMENT_NODE => Value::str("#comment"),
            sys::XML_HTML_DOCUMENT_NODE | sys::XML_DOCUMENT_NODE => Value::str("#document"),
            sys::XML_DOCUMENT_FRAG_NODE => Value::str("#document-fragment"),
            sys::XML_TEXT_NODE => Value::str("#text"),
            _ => Value::Null,
        })
    }
}

fn node_value(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    unsafe {
        some(match (*n).type_ {
            sys::XML_ELEMENT_NODE
            | sys::XML_ATTRIBUTE_NODE
            | sys::XML_TEXT_NODE
            | sys::XML_COMMENT_NODE
            | sys::XML_CDATA_SECTION_NODE
            | sys::XML_PI_NODE => get_content(n, true),
            sys::XML_NAMESPACE_DECL => {
                match sys::take_string(sys::xmlNodeGetContent((*n).children)) {
                    Some(s) => Value::str(s),
                    None => Value::Null,
                }
            }
            _ => Value::Null,
        })
    }
}

fn node_value_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let n = node_of(cx, d)?;
    let s = match v {
        Value::Null => Vec::new(),
        v => conv::to_string_bytes(&v),
    };
    unsafe {
        let t = (*n).type_;
        if t == sys::XML_ATTRIBUTE_NODE {
            attr_value_will_change(d.document(), n.cast());
        }
        if t == sys::XML_ATTRIBUTE_NODE || t == sys::XML_ELEMENT_NODE {
            remove_all_children(n);
        }
        if matches!(
            t,
            sys::XML_ATTRIBUTE_NODE
                | sys::XML_ELEMENT_NODE
                | sys::XML_TEXT_NODE
                | sys::XML_COMMENT_NODE
                | sys::XML_CDATA_SECTION_NODE
                | sys::XML_PI_NODE
        ) {
            set_content_len(cx, n, &s)?;
        }
    }
    lxn::invalidate_node_list_cache(d.document());
    Ok(())
}

/// `xmlNodeSetContentLen`, whose entity errors (for elements and
/// attributes) reach php's error handler.
fn set_content_len(cx: &mut Cx, n: *mut XmlNode, s: &[u8]) -> Result<(), Thrown> {
    cx.with_libxml(|| unsafe { sys::xmlNodeSetContentLen(n, nul(s).as_ptr(), s.len() as i32) })
}

fn node_type(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let t = unsafe { (*n).type_ };
    some(Value::Int(if t == sys::XML_DTD_NODE {
        sys::XML_DOCUMENT_TYPE_NODE as i64
    } else {
        t as i64
    }))
}

fn parent_get(cx: &mut Cx, d: &Rc<DomObject>, only_element: bool) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let p = unsafe { (*n).parent };
    if p.is_null() || (only_element && unsafe { (*p).type_ } != sys::XML_ELEMENT_NODE) {
        return some(Value::Null);
    }
    some(create_object(cx, p, Some(d))?)
}

fn parent_node(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    parent_get(cx, d, false)
}

fn parent_element(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    parent_get(cx, d, true)
}

fn child_nodes(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    node_of(cx, d)?;
    some(lists::new_map(
        cx,
        d,
        lists::Handler::ChildNodes,
        b"DOMNodeList",
    )?)
}

fn first_child(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let first = if node_children_valid(n) {
        unsafe { (*n).children }
    } else {
        std::ptr::null_mut()
    };
    some(create_nullable(cx, first, Some(d))?)
}

fn last_child(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let last = if node_children_valid(n) {
        unsafe { (*n).last }
    } else {
        std::ptr::null_mut()
    };
    some(create_nullable(cx, last, Some(d))?)
}

fn previous_sibling(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(create_nullable(cx, unsafe { (*n).prev }, Some(d))?)
}

fn next_sibling(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(create_nullable(cx, unsafe { (*n).next }, Some(d))?)
}

fn previous_element_sibling(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let mut p = unsafe { (*n).prev };
    while !p.is_null() && unsafe { (*p).type_ } != sys::XML_ELEMENT_NODE {
        p = unsafe { (*p).prev };
    }
    some(create_nullable(cx, p, Some(d))?)
}

fn next_element_sibling(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let mut p = unsafe { (*n).next };
    while !p.is_null() && unsafe { (*p).type_ } != sys::XML_ELEMENT_NODE {
        p = unsafe { (*p).next };
    }
    some(create_nullable(cx, p, Some(d))?)
}

fn attributes(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    if unsafe { (*n).type_ } == sys::XML_ELEMENT_NODE {
        some(lists::new_map(
            cx,
            d,
            lists::Handler::Attributes,
            b"DOMNamedNodeMap",
        )?)
    } else {
        some(Value::Null)
    }
}

fn is_connected_read(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(Value::Bool(is_connected(n)))
}

fn owner_document(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    unsafe {
        let t = (*n).type_;
        if t == sys::XML_DOCUMENT_NODE || t == sys::XML_HTML_DOCUMENT_NODE {
            return some(Value::Null);
        }
        let doc = (*n).doc;
        if doc.is_null() {
            return Ok(None);
        }
        some(create_object(cx, doc.cast(), Some(d))?)
    }
}

fn namespace_uri(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    unsafe {
        let ns = match (*n).type_ {
            sys::XML_ELEMENT_NODE | sys::XML_ATTRIBUTE_NODE | sys::XML_NAMESPACE_DECL => (*n).ns,
            _ => std::ptr::null_mut(),
        };
        some(if ns.is_null() {
            Value::Null
        } else {
            xs_opt((*ns).href)
        })
    }
}

fn prefix(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    unsafe {
        let ns = match (*n).type_ {
            sys::XML_ELEMENT_NODE | sys::XML_ATTRIBUTE_NODE | sys::XML_NAMESPACE_DECL => (*n).ns,
            _ => std::ptr::null_mut(),
        };
        some(if ns.is_null() {
            Value::str("")
        } else {
            xs((*ns).prefix)
        })
    }
}

fn prefix_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let n = node_of(cx, d)?;
    let prefix_str = conv::to_string_bytes(&v);
    unsafe {
        let t = (*n).type_;
        if t != sys::XML_ELEMENT_NODE && t != sys::XML_ATTRIBUTE_NODE {
            return Ok(());
        }
        let mut nsnode = if t == sys::XML_ELEMENT_NODE {
            n
        } else {
            std::ptr::null_mut()
        };
        if nsnode.is_null() {
            nsnode = (*n).parent;
            if nsnode.is_null() {
                nsnode = sys::xmlDocGetRootElement((*n).doc);
            }
        }
        let cprefix = c(&prefix_str);
        // php reads the prefix as a C string: "\0x" is empty too.
        let prefix: *const u8 = if cprefix[0] == 0 {
            std::ptr::null()
        } else {
            cprefix.as_ptr()
        };
        if !nsnode.is_null() && !(*n).ns.is_null() && !str_eq((*(*n).ns).prefix, prefix) {
            let uri = (*(*n).ns).href;
            let prefix_str = &cprefix[..cprefix.len() - 1];
            if uri.is_null()
                || (prefix_str == b"xml" && b(uri) != XML_NS_URI)
                || (t == sys::XML_ATTRIBUTE_NODE
                    && prefix_str == b"xmlns"
                    && b(uri) != XMLNS_NS_URI)
                || (t == sys::XML_ATTRIBUTE_NODE && b((*n).name) == b"xmlns")
            {
                throw(cx, NAMESPACE_ERR, d.strict())?;
                return Ok(());
            }
            let mut ns = std::ptr::null_mut();
            let mut cur = (*nsnode).nsDef;
            while !cur.is_null() {
                if str_eq(prefix, (*cur).prefix) && str_eq((*(*n).ns).href, (*cur).href) {
                    ns = cur;
                    break;
                }
                cur = (*cur).next;
            }
            if ns.is_null() {
                ns = sys::xmlNewNs(nsnode, (*(*n).ns).href, prefix);
                if ns.is_null() {
                    throw(cx, NAMESPACE_ERR, true)?;
                    return Ok(());
                }
            }
            sys::xmlSetNs(n, ns);
        }
    }
    Ok(())
}

fn local_name(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    unsafe {
        some(match (*n).type_ {
            sys::XML_ELEMENT_NODE | sys::XML_ATTRIBUTE_NODE | sys::XML_NAMESPACE_DECL => {
                xs((*n).name)
            }
            _ => Value::Null,
        })
    }
}

fn base_uri(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    unsafe {
        some(match sys::take_string(sys::xmlNodeGetBase((*n).doc, n)) {
            Some(s) => Value::str(s),
            None => Value::Null,
        })
    }
}

fn text_content(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(get_content(n, false))
}

fn text_content_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let n = node_of(cx, d)?;
    lxn::invalidate_node_list_cache(d.document());
    let s = match v {
        Value::Null => Vec::new(),
        v => conv::to_string_bytes(&v),
    };
    unsafe {
        let t = (*n).type_;
        if t == sys::XML_DOCUMENT_FRAG_NODE
            || t == sys::XML_ELEMENT_NODE
            || t == sys::XML_ATTRIBUTE_NODE
        {
            remove_all_children(n);
            let text = sys::xmlNewDocTextLen((*n).doc, nul(&s).as_ptr(), s.len() as i32);
            sys::xmlAddChild(n, text);
        } else {
            let cs = c(&s);
            cx.with_libxml(|| sys::xmlNodeSetContent(n, cs.as_ptr()))?;
        }
    }
    Ok(())
}

fn null_read(_: &mut Cx, _: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    some(Value::Null)
}

// ---- ParentNode ----

fn first_element_child(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let mut f = unsafe { (*n).children };
    while !f.is_null() && unsafe { (*f).type_ } != sys::XML_ELEMENT_NODE {
        f = unsafe { (*f).next };
    }
    some(create_nullable(cx, f, Some(d))?)
}

fn last_element_child(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let mut l = unsafe { (*n).last };
    while !l.is_null() && unsafe { (*l).type_ } != sys::XML_ELEMENT_NODE {
        l = unsafe { (*l).prev };
    }
    some(create_nullable(cx, l, Some(d))?)
}

fn child_element_count(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let mut count = 0;
    let mut f = unsafe { (*n).children };
    while !f.is_null() {
        if unsafe { (*f).type_ } == sys::XML_ELEMENT_NODE {
            count += 1;
        }
        f = unsafe { (*f).next };
    }
    some(Value::Int(count))
}

// ---- DOMDocument ----

fn doc_of(cx: &mut Cx, d: &DomObject) -> Result<*mut XmlDoc, Thrown> {
    Ok(node_of(cx, d)?.cast())
}

fn doc_doctype(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = doc_of(cx, d)?;
    let dtd = unsafe { sys::xmlGetIntSubset(doc) };
    some(create_nullable(cx, dtd.cast(), Some(d))?)
}

fn doc_implementation(cx: &mut Cx, _: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    some(
        cx.host
            .instantiate(b"DOMImplementation", Vec::new())
            .unwrap_or(Value::Null),
    )
}

fn doc_document_element(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = doc_of(cx, d)?;
    let root = unsafe { sys::xmlDocGetRootElement(doc) };
    some(create_nullable(cx, root, Some(d))?)
}

fn doc_encoding(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = doc_of(cx, d)?;
    some(xs_opt(unsafe { (*doc).encoding }))
}

fn doc_actual_encoding(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    deprecated(cx, "Property DOMDocument::$actualEncoding is deprecated")?;
    doc_encoding(cx, d)
}

fn doc_encoding_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let doc = doc_of(cx, d)?;
    let invalid = || Thrown::error("ValueError", "Invalid document encoding");
    let Value::Str(s) = v else {
        return Err(invalid());
    };
    let cs = c(s.as_bytes());
    unsafe {
        let handler = sys::xmlFindCharEncodingHandler(cs.as_ptr().cast());
        if handler.is_null() {
            return Err(invalid());
        }
        sys::xmlCharEncCloseFunc(handler);
        sys::xml_free((*doc).encoding as *mut u8);
        (*doc).encoding = sys::xmlStrdup(cs.as_ptr());
    }
    Ok(())
}

fn doc_standalone(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = doc_of(cx, d)?;
    some(Value::Bool(unsafe { (*doc).standalone } > 0))
}

fn doc_standalone_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let doc = doc_of(cx, d)?;
    unsafe { (*doc).standalone = conv::to_bool(&v) as i32 };
    Ok(())
}

fn doc_version(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = doc_of(cx, d)?;
    some(xs_opt(unsafe { (*doc).version }))
}

fn doc_version_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let doc = doc_of(cx, d)?;
    let s = match v {
        Value::Null => Vec::new(),
        v => conv::to_string_bytes(&v),
    };
    let cs = c(&s);
    unsafe {
        sys::xml_free((*doc).version as *mut u8);
        (*doc).version = sys::xmlStrdup(cs.as_ptr());
    }
    Ok(())
}

macro_rules! doc_flag {
    ($read:ident, $write:ident, $field:ident) => {
        fn $read(_: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
            some(Value::Bool(lxn::doc_props_read_only(d.document()).$field))
        }
        fn $write(_: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
            if let Some(p) = unsafe { lxn::doc_props_mut(d.document()) } {
                p.$field = conv::to_bool(&v);
            }
            Ok(())
        }
    };
}

doc_flag!(doc_strict, doc_strict_write, stricterror);
doc_flag!(doc_format, doc_format_write, formatoutput);
doc_flag!(doc_validate, doc_validate_write, validateonparse);
doc_flag!(doc_resolve, doc_resolve_write, resolveexternals);
doc_flag!(doc_preserve, doc_preserve_write, preservewhitespace);
doc_flag!(doc_recover, doc_recover_write, recover);
doc_flag!(doc_substitute, doc_substitute_write, substituteentities);

fn doc_uri(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = doc_of(cx, d)?;
    some(xs_opt(unsafe { (*doc).URL }))
}

fn doc_uri_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let doc = doc_of(cx, d)?;
    let s = match v {
        Value::Null => Vec::new(),
        v => conv::to_string_bytes(&v),
    };
    let cs = c(&s);
    unsafe {
        sys::xml_free((*doc).URL as *mut u8);
        (*doc).URL = sys::xmlStrdup(cs.as_ptr());
    }
    Ok(())
}

fn doc_config(cx: &mut Cx, _: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    deprecated(cx, "Property DOMDocument::$config is deprecated")?;
    some(Value::Null)
}

// ---- DOMCharacterData, DOMText ----

fn chardata_data(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(get_content(n, false))
}

fn chardata_data_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let n = node_of(cx, d)?;
    let s = conv::to_string_bytes(&v);
    set_content_len(cx, n, &s)
}

fn chardata_length(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let content = unsafe { (*n).content };
    let len = if content.is_null() {
        0
    } else {
        unsafe { sys::xmlUTF8Strlen(content) }
    };
    some(Value::Int(len as i64))
}

fn whole_text(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let mut n = node_of(cx, d)?;
    let is_text = |n: *mut XmlNode| unsafe {
        !n.is_null() && matches!((*n).type_, sys::XML_TEXT_NODE | sys::XML_CDATA_SECTION_NODE)
    };
    let mut out = Vec::new();
    unsafe {
        while is_text((*n).prev) {
            n = (*n).prev;
        }
        while is_text(n) {
            out.extend_from_slice(b((*n).content));
            n = (*n).next;
        }
    }
    some(Value::str(out))
}

// ---- DOMAttr ----

fn attr_name(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(xs(unsafe { (*n).name }))
}

fn attr_specified(_: &mut Cx, _: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    some(Value::Bool(true))
}

fn attr_value(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(get_content(n, false))
}

fn attr_value_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let n = node_of(cx, d)?;
    attr_value_will_change(d.document(), n.cast());
    let s = conv::to_string_bytes(&v);
    remove_all_children(n);
    set_content_len(cx, n, &s)
}

fn attr_owner_element(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(create_nullable(cx, unsafe { (*n).parent }, Some(d))?)
}

// ---- DOMElement ----

fn tag_name(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(Value::str(qualified_name(n)))
}

fn reflected_attribute(cx: &mut Cx, d: &DomObject, name: &[u8]) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let cn = c(name);
    unsafe {
        some(Value::str(
            sys::take_string(sys::xmlGetNoNsProp(n, cn.as_ptr())).unwrap_or_default(),
        ))
    }
}

fn reflected_attribute_write(
    cx: &mut Cx,
    d: &DomObject,
    name: &[u8],
    v: &Value,
) -> Result<*mut sys::XmlAttr, Thrown> {
    let n = d.node();
    if n.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
    }
    lxn::invalidate_node_list_cache(d.document());
    let cn = c(name);
    let cv = c(&conv::to_string_bytes(v));
    Ok(unsafe { sys::xmlSetNsProp(n, std::ptr::null_mut(), cn.as_ptr(), cv.as_ptr()) })
}

fn class_name(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    reflected_attribute(cx, d, b"class")
}

fn class_name_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    reflected_attribute_write(cx, d, b"class", &v).map(|_| ())
}

fn id_read(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    reflected_attribute(cx, d, b"id")
}

fn id_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let attr = reflected_attribute_write(cx, d, b"id", &v)?;
    if !attr.is_null() {
        super::element::set_attribute_id(attr, true, d.document());
    }
    Ok(())
}

// ---- DOMDocumentType ----

fn dtd_of(cx: &mut Cx, d: &DomObject) -> Result<*mut XmlDtd, Thrown> {
    Ok(node_of(cx, d)?.cast())
}

fn doctype_name(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let dtd = dtd_of(cx, d)?;
    some(xs(unsafe { (*dtd).name }))
}

fn doctype_entities(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let dtd = dtd_of(cx, d)?;
    let ht = unsafe { (*dtd).entities };
    some(lists::new_map(
        cx,
        d,
        lists::Handler::Entities(ht),
        b"DOMNamedNodeMap",
    )?)
}

fn doctype_notations(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let dtd = dtd_of(cx, d)?;
    let ht = unsafe { (*dtd).notations };
    some(lists::new_map(
        cx,
        d,
        lists::Handler::Notations(ht),
        b"DOMNamedNodeMap",
    )?)
}

fn doctype_public_id(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let dtd = dtd_of(cx, d)?;
    some(xs(unsafe { (*dtd).ExternalID }))
}

fn doctype_system_id(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let dtd = dtd_of(cx, d)?;
    some(xs(unsafe { (*dtd).SystemID }))
}

fn doctype_internal_subset(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let dtd = dtd_of(cx, d)?;
    unsafe {
        let doc = (*dtd).doc;
        if doc.is_null() {
            return some(Value::Null);
        }
        let intsubset = sys::xmlGetIntSubset(doc);
        if intsubset.is_null() {
            return some(Value::Null);
        }
        let mut out: Option<Vec<u8>> = None;
        let mut cur = (*intsubset).children;
        while !cur.is_null() {
            let buff = sys::xmlAllocOutputBuffer(std::ptr::null_mut());
            if !buff.is_null() {
                sys::xmlNodeDumpOutput(buff, std::ptr::null_mut(), cur, 0, 0, std::ptr::null());
                sys::xmlOutputBufferFlush(buff);
                let p = sys::xmlOutputBufferGetContent(buff);
                let len = sys::xmlOutputBufferGetSize(buff);
                let v = out.get_or_insert_with(Vec::new);
                if !p.is_null() {
                    v.extend_from_slice(std::slice::from_raw_parts(p, len));
                }
                sys::xmlOutputBufferClose(buff);
            }
            cur = (*cur).next;
        }
        some(match out {
            Some(v) if !v.is_empty() => Value::str(v),
            _ => Value::Null,
        })
    }
}

// ---- DOMNotation, DOMEntity ----

fn entity_of(cx: &mut Cx, d: &DomObject) -> Result<*mut XmlEntity, Thrown> {
    Ok(node_of(cx, d)?.cast())
}

fn notation_public_id(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let e = entity_of(cx, d)?;
    some(xs(unsafe { (*e).ExternalID }))
}

fn notation_system_id(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let e = entity_of(cx, d)?;
    some(xs(unsafe { (*e).SystemID }))
}

/// XML_EXTERNAL_GENERAL_UNPARSED_ENTITY
const UNPARSED: i32 = 3;

fn entity_public_id(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let e = entity_of(cx, d)?;
    unsafe {
        some(if (*e).etype != UNPARSED || (*e).ExternalID.is_null() {
            Value::Null
        } else {
            xs((*e).ExternalID)
        })
    }
}

fn entity_system_id(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let e = entity_of(cx, d)?;
    unsafe {
        some(if (*e).etype != UNPARSED {
            Value::Null
        } else {
            xs((*e).SystemID)
        })
    }
}

fn entity_notation_name(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let e = entity_of(cx, d)?;
    unsafe {
        some(if (*e).etype != UNPARSED {
            Value::Null
        } else {
            xs((*e).content)
        })
    }
}

fn entity_actual_encoding(cx: &mut Cx, _: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    deprecated(cx, "Property DOMEntity::$actualEncoding is deprecated")?;
    some(Value::Null)
}

fn entity_encoding(cx: &mut Cx, _: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    deprecated(cx, "Property DOMEntity::$encoding is deprecated")?;
    some(Value::Null)
}

fn entity_version(cx: &mut Cx, _: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    deprecated(cx, "Property DOMEntity::$version is deprecated")?;
    some(Value::Null)
}

// ---- DOMEntityReference ----

/// dom_entity_reference_fetch_and_sync_declaration: an entity reference's
/// children are its declaration, looked up afresh (it may be gone).
pub fn entity_reference_sync(reference: *mut XmlNode) -> *mut XmlEntity {
    unsafe {
        let entity = sys::xmlGetDocEntity((*reference).doc, (*reference).name);
        (*reference).children = entity.cast();
        (*reference).last = entity.cast();
        (*reference).content = if entity.is_null() {
            std::ptr::null_mut()
        } else {
            (*entity).content
        };
        entity
    }
}

fn entityref_child(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    let e = entity_reference_sync(n);
    some(create_nullable(cx, e.cast(), Some(d))?)
}

fn entityref_text_content(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    entity_reference_sync(n);
    text_content(cx, d)
}

fn entityref_child_nodes(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    entity_reference_sync(n);
    child_nodes(cx, d)
}

// ---- DOMProcessingInstruction ----

fn pi_target(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(xs(unsafe { (*n).name }))
}

fn pi_data(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(get_content(n, false))
}

fn pi_data_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let n = node_of(cx, d)?;
    let s = conv::to_string_bytes(&v);
    set_content_len(cx, n, &s)
}

// ---- DOMNodeList, DOMNamedNodeMap ----

fn nodelist_length(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    some(Value::Int(lists::nodelist_length(cx, d)))
}

fn map_length(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    some(Value::Int(lists::map_length(cx, d)))
}

// ---- DOMXPath ----

fn xpath_document(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = match &d.kind {
        Kind::XPath(x) => {
            let ctx = x.borrow().ctx;
            if ctx.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { sys::rhp_xml_xpath_doc(ctx) }
            }
        }
        _ => std::ptr::null_mut(),
    };
    if doc.is_null() {
        throw(cx, INVALID_STATE_ERR, true)?;
        return Ok(None);
    }
    some(create_object(cx, doc.cast(), Some(d))?)
}

fn xpath_register_ns(_: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    some(Value::Bool(match &d.kind {
        Kind::XPath(x) => x.borrow().register_node_ns,
        _ => true,
    }))
}

fn xpath_register_ns_write(_: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    if let Kind::XPath(x) = &d.kind {
        x.borrow_mut().register_node_ns = conv::to_bool(&v);
    }
    Ok(())
}
