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
    /// `?Dom\HTMLElement` (Dom\Document::$body).
    HtmlElement,
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

// ---- the Dom\ classes (php_dom.c's MINIT, modern half) ----

const MN: &str = "Dom\\Node";
static M_NODE: &[Prop] = &[
    r("nodeType", MN, node_type),
    r("nodeName", MN, node_name),
    r("baseURI", MN, m_base_uri),
    r("isConnected", MN, is_connected_read),
    r("ownerDocument", MN, owner_document),
    r("parentNode", MN, parent_node),
    r("parentElement", MN, parent_element),
    r("childNodes", MN, m_child_nodes),
    r("firstChild", MN, first_child),
    r("lastChild", MN, last_child),
    r("previousSibling", MN, previous_sibling),
    r("nextSibling", MN, next_sibling),
    r("nodeValue", MN, node_value),
    r("textContent", MN, text_content),
];
static M_NODE_VALUE_RW: &[Prop] = &[w("nodeValue", MN, node_value, node_value_write, Ty::NStr)];
static M_TEXT_CONTENT_RW: &[Prop] = &[w(
    "textContent",
    MN,
    text_content,
    text_content_write,
    Ty::NStr,
)];

const MDF: &str = "Dom\\DocumentFragment";
static M_FRAGMENT: &[Prop] = &[
    r("children", MDF, m_children),
    r("firstElementChild", MDF, first_element_child),
    r("lastElementChild", MDF, last_element_child),
    r("childElementCount", MDF, child_element_count),
];

const MD: &str = "Dom\\Document";
static M_DOCUMENT: &[Prop] = &[
    r("implementation", MD, m_doc_implementation),
    w("URL", MD, m_doc_uri, doc_uri_write, Ty::Str),
    w("documentURI", MD, m_doc_uri, doc_uri_write, Ty::Str),
    w(
        "characterSet",
        MD,
        doc_encoding,
        m_doc_encoding_write,
        Ty::Str,
    ),
    w("charset", MD, doc_encoding, m_doc_encoding_write, Ty::Str),
    w(
        "inputEncoding",
        MD,
        doc_encoding,
        m_doc_encoding_write,
        Ty::Str,
    ),
    r("doctype", MD, doc_doctype),
    r("documentElement", MD, doc_document_element),
    r("children", MD, m_children),
    r("firstElementChild", MD, first_element_child),
    r("lastElementChild", MD, last_element_child),
    r("childElementCount", MD, child_element_count),
    w("body", MD, m_body, m_body_write, Ty::HtmlElement),
    r("head", MD, m_head),
    w("title", MD, m_title, m_title_write, Ty::Str),
];

const MX: &str = "Dom\\XMLDocument";
static M_XML_DOCUMENT: &[Prop] = &[
    r("xmlEncoding", MX, doc_encoding),
    w(
        "xmlStandalone",
        MX,
        doc_standalone,
        doc_standalone_write,
        Ty::Bool,
    ),
    w("xmlVersion", MX, doc_version, m_doc_version_write, Ty::Str),
    w("formatOutput", MX, doc_format, doc_format_write, Ty::Bool),
];

const MCD: &str = "Dom\\CharacterData";
static M_CHARDATA: &[Prop] = &[
    w("data", MCD, chardata_data, chardata_data_write, Ty::Str),
    r("length", MCD, chardata_length),
    r("previousElementSibling", MCD, previous_element_sibling),
    r("nextElementSibling", MCD, next_element_sibling),
];

const MA: &str = "Dom\\Attr";
static M_ATTR: &[Prop] = &[
    r("namespaceURI", MA, namespace_uri),
    r("prefix", MA, m_prefix),
    r("localName", MA, local_name),
    r("name", MA, attr_name),
    w("value", MA, attr_value, attr_value_write, Ty::Str),
    r("ownerElement", MA, attr_owner_element),
    r("specified", MA, attr_specified),
];

