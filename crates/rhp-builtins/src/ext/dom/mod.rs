//! The `dom` extension: php's legacy DOM classes (`DOMDocument`, `DOMElement`,
//! `DOMXPath`, …) over the shared libxml tree (`ext/libxml/tree.rs`).
//!
//! The classes are PHP-source (see [`PRELUDE_PHP`]); every node object holds
//! its document id and node id in the internal slots `__d` / `__n` and
//! forwards to the dispatcher builtin `__dom($op, ...)`. Node identity is
//! kept the way php does it: one live PHP object per libxml node (a weak
//! cache, so a node whose object was freed gets a fresh object).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use rhp_values::{conv, ArrayKey, ObjectData, PhpArray, PhpStr, Value};

use crate::ext::libxml::parser::{self, ParseOpts, XmlError};
use crate::ext::libxml::tree::{self, Doc, DocId, Kind, Node, NodeId, SaveOpts, XMLNS_NS, XML_NS};
use crate::ext::libxml::{self as lx, xpath, xsd, Ctx};
use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(deref).unwrap_or(Value::Null)
}

fn int(args: &[Value], i: usize) -> i64 {
    conv::to_int(&arg(args, i))
}

fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&arg(args, i))
}

fn opt_bytes(args: &[Value], i: usize) -> Option<Vec<u8>> {
    match arg(args, i) {
        Value::Null => None,
        v => Some(conv::to_string_bytes(&v)),
    }
}

fn s(b: impl Into<Vec<u8>>) -> Value {
    Value::str(b)
}

// ------------------------------------------------------------ wrapper cache

type WrapperCache = HashMap<(DocId, NodeId), Weak<RefCell<ObjectData>>>;

thread_local! {
    static OBJS: RefCell<WrapperCache> = RefCell::new(HashMap::new());
}

fn cache_get(d: DocId, n: NodeId) -> Option<Value> {
    OBJS.with(|o| {
        o.borrow()
            .get(&(d, n))
            .and_then(|w| w.upgrade())
            .map(Value::Object)
    })
}

fn cache_put(d: DocId, n: NodeId, v: &Value) {
    if let Value::Object(h) = v {
        OBJS.with(|o| {
            o.borrow_mut().insert((d, n), Rc::downgrade(h));
        });
    }
}

/// Re-point the wrapper objects of moved nodes (`map`: old id -> new id).
fn cache_move(from: DocId, to: DocId, map: &[(NodeId, NodeId)]) {
    OBJS.with(|o| {
        let mut o = o.borrow_mut();
        for &(old, new) in map {
            if let Some(w) = o.remove(&(from, old)) {
                if let Some(h) = w.upgrade() {
                    let mut b = h.borrow_mut();
                    b.properties
                        .insert(PhpStr::from_bytes(b"__d".to_vec()), Value::Int(to as i64));
                    b.properties
                        .insert(PhpStr::from_bytes(b"__n".to_vec()), Value::Int(new as i64));
                }
                o.insert((to, new), w);
            }
        }
    });
}

fn class_for(kind: Kind) -> &'static [u8] {
    match kind {
        Kind::Element => b"DOMElement",
        Kind::Attr => b"DOMAttr",
        Kind::Text => b"DOMText",
        Kind::CData => b"DOMCdataSection",
        Kind::EntityRef => b"DOMEntityReference",
        Kind::Pi => b"DOMProcessingInstruction",
        Kind::Comment => b"DOMComment",
        Kind::Document => b"DOMDocument",
        Kind::DocType => b"DOMDocumentType",
        Kind::Fragment => b"DOMDocumentFragment",
    }
}

/// The PHP object for node `n` of document `d` (php_dom_create_object).
pub fn wrap(host: &mut dyn Host, d: DocId, n: NodeId) -> Value {
    if let Some(v) = cache_get(d, n) {
        return v;
    }
    let Some(kind) = tree::with_doc(d, |doc| doc.nodes.get(n).map(|x| x.kind)).flatten() else {
        return Value::Null;
    };
    let obj = host
        .instantiate(
            class_for(kind),
            vec![
                (b"__d".to_vec(), Value::Int(d as i64)),
                (b"__n".to_vec(), Value::Int(n as i64)),
            ],
        )
        .unwrap_or(Value::Null);
    cache_put(d, n, &obj);
    obj
}

fn wrap_opt(host: &mut dyn Host, d: DocId, n: Option<NodeId>) -> Value {
    match n {
        Some(n) => wrap(host, d, n),
        None => Value::Null,
    }
}

/// Register `obj` (a user-constructed node / document) as the wrapper of `(d, n)`.
pub fn bind(obj: &Value, d: DocId, n: NodeId) {
    cache_put(d, n, obj);
    if let Value::Object(h) = obj {
        let mut b = h.borrow_mut();
        b.properties
            .insert(PhpStr::from_bytes(b"__d".to_vec()), Value::Int(d as i64));
        b.properties
            .insert(PhpStr::from_bytes(b"__n".to_vec()), Value::Int(n as i64));
    }
}

/// `(doc, node)` ids of a DOM object (via its internal slots).
pub fn ids_of(v: &Value) -> Option<(DocId, NodeId)> {
    let Value::Object(h) = v else {
        return None;
    };
    let b = h.borrow();
    let d = b.properties.get(&PhpStr::from_bytes(b"__d".to_vec()))?;
    let n = b.properties.get(&PhpStr::from_bytes(b"__n".to_vec()))?;
    match (d, n) {
        (Value::Int(d), Value::Int(n)) => Some((*d as usize, *n as usize)),
        _ => None,
    }
}

fn node_list(host: &mut dyn Host, d: DocId, n: NodeId, mode: i64, a: Value, b: Value) -> Value {
    host.instantiate(
        b"DOMNodeList",
        vec![
            (b"__d".to_vec(), Value::Int(d as i64)),
            (b"__n".to_vec(), Value::Int(n as i64)),
            (b"__m".to_vec(), Value::Int(mode)),
            (b"__a".to_vec(), a),
            (b"__b".to_vec(), b),
        ],
    )
    .unwrap_or(Value::Null)
}

fn static_list(host: &mut dyn Host, d: DocId, ids: &[NodeId]) -> Value {
    let mut arr = PhpArray::new();
    for &i in ids {
        arr.push(Value::Int(i as i64));
    }
    node_list(host, d, 0, 3, Value::Array(arr), Value::Null)
}

// ------------------------------------------------------------ DOM errors

pub const INDEX_SIZE_ERR: i64 = 1;
pub const HIERARCHY_REQUEST_ERR: i64 = 3;
pub const WRONG_DOCUMENT_ERR: i64 = 4;
pub const INVALID_CHARACTER_ERR: i64 = 5;
pub const NOT_FOUND_ERR: i64 = 8;
pub const NOT_SUPPORTED_ERR: i64 = 9;
pub const INUSE_ATTRIBUTE_ERR: i64 = 10;
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

/// php_dom_throw_error: a DOMException when strict, else an E_WARNING;
/// either way the method then returns false.
fn dom_throw(host: &mut dyn Host, code: i64, strict: bool) -> Result<Value, Thrown> {
    if strict {
        host.call_callable(
            &s(b"__dom_throw".to_vec()),
            &[Value::Int(code), s(err_message(code).as_bytes().to_vec())],
        )?;
        return Ok(Value::Bool(false));
    }
    host.emit_diagnostic(crate::E_WARNING, err_message(code))?;
    Ok(Value::Bool(false))
}

fn strict(d: DocId) -> bool {
    tree::with_doc(d, |doc| doc.strict_error_checking).unwrap_or(true)
}

// ------------------------------------------------------------ names

pub fn valid_name(n: &[u8]) -> bool {
    let Some(&f) = n.first() else {
        return false;
    };
    if !(f.is_ascii_alphabetic() || f == b'_' || f == b':' || f >= 0x80) {
        return false;
    }
    n.iter()
        .all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'-' | b'.') || c >= 0x80)
}

pub fn valid_qname(n: &[u8]) -> bool {
    if !valid_name(n) {
        return false;
    }
    let parts: Vec<&[u8]> = n.split(|&c| c == b':').collect();
    match parts.len() {
        1 => true,
        2 => {
            !parts[0].is_empty()
                && !parts[1].is_empty()
                && !parts[1][0].is_ascii_digit()
                && parts[1][0] != b'-'
                && parts[1][0] != b'.'
        }
        _ => false,
    }
}

fn split_qname(q: &[u8]) -> (Vec<u8>, Vec<u8>) {
    match q.iter().position(|&c| c == b':') {
        Some(i) => (q[..i].to_vec(), q[i + 1..].to_vec()),
        None => (Vec::new(), q.to_vec()),
    }
}

// ------------------------------------------------------------ content helpers

/// libxml2's `xmlStringGetNodeList`: element content given in escaped form.
/// Returns the new child nodes, or an "unterminated entity" warning text.
pub(crate) fn string_node_list(doc: &mut Doc, v: &[u8]) -> Result<Vec<NodeId>, String> {
    let mut out = Vec::new();
    let mut buf = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if v[i] == b'&' {
            let rest = &v[i + 1..];
            let Some(end) = rest.iter().position(|&c| c == b';') else {
                let q = String::from_utf8_lossy(rest).into_owned();
                return Err(format!("unterminated entity reference {q:>15}"));
            };
            let name = &rest[..end];
            if let Some(num) = name.strip_prefix(b"#") {
                let cp = if let Some(h) = num.strip_prefix(b"x") {
                    u32::from_str_radix(std::str::from_utf8(h).unwrap_or(""), 16).ok()
                } else {
                    std::str::from_utf8(num).ok().and_then(|x| x.parse().ok())
                };
                if let Some(c) = cp.and_then(char::from_u32) {
                    let mut b = [0u8; 4];
                    buf.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
                }
            } else if let Some(val) = match name {
                b"lt" => Some(&b"<"[..]),
                b"gt" => Some(&b">"[..]),
                b"amp" => Some(&b"&"[..]),
                b"quot" => Some(&b"\""[..]),
                b"apos" => Some(&b"'"[..]),
                _ => None,
            } {
                buf.extend_from_slice(val);
            } else {
                if !buf.is_empty() {
                    out.push(doc.new_text(Kind::Text, &std::mem::take(&mut buf)));
                }
                let mut n = Node::new(Kind::EntityRef);
                n.name = name.to_vec();
                out.push(doc.add(n));
            }
            i += end + 2;
        } else {
            buf.push(v[i]);
            i += 1;
        }
    }
    if !buf.is_empty() || out.is_empty() {
        out.push(doc.new_text(Kind::Text, &buf));
    }
    Ok(out)
}

/// Like [`string_node_list`] but for attribute values (flattened to text).
fn decode_entities(doc: &Doc, v: &[u8]) -> Result<Vec<u8>, String> {
    let mut tmp = doc.clone();
    let ids = string_node_list(&mut tmp, v)?;
    let mut out = Vec::new();
    for i in ids {
        out.extend(tmp.text_content(i));
    }
    Ok(out)
}

fn element_children(doc: &Doc, n: NodeId) -> Vec<NodeId> {
    doc.children(n)
        .into_iter()
        .filter(|&c| doc.nodes[c].kind == Kind::Element)
        .collect()
}