const ME: &str = "Dom\\Element";
static M_ELEMENT: &[Prop] = &[
    r("namespaceURI", ME, namespace_uri),
    r("prefix", ME, m_prefix),
    r("localName", ME, local_name),
    r("tagName", ME, tag_name),
    w("id", ME, id_read, id_write, Ty::Str),
    w("className", ME, class_name, class_name_write, Ty::Str),
    r("classList", ME, m_class_list),
    r("attributes", ME, attributes),
    r("children", ME, m_children),
    r("firstElementChild", ME, first_element_child),
    r("lastElementChild", ME, last_element_child),
    r("childElementCount", ME, child_element_count),
    r("previousElementSibling", ME, previous_element_sibling),
    r("nextElementSibling", ME, next_element_sibling),
    w("innerHTML", ME, m_inner_html, m_inner_html_write, Ty::Str),
    w("outerHTML", ME, m_outer_html, m_outer_html_write, Ty::Str),
    w(
        "substitutedNodeValue",
        ME,
        m_substituted,
        m_substituted_write,
        Ty::Str,
    ),
];

static M_TEXT: &[Prop] = &[r("wholeText", "Dom\\Text", whole_text)];

const MDT: &str = "Dom\\DocumentType";
static M_DOCTYPE: &[Prop] = &[
    r("name", MDT, doctype_name),
    r("entities", MDT, m_doctype_entities),
    r("notations", MDT, m_doctype_notations),
    r("publicId", MDT, doctype_public_id),
    r("systemId", MDT, doctype_system_id),
    r("internalSubset", MDT, doctype_internal_subset),
];

const MEN: &str = "Dom\\Entity";
static M_ENTITY: &[Prop] = &[
    r("publicId", MEN, entity_public_id),
    r("systemId", MEN, entity_system_id),
    r("notationName", MEN, entity_notation_name),
];

static M_ENTITY_REF_OVERRIDES: &[Prop] = &[
    r("firstChild", MN, entityref_child),
    r("lastChild", MN, entityref_child),
    r("textContent", MN, entityref_text_content),
    r("childNodes", MN, m_entityref_child_nodes),
];

const MPI: &str = "Dom\\ProcessingInstruction";
static M_PROC_INST: &[Prop] = &[
    r("target", MPI, pi_target),
    w("data", MPI, pi_data, pi_data_write, Ty::Str),
];

static M_NODELIST: &[Prop] = &[r("length", "Dom\\NodeList", nodelist_length)];
static M_HTMLCOLLECTION: &[Prop] = &[r("length", "Dom\\HTMLCollection", nodelist_length)];
static M_NAMEDNODEMAP: &[Prop] = &[r("length", "Dom\\NamedNodeMap", map_length)];
static M_DTDNAMEDNODEMAP: &[Prop] = &[r("length", "Dom\\DtdNamedNodeMap", map_length)];

const MXP: &str = "Dom\\XPath";
static M_XPATH: &[Prop] = &[
    r("document", MXP, xpath_document),
    w(
        "registerNodeNamespaces",
        MXP,
        xpath_register_ns,
        xpath_register_ns_write,
        Ty::Bool,
    ),
];

const MTL: &str = "Dom\\TokenList";
static M_TOKENLIST: &[Prop] = &[
    r("length", MTL, super::collections::tl_length),
    w(
        "value",
        MTL,
        super::collections::tl_value,
        super::collections::tl_value_write,
        Ty::Str,
    ),
];

/// A table's own properties, then `merge`d ones it lacks, then the
/// overwrites (DOM_OVERWRITE_PROP_HANDLER).
fn merged(parts: &[&'static [Prop]], overwrites: &[&'static [Prop]]) -> Vec<&'static Prop> {
    let mut out: Vec<&'static Prop> = Vec::new();
    for part in parts {
        for p in part.iter() {
            if !out.iter().any(|q| q.name == p.name) {
                out.push(p);
            }
        }
    }
    for o in overwrites {
        for p in o.iter() {
            if let Some(slot) = out.iter_mut().find(|q| q.name == p.name) {
                *slot = p;
            }
        }
    }
    out
}