fn sibling_element(doc: &Doc, n: NodeId, forward: bool) -> Option<NodeId> {
    let mut c = if forward {
        doc.nodes[n].next
    } else {
        doc.nodes[n].prev
    };
    while let Some(x) = c {
        if doc.nodes[x].kind == Kind::Element {
            return Some(x);
        }
        c = if forward {
            doc.nodes[x].next
        } else {
            doc.nodes[x].prev
        };
    }
    None
}

/// Elements under `root` (descendants, document order) matching a tag name
/// (`*` matches all) — getElementsByTagName.
fn by_tag(doc: &Doc, root: NodeId, name: &[u8]) -> Vec<NodeId> {
    let mut all = Vec::new();
    doc.descendants(root, &mut all);
    all.into_iter()
        .filter(|&x| {
            let n = &doc.nodes[x];
            // legacy DOM compares libxml2's node->name: the local part
            n.kind == Kind::Element && (name == b"*" || n.name == name)
        })
        .collect()
}

fn by_tag_ns(doc: &Doc, root: NodeId, ns: Option<&[u8]>, local: &[u8]) -> Vec<NodeId> {
    let mut all = Vec::new();
    doc.descendants(root, &mut all);
    all.into_iter()
        .filter(|&x| {
            let n = &doc.nodes[x];
            n.kind == Kind::Element
                && (local == b"*" || n.name == local)
                && match ns {
                    Some(b"*") => true,
                    Some(b"") | None => n.ns.is_none(),
                    Some(u) => n.ns.as_deref() == Some(u),
                }
        })
        .collect()
}

/// xmlGetNodePath.
fn node_path(doc: &Doc, id: NodeId) -> Vec<u8> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = id;
    loop {
        let n = &doc.nodes[cur];
        let same = |pred: &dyn Fn(&Node) -> bool| -> (usize, usize) {
            // (index among matching siblings, total)
            let Some(p) = n.parent else {
                return (1, 1);
            };
            let mut idx = 0;
            let mut total = 0;
            let mut c = doc.nodes[p].first;
            while let Some(x) = c {
                if pred(&doc.nodes[x]) {
                    total += 1;
                    if x == cur {
                        idx = total;
                    }
                }
                c = doc.nodes[x].next;
            }
            (idx, total)
        };
        let seg = match n.kind {
            Kind::Document => break,
            Kind::Element => {
                let q = String::from_utf8_lossy(&n.qname()).into_owned();
                let (i, t) = same(&|x: &Node| x.kind == Kind::Element && x.qname() == n.qname());
                if t > 1 {
                    format!("{q}[{i}]")
                } else {
                    q
                }
            }
            Kind::Attr => format!("@{}", String::from_utf8_lossy(&n.qname())),
            Kind::Text | Kind::CData => {
                let (i, t) = same(&|x: &Node| matches!(x.kind, Kind::Text | Kind::CData));
                if t > 1 {
                    format!("text()[{i}]")
                } else {
                    "text()".into()
                }
            }
            Kind::Comment => {
                let (i, t) = same(&|x: &Node| x.kind == Kind::Comment);
                if t > 1 {
                    format!("comment()[{i}]")
                } else {
                    "comment()".into()
                }
            }
            Kind::Pi => {
                let (i, t) = same(&|x: &Node| x.kind == Kind::Pi);
                if t > 1 {
                    format!(
                        "processing-instruction('{}')[{i}]",
                        String::from_utf8_lossy(&n.name)
                    )
                } else {
                    format!(
                        "processing-instruction('{}')",
                        String::from_utf8_lossy(&n.name)
                    )
                }
            }
            _ => String::new(),
        };
        parts.push(seg);
        match n.parent {
            Some(p) => cur = p,
            None => {
                if !doc.connected(id) {
                    // detached subtree: relative path from its root
                    parts.reverse();
                    return parts.join("/").into_bytes();
                }
                break;
            }
        }
    }
    parts.reverse();
    if parts.is_empty() {
        return b"/".to_vec();
    }
    format!("/{}", parts.join("/")).into_bytes()
}

// ------------------------------------------------------------ dispatcher

fn fn_dom(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = bytes(args, 0);
    let op = String::from_utf8_lossy(&op).into_owned();
    let d = int(args, 1) as usize;
    let n = int(args, 2) as usize;
    match op.as_str() {
        "newdoc" => {
            // (op, obj, version, encoding)
            let mut doc = Doc::new();
            doc.version = Some(bytes(args, 2));
            let enc = bytes(args, 3);
            if !enc.is_empty() {
                doc.encoding = Some(enc);
            }
            let id = tree::register(doc);
            bind(&arg(args, 1), id, 0);
            Ok(Value::Int(id as i64))
        }
        "newnode" => new_node(host, args),
        "load" => load(host, args),
        "save" => save_xml(host, args),
        "savefile" => {
            let mut a = args.to_vec();
            let fname = bytes(args, 3);
            a[2] = Value::Null;
            let out = save_xml(
                host,
                &[
                    s(b"save".to_vec()),
                    a[1].clone(),
                    Value::Null,
                    Value::Null,
                    arg(args, 4),
                ],
            )?;
            let Value::Str(data) = out else {
                return Ok(Value::Bool(false));
            };
            match lx::save_to_uri(host, "DOMDocument::save", &fname, data.as_bytes())? {
                Some(n) => Ok(Value::Int(n as i64)),
                None => Ok(Value::Bool(false)),
            }
        }
        "get" => get_prop(host, d, n, &bytes(args, 3)),
        // (op, len): a fallible zend_string_alloc, for prelude methods php
        // allocates that way (false past memory_limit).
        "reserve" => Ok(Value::Bool(rhp_values::mem::reserve_str(d))),
        "dynprop" => {
            // (op, class, name)
            let class = String::from_utf8_lossy(&bytes(args, 1)).into_owned();
            let name = String::from_utf8_lossy(&bytes(args, 2)).into_owned();
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                &format!("Creation of dynamic property {class}::${name} is deprecated"),
            )?;
            Ok(Value::Null)
        }
        "set" => set_prop(host, d, n, &bytes(args, 3), arg(args, 4)),
        "isset" => {
            let v = get_prop(host, d, n, &bytes(args, 3))?;
            Ok(Value::Bool(!matches!(v, Value::Null) && !is_unknown(&v)))
        }
        "create" => create(host, d, args),
        "append" => insert(host, d, n, args, None, false),
        "insertBefore" => {
            let refn = match arg(args, 4) {
                Value::Null => None,
                v => Some(ids_of(&v)),
            };
            insert(host, d, n, args, refn, true)
        }
        "removeChild" => {
            let Some((cd, cn)) = ids_of(&arg(args, 3)) else {
                return Ok(Value::Bool(false));
            };
            let ok = cd == d
                && tree::with_doc(d, |doc| {
                    doc.nodes[cn].parent == Some(n) && doc.nodes[cn].kind != Kind::Attr
                })
                .unwrap_or(false);
            if !ok {
                return dom_throw(host, NOT_FOUND_ERR, strict(d));
            }
            tree::with_doc(d, |doc| doc.unlink(cn));
            Ok(wrap(host, d, cn))
        }
        "replaceChild" => {
            let (Some((nd, nn)), Some((od, on))) = (ids_of(&arg(args, 3)), ids_of(&arg(args, 4)))
            else {
                return Ok(Value::Bool(false));
            };
            let ok = od == d
                && tree::with_doc(d, |doc| doc.nodes[on].parent == Some(n)).unwrap_or(false);
            if !ok {
                return dom_throw(host, NOT_FOUND_ERR, strict(d));
            }
            if (nd, nn) == (od, on) {
                return Ok(arg(args, 4));
            }
            let r = insert(
                host,
                d,
                n,
                &[Value::Null, Value::Null, Value::Null, arg(args, 3)],
                Some(Some((od, on))),
                true,
            )?;
            if matches!(r, Value::Bool(false)) {
                return Ok(r);
            }
            tree::with_doc(d, |doc| doc.unlink(on));
            Ok(wrap(host, d, on))
        }
        "clone" => {
            let deep = conv::to_bool(&arg(args, 3));
            let id = tree::with_doc(d, |doc| doc.clone_node(n, deep)).unwrap_or(0);
            Ok(wrap(host, d, id))
        }
        "hasChildNodes" => Ok(Value::Bool(
            tree::with_doc(d, |doc| doc.nodes[n].first.is_some()).unwrap_or(false),
        )),
        "hasAttributes" => Ok(Value::Bool(
            tree::with_doc(d, |doc| {
                doc.nodes[n].kind == Kind::Element
                    && (!doc.nodes[n].attrs.is_empty() || !doc.nodes[n].ns_defs.is_empty())
            })
            .unwrap_or(false),
        )),
        "normalize" => {
            tree::with_doc(d, |doc| normalize(doc, n));
            Ok(Value::Null)
        }
        "isSame" => Ok(Value::Bool(ids_of(&arg(args, 3)) == Some((d, n)))),
        "lookupNamespaceURI" => {
            let p = opt_bytes(args, 3).unwrap_or_default();
            let r = tree::with_doc(d, |doc| {
                let start = if doc.nodes[n].kind == Kind::Document {
                    doc.root()
                } else {
                    Some(n)
                };
                start.and_then(|st| doc.lookup_ns(st, &p))
            })
            .flatten();
            Ok(r.map(s).unwrap_or(Value::Null))
        }
        "lookupPrefix" => {
            let u = bytes(args, 3);
            let r = tree::with_doc(d, |doc| {
                let start = if doc.nodes[n].kind == Kind::Document {
                    doc.root()
                } else {
                    Some(n)
                };
                start
                    .and_then(|st| doc.lookup_prefix(st, &u))
                    .filter(|p| !p.is_empty())
            })
            .flatten();
            Ok(r.map(s).unwrap_or(Value::Null))
        }
        "isDefaultNamespace" => {
            let u = bytes(args, 3);
            let r = tree::with_doc(d, |doc| {
                let start = if doc.nodes[n].kind == Kind::Document {
                    doc.root()
                } else {
                    Some(n)
                };
                start.and_then(|st| doc.lookup_ns(st, b"")) == Some(u.clone())
            })
            .unwrap_or(false);
            Ok(Value::Bool(r))
        }
        "nodePath" => Ok(s(
            tree::with_doc(d, |doc| node_path(doc, n)).unwrap_or_default()
        )),
        "lineNo" => Ok(Value::Int(
            tree::with_doc(d, |doc| doc.nodes[n].line as i64).unwrap_or(0),
        )),
        "contains" => {
            let r = match ids_of(&arg(args, 3)) {
                Some((od, on)) if od == d => {
                    tree::with_doc(d, |doc| doc.is_ancestor(n, on)).unwrap_or(false)
                }
                _ => false,
            };
            Ok(Value::Bool(r))
        }
        "rootNode" => {
            let r = tree::with_doc(d, |doc| {
                let mut c = n;
                while let Some(p) = doc.nodes[c].parent {
                    c = p;
                }
                c
            })
            .unwrap_or(n);
            Ok(wrap(host, d, r))
        }
        "isEqual" => {
            let r = match ids_of(&arg(args, 3)) {
                Some((od, on)) => {
                    let a = tree::clone_doc(d);
                    let b = tree::clone_doc(od);
                    match (a, b) {
                        (Some(a), Some(b)) => nodes_equal(&a, n, &b, on),
                        _ => false,
                    }
                }
                None => false,
            };
            Ok(Value::Bool(r))
        }
        "c14n" => Ok(s(tree::with_doc(d, |doc| c14n(doc, n)).unwrap_or_default())),
        // ---- elements
        "getAttribute" => {
            let name = bytes(args, 3);
            Ok(s(tree::with_doc(d, |doc| get_attribute(doc, n, &name))
                .flatten()
                .unwrap_or_default()))
        }
        "hasAttribute" => {
            let name = bytes(args, 3);
            Ok(Value::Bool(
                tree::with_doc(d, |doc| get_attribute(doc, n, &name).is_some()).unwrap_or(false),
            ))
        }
        "setAttribute" => set_attribute(host, d, n, &bytes(args, 3), &bytes(args, 4)),
        "removeAttribute" => {
            let name = bytes(args, 3);
            let r = tree::with_doc(d, |doc| remove_attribute(doc, n, &name)).unwrap_or(false);
            Ok(Value::Bool(r))
        }
        "getAttributeNode" => {
            let name = bytes(args, 3);
            let a = tree::with_doc(d, |doc| find_attr_qname(doc, n, &name)).flatten();
            match a {
                Some(a) => Ok(wrap(host, d, a)),
                None => Ok(Value::Bool(false)),
            }
        }
        "getAttributeNS" => {
            let ns = opt_bytes(args, 3);
            let local = bytes(args, 4);
            let r = tree::with_doc(d, |doc| {
                if ns.as_deref() == Some(XMLNS_NS) {
                    let p = if local == b"xmlns" {
                        Vec::new()
                    } else {
                        local.clone()
                    };
                    return doc.nodes[n]
                        .ns_defs
                        .iter()
                        .find(|(x, _)| *x == p)
                        .map(|(_, u)| u.clone());
                }
                let ns = ns.as_deref().filter(|x| !x.is_empty());
                doc.find_attr_ns(n, ns, &local)
                    .map(|a| doc.nodes[a].content.clone())
            })
            .flatten();
            Ok(s(r.unwrap_or_default()))
        }
        "hasAttributeNS" => {
            let ns = opt_bytes(args, 3);
            let local = bytes(args, 4);
            let r = tree::with_doc(d, |doc| {
                if ns.as_deref() == Some(XMLNS_NS) {
                    let p = if local == b"xmlns" {
                        Vec::new()
                    } else {
                        local.clone()
                    };
                    return doc.nodes[n].ns_defs.iter().any(|(x, _)| *x == p);
                }
                let ns = ns.as_deref().filter(|x| !x.is_empty());
                doc.find_attr_ns(n, ns, &local).is_some()
            })
            .unwrap_or(false);
            Ok(Value::Bool(r))
        }
        "getAttributeNodeNS" => {
            let ns = opt_bytes(args, 3);
            let local = bytes(args, 4);
            let a = tree::with_doc(d, |doc| {
                let ns = ns.as_deref().filter(|x| !x.is_empty());
                doc.find_attr_ns(n, ns, &local)
            })
            .flatten();
            Ok(match a {
                Some(a) => wrap(host, d, a),
                None => Value::Null,
            })
        }
        "setAttributeNS" => set_attribute_ns(
            host,
            d,
            n,
            opt_bytes(args, 3),
            &bytes(args, 4),
            &bytes(args, 5),
        ),
        "removeAttributeNS" => {
            let ns = opt_bytes(args, 3);
            let local = bytes(args, 4);
            tree::with_doc(d, |doc| {
                if ns.as_deref() == Some(XMLNS_NS) {
                    let p = if local == b"xmlns" {
                        Vec::new()
                    } else {
                        local.clone()
                    };
                    doc.nodes[n].ns_defs.retain(|(x, _)| *x != p);
                    return;
                }
                let nsx = ns.as_deref().filter(|x| !x.is_empty());
                if let Some(a) = doc.find_attr_ns(n, nsx, &local) {
                    doc.unlink(a);
                }
            });
            Ok(Value::Null)
        }
        "setAttributeNode" => {
            let Some((ad, an)) = ids_of(&arg(args, 3)) else {
                return Ok(Value::Bool(false));
            };
            if ad != d {
                return dom_throw(host, WRONG_DOCUMENT_ERR, strict(d));
            }
            let (owned_elsewhere, old) = tree::with_doc(d, |doc| {
                let p = doc.nodes[an].parent;
                let q = doc.nodes[an].qname();
                let ns = doc.nodes[an].ns.clone();
                let old = doc.nodes[n]
                    .attrs
                    .iter()
                    .copied()
                    .find(|&a| a != an && doc.nodes[a].qname() == q && doc.nodes[a].ns == ns);
                (p.is_some() && p != Some(n), old)
            })
            .unwrap_or((false, None));
            if owned_elsewhere {
                return dom_throw(host, INUSE_ATTRIBUTE_ERR, strict(d));
            }
            tree::with_doc(d, |doc| {
                if let Some(o) = old {
                    doc.unlink(o);
                }
                if doc.nodes[an].parent != Some(n) {
                    doc.insert(n, an, None);
                }
            });
            Ok(match old {
                Some(o) => wrap(host, d, o),
                None => Value::Null,
            })
        }
        "removeAttributeNode" => {
            let Some((ad, an)) = ids_of(&arg(args, 3)) else {
                return Ok(Value::Bool(false));
            };
            let ok = ad == d
                && tree::with_doc(d, |doc| doc.nodes[an].parent == Some(n)).unwrap_or(false);
            if !ok {
                return dom_throw(host, NOT_FOUND_ERR, strict(d));
            }
            tree::with_doc(d, |doc| doc.unlink(an));
            Ok(wrap(host, d, an))
        }
        "attrNames" => {
            let names = tree::with_doc(d, |doc| {
                let mut v: Vec<Vec<u8>> = doc.nodes[n]
                    .ns_defs
                    .iter()
                    .map(|(p, _)| {
                        if p.is_empty() {
                            b"xmlns".to_vec()
                        } else {
                            [b"xmlns:".as_slice(), p].concat()
                        }
                    })
                    .collect();
                v.extend(doc.nodes[n].attrs.iter().map(|&a| doc.nodes[a].qname()));
                v
            })
            .unwrap_or_default();
            let mut a = PhpArray::new();
            for x in names {
                a.push(s(x));
            }
            Ok(Value::Array(a))
        }
        "byTag" => {
            let name = bytes(args, 3);
            Ok(node_list(host, d, n, 1, s(name), Value::Null))
        }
        "byTagNS" => {
            let ns = arg(args, 3);
            let local = bytes(args, 4);
            Ok(node_list(host, d, n, 2, ns, s(local)))
        }
        "byId" => {
            let id = bytes(args, 3);
            let r = tree::with_doc(d, |doc| {
                let mut all = Vec::new();
                doc.descendants(0, &mut all);
                all.into_iter().find(|&x| {
                    doc.nodes[x].kind == Kind::Element
                        && doc.nodes[x].attrs.iter().any(|&a| {
                            let an = &doc.nodes[a];
                            an.name == b"id" && an.prefix == b"xml" && an.content == id
                        })
                })
            })
            .flatten();
            Ok(wrap_opt(host, d, r))
        }
        // ---- node lists
        "listLen" | "listItem" => list_op(host, &op, args),
        "mapLen" | "mapItem" | "mapNamed" | "mapNamedNS" => map_op(host, &op, args),
        // ---- character data
        "substringData" | "appendData" | "insertData" | "deleteData" | "replaceData"
        | "splitText" => chardata_op(host, &op, d, n, args),
        "remove" => {
            tree::with_doc(d, |doc| doc.unlink(n));
            Ok(Value::Null)
        }
        "importNode" => {
            let Some((sd, sn)) = ids_of(&arg(args, 3)) else {
                return Ok(Value::Bool(false));
            };
            let deep = conv::to_bool(&arg(args, 4));
            let kind = tree::with_doc(sd, |doc| doc.nodes[sn].kind);
            if matches!(kind, Some(Kind::Document) | Some(Kind::DocType)) {
                return dom_throw(host, NOT_SUPPORTED_ERR, strict(d));
            }
            if sd == d {
                let id = tree::with_doc(d, |doc| doc.clone_node(sn, deep)).unwrap_or(0);
                return Ok(wrap(host, d, id));
            }
            let id = tree::with_two(d, sd, |dst, src| {
                let id = dst.import(src, sn, deep);
                reconcile_imported_ns(dst, src, sn, id);
                id
            })
            .unwrap_or(0);
            Ok(wrap(host, d, id))
        }
        "adoptNode" => {
            let Some((sd, sn)) = ids_of(&arg(args, 3)) else {
                return Ok(Value::Bool(false));
            };
            if sd == d {
                tree::with_doc(d, |doc| doc.unlink(sn));
                return Ok(arg(args, 3));
            }
            let new = move_node(sd, sn, d);
            Ok(wrap(host, d, new))
        }
        "schemaValidate" => schema_validate(host, d, args),
        "xinclude" => Ok(Value::Bool(false)),
        "xpath" => xpath_eval(host, args),
        "appendXML" => {
            let data = bytes(args, 3);
            let opts = ParseOpts::from_flags(0, true, false);
            let (frag, errs) = parser::parse_fragment(&data, &opts);
            lx::report(host, "DOMDocumentFragment::appendXML", errs, Ctx::Parser)?;
            let Some(frag) = frag else {
                return Ok(Value::Bool(false));
            };
            tree::with_doc(d, |doc| {
                let root = frag.root().unwrap_or(0);
                let mut c = frag.nodes[root].first;
                while let Some(x) = c {
                    let id = doc.import(&frag, x, true);
                    doc.append(n, id);
                    c = frag.nodes[x].next;
                }
            });
            Ok(Value::Bool(true))
        }
        "dumpNode" => {
            let fmt = tree::with_doc(d, |doc| doc.format_output).unwrap_or(false);
            Ok(s(tree::with_doc(d, |doc| {
                tree::dump_node(
                    doc,
                    n,
                    &SaveOpts {
                        format: fmt,
                        no_empty_tag: false,
                    },
                )
            })
            .unwrap_or_default()))
        }
        _ => Err(Thrown::Error {
            class: "Error",
            message: format!("__dom: unknown op {op}"),
        }),
    }
}

/// Marker returned by `get` for an unknown property name.
fn unknown() -> Value {
    let mut a = PhpArray::new();
    a.insert(ArrayKey::from_bytes(b"__unknown"), Value::Bool(true));
    Value::Array(a)
}

fn is_unknown(v: &Value) -> bool {
    matches!(v, Value::Array(a) if a.get(&ArrayKey::from_bytes(b"__unknown")).is_some())
}

fn opt_str(v: Option<Vec<u8>>) -> Value {
    v.map(s).unwrap_or(Value::Null)
}