fn build_modern_table(base: &str) -> Option<Vec<&'static Prop>> {
    let chardata = || merged(&[M_CHARDATA, M_NODE], &[M_NODE_VALUE_RW, M_TEXT_CONTENT_RW]);
    let text = || {
        let mut t: Vec<&'static Prop> = M_TEXT.iter().collect();
        for p in chardata() {
            if !t.iter().any(|q| q.name == p.name) {
                t.push(p);
            }
        }
        t
    };
    let document = || merged(&[M_DOCUMENT, M_NODE], &[]);
    Some(match base {
        "Dom\\Node" | "Dom\\Notation" => merged(&[M_NODE], &[]),
        "Dom\\DocumentFragment" => merged(&[M_FRAGMENT, M_NODE], &[M_TEXT_CONTENT_RW]),
        "Dom\\Document" | "Dom\\HTMLDocument" => document(),
        "Dom\\XMLDocument" => {
            let mut t: Vec<&'static Prop> = M_XML_DOCUMENT.iter().collect();
            for p in document() {
                if !t.iter().any(|q| q.name == p.name) {
                    t.push(p);
                }
            }
            t
        }
        "Dom\\CharacterData" | "Dom\\Comment" => chardata(),
        "Dom\\Text" | "Dom\\CDATASection" => text(),
        "Dom\\ProcessingInstruction" => {
            let mut t: Vec<&'static Prop> = M_PROC_INST.iter().collect();
            for p in chardata() {
                if !t.iter().any(|q| q.name == p.name) {
                    t.push(p);
                }
            }
            t
        }
        "Dom\\Attr" => merged(&[M_ATTR, M_NODE], &[M_NODE_VALUE_RW, M_TEXT_CONTENT_RW]),
        "Dom\\Element" | "Dom\\HTMLElement" => merged(&[M_ELEMENT, M_NODE], &[M_TEXT_CONTENT_RW]),
        "Dom\\DocumentType" => merged(&[M_DOCTYPE, M_NODE], &[]),
        "Dom\\Entity" => merged(&[M_ENTITY, M_NODE], &[]),
        "Dom\\EntityReference" => merged(&[M_NODE], &[M_ENTITY_REF_OVERRIDES]),
        "Dom\\NodeList" => merged(&[M_NODELIST], &[]),
        "Dom\\HTMLCollection" => merged(&[M_HTMLCOLLECTION], &[]),
        "Dom\\NamedNodeMap" => merged(&[M_NAMEDNODEMAP], &[]),
        "Dom\\DtdNamedNodeMap" => merged(&[M_DTDNAMEDNODEMAP], &[]),
        "Dom\\XPath" => merged(&[M_XPATH], &[]),
        "Dom\\TokenList" => merged(&[M_TOKENLIST], &[]),
        _ => return None,
    })
}

/// A class's property table: its own properties, then the inherited ones
/// it does not redefine (php's zend_hash_merge without overwrite), with
/// DOMEntityReference's overrides in place.
fn build_table(base: &str) -> Vec<&'static Prop> {
    if let Some(t) = build_modern_table(base) {
        return t;
    }
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
    "Dom\\Node",
    "Dom\\DocumentFragment",
    "Dom\\HTMLDocument",
    "Dom\\XMLDocument",
    "Dom\\Document",
    "Dom\\Comment",
    "Dom\\CDATASection",
    "Dom\\Text",
    "Dom\\ProcessingInstruction",
    "Dom\\CharacterData",
    "Dom\\Attr",
    "Dom\\HTMLElement",
    "Dom\\Element",
    "Dom\\DocumentType",
    "Dom\\Notation",
    "Dom\\Entity",
    "Dom\\EntityReference",
    "Dom\\NodeList",
    "Dom\\HTMLCollection",
    "Dom\\NamedNodeMap",
    "Dom\\DtdNamedNodeMap",
    "Dom\\XPath",
    "Dom\\TokenList",
    "Dom\\Implementation",
];