enum Pv {
    V(Value),
    N(Option<NodeId>),
    List,
    Map(i64),
    Impl,
}

fn get_prop(host: &mut dyn Host, d: DocId, n: NodeId, name: &[u8]) -> Result<Value, Thrown> {
    let Some(pv) = tree::with_doc(d, |doc| prop_value(doc, n, name)) else {
        return Ok(Value::Null);
    };
    Ok(match pv {
        Pv::V(v) => v,
        Pv::N(x) => wrap_opt(host, d, x),
        Pv::List => node_list(host, d, n, 0, Value::Null, Value::Null),
        Pv::Map(m) => host
            .instantiate(
                b"DOMNamedNodeMap",
                vec![
                    (b"__d".to_vec(), Value::Int(d as i64)),
                    (b"__n".to_vec(), Value::Int(m)),
                ],
            )
            .unwrap_or(Value::Null),
        Pv::Impl => host
            .instantiate(b"DOMImplementation", vec![])
            .unwrap_or(Value::Null),
    })
}

fn prop_value(doc: &Doc, n: NodeId, name: &[u8]) -> Pv {
    use Pv::{N, V};
    let node = &doc.nodes[n];
    let k = node.kind;
    let is_doc = k == Kind::Document;
    let is_el = k == Kind::Element;
    let is_cd = matches!(k, Kind::Text | Kind::CData | Kind::Comment);
    let is_parent = is_doc || is_el || k == Kind::Fragment;
    let dt = || node.doctype.as_deref().cloned().unwrap_or_default();
    match name {
        b"nodeName" => V(s(match k {
            Kind::Element | Kind::Attr => node.qname(),
            Kind::Text => b"#text".to_vec(),
            Kind::CData => b"#cdata-section".to_vec(),
            Kind::Comment => b"#comment".to_vec(),
            Kind::Document => b"#document".to_vec(),
            Kind::Fragment => b"#document-fragment".to_vec(),
            Kind::Pi | Kind::DocType | Kind::EntityRef => node.name.clone(),
        })),
        b"nodeValue" => V(match k {
            Kind::Element | Kind::Attr | Kind::Text | Kind::CData | Kind::Comment | Kind::Pi => {
                s(doc.text_content(n))
            }
            _ => Value::Null,
        }),
        b"nodeType" => V(Value::Int(k as i64)),
        b"parentNode" => N(if k == Kind::Attr { None } else { node.parent }),
        b"parentElement" => N(node
            .parent
            .filter(|&p| doc.nodes[p].kind == Kind::Element && k != Kind::Attr)),
        b"childNodes" => Pv::List,
        b"firstChild" => N(node.first),
        b"lastChild" => N(node.last),
        b"previousSibling" => N(node.prev),
        b"nextSibling" => N(node.next),
        b"attributes" => {
            if is_el {
                Pv::Map(n as i64)
            } else {
                V(Value::Null)
            }
        }
        b"isConnected" => V(Value::Bool(doc.connected(n))),
        b"ownerDocument" => N(if is_doc || doc.hidden { None } else { Some(0) }),
        b"namespaceURI" => V(match k {
            Kind::Element | Kind::Attr => opt_str(node.ns.clone()),
            _ => Value::Null,
        }),
        b"prefix" => V(match k {
            Kind::Element | Kind::Attr => s(node.prefix.clone()),
            _ => s(Vec::new()),
        }),
        b"localName" => V(match k {
            Kind::Element | Kind::Attr => s(node.name.clone()),
            _ => Value::Null,
        }),
        b"baseURI" => V(opt_str(doc.url.clone())),
        b"textContent" => V(s(doc.text_content(n))),
        // DOMDocument
        b"doctype" if is_doc => N(doc.doctype()),
        b"implementation" if is_doc => Pv::Impl,
        b"documentElement" if is_doc => N(doc.root()),
        b"actualEncoding" | b"encoding" | b"xmlEncoding" if is_doc => {
            V(opt_str(doc.encoding.clone()))
        }
        b"standalone" | b"xmlStandalone" if is_doc => V(Value::Bool(doc.standalone == 1)),
        b"version" | b"xmlVersion" if is_doc => V(opt_str(doc.version.clone())),
        b"strictErrorChecking" if is_doc => V(Value::Bool(doc.strict_error_checking)),
        b"documentURI" if is_doc => V(opt_str(doc.url.clone())),
        b"config" if is_doc => V(Value::Null),
        b"formatOutput" if is_doc => V(Value::Bool(doc.format_output)),
        b"validateOnParse" if is_doc => V(Value::Bool(doc.validate_on_parse)),
        b"resolveExternals" if is_doc => V(Value::Bool(doc.resolve_externals)),
        b"preserveWhiteSpace" if is_doc => V(Value::Bool(doc.preserve_ws)),
        b"recover" if is_doc => V(Value::Bool(doc.recover)),
        b"substituteEntities" if is_doc => V(Value::Bool(doc.substitute_entities)),
        b"firstElementChild" if is_parent => N(element_children(doc, n).first().copied()),
        b"lastElementChild" if is_parent => N(element_children(doc, n).last().copied()),
        b"childElementCount" if is_parent => V(Value::Int(element_children(doc, n).len() as i64)),
        // DOMElement
        b"tagName" if is_el => V(s(node.qname())),
        b"className" if is_el => V(s(get_attribute(doc, n, b"class").unwrap_or_default())),
        b"id" if is_el => V(s(get_attribute(doc, n, b"id").unwrap_or_default())),
        b"schemaTypeInfo" if is_el || k == Kind::Attr => V(Value::Null),
        b"previousElementSibling" if is_el || is_cd || k == Kind::Pi => {
            N(sibling_element(doc, n, false))
        }
        b"nextElementSibling" if is_el || is_cd || k == Kind::Pi => {
            N(sibling_element(doc, n, true))
        }
        // DOMAttr
        b"name" if k == Kind::Attr => V(s(node.qname())),
        b"specified" if k == Kind::Attr => V(Value::Bool(true)),
        b"value" if k == Kind::Attr => V(s(node.content.clone())),
        b"ownerElement" if k == Kind::Attr => N(node.parent),
        // DOMCharacterData / PI
        b"data" if is_cd || k == Kind::Pi => V(s(node.content.clone())),
        b"length" if is_cd => V(Value::Int(
            String::from_utf8_lossy(&node.content).chars().count() as i64,
        )),
        b"wholeText" if matches!(k, Kind::Text | Kind::CData) => {
            let mut first = n;
            while let Some(p) = doc.nodes[first].prev {
                if matches!(doc.nodes[p].kind, Kind::Text | Kind::CData) {
                    first = p;
                } else {
                    break;
                }
            }
            let mut out = Vec::new();
            let mut c = Some(first);
            while let Some(x) = c {
                if !matches!(doc.nodes[x].kind, Kind::Text | Kind::CData) {
                    break;
                }
                out.extend_from_slice(&doc.nodes[x].content);
                c = doc.nodes[x].next;
            }
            V(s(out))
        }
        b"target" if k == Kind::Pi => V(s(node.name.clone())),
        // DOMDocumentType
        b"name" if k == Kind::DocType => V(s(node.name.clone())),
        b"publicId" if k == Kind::DocType => V(s(dt().public_id.unwrap_or_default())),
        b"systemId" if k == Kind::DocType => V(s(dt().system_id.unwrap_or_default())),
        b"internalSubset" if k == Kind::DocType => V(match dt().subset {
            Some(decls) => s(decls
                .iter()
                .flat_map(|x| x.iter().copied().chain(*b"\n"))
                .collect::<Vec<u8>>()),
            None => Value::Null,
        }),
        b"entities" | b"notations" if k == Kind::DocType => Pv::Map(-1),
        _ => V(unknown()),
    }
}

/// Set a property; returns true when handled, false for an unknown name,
/// "ro" for a read-only one.
fn set_prop(
    host: &mut dyn Host,
    d: DocId,
    n: NodeId,
    name: &[u8],
    v: Value,
) -> Result<Value, Thrown> {
    let kind = tree::with_doc(d, |doc| doc.nodes[n].kind).unwrap_or(Kind::Element);
    let is_doc = kind == Kind::Document;
    let sv = || conv::to_string_bytes(&v);
    let bv = conv::to_bool(&v);
    let ro = || Ok(s(b"ro".to_vec()));
    match name {
        b"nodeValue" => {
            let val = sv();
            match kind {
                Kind::Element | Kind::Attr => {
                    let r = tree::with_doc(d, |doc| {
                        if kind == Kind::Attr {
                            match decode_entities(doc, &val) {
                                Ok(t) => {
                                    doc.nodes[n].content = t;
                                    Ok(())
                                }
                                Err(e) => Err(e),
                            }
                        } else {
                            doc.remove_children(n);
                            match string_node_list(doc, &val) {
                                Ok(ids) => {
                                    for id in ids {
                                        doc.append(n, id);
                                    }
                                    Ok(())
                                }
                                Err(e) => Err(e),
                            }
                        }
                    });
                    if let Some(Err(e)) = r {
                        let f = lx::active_user_function(host);
                        lx::report_tree_error(host, &f, &e)?;
                    }
                }
                Kind::Text | Kind::CData | Kind::Comment | Kind::Pi => {
                    tree::with_doc(d, |doc| doc.nodes[n].content = val);
                }
                _ => {}
            }
            Ok(Value::Bool(true))
        }
        b"textContent" => {
            let val = sv();
            tree::with_doc(d, |doc| match kind {
                Kind::Element | Kind::Fragment => {
                    doc.remove_children(n);
                    let t = doc.new_text(Kind::Text, &val);
                    doc.append(n, t);
                }
                Kind::Attr | Kind::Text | Kind::CData | Kind::Comment | Kind::Pi => {
                    doc.nodes[n].content = val
                }
                _ => {}
            });
            Ok(Value::Bool(true))
        }
        b"data" if matches!(kind, Kind::Text | Kind::CData | Kind::Comment | Kind::Pi) => {
            let val = sv();
            tree::with_doc(d, |doc| doc.nodes[n].content = val);
            Ok(Value::Bool(true))
        }
        b"value" if kind == Kind::Attr => {
            let val = sv();
            tree::with_doc(d, |doc| doc.nodes[n].content = val);
            Ok(Value::Bool(true))
        }
        b"prefix" => {
            let p = sv();
            tree::with_doc(d, |doc| {
                if matches!(kind, Kind::Element | Kind::Attr) && doc.nodes[n].ns.is_some() {
                    doc.nodes[n].prefix = p;
                }
            });
            Ok(Value::Bool(true))
        }
        b"id" | b"className" if kind == Kind::Element => {
            let attr = if name == b"id" {
                b"id".to_vec()
            } else {
                b"class".to_vec()
            };
            set_attribute(host, d, n, &attr, &sv())?;
            Ok(Value::Bool(true))
        }
        b"formatOutput" if is_doc => {
            tree::with_doc(d, |doc| doc.format_output = bv);
            Ok(Value::Bool(true))
        }
        b"preserveWhiteSpace" if is_doc => {
            tree::with_doc(d, |doc| doc.preserve_ws = bv);
            Ok(Value::Bool(true))
        }
        b"validateOnParse" if is_doc => {
            tree::with_doc(d, |doc| doc.validate_on_parse = bv);
            Ok(Value::Bool(true))
        }
        b"resolveExternals" if is_doc => {
            tree::with_doc(d, |doc| doc.resolve_externals = bv);
            Ok(Value::Bool(true))
        }
        b"substituteEntities" if is_doc => {
            tree::with_doc(d, |doc| doc.substitute_entities = bv);
            Ok(Value::Bool(true))
        }
        b"strictErrorChecking" if is_doc => {
            tree::with_doc(d, |doc| doc.strict_error_checking = bv);
            Ok(Value::Bool(true))
        }
        b"recover" if is_doc => {
            tree::with_doc(d, |doc| doc.recover = bv);
            Ok(Value::Bool(true))
        }
        b"encoding" | b"xmlEncoding" if is_doc => {
            let e = match v {
                Value::Null => None,
                _ => Some(sv()),
            };
            tree::with_doc(d, |doc| doc.encoding = e);
            Ok(Value::Bool(true))
        }
        b"version" | b"xmlVersion" if is_doc => {
            let e = sv();
            tree::with_doc(d, |doc| doc.version = Some(e));
            Ok(Value::Bool(true))
        }
        b"standalone" | b"xmlStandalone" if is_doc => {
            tree::with_doc(d, |doc| doc.standalone = if bv { 1 } else { 0 });
            Ok(Value::Bool(true))
        }
        b"documentURI" if is_doc => {
            let e = sv();
            tree::with_doc(d, |doc| doc.url = Some(e));
            Ok(Value::Bool(true))
        }
        b"nodeName" | b"nodeType" | b"parentNode" | b"parentElement" | b"childNodes"
        | b"firstChild" | b"lastChild" | b"previousSibling" | b"nextSibling" | b"attributes"
        | b"isConnected" | b"ownerDocument" | b"namespaceURI" | b"localName" | b"baseURI" => ro(),
        _ => {
            let known = get_prop(host, d, n, name)?;
            if is_unknown(&known) {
                Ok(Value::Bool(false))
            } else {
                ro()
            }
        }
    }
}

fn normalize(doc: &mut Doc, n: NodeId) {
    let mut c = doc.nodes[n].first;
    while let Some(x) = c {
        let next = doc.nodes[x].next;
        match doc.nodes[x].kind {
            Kind::Text => {
                if doc.nodes[x].content.is_empty() {
                    doc.unlink(x);
                } else {
                    while let Some(nx) = doc.nodes[x].next {
                        if doc.nodes[nx].kind != Kind::Text {
                            break;
                        }
                        let t = doc.nodes[nx].content.clone();
                        doc.nodes[x].content.extend_from_slice(&t);
                        doc.unlink(nx);
                    }
                }
                c = doc.nodes[x]
                    .next
                    .or(next.filter(|&y| doc.nodes[y].parent == Some(n)));
                if doc.nodes[x].parent.is_none() {
                    c = next;
                }
                continue;
            }
            Kind::Element => normalize(doc, x),
            _ => {}
        }
        c = next;
    }
}

fn nodes_equal(a: &Doc, x: NodeId, b: &Doc, y: NodeId) -> bool {
    let (p, q) = (&a.nodes[x], &b.nodes[y]);
    if p.kind != q.kind
        || p.name != q.name
        || p.prefix != q.prefix
        || p.ns != q.ns
        || p.content != q.content
    {
        return false;
    }
    if p.attrs.len() != q.attrs.len() {
        return false;
    }
    for &aa in &p.attrs {
        let an = &a.nodes[aa];
        if !q.attrs.iter().any(|&bb| {
            let bn = &b.nodes[bb];
            bn.name == an.name && bn.ns == an.ns && bn.content == an.content
        }) {
            return false;
        }
    }
    let (ca, cb) = (a.children(x), b.children(y));
    ca.len() == cb.len()
        && ca
            .iter()
            .zip(cb.iter())
            .all(|(&i, &j)| nodes_equal(a, i, b, j))
}