/// The DOM class an object's class derives from (dom_objects_set_class_ex
/// picks the property table by it).
fn dom_base(cx: &Cx, v: &Value, st: &DomObject) -> Option<&'static str> {
    if let Some(c) = st.prop_class.get() {
        return Some(c);
    }
    let c = dom_base_uncached(cx, v)?;
    st.prop_class.set(Some(c));
    Some(c)
}

fn dom_base_uncached(cx: &Cx, v: &Value) -> Option<&'static str> {
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

/// The table of a DOM class, built once.
fn table(base: &'static str) -> &'static [&'static Prop] {
    static TABLES: std::sync::OnceLock<Vec<(&'static str, Vec<&'static Prop>)>> =
        std::sync::OnceLock::new();
    let tables = TABLES.get_or_init(|| DOM_CLASSES.iter().map(|c| (*c, build_table(c))).collect());
    tables
        .iter()
        .find(|(c, _)| *c == base)
        .map_or(&[], |(_, t)| t.as_slice())
}

fn find(base: &'static str, name: &[u8]) -> Option<&'static Prop> {
    table(base)
        .iter()
        .copied()
        .find(|p| p.name.as_bytes() == name)
}

/// Whether DOM class `class` (or a subclass) declares property `name`
/// (php_dom.stub.php declares them, so property_exists() sees them).
pub(crate) fn has_declared_property(host: &dyn crate::Host, class: u32, name: &[u8]) -> bool {
    let mut id = class;
    for _ in 0..64 {
        let Some(cname) = host.class_name(id) else {
            return false;
        };
        if let Some(c) = DOM_CLASSES
            .iter()
            .find(|c| c.as_bytes().eq_ignore_ascii_case(&cname))
        {
            return find(c, name).is_some();
        }
        match host
            .class_parent_name(id)
            .and_then(|p| host.resolve_class(&p))
        {
            Some(p) => id = p,
            None => return false,
        }
    }
    false
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
    let st = state_of(this);
    let Some(base) = dom_base(cx, this, &st) else {
        return no_prop();
    };
    let Some(prop) = find(base, name) else {
        // A write: the prelude's __set then creates a dynamic property,
        // which the engine checks as it would for php's write_property.
        return no_prop();
    };
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
        Ty::HtmlElement => "?Dom\\HTMLElement",
    };
    let fail = |cx: &mut Cx, v: &Value| {
        Thrown::type_error(format!(
            "Cannot assign {} to property {}::${} of type {tname}",
            crate::zend::zpp::zval_value_name(cx.host, v),
            prop.decl,
            prop.name
        ))
    };
    if let Ty::HtmlElement = ty {
        return match &v {
            Value::Null => Ok(Value::Null),
            Value::Object(_)
                if cx
                    .host
                    .object_class_name(&v)
                    .is_some_and(|c| cx.host.reflect_is_a(&c, b"Dom\\HTMLElement")) =>
            {
                Ok(v)
            }
            _ => Err(fail(cx, &v)),
        };
    }
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
        for (k, val) in b.props().iter() {
            out.insert(rhp_values::ArrayKey::Str(k.clone()), val.clone());
        }
    }
    let st = state_of(v);
    let Some(base) = dom_base(cx, v, &st) else {
        return Ok(out);
    };
    let mut thrown = Vec::new();
    for p in table(base).iter().copied() {
        let val = match (p.read)(cx, &st) {
            Ok(Some(val)) => val,
            Ok(None) => continue,
            // The read handler's exception stays pending (EG(exception))
            // while the dump goes on; later ones chain onto it.
            Err(e) => {
                thrown.push(e);
                continue;
            }
        };
        // The modern $children, $classList and $implementation live in a
        // property slot that php's read handler fills (later dumps list
        // them first, and the same object comes back).
        if let (Value::Object(_), Value::Object(h)) = (&val, v) {
            if p.decl.starts_with("Dom\\")
                && matches!(p.name, "children" | "classList" | "implementation")
            {
                let key = PhpStr::from_bytes(p.name.as_bytes());
                let mut b = h.borrow_mut();
                if b.props().slot_of(&key).is_some() && !b.props().contains_key(&key) {
                    b.props_mut().insert(key, val.clone());
                }
            }
        }
        let val = match val {
            Value::Object(_) => Value::str("(object value omitted)"),
            other => other,
        };
        out.insert(
            rhp_values::ArrayKey::Str(PhpStr::from_bytes(p.name.as_bytes())),
            val,
        );
    }
    for e in thrown {
        cx.host.set_pending_throw(e);
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
    cx.host.emit_diagnostic(crate::E_DEPRECATED, msg.as_bytes())
}

fn some(v: Value) -> Result<Option<Value>, Thrown> {
    Ok(Some(v))
}

// ---- DOMNode ----

fn node_name(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    unsafe {
        some(match (*n).type_ {
            sys::XML_ELEMENT_NODE => Value::str(element_name(d, n)),
            sys::XML_ATTRIBUTE_NODE => Value::str(qualified_name(n)),
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
        if (*n).type_ == sys::XML_ELEMENT_NODE && super::modern::is_modern(d) {
            return some(Value::Null);
        }
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
            if super::modern::is_modern(d) {
                remove_all_children(n);
                sys::xmlAddChild(n, sys::xmlNewTextLen(s.as_ptr(), s.len() as i32));
                lxn::invalidate_node_list_cache(d.document());
                return Ok(());
            }
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
        let class: &[u8] = if super::modern::is_modern(d) {
            b"Dom\\NamedNodeMap"
        } else {
            b"DOMNamedNodeMap"
        };
        some(lists::new_map(cx, d, lists::Handler::Attributes, class)?)
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
    if super::modern::is_modern(d)
        && !matches!(
            unsafe { (*n).type_ },
            sys::XML_DOCUMENT_FRAG_NODE
                | sys::XML_ELEMENT_NODE
                | sys::XML_ATTRIBUTE_NODE
                | sys::XML_TEXT_NODE
                | sys::XML_CDATA_SECTION_NODE
                | sys::XML_COMMENT_NODE
                | sys::XML_PI_NODE
        )
    {
        return some(Value::Null);
    }
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
    if super::modern::is_modern(d) {
        return some(Value::str(qualified_name(n)));
    }
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
    if super::modern::is_modern(d) {
        unsafe {
            let t = sys::xmlNewDocTextLen((*n).doc, nul(&s).as_ptr(), s.len() as i32);
            sys::xmlAddChild(n, t);
        }
        return Ok(());
    }
    set_content_len(cx, n, &s)
}

fn attr_owner_element(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(create_nullable(cx, unsafe { (*n).parent }, Some(d))?)
}

// ---- DOMElement ----

fn tag_name(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    some(Value::str(element_name(d, n)))
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

// ---- the Dom\ classes' own handlers ----

/// dom_node_get_node_name_attribute_or_element, uppercased for an HTML
/// element of an HTML document in the modern API.
fn element_name(d: &DomObject, n: *mut XmlNode) -> Vec<u8> {
    let mut name = qualified_name(n);
    if super::modern::is_modern(d) && super::nscompat::ns_is_html_and_document_is_html(n) {
        name.make_ascii_uppercase();
    }
    name
}

fn m_base_uri(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    unsafe {
        if let Some(s) = sys::take_string(sys::xmlNodeGetBase((*n).doc, n)) {
            return some(Value::str(s));
        }
        let doc = (*n).doc;
        if !doc.is_null() && !(*doc).URL.is_null() {
            return some(xs((*doc).URL));
        }
    }
    some(Value::str("about:blank"))
}

fn m_child_nodes(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    node_of(cx, d)?;
    some(lists::new_map(
        cx,
        d,
        lists::Handler::ChildNodes,
        b"Dom\\NodeList",
    )?)
}

fn m_entityref_child_nodes(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    entity_reference_sync(n);
    m_child_nodes(cx, d)
}

fn m_children(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    node_of(cx, d)?;
    some(lists::new_map(
        cx,
        d,
        lists::Handler::ChildElements,
        b"Dom\\HTMLCollection",
    )?)
}

fn m_prefix(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    unsafe {
        let ns = (*n).ns;
        some(if !ns.is_null() && !(*ns).prefix.is_null() {
            xs((*ns).prefix)
        } else {
            Value::Null
        })
    }
}

fn m_doc_implementation(cx: &mut Cx, _: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    some(
        cx.host
            .instantiate(b"Dom\\Implementation", Vec::new())
            .unwrap_or(Value::Null),
    )
}

fn m_doc_uri(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = doc_of(cx, d)?;
    unsafe {
        some(if (*doc).URL.is_null() {
            Value::str("about:blank")
        } else {
            xs((*doc).URL)
        })
    }
}

/// dom_html_document_encoding_write (both modern document classes)
fn m_doc_encoding_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let doc = doc_of(cx, d)?;
    rhp_csrc::lexbor::setup();
    let s = conv::to_string_bytes(&v);
    match super::html5::encoding_name(&s) {
        Some(name) => {
            let cn = c(&name);
            unsafe {
                sys::xml_free((*doc).encoding as *mut u8);
                (*doc).encoding = sys::xmlStrdup(cn.as_ptr());
            }
            Ok(())
        }
        None => Err(Thrown::error("ValueError", "Invalid document encoding")),
    }
}

fn m_doc_version_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let s = conv::to_string_bytes(&v);
    if s != b"1.0" && s != b"1.1" {
        return Err(Thrown::error("ValueError", "Invalid XML version"));
    }
    doc_version_write(cx, d, v)
}

/// dom_html_document_element_read_raw
fn html_child(doc: *mut XmlDoc, accept: fn(&[u8]) -> bool) -> *mut XmlNode {
    unsafe {
        let root = sys::xmlDocGetRootElement(doc);
        if root.is_null()
            || !(super::nscompat::ns_is_fast(root, &super::nscompat::HTML)
                && b((*root).name) == b"html")
        {
            return std::ptr::null_mut();
        }
        let mut cur = (*root).children;
        while !cur.is_null() {
            if (*cur).type_ == sys::XML_ELEMENT_NODE
                && super::nscompat::ns_is_fast(cur, &super::nscompat::HTML)
                && accept(b((*cur).name))
            {
                return cur;
            }
            cur = (*cur).next;
        }
    }
    std::ptr::null_mut()
}

fn accept_body(n: &[u8]) -> bool {
    n == b"body" || n == b"frameset"
}

fn accept_head(n: &[u8]) -> bool {
    n == b"head"
}

fn m_body(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = doc_of(cx, d)?;
    some(create_nullable(cx, html_child(doc, accept_body), Some(d))?)
}

fn m_head(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = doc_of(cx, d)?;
    some(create_nullable(cx, html_child(doc, accept_head), Some(d))?)
}

fn m_body_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let doc = doc_of(cx, d)?;
    let fail = |cx: &mut Cx, msg: &str| -> Result<(), Thrown> {
        super::throw_msg(cx, super::HIERARCHY_REQUEST_ERR, msg, true)?;
        Ok(())
    };
    let newnode = super::intern_of(&v).map_or(std::ptr::null_mut(), |o| o.node());
    if !newnode.is_null() && accept_body(unsafe { b((*newnode).name) }) {
        let current = html_child(doc, accept_body);
        if current == newnode {
            return Ok(());
        }
        unsafe {
            if !current.is_null() {
                super::modern::adopt(d, newnode, doc);
                let old = sys::xmlReplaceNode(current, newnode);
                if !old.is_null() && (*old)._private.is_null() {
                    lxn::node_free_resource(old);
                }
                return Ok(());
            }
            let root = sys::xmlDocGetRootElement(doc);
            if root.is_null() {
                return fail(cx, "A body can only be set if there is a document element");
            }
            super::modern::adopt(d, newnode, doc);
            sys::xmlAddChild(root, newnode);
        }
        return Ok(());
    }
    fail(cx, "The new body must either be a body or a frameset tag")
}

/// dom_get_child_text_content
fn child_text_content(node: *const XmlNode) -> Vec<u8> {
    let mut out = Vec::new();
    unsafe {
        let mut t = (*node).children;
        while !t.is_null() {
            if matches!((*t).type_, sys::XML_TEXT_NODE | sys::XML_CDATA_SECTION_NODE)
                && !(*t).content.is_null()
            {
                out.extend_from_slice(b((*t).content));
            }
            t = (*t).next;
        }
    }
    out
}

fn title_element(doc: *mut XmlDoc) -> *mut XmlNode {
    unsafe {
        let mut node = (*doc).children;
        while !node.is_null() {
            if (*node).type_ == sys::XML_ELEMENT_NODE
                && super::nscompat::ns_is_fast(node, &super::nscompat::HTML)
                && b((*node).name) == b"title"
            {
                return node;
            }
            node = next_in_tree_order(node, std::ptr::null());
        }
    }
    std::ptr::null_mut()
}

fn svg_title_element(svg: *mut XmlNode) -> *mut XmlNode {
    unsafe {
        let mut cur = (*svg).children;
        while !cur.is_null() {
            if (*cur).type_ == sys::XML_ELEMENT_NODE
                && super::nscompat::ns_is_fast(cur, &super::nscompat::SVG)
                && b((*cur).name) == b"title"
            {
                return cur;
            }
            cur = (*cur).next;
        }
    }
    std::ptr::null_mut()
}

/// dom_strip_and_collapse_ascii_whitespace
fn strip_and_collapse(s: &[u8]) -> Vec<u8> {
    let ws = super::modern::ASCII_WHITESPACE;
    let parts: Vec<&[u8]> = s
        .split(|c| ws.contains(c))
        .filter(|p| !p.is_empty())
        .collect();
    parts.join(&b' ')
}

fn m_title(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let doc = doc_of(cx, d)?;
    unsafe {
        let root = sys::xmlDocGetRootElement(doc);
        if root.is_null() {
            return some(Value::str(""));
        }
        let mut value = Vec::new();
        if super::nscompat::ns_is_fast(root, &super::nscompat::SVG) && b((*root).name) == b"svg" {
            let t = svg_title_element(root);
            if !t.is_null() {
                value = child_text_content(t);
            }
        } else {
            let t = title_element(doc);
            if !t.is_null() {
                value = child_text_content(t);
            }
        }
        some(Value::str(strip_and_collapse(&value)))
    }
}

fn m_title_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let doc = doc_of(cx, d)?;
    let s = conv::to_string_bytes(&v);
    unsafe {
        let root = sys::xmlDocGetRootElement(doc);
        if root.is_null() {
            return Ok(());
        }
        let replace_all = |el: *mut XmlNode| {
            remove_all_children(el);
            let cs = c(&s);
            let text = sys::xmlNewDocText(doc, cs.as_ptr());
            sys::xmlAddChild(el, text);
        };
        if super::nscompat::ns_is_fast(root, &super::nscompat::SVG) && b((*root).name) == b"svg" {
            let mut element = svg_title_element(root);
            if element.is_null() {
                let mut ns = (*root).ns;
                if !(*ns).prefix.is_null() {
                    ns = super::modern::mapper(d).get_ns(b"", super::nscompat::SVG_NS_URI);
                }
                element = sys::xmlNewDocNode(doc, ns, c"title".as_ptr().cast(), std::ptr::null());
                if element.is_null() {
                    super::throw(cx, INVALID_STATE_ERR, true)?;
                }
                if (*root).children.is_null() {
                    (*root).last = element;
                } else {
                    (*element).next = (*root).children;
                    (*(*root).children).prev = element;
                }
                (*root).children = element;
                (*element).parent = root;
            }
            replace_all(element);
        } else if super::nscompat::ns_is_fast(root, &super::nscompat::HTML) {
            let title = title_element(doc);
            let head = html_child(doc, accept_head);
            if title.is_null() && head.is_null() {
                return Ok(());
            }
            let mut element = title;
            if element.is_null() {
                let ns = super::modern::mapper(d).ensure_html_ns();
                element = sys::xmlNewDocNode(doc, ns, c"title".as_ptr().cast(), std::ptr::null());
                if element.is_null() {
                    super::throw(cx, INVALID_STATE_ERR, true)?;
                }
                sys::xmlAddChild(head, element);
            }
            replace_all(element);
        }
    }
    Ok(())
}

fn m_class_list(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    node_of(cx, d)?;
    some(super::collections::new_token_list(cx, d)?)
}

/// dom_element_html_fragment_serialize
fn fragment_serialize(
    cx: &mut Cx,
    d: &DomObject,
    node: *mut XmlNode,
) -> Result<Option<Value>, Thrown> {
    let private = unsafe { super::modern::private(d) }.map(|p| &*p);
    unsafe {
        if (*(*node).doc).type_ == sys::XML_HTML_DOCUMENT_NODE {
            let mut out = super::html5_serializer::Out::default();
            super::html5_serializer::serialize(&mut out, private, node);
            return some(Value::str(out.buf));
        }
    }
    match super::xml_serializer::fragment_to_str(node, private)? {
        Some(s) => some(Value::str(s)),
        None => {
            super::throw_msg(
                cx,
                super::SYNTAX_ERR,
                "The resulting XML serialization is not well-formed",
                true,
            )?;
            Ok(None)
        }
    }
}

fn m_inner_html(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    fragment_serialize(cx, d, n)
}

fn m_outer_html(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let this = node_of(cx, d)?;
    unsafe {
        let mut element: XmlNode = std::mem::zeroed();
        element.type_ = sys::XML_DOCUMENT_FRAG_NODE;
        element.children = this;
        element.last = this;
        element.doc = (*this).doc;
        let old_parent = (*this).parent;
        let old_next = (*this).next;
        (*this).parent = &mut element;
        (*this).next = std::ptr::null_mut();
        let r = fragment_serialize(cx, d, &mut element);
        (*this).parent = old_parent;
        (*this).next = old_next;
        r
    }
}

fn m_inner_html_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    super::modern::inner_html_write(cx, d, &conv::to_string_bytes(&v))
}