/// Minimal canonical XML (C14N 1.0 without comments) of a subtree.
fn c14n(doc: &Doc, n: NodeId) -> Vec<u8> {
    fn rec(doc: &Doc, n: NodeId, out: &mut Vec<u8>) {
        let node = &doc.nodes[n];
        match node.kind {
            Kind::Document => {
                let mut c = node.first;
                while let Some(x) = c {
                    rec(doc, x, out);
                    c = doc.nodes[x].next;
                }
            }
            Kind::Element => {
                out.push(b'<');
                out.extend(node.qname());
                let mut nsd = node.ns_defs.clone();
                nsd.sort();
                for (p, u) in nsd {
                    out.extend_from_slice(b" xmlns");
                    if !p.is_empty() {
                        out.push(b':');
                        out.extend(p);
                    }
                    out.extend_from_slice(b"=\"");
                    out.extend(u);
                    out.push(b'"');
                }
                let mut attrs: Vec<&Node> = node.attrs.iter().map(|&a| &doc.nodes[a]).collect();
                attrs.sort_by(|x, y| {
                    (x.ns.clone(), x.name.clone()).cmp(&(y.ns.clone(), y.name.clone()))
                });
                for a in attrs {
                    out.push(b' ');
                    out.extend(a.qname());
                    out.extend_from_slice(b"=\"");
                    for &c in &a.content {
                        match c {
                            b'&' => out.extend_from_slice(b"&amp;"),
                            b'<' => out.extend_from_slice(b"&lt;"),
                            b'"' => out.extend_from_slice(b"&quot;"),
                            b'\t' => out.extend_from_slice(b"&#x9;"),
                            b'\n' => out.extend_from_slice(b"&#xA;"),
                            b'\r' => out.extend_from_slice(b"&#xD;"),
                            _ => out.push(c),
                        }
                    }
                    out.push(b'"');
                }
                out.push(b'>');
                let mut c = node.first;
                while let Some(x) = c {
                    rec(doc, x, out);
                    c = doc.nodes[x].next;
                }
                out.extend_from_slice(b"</");
                out.extend(node.qname());
                out.push(b'>');
            }
            Kind::Text | Kind::CData => {
                for &c in &node.content {
                    match c {
                        b'&' => out.extend_from_slice(b"&amp;"),
                        b'<' => out.extend_from_slice(b"&lt;"),
                        b'>' => out.extend_from_slice(b"&gt;"),
                        b'\r' => out.extend_from_slice(b"&#xD;"),
                        _ => out.push(c),
                    }
                }
            }
            Kind::Pi => {
                out.extend_from_slice(b"<?");
                out.extend(node.name.clone());
                if !node.content.is_empty() {
                    out.push(b' ');
                    out.extend(node.content.clone());
                }
                out.extend_from_slice(b"?>");
            }
            Kind::EntityRef => out.extend(doc.text_content(n)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    rec(doc, n, &mut out);
    out
}

/// dom_get_attribute_or_nsdecl + value.
fn get_attribute(doc: &Doc, el: NodeId, name: &[u8]) -> Option<Vec<u8>> {
    if doc.nodes[el].kind != Kind::Element {
        return None;
    }
    if name == b"xmlns" {
        return doc.nodes[el]
            .ns_defs
            .iter()
            .find(|(p, _)| p.is_empty())
            .map(|(_, u)| u.clone());
    }
    if let Some(p) = name.strip_prefix(b"xmlns:") {
        return doc.nodes[el]
            .ns_defs
            .iter()
            .find(|(x, _)| x == p)
            .map(|(_, u)| u.clone());
    }
    find_attr_qname(doc, el, name).map(|a| doc.nodes[a].content.clone())
}

/// The attribute node for a qualified name, resolving its prefix in scope.
fn find_attr_qname(doc: &Doc, el: NodeId, name: &[u8]) -> Option<NodeId> {
    if doc.nodes[el].kind != Kind::Element {
        return None;
    }
    let (p, l) = split_qname(name);
    if !p.is_empty() {
        if let Some(u) = doc.lookup_ns(el, &p) {
            if let Some(a) = doc.find_attr_ns(el, Some(&u), &l) {
                return Some(a);
            }
        }
        return doc.nodes[el]
            .attrs
            .iter()
            .copied()
            .find(|&a| doc.nodes[a].ns.is_none() && doc.nodes[a].name == name);
    }
    doc.nodes[el]
        .attrs
        .iter()
        .copied()
        .find(|&a| doc.nodes[a].ns.is_none() && doc.nodes[a].name == name)
}

fn remove_attribute(doc: &mut Doc, el: NodeId, name: &[u8]) -> bool {
    if doc.nodes[el].kind != Kind::Element {
        return false;
    }
    if name == b"xmlns" || name.starts_with(b"xmlns:") {
        let p = if name == b"xmlns" {
            Vec::new()
        } else {
            name[6..].to_vec()
        };
        let before = doc.nodes[el].ns_defs.len();
        doc.nodes[el].ns_defs.retain(|(x, _)| *x != p);
        if doc.nodes[el].ns_defs.len() != before {
            return true;
        }
    }
    match find_attr_qname(doc, el, name) {
        Some(a) => {
            doc.unlink(a);
            true
        }
        None => false,
    }
}

fn set_attribute(
    host: &mut dyn Host,
    d: DocId,
    el: NodeId,
    name: &[u8],
    value: &[u8],
) -> Result<Value, Thrown> {
    if name.is_empty() {
        return Err(Thrown::Error {
            class: "ValueError",
            message: "DOMElement::setAttribute(): Argument #1 ($qualifiedName) must not be empty"
                .into(),
        });
    }
    if !valid_name(name) {
        return dom_throw(host, INVALID_CHARACTER_ERR, true);
    }
    // 0: false, 1: true (namespace declaration), 2: attribute node
    let r = tree::with_doc(d, |doc| -> (u8, NodeId) {
        if doc.nodes[el].kind != Kind::Element {
            return (0, 0);
        }
        if name == b"xmlns" {
            if doc.nodes[el].ns_defs.iter().any(|(p, _)| p.is_empty()) {
                return (0, 0);
            }
            doc.nodes[el].ns_defs.push((Vec::new(), value.to_vec()));
            return (1, 0);
        }
        if let Some(p) = name.strip_prefix(b"xmlns:") {
            if doc.nodes[el].ns_defs.iter().any(|(x, _)| x == p) {
                return (0, 0);
            }
        }
        let a = match find_attr_qname(doc, el, name) {
            Some(a) => {
                doc.nodes[a].content = value.to_vec();
                a
            }
            None => {
                let mut an = Node::new(Kind::Attr);
                let (p, l) = split_qname(name);
                match (!p.is_empty()).then(|| doc.lookup_ns(el, &p)).flatten() {
                    Some(u) if p != b"xmlns" => {
                        an.prefix = p;
                        an.name = l;
                        an.ns = Some(u);
                    }
                    _ => an.name = name.to_vec(),
                }
                an.content = value.to_vec();
                let id = doc.add(an);
                doc.insert(el, id, None);
                id
            }
        };
        (2, a)
    })
    .unwrap_or((0, 0));
    match r {
        (0, _) => Ok(Value::Bool(false)),
        (1, _) => Ok(Value::Bool(true)),
        (_, a) => Ok(wrap(host, d, a)),
    }
}

fn set_attribute_ns(
    host: &mut dyn Host,
    d: DocId,
    el: NodeId,
    ns: Option<Vec<u8>>,
    qname: &[u8],
    value: &[u8],
) -> Result<Value, Thrown> {
    if !valid_qname(qname) {
        return dom_throw(host, INVALID_CHARACTER_ERR, strict(d));
    }
    let ns = ns.filter(|x| !x.is_empty());
    let (p, l) = split_qname(qname);
    if (!p.is_empty() && ns.is_none())
        || (p == b"xml" && ns.as_deref() != Some(XML_NS))
        || ((p == b"xmlns" || qname == b"xmlns") && ns.as_deref() != Some(XMLNS_NS))
    {
        return dom_throw(host, NAMESPACE_ERR, strict(d));
    }
    tree::with_doc(d, |doc| {
        if ns.as_deref() == Some(XMLNS_NS) {
            let prefix = if qname == b"xmlns" {
                Vec::new()
            } else {
                l.clone()
            };
            match doc.nodes[el].ns_defs.iter_mut().find(|(x, _)| *x == prefix) {
                Some(x) => x.1 = value.to_vec(),
                None => doc.nodes[el].ns_defs.push((prefix, value.to_vec())),
            }
            return;
        }
        let Some(u) = ns else {
            match doc.find_attr_ns(el, None, &l) {
                Some(a) => doc.nodes[a].content = value.to_vec(),
                None => {
                    let mut an = Node::new(Kind::Attr);
                    an.name = l.clone();
                    an.content = value.to_vec();
                    let id = doc.add(an);
                    doc.insert(el, id, None);
                }
            }
            return;
        };
        // find a prefix bound to the namespace (or declare the requested one)
        let prefix = if !p.is_empty() && doc.lookup_ns(el, &p).as_deref() == Some(&u[..]) {
            p.clone()
        } else if let Some(bound) = doc.lookup_prefix(el, &u).filter(|x| !x.is_empty()) {
            bound
        } else {
            let mut pre = if p.is_empty() {
                b"default".to_vec()
            } else {
                p.clone()
            };
            if doc.lookup_ns(el, &pre).is_some()
                && doc.lookup_ns(el, &pre).as_deref() != Some(&u[..])
            {
                let mut k = 1;
                loop {
                    let cand = format!("{}{k}", String::from_utf8_lossy(&pre)).into_bytes();
                    if doc.lookup_ns(el, &cand).is_none() {
                        pre = cand;
                        break;
                    }
                    k += 1;
                }
            }
            doc.nodes[el].ns_defs.push((pre.clone(), u.clone()));
            pre
        };
        match doc.find_attr_ns(el, Some(&u), &l) {
            Some(a) => {
                doc.nodes[a].content = value.to_vec();
                doc.nodes[a].prefix = prefix;
            }
            None => {
                let mut an = Node::new(Kind::Attr);
                an.name = l.clone();
                an.prefix = prefix;
                an.ns = Some(u);
                an.content = value.to_vec();
                let id = doc.add(an);
                doc.insert(el, id, None);
            }
        }
    });
    Ok(Value::Null)
}

/// Move a node (and subtree) from document `from` into `to`, re-pointing
/// any live wrapper objects; returns the new id.
fn move_node(from: DocId, n: NodeId, to: DocId) -> NodeId {
    let mut map = Vec::new();
    let new = tree::with_two(to, from, |dst, src| {
        fn rec(dst: &mut Doc, src: &Doc, n: NodeId, map: &mut Vec<(NodeId, NodeId)>) -> NodeId {
            let id = dst.import(src, n, false);
            map.push((n, id));
            // attributes were copied by import: map them too
            let (sa, da) = (src.nodes[n].attrs.clone(), dst.nodes[id].attrs.clone());
            for (a, b) in sa.iter().zip(da.iter()) {
                map.push((*a, *b));
            }
            let mut c = src.nodes[n].first;
            while let Some(x) = c {
                let nc = rec(dst, src, x, map);
                dst.append(id, nc);
                c = src.nodes[x].next;
            }
            id
        }
        let id = rec(dst, src, n, &mut map);
        reconcile_imported_ns(dst, src, n, id);
        id
    })
    .unwrap_or(0);
    tree::with_doc(from, |doc| doc.unlink(n));
    cache_move(from, to, &map);
    new
}

/// After copying a subtree out of its context, declare the namespaces it
/// used from ancestors outside the copied part (xmlReconciliateNs-ish).
fn reconcile_imported_ns(dst: &mut Doc, src: &Doc, sn: NodeId, dn: NodeId) {
    if dst.nodes[dn].kind != Kind::Element {
        return;
    }
    let mut needed: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut all = vec![dn];
    dst.descendants(dn, &mut all);
    for x in all {
        let node = &dst.nodes[x];
        let mut check = |p: &[u8], u: &Option<Vec<u8>>| {
            if let Some(u) = u {
                if dst.lookup_ns(x, p).as_deref() != Some(&u[..])
                    && p != b"xml"
                    && !needed.iter().any(|(a, _)| a == p)
                {
                    needed.push((p.to_vec(), u.clone()));
                }
            }
        };
        if node.kind == Kind::Element {
            check(&node.prefix.clone(), &node.ns.clone());
            for &a in &node.attrs.clone() {
                let an = &dst.nodes[a];
                if !an.prefix.is_empty() {
                    check(&an.prefix.clone(), &an.ns.clone());
                }
            }
        }
    }
    let _ = (src, sn);
    for (p, u) in needed {
        if !dst.nodes[dn].ns_defs.iter().any(|(x, _)| *x == p) {
            dst.nodes[dn].ns_defs.push((p, u));
        }
    }
}

/// appendChild / insertBefore (legacy DOM semantics).
fn insert(
    host: &mut dyn Host,
    d: DocId,
    parent: NodeId,
    args: &[Value],
    before: Option<Option<(DocId, NodeId)>>,
    is_insert_before: bool,
) -> Result<Value, Thrown> {
    let childv = arg(args, 3);
    let Some((mut cd, mut cn)) = ids_of(&childv) else {
        return Ok(Value::Bool(false));
    };
    let st = strict(d);
    let pkind = tree::with_doc(d, |doc| doc.nodes[parent].kind).unwrap_or(Kind::Element);
    if !matches!(
        pkind,
        Kind::Element | Kind::Document | Kind::Fragment | Kind::Attr
    ) {
        return Ok(Value::Bool(false));
    }
    // hierarchy: the child may not be an ancestor of the parent
    if cd == d && tree::with_doc(d, |doc| doc.is_ancestor(cn, parent)).unwrap_or(false) {
        return dom_throw(host, HIERARCHY_REQUEST_ERR, st);
    }
    let child_hidden = tree::with_doc(cd, |doc| doc.hidden).unwrap_or(false);
    if cd != d && !child_hidden {
        return dom_throw(host, WRONG_DOCUMENT_ERR, st);
    }
    let ckind = tree::with_doc(cd, |doc| doc.nodes[cn].kind).unwrap_or(Kind::Element);
    if ckind == Kind::Fragment
        && tree::with_doc(cd, |doc| doc.nodes[cn].first.is_none()).unwrap_or(true)
    {
        crate::warn(
            host,
            if is_insert_before {
                "DOMNode::insertBefore"
            } else {
                "DOMNode::appendChild"
            },
            "Document Fragment is empty",
        )?;
        return Ok(Value::Bool(false));
    }
    if pkind == Kind::Attr && !matches!(ckind, Kind::Text | Kind::EntityRef) {
        return dom_throw(host, HIERARCHY_REQUEST_ERR, st);
    }
    if ckind == Kind::Attr && pkind != Kind::Element {
        return dom_throw(host, HIERARCHY_REQUEST_ERR, st);
    }
    if ckind == Kind::Document {
        return dom_throw(host, HIERARCHY_REQUEST_ERR, st);
    }
    let mut refn: Option<NodeId> = None;
    if let Some(Some((rd, rn))) = before {
        let ok = rd == d
            && tree::with_doc(d, |doc| doc.nodes[rn].parent == Some(parent)).unwrap_or(false);
        if !ok {
            return dom_throw(host, NOT_FOUND_ERR, st);
        }
        refn = Some(rn);
        if (rd, rn) == (cd, cn) {
            refn = tree::with_doc(d, |doc| doc.nodes[rn].next).flatten();
        }
    }
    if cd != d {
        cn = move_node(cd, cn, d);
        cd = d;
    }
    let _ = cd;
    let result = tree::with_doc(d, |doc| {
        if ckind == Kind::Attr {
            // replace an attribute with the same name
            let q = doc.nodes[cn].qname();
            let ns = doc.nodes[cn].ns.clone();
            if let Some(old) = doc.nodes[parent].attrs.iter().copied().find(|&a| {
                a != cn
                    && doc.nodes[a].name == doc.nodes[cn].name
                    && doc.nodes[a].ns == ns
                    && (ns.is_some() || doc.nodes[a].qname() == q)
            }) {
                doc.unlink(old);
            }
            doc.insert(parent, cn, None);
            return vec![cn];
        }
        if ckind == Kind::Fragment {
            let kids = doc.children(cn);
            for &k in &kids {
                doc.insert(parent, k, refn);
            }
            for &k in &kids {
                reconcile_ns(doc, k);
            }
            return kids;
        }
        doc.insert(parent, cn, refn);
        reconcile_ns(doc, cn);
        vec![cn]
    })
    .unwrap_or_default();
    if ckind == Kind::Fragment {
        return Ok(childv);
    }
    Ok(match result.first() {
        Some(&x) => wrap(host, d, x),
        None => Value::Bool(false),
    })
}

/// dom_reconcile_ns_internal: drop declarations the new parent already
/// provides.
fn reconcile_ns(doc: &mut Doc, n: NodeId) {
    if doc.nodes[n].kind != Kind::Element {
        return;
    }
    let Some(p) = doc.nodes[n].parent else {
        return;
    };
    let defs = doc.nodes[n].ns_defs.clone();
    let mut keep = Vec::new();
    for (pre, uri) in defs {
        let found = doc.lookup_prefix(p, &uri);
        let redundant = match &found {
            Some(fp) => pre.is_empty() || *fp == pre,
            None => false,
        };
        // a default-namespace declaration is only redundant when the parent's
        // default namespace is the same
        let redundant =
            redundant && (!pre.is_empty() || doc.lookup_ns(p, b"").as_deref() == Some(&uri[..]));
        if !redundant {
            keep.push((pre, uri));
        }
    }
    doc.nodes[n].ns_defs = keep;
}

fn create(host: &mut dyn Host, d: DocId, args: &[Value]) -> Result<Value, Thrown> {
    let what = bytes(args, 2);
    let st = strict(d);
    match &what[..] {
        b"element" => {
            let name = bytes(args, 3);
            if !valid_name(&name) {
                return dom_throw(host, INVALID_CHARACTER_ERR, st);
            }
            let value = bytes(args, 4);
            let r = tree::with_doc(d, |doc| {
                let el = doc.new_element(&name);
                if !value.is_empty() {
                    match string_node_list(doc, &value) {
                        Ok(ids) => {
                            for i in ids {
                                doc.append(el, i);
                            }
                        }
                        Err(e) => return (el, Some(e)),
                    }
                }
                (el, None)
            });
            let Some((el, w)) = r else {
                return Ok(Value::Bool(false));
            };
            if let Some(w) = w {
                lx::report_tree_error(host, "DOMDocument::createElement", &w)?;
            }
            Ok(wrap(host, d, el))
        }
        b"elementNS" => {
            let ns = opt_bytes(args, 3).filter(|x| !x.is_empty());
            let q = bytes(args, 4);
            let value = bytes(args, 5);
            if !valid_qname(&q) {
                return dom_throw(host, INVALID_CHARACTER_ERR, st);
            }
            let (p, l) = split_qname(&q);
            if (!p.is_empty() && ns.is_none()) || (p == b"xml" && ns.as_deref() != Some(XML_NS)) {
                return dom_throw(host, NAMESPACE_ERR, st);
            }
            let el = tree::with_doc(d, |doc| {
                let el = doc.new_element(&l);
                if let Some(u) = &ns {
                    doc.nodes[el].prefix = p.clone();
                    doc.nodes[el].ns = Some(u.clone());
                    if p != b"xml" {
                        doc.nodes[el].ns_defs.push((p.clone(), u.clone()));
                    }
                } else {
                    doc.nodes[el].name = q.clone();
                }
                let mut warn = None;
                if !value.is_empty() {
                    match string_node_list(doc, &value) {
                        Ok(ids) => {
                            for i in ids {
                                doc.append(el, i);
                            }
                        }
                        Err(e) => warn = Some(e),
                    }
                }
                (el, warn)
            });
            let Some((el, w)) = el else {
                return Ok(Value::Bool(false));
            };
            if let Some(w) = w {
                lx::report_tree_error(host, "DOMDocument::createElementNS", &w)?;
            }
            Ok(wrap(host, d, el))
        }
        b"text" | b"comment" | b"cdata" => {
            let kind = match &what[..] {
                b"text" => Kind::Text,
                b"comment" => Kind::Comment,
                _ => Kind::CData,
            };
            let v = bytes(args, 3);
            let id = tree::with_doc(d, |doc| doc.new_text(kind, &v)).unwrap_or(0);
            Ok(wrap(host, d, id))
        }
        b"pi" => {
            let target = bytes(args, 3);
            if !valid_name(&target) {
                return dom_throw(host, INVALID_CHARACTER_ERR, st);
            }
            let v = bytes(args, 4);
            let id = tree::with_doc(d, |doc| {
                let mut n = Node::new(Kind::Pi);
                n.name = target;
                n.content = v;
                doc.add(n)
            })
            .unwrap_or(0);
            Ok(wrap(host, d, id))
        }
        b"attr" => {
            let name = bytes(args, 3);
            if !valid_name(&name) {
                return dom_throw(host, INVALID_CHARACTER_ERR, st);
            }
            let id = tree::with_doc(d, |doc| {
                let mut n = Node::new(Kind::Attr);
                n.name = name;
                doc.add(n)
            })
            .unwrap_or(0);
            Ok(wrap(host, d, id))
        }
        b"attrNS" => {
            let ns = opt_bytes(args, 3).filter(|x| !x.is_empty());
            let q = bytes(args, 4);
            if !valid_qname(&q) {
                return dom_throw(host, INVALID_CHARACTER_ERR, st);
            }
            let (p, l) = split_qname(&q);
            if (!p.is_empty() && ns.is_none()) || (p == b"xml" && ns.as_deref() != Some(XML_NS)) {
                return dom_throw(host, NAMESPACE_ERR, st);
            }
            let id = tree::with_doc(d, |doc| {
                let mut n = Node::new(Kind::Attr);
                n.name = l;
                n.prefix = p;
                n.ns = ns;
                doc.add(n)
            })
            .unwrap_or(0);
            Ok(wrap(host, d, id))
        }
        b"fragment" => {
            let id = tree::with_doc(d, |doc| doc.add(Node::new(Kind::Fragment))).unwrap_or(0);
            Ok(wrap(host, d, id))
        }
        b"entityref" => {
            let name = bytes(args, 3);
            if !valid_name(&name) {
                return dom_throw(host, INVALID_CHARACTER_ERR, st);
            }
            let id = tree::with_doc(d, |doc| {
                let mut n = Node::new(Kind::EntityRef);
                n.name = name;
                doc.add(n)
            })
            .unwrap_or(0);
            Ok(wrap(host, d, id))
        }
        _ => Ok(Value::Bool(false)),
    }
}

/// `new DOMElement(...)` & co: a node owned by a hidden document.
fn new_node(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // (op, obj, kind, a, b, c)
    let obj = arg(args, 1);
    let what = bytes(args, 2);
    let mut doc = Doc::new();
    doc.hidden = true;
    let id = match &what[..] {
        b"element" => {
            let q = bytes(args, 3);
            if !valid_qname(&q) {
                return dom_throw(host, INVALID_CHARACTER_ERR, true);
            }
            let ns = bytes(args, 5);
            let el = doc.new_element(&q);
            if !ns.is_empty() {
                let (p, l) = split_qname(&q);
                doc.nodes[el].name = l;
                doc.nodes[el].prefix = p.clone();
                doc.nodes[el].ns = Some(ns.clone());
                doc.nodes[el].ns_defs.push((p, ns));
            }
            if let Some(v) = opt_bytes(args, 4) {
                if !v.is_empty() {
                    let t = doc.new_text(Kind::Text, &v);
                    doc.append(el, t);
                }
            }
            el
        }
        b"text" | b"comment" | b"cdata" => {
            let kind = match &what[..] {
                b"text" => Kind::Text,
                b"comment" => Kind::Comment,
                _ => Kind::CData,
            };
            doc.new_text(kind, &bytes(args, 3))
        }
        b"attr" => {
            let name = bytes(args, 3);
            if !valid_name(&name) {
                return dom_throw(host, INVALID_CHARACTER_ERR, true);
            }
            let mut n = Node::new(Kind::Attr);
            n.name = name;
            n.content = bytes(args, 4);
            doc.add(n)
        }
        b"pi" => {
            let mut n = Node::new(Kind::Pi);
            n.name = bytes(args, 3);
            n.content = bytes(args, 4);
            doc.add(n)
        }
        b"fragment" => doc.add(Node::new(Kind::Fragment)),
        b"entityref" => {
            let mut n = Node::new(Kind::EntityRef);
            n.name = bytes(args, 3);
            doc.add(n)
        }
        _ => 0,
    };
    let d = tree::register(doc);
    bind(&obj, d, id);
    Ok(Value::Null)
}

fn load(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // (op, obj, d, source, options, is_file)
    let obj = arg(args, 1);
    let d = int(args, 2) as usize;
    let src = bytes(args, 3);
    let flags = int(args, 4);
    let is_file = conv::to_bool(&arg(args, 5));
    let fname = if is_file {
        "DOMDocument::load"
    } else {
        "DOMDocument::loadXML"
    };
    if src.is_empty() {
        return Err(Thrown::Error {
            class: "ValueError",
            message: format!(
                "{fname}(): Argument #1 (${}) must not be empty",
                if is_file { "filename" } else { "source" }
            ),
        });
    }
    let (data, url) = if is_file {
        if src.contains(&0) {
            return Err(Thrown::Error {
                class: "ValueError",
                message: format!(
                    "{fname}(): Argument #1 ($filename) must not contain any null bytes"
                ),
            });
        }
        match read_source(host, &src) {
            Some((data, url)) => (data, Some(url)),
            None => {
                lx::report_io(host, fname, &expand_filepath(&src))?;
                return Ok(Value::Bool(false));
            }
        }
    } else {
        (src, None)
    };
    let (keep, subst) = tree::with_doc(d, |doc| (doc.preserve_ws, doc.substitute_entities))
        .unwrap_or((true, false));
    let opts = ParseOpts::from_flags(flags, keep, subst);
    let (doc, errs) = parser::parse(&data, &opts, url.as_deref());
    let errs = lx::filter_by_flags(errs, flags);
    lx::report(host, fname, errs, Ctx::Parser)?;
    let Some(mut newdoc) = doc else {
        return Ok(Value::Bool(false));
    };
    if let Some(old) = tree::with_doc(d, |o| o.clone()) {
        newdoc.format_output = old.format_output;
        newdoc.preserve_ws = old.preserve_ws;
        newdoc.validate_on_parse = old.validate_on_parse;
        newdoc.resolve_externals = old.resolve_externals;
        newdoc.substitute_entities = old.substitute_entities;
        newdoc.strict_error_checking = old.strict_error_checking;
        newdoc.recover = old.recover;
    }
    if !is_file && newdoc.url.is_none() {
        // php sets a memory-loaded document's URL to the working directory
        if let Ok(cwd) = std::env::current_dir() {
            let mut u = cwd.to_string_lossy().into_owned();
            if !u.ends_with('/') {
                u.push('/');
            }
            newdoc.url = Some(u.into_bytes());
        }
    }
    let id = tree::register(newdoc);
    bind(&obj, id, 0);
    Ok(Value::Int(id as i64))
}

/// Read a document source (path or URL) for `load()` / `schemaValidate()`:
/// (bytes, the URL libxml2 records).
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

/// php's `_dom_get_valid_file_path` / `expand_filepath`: the absolute path
/// (symlinks in the existing leading part resolved) libxml2 is handed.
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

fn save_xml(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // (op, d, nodeobj|null, unused, options)
    let d = int(args, 1) as usize;
    let nodev = arg(args, 2);
    let flags = int(args, 4);
    let fmt = tree::with_doc(d, |doc| doc.format_output).unwrap_or(false);
    let opts = SaveOpts {
        format: fmt,
        no_empty_tag: flags & 4 != 0,
    };
    match nodev {
        Value::Null => {
            tree::take_save_diags();
            let out = tree::with_doc(d, |doc| tree::dump_doc(doc, &opts)).unwrap_or_default();
            lx::report_save_diags(host, "DOMDocument::saveXML")?;
            Ok(s(out))
        }
        v => {
            let Some((nd, nn)) = ids_of(&v) else {
                return Ok(Value::Bool(false));
            };
            if nd != d {
                return dom_throw(host, WRONG_DOCUMENT_ERR, strict(d));
            }
            Ok(s(
                tree::with_doc(d, |doc| tree::dump_node(doc, nn, &opts)).unwrap_or_default()
            ))
        }
    }
}

fn list_op(host: &mut dyn Host, op: &str, args: &[Value]) -> Result<Value, Thrown> {
    // (op, d, n, mode, a, b, index)
    let d = int(args, 1) as usize;
    let n = int(args, 2) as usize;
    let mode = int(args, 3);
    let a = arg(args, 4);
    let b = arg(args, 5);
    let ids: Vec<NodeId> = match mode {
        3 => match &a {
            Value::Array(arr) => arr.iter().map(|(_, v)| conv::to_int(v) as usize).collect(),
            _ => Vec::new(),
        },
        _ => tree::with_doc(d, |doc| match mode {
            0 => doc.children(n),
            1 => by_tag(doc, n, &conv::to_string_bytes(&a)),
            _ => {
                let ns = match &a {
                    Value::Null => None,
                    v => Some(conv::to_string_bytes(v)),
                };
                by_tag_ns(doc, n, ns.as_deref(), &conv::to_string_bytes(&b))
            }
        })
        .unwrap_or_default(),
    };
    if op == "listLen" {
        return Ok(Value::Int(ids.len() as i64));
    }
    let i = int(args, 6);
    if i < 0 || i as usize >= ids.len() {
        return Ok(Value::Null);
    }
    Ok(wrap(host, d, ids[i as usize]))
}

fn map_op(host: &mut dyn Host, op: &str, args: &[Value]) -> Result<Value, Thrown> {
    // (op, d, n, x, y)
    let d = int(args, 1) as usize;
    let n = int(args, 2);
    if n < 0 {
        return Ok(if op == "mapLen" {
            Value::Int(0)
        } else {
            Value::Null
        });
    }
    let n = n as usize;
    let attrs = tree::with_doc(d, |doc| doc.nodes[n].attrs.clone()).unwrap_or_default();
    match op {
        "mapLen" => Ok(Value::Int(attrs.len() as i64)),
        "mapItem" => {
            let i = int(args, 3);
            if i < 0 || i as usize >= attrs.len() {
                return Ok(Value::Null);
            }
            Ok(wrap(host, d, attrs[i as usize]))
        }
        "mapNamed" => {
            let name = bytes(args, 3);
            let a = tree::with_doc(d, |doc| find_attr_qname(doc, n, &name)).flatten();
            Ok(wrap_opt(host, d, a))
        }
        _ => {
            let ns = opt_bytes(args, 3).filter(|x| !x.is_empty());
            let local = bytes(args, 4);
            let a = tree::with_doc(d, |doc| doc.find_attr_ns(n, ns.as_deref(), &local)).flatten();
            Ok(wrap_opt(host, d, a))
        }
    }
}

fn chardata_op(
    host: &mut dyn Host,
    op: &str,
    d: DocId,
    n: NodeId,
    args: &[Value],
) -> Result<Value, Thrown> {
    let content = tree::with_doc(d, |doc| doc.nodes[n].content.clone()).unwrap_or_default();
    let chars: Vec<char> = String::from_utf8_lossy(&content).chars().collect();
    let len = chars.len() as i64;
    let st = strict(d);
    let set = |c: Vec<char>| {
        let v: String = c.into_iter().collect();
        tree::with_doc(d, |doc| doc.nodes[n].content = v.into_bytes());
    };
    match op {
        "appendData" => {
            let mut c = chars;
            c.extend(String::from_utf8_lossy(&bytes(args, 3)).chars());
            set(c);
            Ok(Value::Bool(true))
        }
        "substringData" => {
            let (off, cnt) = (int(args, 3), int(args, 4));
            if off < 0 || cnt < 0 || off > len {
                return dom_throw(host, INDEX_SIZE_ERR, st);
            }
            let end = (off + cnt).min(len);
            Ok(s(chars[off as usize..end as usize]
                .iter()
                .collect::<String>()
                .into_bytes()))
        }
        "insertData" => {
            let off = int(args, 3);
            if off < 0 || off > len {
                return dom_throw(host, INDEX_SIZE_ERR, st);
            }
            let mut c = chars;
            let ins: Vec<char> = String::from_utf8_lossy(&bytes(args, 4)).chars().collect();
            c.splice(off as usize..off as usize, ins);
            set(c);
            Ok(Value::Bool(true))
        }
        "deleteData" | "replaceData" => {
            let (off, cnt) = (int(args, 3), int(args, 4));
            if off < 0 || cnt < 0 || off > len {
                return dom_throw(host, INDEX_SIZE_ERR, st);
            }
            let end = (off + cnt).min(len);
            let mut c = chars;
            let ins: Vec<char> = if op == "replaceData" {
                String::from_utf8_lossy(&bytes(args, 5)).chars().collect()
            } else {
                Vec::new()
            };
            c.splice(off as usize..end as usize, ins);
            set(c);
            Ok(Value::Bool(true))
        }
        _ => {
            // splitText
            let off = int(args, 3);
            if off < 0 || off > len {
                return dom_throw(host, INDEX_SIZE_ERR, st);
            }
            let first: String = chars[..off as usize].iter().collect();
            let second: String = chars[off as usize..].iter().collect();
            let new = tree::with_doc(d, |doc| {
                doc.nodes[n].content = first.into_bytes();
                let kind = doc.nodes[n].kind;
                let id = doc.new_text(kind, second.as_bytes());
                if let Some(p) = doc.nodes[n].parent {
                    let next = doc.nodes[n].next;
                    doc.insert(p, id, next);
                }
                id
            })
            .unwrap_or(0);
            Ok(wrap(host, d, new))
        }
    }
}

fn schema_validate(host: &mut dyn Host, d: DocId, args: &[Value]) -> Result<Value, Thrown> {
    // (op, d, n, source, is_file, flags)
    let src = bytes(args, 3);
    let is_file = conv::to_bool(&arg(args, 4));
    let fname = if is_file {
        "DOMDocument::schemaValidate"
    } else {
        "DOMDocument::schemaValidateSource"
    };
    if src.is_empty() {
        return Err(Thrown::Error {
            class: "ValueError",
            message: format!(
                "{fname}(): Argument #1 (${}) must not be empty",
                if is_file { "filename" } else { "source" }
            ),
        });
    }
    let (data, base) = if is_file {
        match read_source(host, &src) {
            Some((data, url)) => (data, Some(url)),
            None => {
                lx::report_io(host, fname, &src)?;
                let e = lx::plain_error(
                    3,
                    1,
                    16,
                    0,
                    format!(
                        "Failed to locate the main schema resource at '{}'.\n",
                        String::from_utf8_lossy(&src)
                    ),
                );
                lx::report(host, fname, vec![e], Ctx::Plain)?;
                crate::warn(host, fname, "Invalid Schema")?;
                return Ok(Value::Bool(false));
            }
        }
    } else {
        (src, None)
    };
    let Some(doc) = tree::clone_doc(d) else {
        return Ok(Value::Bool(false));
    };
    let (ok, schema_errs, valid_errs) = xsd::validate_source(&data, base.as_deref(), &doc);
    if !schema_errs.is_empty() {
        lx::report(host, fname, schema_errs, Ctx::Plain)?;
    }
    if !ok && valid_errs.is_empty() {
        crate::warn(host, fname, "Invalid Schema")?;
        return Ok(Value::Bool(false));
    }
    let valid = valid_errs.is_empty() && ok;
    lx::report(host, fname, valid_errs, Ctx::Plain)?;
    Ok(Value::Bool(valid))
}

fn xpath_eval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // (op, d, _, expr, ctxnode|null, registerNodeNS, nsmap, is_query)
    let d = int(args, 1) as usize;
    let expr = bytes(args, 3);
    let ctxv = arg(args, 4);
    let reg = conv::to_bool(&arg(args, 5));
    let is_query = conv::to_bool(&arg(args, 7));
    let fname = if is_query {
        "DOMXPath::query"
    } else {
        "DOMXPath::evaluate"
    };
    let mut ns: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    let node = match &ctxv {
        Value::Null => tree::with_doc(d, |doc| doc.root()).flatten(),
        v => match ids_of(v) {
            Some((cd, cn)) => {
                if cd != d {
                    return Err(Thrown::Error {
                        class: "Error",
                        message: "Node from wrong document".into(),
                    });
                }
                Some(cn)
            }
            None => None,
        },
    };
    if reg {
        if let Some(cn) = node {
            tree::with_doc(d, |doc| {
                // in-scope namespaces of the context node (innermost wins)
                let mut chain = Vec::new();
                let mut c = Some(cn);
                while let Some(x) = c {
                    chain.push(x);
                    c = doc.nodes[x].parent;
                }
                for &x in chain.iter().rev() {
                    for (p, u) in &doc.nodes[x].ns_defs {
                        if !p.is_empty() {
                            ns.insert(p.clone(), u.clone());
                        }
                    }
                }
            });
        }
    }
    if let Value::Array(m) = arg(args, 6) {
        for (k, v) in m.iter() {
            let key = match k {
                ArrayKey::Int(i) => i.to_string().into_bytes(),
                ArrayKey::Str(s) => s.as_bytes().to_vec(),
            };
            ns.insert(key, conv::to_string_bytes(v));
        }
    }
    let vars = HashMap::new();
    let result = tree::with_doc(d, |doc| {
        xpath::evaluate(doc, &expr, node.unwrap_or(0), &ns, &vars)
    });
    let Some(result) = result else {
        return Ok(Value::Bool(false));
    };
    match result {
        Err(e) => {
            let msgs: Vec<String> = match e {
                xpath::XErr::Syntax => vec!["Invalid expression\n".into()],
                xpath::XErr::UndefinedPrefix => vec!["Undefined namespace prefix\n".into()],
                xpath::XErr::UnknownFunction(f) => vec![
                    format!(
                        "xmlXPathCompOpEval: function {} not found\n",
                        String::from_utf8_lossy(&f)
                    ),
                    "Unregistered function\n".into(),
                ],
                xpath::XErr::UndefinedVariable => vec!["Undefined variable\n".into()],
                xpath::XErr::Eval => vec!["Invalid type\n".into()],
            };
            let errs: Vec<XmlError> = msgs
                .into_iter()
                .map(|m| lx::plain_error(2, 1207, 12, 0, m))
                .collect();
            lx::report(host, fname, errs, Ctx::Plain)?;
            Ok(Value::Bool(false))
        }
        Ok(xpath::XVal::Nodes(ids)) => Ok(static_list(host, d, &ids)),
        Ok(_) if is_query => Ok(static_list(host, d, &[])),
        Ok(xpath::XVal::Bool(b)) => Ok(Value::Bool(b)),
        Ok(xpath::XVal::Num(n)) => Ok(Value::Float(n)),
        Ok(xpath::XVal::Str(v)) => Ok(s(v)),
    }
}

fn fn_dom_import_simplexml(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = arg(args, 0);
    match ids_of(&v) {
        Some((d, n)) => {
            let n = crate::ext::simplexml::target_node(&v).unwrap_or(n);
            Ok(wrap(host, d, n))
        }
        None => Err(Thrown::Error {
            class: "TypeError",
            message: "dom_import_simplexml(): Argument #1 ($node) must be a valid SimpleXML node"
                .into(),
        }),
    }
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    m.insert(
        "__dom",
        Builtin {
            name: "__dom",
            func: fn_dom,
            min_args: 1,
            max_args: None,
        },
    );
    m.insert(
        "dom_import_simplexml",
        Builtin {
            name: "dom_import_simplexml",
            func: fn_dom_import_simplexml,
            min_args: 1,
            max_args: Some(1),
        },
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
    ]
}

pub const PRELUDE_PHP: &str = include_str!("prelude.php");