fn m_outer_html_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    super::modern::outer_html_write(cx, d, &conv::to_string_bytes(&v))
}

fn m_substituted(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let n = node_of(cx, d)?;
    match unsafe { sys::take_string(sys::xmlNodeGetContent(n)) } {
        Some(s) => some(Value::str(s)),
        None => {
            super::throw(cx, INVALID_STATE_ERR, true)?;
            Ok(None)
        }
    }
}

fn m_substituted_write(cx: &mut Cx, d: &Rc<DomObject>, v: Value) -> Result<(), Thrown> {
    let n = node_of(cx, d)?;
    lxn::invalidate_node_list_cache(d.document());
    remove_all_children(n);
    set_content_len(cx, n, &conv::to_string_bytes(&v))
}

fn m_doctype_entities(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let dtd = dtd_of(cx, d)?;
    let ht = unsafe { (*dtd).entities };
    some(lists::new_map(
        cx,
        d,
        lists::Handler::Entities(ht),
        b"Dom\\DtdNamedNodeMap",
    )?)
}

fn m_doctype_notations(cx: &mut Cx, d: &Rc<DomObject>) -> Result<Option<Value>, Thrown> {
    let dtd = dtd_of(cx, d)?;
    let ht = unsafe { (*dtd).notations };
    some(lists::new_map(
        cx,
        d,
        lists::Handler::Notations(ht),
        b"Dom\\DtdNamedNodeMap",
    )?)
}
