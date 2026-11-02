//! The `SimpleXML` extension over the shared libxml tree.
//!
//! A `SimpleXMLElement` mirrors php's `php_sxe_object`: a node (`__n` of
//! document `__d`) plus an iterator descriptor — `__t` (0 none / 1 element
//! list by name / 2 child elements / 3 attribute list), the name filter
//! `__nm`, and the namespace filter `__ns` / `__p` (is-prefix). The class is
//! PHP-source ([`PRELUDE_PHP`]) forwarding to `__sxe($op, ...)`.

use std::cell::RefCell;
use std::collections::HashMap;

use rhp_values::{conv, ArrayKey, PhpArray, PhpStr, Value};

use crate::ext::dom;
use crate::ext::libxml::parser::{self, ParseOpts};
use crate::ext::libxml::tree::{self, Doc, DocId, Kind, Node, NodeId, SaveOpts};
use crate::ext::libxml::{self as lx, xpath, Ctx};
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

fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&arg(args, i))
}

fn opt_bytes(args: &[Value], i: usize) -> Option<Vec<u8>> {
    match arg(args, i) {
        Value::Null => None,
        v => Some(conv::to_string_bytes(&v)),
    }
}

const T_NONE: i64 = 0;
const T_ELEMENT: i64 = 1;
const T_CHILD: i64 = 2;
const T_ATTRLIST: i64 = 3;

/// The decoded state of a SimpleXMLElement object.
#[derive(Clone)]
struct Sxe {
    class: Vec<u8>,
    d: DocId,
    n: NodeId,
    t: i64,
    name: Option<Vec<u8>>,
    ns: Option<Vec<u8>>,
    is_prefix: bool,
}

fn slot(h: &rhp_values::ObjectHandle, k: &[u8]) -> Value {
    h.borrow()
        .properties
        .get(&PhpStr::from_bytes(k.to_vec()))
        .cloned()
        .unwrap_or(Value::Null)
}

fn sxe_of(host: &dyn Host, v: &Value) -> Option<Sxe> {
    let Value::Object(h) = v else {
        return None;
    };
    let d = slot(h, b"__d");
    if matches!(d, Value::Null) {
        return None;
    }
    let class = host
        .object_class_name(v)
        .unwrap_or_else(|| b"SimpleXMLElement".to_vec());
    Some(Sxe {
        class,
        d: conv::to_int(&d) as usize,
        n: conv::to_int(&slot(h, b"__n")) as usize,
        t: conv::to_int(&slot(h, b"__t")),
        name: match slot(h, b"__nm") {
            Value::Null => None,
            x => Some(conv::to_string_bytes(&x)),
        },
        ns: match slot(h, b"__ns") {
            Value::Null => None,
            x => Some(conv::to_string_bytes(&x)),
        },
        is_prefix: conv::to_bool(&slot(h, b"__p")),
    })
}

#[allow(clippy::too_many_arguments)]
fn make(
    host: &mut dyn Host,
    class: &[u8],
    d: DocId,
    n: NodeId,
    t: i64,
    name: Option<Vec<u8>>,
    ns: Option<Vec<u8>>,
    is_prefix: bool,
) -> Value {
    host.instantiate(
        class,
        vec![
            (b"__d".to_vec(), Value::Int(d as i64)),
            (b"__n".to_vec(), Value::Int(n as i64)),
            (b"__t".to_vec(), Value::Int(t)),
            (
                b"__nm".to_vec(),
                name.map(Value::str).unwrap_or(Value::Null),
            ),
            (b"__ns".to_vec(), ns.map(Value::str).unwrap_or(Value::Null)),
            (b"__p".to_vec(), Value::Bool(is_prefix)),
        ],
    )
    .unwrap_or(Value::Null)
}

/// php's match_ns.
fn match_ns(doc: &Doc, id: NodeId, ns: &Option<Vec<u8>>, is_prefix: bool) -> bool {
    let n = &doc.nodes[id];
    match ns {
        None => n.ns.is_none() || n.prefix.is_empty(),
        Some(want) => {
            if n.ns.is_none() {
                return false;
            }
            if is_prefix {
                n.prefix == *want
            } else {
                n.ns.as_deref() == Some(&want[..])
            }
        }
    }
}

/// The nodes an sxe iterates (php_sxe_reset_iterator + fetch).
fn items(doc: &Doc, x: &Sxe) -> Vec<NodeId> {
    let node = &doc.nodes[x.n];
    match x.t {
        T_ATTRLIST => node
            .attrs
            .iter()
            .copied()
            .filter(|&a| {
                match_ns(doc, a, &x.ns, x.is_prefix)
                    && x.name.as_ref().is_none_or(|nm| doc.nodes[a].name == *nm)
            })
            .collect(),
        _ => {
            if node.kind == Kind::Attr {
                return Vec::new();
            }
            doc.children(x.n)
                .into_iter()
                .filter(|&c| {
                    let cn = &doc.nodes[c];
                    cn.kind == Kind::Element
                        && match_ns(doc, c, &x.ns, x.is_prefix)
                        && (x.t != T_ELEMENT || x.name.as_ref().is_none_or(|nm| cn.name == *nm))
                })
                .collect()
        }
    }
}

/// php_sxe_get_first_node.
fn first_node(doc: &Doc, x: &Sxe) -> Option<NodeId> {
    if x.t == T_NONE || x.t == T_CHILD {
        return Some(x.n);
    }
    items(doc, x).first().copied()
}

/// The node a SimpleXMLElement stands for (dom_import_simplexml).
pub fn target_node(v: &Value) -> Option<NodeId> {
    let Value::Object(h) = v else {
        return None;
    };
    let t = conv::to_int(&slot(h, b"__t"));
    let d = conv::to_int(&slot(h, b"__d")) as usize;
    let n = conv::to_int(&slot(h, b"__n")) as usize;
    let x = Sxe {
        class: Vec::new(),
        d,
        n,
        t,
        name: match slot(h, b"__nm") {
            Value::Null => None,
            v => Some(conv::to_string_bytes(&v)),
        },
        ns: match slot(h, b"__ns") {
            Value::Null => None,
            v => Some(conv::to_string_bytes(&v)),
        },
        is_prefix: conv::to_bool(&slot(h, b"__p")),
    };
    tree::with_doc(d, |doc| first_node(doc, &x)).flatten()
}

/// xmlNodeListGetString(node->children, 1): direct text content.
fn direct_text(doc: &Doc, id: NodeId) -> Vec<u8> {
    let n = &doc.nodes[id];
    if n.kind == Kind::Attr {
        return n.content.clone();
    }
    let mut out = Vec::new();
    let mut c = n.first;
    while let Some(x) = c {
        let cn = &doc.nodes[x];
        match cn.kind {
            Kind::Text | Kind::CData => out.extend_from_slice(&cn.content),
            Kind::EntityRef => out.extend_from_slice(doc.entity_value(&cn.name).unwrap_or(b"")),
            _ => {}
        }
        c = cn.next;
    }
    out
}

/// The string value of an sxe (its cast handler), for the VM's casts.
pub fn sxe_string(v: &Value) -> Option<Vec<u8>> {
    let Value::Object(h) = v else {
        return None;
    };
    let d = conv::to_int(&slot(h, b"__d")) as usize;
    let first = target_node(v)?;
    let t = conv::to_int(&slot(h, b"__t"));
    tree::with_doc(d, |doc| {
        if t == T_ATTRLIST {
            return doc.nodes[first].content.clone();
        }
        direct_text(doc, first)
    })
}

/// sxe_objects_compare: whether two SimpleXML objects wrap the same node
/// (a child/attribute list wraps its parent element).
pub fn sxe_same_node(a: &Value, b: &Value) -> bool {
    let (Value::Object(x), Value::Object(y)) = (a, b) else {
        return false;
    };
    let key = |h| {
        (
            conv::to_int(&slot(h, b"__d")),
            conv::to_int(&slot(h, b"__n")),
        )
    };
    key(x) == key(y)
}

/// A SimpleXMLElement's text as a string value (its scalar cast source), or
/// `None` for anything else.
pub fn sxe_scalar_text(v: &Value) -> Option<Value> {
    if !is_sxe_object(v) {
        return None;
    }
    Some(Value::str(sxe_string(v).unwrap_or_default()))
}

/// `(bool) $sxe`: true when it has a node.
pub fn sxe_bool(v: &Value) -> bool {
    target_node(v).is_some()
}

fn parse_doc(
    host: &mut dyn Host,
    fname: &str,
    data: &[u8],
    url: Option<&[u8]>,
    flags: i64,
) -> Result<Option<DocId>, Thrown> {
    let opts = ParseOpts::from_flags(flags, true, false);
    let (doc, errs) = parser::parse(data, &opts, url);
    let errs = lx::filter_by_flags(errs, flags);
    lx::report(host, fname, errs, Ctx::Generic)?;
    Ok(doc.map(tree::register))
}

fn load_common(host: &mut dyn Host, args: &[Value], is_file: bool) -> Result<Value, Thrown> {
    let fname = if is_file {
        "simplexml_load_file"
    } else {
        "simplexml_load_string"
    };
    let src = bytes(args, 0);
    let class = match arg(args, 1) {
        Value::Null => b"SimpleXMLElement".to_vec(),
        v => conv::to_string_bytes(&v),
    };
    let flags = conv::to_int(&arg(args, 2));
    let ns = bytes(args, 3);
    let is_prefix = conv::to_bool(&arg(args, 4));
    let (data, url) = if is_file {
        match read_plain(host, &src) {
            Some((d, u)) => (d, Some(u)),
            None => {
                lx::report_io(host, fname, &src)?;
                return Ok(Value::Bool(false));
            }
        }
    } else {
        (src, None)
    };
    let Some(d) = parse_doc(host, fname, &data, url.as_deref(), flags)? else {
        return Ok(Value::Bool(false));
    };
    let Some(root) = tree::with_doc(d, |doc| doc.root()).flatten() else {
        return Ok(Value::Bool(false));
    };
    let nsf = if ns.is_empty() { None } else { Some(ns) };
    Ok(make(host, &class, d, root, T_NONE, None, nsf, is_prefix))
}

fn fn_load_string(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    load_common(host, args, false)
}

fn fn_load_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    load_common(host, args, true)
}

fn fn_import_dom(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = arg(args, 0);
    let class = match arg(args, 1) {
        Value::Null => b"SimpleXMLElement".to_vec(),
        v => conv::to_string_bytes(&v),
    };
    let Some((d, n)) = dom::ids_of(&v) else {
        return Err(Thrown::Error {
            class: "TypeError",
            message: "simplexml_import_dom(): Argument #1 ($node) must be of type SimpleXMLElement|DOMNode".into(),
        });
    };
    let target = tree::with_doc(d, |doc| match doc.nodes[n].kind {
        Kind::Document => doc.root(),
        Kind::Element | Kind::Attr => Some(n),
        _ => None,
    })
    .flatten();
    match target {
        Some(t) => Ok(make(host, &class, d, t, T_NONE, None, None, false)),
        None => {
            crate::warn(host, "simplexml_import_dom", "Invalid Nodetype to import")?;
            Ok(Value::Null)
        }
    }
}

fn fn_sxe(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = String::from_utf8_lossy(&bytes(args, 0)).into_owned();
    if op == "construct" {
        // (op, obj, data, options, dataIsURL, ns, isPrefix)
        let obj = arg(args, 1);
        let data = bytes(args, 2);
        let flags = conv::to_int(&arg(args, 3));
        let is_url = conv::to_bool(&arg(args, 4));
        let (src, url) = if is_url {
            match read_plain(host, &data) {
                Some((d, u)) => (d, Some(u)),
                None => {
                    lx::report_io(host, "SimpleXMLElement::__construct", &data)?;
                    return Ok(Value::Bool(false));
                }
            }
        } else {
            (data, None)
        };
        let Some(d) = parse_doc(
            host,
            "SimpleXMLElement::__construct",
            &src,
            url.as_deref(),
            flags,
        )?
        else {
            return Ok(Value::Bool(false));
        };
        let Some(root) = tree::with_doc(d, |doc| doc.root()).flatten() else {
            return Ok(Value::Bool(false));
        };
        let ns = bytes(args, 5);
        if let Value::Object(h) = &obj {
            let mut b = h.borrow_mut();
            let mut put = |k: &[u8], v: Value| {
                b.properties.insert(PhpStr::from_bytes(k.to_vec()), v);
            };
            put(b"__d", Value::Int(d as i64));
            put(b"__n", Value::Int(root as i64));
            put(b"__t", Value::Int(T_NONE));
            put(b"__nm", Value::Null);
            put(
                b"__ns",
                if ns.is_empty() {
                    Value::Null
                } else {
                    Value::str(ns)
                },
            );
            put(b"__p", Value::Bool(conv::to_bool(&arg(args, 6))));
        }
        return Ok(Value::Bool(true));
    }
    let this = arg(args, 1);
    let Some(x) = sxe_of(host, &this) else {
        return Ok(Value::Null);
    };
    let d = x.d;
    match op.as_str() {
        "get" => {
            let name = bytes(args, 2);
            if x.t == T_ATTRLIST {
                let a = tree::with_doc(d, |doc| {
                    doc.nodes[x.n].attrs.iter().copied().find(|&a| {
                        doc.nodes[a].name == name && match_ns(doc, a, &x.ns, x.is_prefix)
                    })
                })
                .flatten();
                return Ok(match a {
                    Some(a) => make(
                        host,
                        &x.class,
                        d,
                        a,
                        T_NONE,
                        None,
                        x.ns.clone(),
                        x.is_prefix,
                    ),
                    None => Value::Null,
                });
            }
            let node = tree::with_doc(d, |doc| first_node(doc, &x)).flatten();
            let Some(node) = node else {
                return Ok(Value::Null);
            };
            if tree::with_doc(d, |doc| doc.nodes[node].kind == Kind::Attr).unwrap_or(false) {
                return Ok(Value::Null);
            }
            Ok(make(
                host,
                &x.class,
                d,
                node,
                T_ELEMENT,
                Some(name),
                x.ns.clone(),
                x.is_prefix,
            ))
        }
        "isset" => {
            let name = bytes(args, 2);
            let r = tree::with_doc(d, |doc| {
                if x.t == T_ATTRLIST {
                    return doc.nodes[x.n].attrs.iter().any(|&a| {
                        doc.nodes[a].name == name && match_ns(doc, a, &x.ns, x.is_prefix)
                    });
                }
                let Some(node) = first_node(doc, &x) else {
                    return false;
                };
                let sub = Sxe {
                    t: T_ELEMENT,
                    n: node,
                    name: Some(name.clone()),
                    ..x.clone()
                };
                !items(doc, &sub).is_empty()
            })
            .unwrap_or(false);
            Ok(Value::Bool(r))
        }
        "set" => {
            let name = bytes(args, 2);
            let val = bytes(args, 3);
            tree::with_doc(d, |doc| {
                if x.t == T_ATTRLIST {
                    set_attr(doc, x.n, &name, &val);
                    return;
                }
                let Some(node) = first_node(doc, &x) else {
                    return;
                };
                let sub = Sxe {
                    t: T_ELEMENT,
                    n: node,
                    name: Some(name.clone()),
                    ..x.clone()
                };
                match items(doc, &sub).first() {
                    Some(&c) => {
                        doc.remove_children(c);
                        let t = doc.new_text(Kind::Text, &val);
                        doc.append(c, t);
                    }
                    None => {
                        let c = doc.new_element(&name);
                        if let Some(u) = doc.nodes[node].ns.clone() {
                            if doc.nodes[node].prefix.is_empty() {
                                doc.nodes[c].ns = Some(u);
                            }
                        }
                        let t = doc.new_text(Kind::Text, &val);
                        doc.append(c, t);
                        doc.append(node, c);
                    }
                }
            });
            Ok(Value::Null)
        }
        "clone" => {
            // sxe_object_clone: the root element copies the whole document;
            // any other node is deep-copied (detached) into the same one.
            let is_root = tree::with_doc(d, |doc| {
                doc.nodes[x.n].parent == Some(0) && doc.nodes[x.n].kind == Kind::Element
            })
            .unwrap_or(false);
            let (nd, nn) = if is_root {
                let Some(copy) = tree::clone_doc(d) else {
                    return Ok(Value::Null);
                };
                let root = copy.root().unwrap_or(x.n);
                (tree::register(copy), root)
            } else {
                let nn = tree::with_doc(d, |doc| doc.clone_node(x.n, true)).unwrap_or(x.n);
                (d, nn)
            };
            if let Value::Object(h) = &this {
                let mut b = h.borrow_mut();
                b.properties
                    .insert(PhpStr::from_bytes(b"__d".to_vec()), Value::Int(nd as i64));
                b.properties
                    .insert(PhpStr::from_bytes(b"__n".to_vec()), Value::Int(nn as i64));
                b.properties
                    .insert(PhpStr::from_bytes(b"__it".to_vec()), Value::Null);
            }
            Ok(Value::Null)
        }
        "unset" => {
            let name = bytes(args, 2);
            tree::with_doc(d, |doc| {
                if x.t == T_ATTRLIST {
                    if let Some(a) = doc.nodes[x.n]
                        .attrs
                        .iter()
                        .copied()
                        .find(|&a| doc.nodes[a].name == name)
                    {
                        doc.unlink(a);
                    }
                    return;
                }
                let Some(node) = first_node(doc, &x) else {
                    return;
                };
                let sub = Sxe {
                    t: T_ELEMENT,
                    n: node,
                    name: Some(name.clone()),
                    ..x.clone()
                };
                for c in items(doc, &sub) {
                    doc.unlink(c);
                }
            });
            Ok(Value::Null)
        }
        "dimGet" | "dimIsset" | "dimUnset" | "dimSet" => {
            let key = arg(args, 2);
            let is_int = matches!(key, Value::Int(_))
                || matches!(&key, Value::Str(s) if is_int_string(s.as_bytes()));
            if is_int && x.t != T_ATTRLIST {
                let k = conv::to_int(&key);
                let target = tree::with_doc(d, |doc| {
                    if x.t == T_ELEMENT {
                        let it = items(doc, &x);
                        if k >= 0 {
                            it.get(k as usize).copied()
                        } else {
                            None
                        }
                    } else if k == 0 {
                        Some(x.n)
                    } else {
                        None
                    }
                })
                .flatten();
                return match op.as_str() {
                    "dimGet" => Ok(match target {
                        Some(t) => make(
                            host,
                            &x.class,
                            d,
                            t,
                            T_NONE,
                            None,
                            x.ns.clone(),
                            x.is_prefix,
                        ),
                        None => Value::Null,
                    }),
                    "dimIsset" => Ok(Value::Bool(target.is_some())),
                    "dimUnset" => {
                        if let Some(t) = target {
                            tree::with_doc(d, |doc| doc.unlink(t));
                        }
                        Ok(Value::Null)
                    }
                    _ => {
                        let val = bytes(args, 3);
                        if let Some(t) = target {
                            tree::with_doc(d, |doc| {
                                doc.remove_children(t);
                                let tx = doc.new_text(Kind::Text, &val);
                                doc.append(t, tx);
                            });
                        }
                        Ok(Value::Null)
                    }
                };
            }
            let name = conv::to_string_bytes(&key);
            let elem = if x.t == T_ATTRLIST {
                Some(x.n)
            } else {
                tree::with_doc(d, |doc| first_node(doc, &x)).flatten()
            };
            let Some(elem) = elem else {
                return Ok(if op == "dimIsset" {
                    Value::Bool(false)
                } else {
                    Value::Null
                });
            };
            let find =
                |doc: &Doc| -> Option<NodeId> {
                    if doc.nodes[elem].kind != Kind::Element {
                        return None;
                    }
                    doc.nodes[elem].attrs.iter().copied().find(|&a| {
                        doc.nodes[a].name == name && match_ns(doc, a, &x.ns, x.is_prefix)
                    })
                };
            match op.as_str() {
                "dimGet" => {
                    let a = tree::with_doc(d, |doc| find(doc)).flatten();
                    Ok(match a {
                        Some(a) => make(
                            host,
                            &x.class,
                            d,
                            a,
                            T_NONE,
                            None,
                            x.ns.clone(),
                            x.is_prefix,
                        ),
                        None => Value::Null,
                    })
                }
                "dimIsset" => Ok(Value::Bool(
                    tree::with_doc(d, |doc| find(doc).is_some()).unwrap_or(false),
                )),
                "dimUnset" => {
                    tree::with_doc(d, |doc| {
                        if let Some(a) = find(doc) {
                            doc.unlink(a);
                        }
                    });
                    Ok(Value::Null)
                }
                _ => {
                    let val = bytes(args, 3);
                    tree::with_doc(d, |doc| set_attr(doc, elem, &name, &val));
                    Ok(Value::Null)
                }
            }
        }
        "string" => Ok(Value::str(sxe_string(&this).unwrap_or_default())),
        "count" => Ok(Value::Int(
            tree::with_doc(d, |doc| {
                if x.t == T_ELEMENT || x.t == T_ATTRLIST {
                    items(doc, &x).len()
                } else {
                    let sub = Sxe {
                        t: T_CHILD,
                        ..x.clone()
                    };
                    items(doc, &sub).len()
                }
            })
            .unwrap_or(0) as i64,
        )),
        "getName" => Ok(Value::str(
            tree::with_doc(d, |doc| {
                first_node(doc, &x).map(|n| doc.nodes[n].name.clone())
            })
            .flatten()
            .unwrap_or_default(),
        )),
        "children" | "attributes" => {
            let ns = opt_bytes(args, 2).filter(|s| !s.is_empty());
            let is_prefix = conv::to_bool(&arg(args, 3));
            if x.t == T_ATTRLIST {
                return Ok(Value::Null);
            }
            let node = tree::with_doc(d, |doc| first_node(doc, &x)).flatten();
            let Some(node) = node else {
                return Ok(Value::Null);
            };
            if tree::with_doc(d, |doc| doc.nodes[node].kind == Kind::Attr).unwrap_or(false) {
                return Ok(Value::Null);
            }
            let t = if op == "children" {
                T_CHILD
            } else {
                T_ATTRLIST
            };
            Ok(make(host, &x.class, d, node, t, None, ns, is_prefix))
        }
        "iter" => {
            // the iteration list: node ids
            let ids = tree::with_doc(d, |doc| {
                if x.t == T_NONE {
                    let sub = Sxe {
                        t: T_CHILD,
                        ..x.clone()
                    };
                    items(doc, &sub)
                } else {
                    items(doc, &x)
                }
            })
            .unwrap_or_default();
            let mut a = PhpArray::new();
            for i in ids {
                a.push(Value::Int(i as i64));
            }
            Ok(Value::Array(a))
        }
        "iterItem" => {
            let id = conv::to_int(&arg(args, 2)) as usize;
            Ok(make(
                host,
                &x.class,
                d,
                id,
                T_NONE,
                None,
                x.ns.clone(),
                x.is_prefix,
            ))
        }
        "iterKey" => {
            let id = conv::to_int(&arg(args, 2)) as usize;
            Ok(Value::str(
                tree::with_doc(d, |doc| doc.nodes[id].name.clone()).unwrap_or_default(),
            ))
        }
        "hasChildren" => {
            let id = conv::to_int(&arg(args, 2)) as usize;
            let r = tree::with_doc(d, |doc| {
                let sub = Sxe {
                    t: T_CHILD,
                    n: id,
                    ..x.clone()
                };
                doc.nodes[id].kind == Kind::Element && !items(doc, &sub).is_empty()
            })
            .unwrap_or(false);
            Ok(Value::Bool(r))
        }
        "asXML" => {
            let node = tree::with_doc(d, |doc| first_node(doc, &x)).flatten();
            let Some(node) = node else {
                return Ok(Value::Bool(false));
            };
            tree::take_save_diags();
            let out = tree::with_doc(d, |doc| {
                let opts = SaveOpts {
                    format: false,
                    no_empty_tag: false,
                };
                if doc.nodes[node].parent == Some(0) {
                    tree::dump_doc(doc, &opts)
                } else {
                    tree::dump_node(doc, node, &opts)
                }
            })
            .unwrap_or_default();
            lx::report_save_diags(host, "SimpleXMLElement::asXML")?;
            match opt_bytes(args, 2) {
                Some(f) => Ok(Value::Bool(
                    lx::save_to_uri(host, "SimpleXMLElement::asXML", &f, &out)?.is_some(),
                )),
                None => Ok(Value::str(out)),
            }
        }
        "xpath" => {
            let expr = bytes(args, 2);
            let mut ns: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
            let node = tree::with_doc(d, |doc| first_node(doc, &x))
                .flatten()
                .unwrap_or(x.n);
            tree::with_doc(d, |doc| {
                let mut chain = Vec::new();
                let mut c = Some(node);
                while let Some(y) = c {
                    chain.push(y);
                    c = doc.nodes[y].parent;
                }
                for &y in chain.iter().rev() {
                    for (p, u) in &doc.nodes[y].ns_defs {
                        if !p.is_empty() {
                            ns.insert(p.clone(), u.clone());
                        }
                    }
                }
            });
            if let Value::Array(m) = arg(args, 3) {
                for (k, v) in m.iter() {
                    let key = match k {
                        ArrayKey::Int(i) => i.to_string().into_bytes(),
                        ArrayKey::Str(s) => s.as_bytes().to_vec(),
                    };
                    ns.insert(key, conv::to_string_bytes(v));
                }
            }
            let vars = HashMap::new();
            let r = tree::with_doc(d, |doc| xpath::evaluate(doc, &expr, node, &ns, &vars));
            match r {
                Some(Ok(xpath::XVal::Nodes(ids))) => {
                    let mut a = PhpArray::new();
                    for id in ids {
                        let kind =
                            tree::with_doc(d, |doc| doc.nodes[id].kind).unwrap_or(Kind::Text);
                        match kind {
                            Kind::Element | Kind::Attr => {
                                a.push(make(host, &x.class, d, id, T_NONE, None, None, false));
                            }
                            Kind::Text | Kind::CData | Kind::Comment => {
                                // php wraps the parent element
                                if let Some(p) =
                                    tree::with_doc(d, |doc| doc.nodes[id].parent).flatten()
                                {
                                    a.push(make(host, &x.class, d, p, T_NONE, None, None, false));
                                }
                            }
                            _ => {}
                        }
                    }
                    Ok(Value::Array(a))
                }
                Some(Ok(_)) => Ok(Value::Array(PhpArray::new())),
                Some(Err(e)) => {
                    let msg = match e {
                        xpath::XErr::Syntax => "Invalid expression\n".to_string(),
                        xpath::XErr::UndefinedPrefix => "Undefined namespace prefix\n".to_string(),
                        xpath::XErr::UnknownFunction(f) => format!(
                            "xmlXPathCompOpEval: function {} not found\n",
                            String::from_utf8_lossy(&f)
                        ),
                        _ => "Invalid type\n".to_string(),
                    };
                    lx::report(
                        host,
                        "SimpleXMLElement::xpath",
                        vec![lx::plain_error(2, 1207, 12, 0, msg)],
                        Ctx::Plain,
                    )?;
                    Ok(Value::Bool(false))
                }
                None => Ok(Value::Bool(false)),
            }
        }
        "addChild" => {
            let q = bytes(args, 2);
            let val = opt_bytes(args, 3);
            let ns = opt_bytes(args, 4);
            if q.is_empty() {
                return Err(Thrown::Error {
                    class: "ValueError",
                    message: "SimpleXMLElement::addChild(): Argument #1 ($qualifiedName) must not be empty".into(),
                });
            }
            let node = tree::with_doc(d, |doc| first_node(doc, &x)).flatten();
            let Some(node) = node else {
                crate::warn(
                    host,
                    "SimpleXMLElement::addChild",
                    "Cannot add child. Parent is not a permanent member of the XML tree",
                )?;
                return Ok(Value::Null);
            };
            let id = tree::with_doc(d, |doc| {
                let (p, l) = match q.iter().position(|&c| c == b':') {
                    Some(i) => (q[..i].to_vec(), q[i + 1..].to_vec()),
                    None => (Vec::new(), q.clone()),
                };
                let c = doc.new_element(&l);
                match &ns {
                    Some(u) if !u.is_empty() => {
                        doc.nodes[c].ns = Some(u.clone());
                        doc.nodes[c].prefix = p.clone();
                        if doc.lookup_ns(node, &p).as_deref() != Some(&u[..]) {
                            doc.nodes[c].ns_defs.push((p, u.clone()));
                        }
                    }
                    Some(_) => {
                        doc.nodes[c].name = q.clone();
                    }
                    None => {
                        if !p.is_empty() {
                            match doc.lookup_ns(node, &p) {
                                Some(u) => {
                                    doc.nodes[c].prefix = p;
                                    doc.nodes[c].ns = Some(u);
                                }
                                None => doc.nodes[c].name = q.clone(),
                            }
                        } else if let Some(u) = doc.nodes[node].ns.clone() {
                            // inherit the parent's namespace (xmlNewChild with ns = parent->ns)
                            doc.nodes[c].ns = Some(u);
                            doc.nodes[c].prefix = doc.nodes[node].prefix.clone();
                        }
                    }
                }
                // xmlNewChild: the value is parsed for entity references
                let mut warn = None;
                if let Some(v) = &val {
                    match dom::string_node_list(doc, v) {
                        Ok(ids) => {
                            for i in ids {
                                doc.append(c, i);
                            }
                        }
                        Err(e) => warn = Some(e),
                    }
                }
                doc.append(node, c);
                (c, warn)
            });
            let (id, warn) = id.unwrap_or((0, None));
            if let Some(w) = warn {
                lx::report_tree_error(host, "SimpleXMLElement::addChild", &w)?;
            }
            Ok(make(
                host,
                &x.class,
                d,
                id,
                T_NONE,
                None,
                x.ns.clone(),
                x.is_prefix,
            ))
        }
        "addAttribute" => {
            let q = bytes(args, 2);
            let val = bytes(args, 3);
            let ns = opt_bytes(args, 4).filter(|s| !s.is_empty());
            let node = tree::with_doc(d, |doc| first_node(doc, &x)).flatten();
            if let Some(node) = node {
                tree::with_doc(d, |doc| {
                    let (p, l) = match q.iter().position(|&c| c == b':') {
                        Some(i) => (q[..i].to_vec(), q[i + 1..].to_vec()),
                        None => (Vec::new(), q.clone()),
                    };
                    let mut a = Node::new(Kind::Attr);
                    match &ns {
                        Some(u) => {
                            a.name = l;
                            a.prefix = p.clone();
                            a.ns = Some(u.clone());
                            if doc.lookup_ns(node, &p).as_deref() != Some(&u[..]) {
                                doc.nodes[node].ns_defs.push((p, u.clone()));
                            }
                        }
                        None => a.name = q.clone(),
                    }
                    a.content = val.clone();
                    let id = doc.add(a);
                    doc.insert(node, id, None);
                });
            }
            Ok(Value::Null)
        }
        "getNamespaces" | "getDocNamespaces" => {
            let recursive = conv::to_bool(&arg(args, 2));
            let from_root = args.len() < 4 || conv::to_bool(&arg(args, 3));
            let map = tree::with_doc(d, |doc| {
                let mut out: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
                let mut add = |p: &[u8], u: &[u8]| {
                    if !out.iter().any(|(x, _)| x == p) {
                        out.push((p.to_vec(), u.to_vec()));
                    }
                };
                if op == "getNamespaces" {
                    let start = first_node(doc, &x).unwrap_or(x.n);
                    let mut nodes = vec![start];
                    if recursive {
                        doc.descendants(start, &mut nodes);
                    }
                    for id in nodes {
                        let n = &doc.nodes[id];
                        if matches!(n.kind, Kind::Element | Kind::Attr) {
                            if let Some(u) = &n.ns {
                                add(&n.prefix, u);
                            }
                            for &a in &n.attrs {
                                if let Some(u) = &doc.nodes[a].ns {
                                    add(&doc.nodes[a].prefix, u);
                                }
                            }
                        }
                    }
                } else {
                    let start = if from_root {
                        doc.root().unwrap_or(x.n)
                    } else {
                        x.n
                    };
                    let mut nodes = vec![start];
                    if recursive {
                        doc.descendants(start, &mut nodes);
                    }
                    for id in nodes {
                        for (p, u) in &doc.nodes[id].ns_defs {
                            add(p, u);
                        }
                    }
                }
                out
            })
            .unwrap_or_default();
            let mut a = PhpArray::new();
            for (p, u) in map {
                a.insert(ArrayKey::from_bytes(&p), Value::str(u));
            }
            Ok(Value::Array(a))
        }
        _ => Err(Thrown::Error {
            class: "Error",
            message: format!("__sxe: unknown op {op}"),
        }),
    }
}

/// Read a file the way xmlReadFile does (the path recorded as given).
fn read_plain(host: &mut dyn Host, path: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let p = String::from_utf8_lossy(path).into_owned();
    if p.contains("://") && !p.starts_with("file://") {
        return host.stream_get_contents(path).map(|d| (d, path.to_vec()));
    }
    let f = p.strip_prefix("file://").unwrap_or(&p);
    std::fs::read(f).ok().map(|d| (d, path.to_vec()))
}

fn is_int_string(s: &[u8]) -> bool {
    let b = s.strip_prefix(b"-").unwrap_or(s);
    !b.is_empty() && b.iter().all(|c| c.is_ascii_digit()) && (b == b"0" || b[0] != b'0')
}

fn set_attr(doc: &mut Doc, el: NodeId, name: &[u8], val: &[u8]) {
    if let Some(a) = doc.nodes[el]
        .attrs
        .iter()
        .copied()
        .find(|&a| doc.nodes[a].name == name && doc.nodes[a].ns.is_none())
    {
        doc.nodes[a].content = val.to_vec();
        return;
    }
    let mut a = Node::new(Kind::Attr);
    a.name = name.to_vec();
    a.content = val.to_vec();
    let id = doc.add(a);
    doc.insert(el, id, None);
}

// ---------------------------------------------------------------- property views

fn is_blank_text(doc: &Doc, id: NodeId) -> bool {
    let n = &doc.nodes[id];
    matches!(n.kind, Kind::Text | Kind::CData)
        && n.content
            .iter()
            .all(|&c| matches!(c, b' ' | b'\t' | b'\n' | b'\r'))
}

/// libxml2's node->name as SimpleXML sees it (text nodes are "text",
/// comments "comment", CDATA sections have none).
fn sxe_node_name(doc: &Doc, id: NodeId) -> Option<Vec<u8>> {
    let n = &doc.nodes[id];
    match n.kind {
        Kind::Element | Kind::Attr | Kind::Pi | Kind::EntityRef => Some(n.name.clone()),
        Kind::Text => Some(b"text".to_vec()),
        Kind::Comment => Some(b"comment".to_vec()),
        _ => None,
    }
}

/// What a property view entry holds before objects are materialized.
enum PropVal {
    Str(Vec<u8>),
    Node(NodeId),
    Arr(Vec<PropVal>),
    Assoc(Vec<(Vec<u8>, Vec<u8>)>),
}

fn props_add(rv: &mut Vec<(Vec<u8>, PropVal)>, name: Vec<u8>, v: PropVal) {
    if let Some((_, existing)) = rv.iter_mut().find(|(k, _)| *k == name) {
        match existing {
            PropVal::Arr(a) => a.push(v),
            _ => {
                let old = std::mem::replace(existing, PropVal::Arr(Vec::new()));
                if let PropVal::Arr(a) = existing {
                    a.push(old);
                    a.push(v);
                }
            }
        }
    } else {
        rv.push((name, v));
    }
}

/// _get_base_node_value: the text of a leaf element, else the element.
fn base_node_value(doc: &Doc, id: NodeId) -> PropVal {
    let n = &doc.nodes[id];
    if let Some(c) = n.first {
        if doc.nodes[c].kind == Kind::Text && !is_blank_text(doc, c) {
            return PropVal::Str(direct_text(doc, id));
        }
    }
    PropVal::Node(id)
}

/// php's sxe_get_prop_hash, unmaterialized. Numeric keys are `None`.
fn prop_hash(doc: &Doc, x: &Sxe, is_debug: bool) -> Vec<(Option<Vec<u8>>, PropVal)> {
    let mut named: Vec<(Vec<u8>, PropVal)> = Vec::new();
    let mut order: Vec<Option<Vec<u8>>> = Vec::new();
    let mut numeric: Vec<PropVal> = Vec::new();
    let add_named = |named: &mut Vec<(Vec<u8>, PropVal)>,
                     order: &mut Vec<Option<Vec<u8>>>,
                     k: Vec<u8>,
                     v: PropVal| {
        if !named.iter().any(|(n, _)| *n == k) {
            order.push(Some(k.clone()));
        }
        props_add(named, k, v);
    };
    let first = first_node(doc, x);
    if is_debug || x.t != T_CHILD {
        let node = if x.t == T_ELEMENT { first } else { Some(x.n) };
        if let Some(node) = node {
            if doc.nodes[node].kind == Kind::Element {
                let test = x.name.is_some() && x.t == T_ATTRLIST;
                let mut attrs = Vec::new();
                for &a in &doc.nodes[node].attrs {
                    let an = &doc.nodes[a];
                    if (!test || Some(&an.name) == x.name.as_ref())
                        && match_ns(doc, a, &x.ns, x.is_prefix)
                    {
                        attrs.push((an.name.clone(), an.content.clone()));
                    }
                }
                if !attrs.is_empty() {
                    add_named(
                        &mut named,
                        &mut order,
                        b"@attributes".to_vec(),
                        PropVal::Assoc(attrs),
                    );
                }
            }
        }
    }
    if let Some(node) = first {
        if x.t != T_ATTRLIST {
            let nd = &doc.nodes[node];
            if nd.kind == Kind::Attr {
                numeric.push(PropVal::Str(nd.content.clone()));
                order.push(None);
            } else {
                let mut use_iter = false;
                let mut cur: Option<NodeId> = Some(node);
                if x.t != T_CHILD {
                    let leaf_sibling_list = x.t != T_NONE
                        && nd.first.is_some()
                        && nd.parent.is_some()
                        && nd.next.is_some()
                        && nd.first.and_then(|c| doc.nodes[c].next).is_none()
                        && nd.first.and_then(|c| doc.nodes[c].first).is_none()
                        && nd
                            .parent
                            .map(|p| doc.nodes[p].first != doc.nodes[p].last)
                            .unwrap_or(false);
                    if leaf_sibling_list {
                        use_iter = true;
                    } else {
                        cur = nd.first;
                    }
                } else {
                    cur = nd.first;
                }
                let iter_items = if use_iter { items(doc, x) } else { Vec::new() };
                let mut idx = 0;
                if use_iter {
                    cur = iter_items.first().copied();
                }
                while let Some(c) = cur {
                    let cn = &doc.nodes[c];
                    let skip_text = cn.first.is_some()
                        || cn.prev.is_some()
                        || cn.next.is_some()
                        || is_blank_text(doc, c);
                    let mut handled = false;
                    if skip_text {
                        if cn.kind == Kind::Text {
                            handled = true;
                        }
                    } else if cn.kind == Kind::Text {
                        if !cn.content.is_empty() {
                            numeric.push(PropVal::Str(cn.content.clone()));
                            order.push(None);
                        }
                        handled = true;
                    }
                    if !handled {
                        let ns_ok =
                            cn.kind != Kind::Element || match_ns(doc, c, &x.ns, x.is_prefix);
                        if ns_ok {
                            if let Some(name) = sxe_node_name(doc, c) {
                                let v = base_node_value(doc, c);
                                if use_iter {
                                    numeric.push(v);
                                    order.push(None);
                                } else {
                                    add_named(&mut named, &mut order, name, v);
                                }
                            }
                        }
                    }
                    if use_iter {
                        idx += 1;
                        cur = iter_items.get(idx).copied();
                    } else {
                        cur = doc.nodes[c].next;
                    }
                }
            }
        }
    }
    // reassemble in insertion order
    let mut out = Vec::new();
    let mut nums = numeric.into_iter();
    let mut named_map: Vec<(Vec<u8>, Option<PropVal>)> =
        named.into_iter().map(|(k, v)| (k, Some(v))).collect();
    for o in order {
        match o {
            None => {
                if let Some(v) = nums.next() {
                    out.push((None, v));
                }
            }
            Some(k) => {
                if let Some((_, slot)) = named_map.iter_mut().find(|(n, _)| *n == k) {
                    if let Some(v) = slot.take() {
                        out.push((Some(k), v));
                    }
                }
            }
        }
    }
    out
}

fn materialize(host: &mut dyn Host, x: &Sxe, v: PropVal) -> Value {
    match v {
        PropVal::Str(s) => Value::str(s),
        PropVal::Node(id) => {
            let ns = x.ns.clone().filter(|n| !n.is_empty());
            make(host, &x.class, x.d, id, T_NONE, None, ns, x.is_prefix)
        }
        PropVal::Arr(items) => {
            let mut a = PhpArray::new();
            for i in items {
                let v = materialize(host, x, i);
                a.push(v);
            }
            Value::Array(a)
        }
        PropVal::Assoc(pairs) => {
            let mut a = PhpArray::new();
            for (k, v) in pairs {
                a.insert(ArrayKey::from_bytes(&k), Value::str(v));
            }
            Value::Array(a)
        }
    }
}

/// A SimpleXMLElement's property table as var_dump / print_r (`is_debug`)
/// or json_encode / casts see it; None for any other value.
pub fn sxe_props(host: &mut dyn Host, v: &Value, is_debug: bool) -> Option<Vec<(PhpStr, Value)>> {
    let x = sxe_of(host, v)?;
    if !is_sxe_object(v) {
        return None;
    }
    let raw = tree::with_doc(x.d, |doc| prop_hash(doc, &x, is_debug))?;
    let mut out = Vec::new();
    let mut next = 0i64;
    for (k, pv) in raw {
        let val = materialize(host, &x, pv);
        let key = match k {
            Some(k) => {
                if let Some(n) = std::str::from_utf8(&k)
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                {
                    next = next.max(n + 1);
                }
                PhpStr::from_bytes(k)
            }
            None => {
                let key = PhpStr::from_bytes(next.to_string().into_bytes());
                next += 1;
                key
            }
        };
        out.push((key, val));
    }
    Some(out)
}

/// `sxe_get_properties` as an array: what `(array)` and `get_object_vars`
/// see. `None` when `v` is not a SimpleXMLElement.
pub fn sxe_props_array(host: &mut dyn Host, v: &Value) -> Option<PhpArray> {
    let props = sxe_props(host, v, false)?;
    let mut arr = PhpArray::new();
    for (k, val) in props {
        arr.insert(ArrayKey::from_bytes(k.as_bytes()), val);
    }
    Some(arr)
}

/// Whether `v` is a SimpleXMLElement (by its internal slots).
pub fn is_sxe_object(v: &Value) -> bool {
    let Value::Object(h) = v else {
        return false;
    };
    let b = h.borrow();
    b.properties
        .contains_key(&PhpStr::from_bytes(b"__nm".to_vec()))
        && b.properties
            .contains_key(&PhpStr::from_bytes(b"__t".to_vec()))
        && b.properties
            .contains_key(&PhpStr::from_bytes(b"__xns".to_vec()))
}

thread_local! {
    static DEBUG_VIEWS: RefCell<HashMap<u32, Vec<(PhpStr, Value)>>> = RefCell::new(HashMap::new());
}

/// Before var_dump / print_r (which only get `&dyn Host`): materialize the
/// debug property views of every SimpleXMLElement reachable from `v`.
pub fn prepare_debug(host: &mut dyn Host, v: &Value, depth: usize) {
    let mut seen = std::collections::HashSet::new();
    prepare_debug_in(host, v, depth, &mut seen);
}

/// Walk `v` once (each object / reference cell visited at most once, so
/// cyclic graphs stay linear), recording SimpleXMLElement debug views.
fn prepare_debug_in(
    host: &mut dyn Host,
    v: &Value,
    depth: usize,
    seen: &mut std::collections::HashSet<usize>,
) {
    if depth > 64 {
        return;
    }
    match v {
        Value::Ref(r) => {
            if !seen.insert(std::rc::Rc::as_ptr(r) as *const () as usize) {
                return;
            }
            let inner = r.borrow().clone();
            prepare_debug_in(host, &inner, depth + 1, seen);
        }
        Value::Array(a) => {
            let vals: Vec<Value> = a.iter().map(|(_, v)| v.clone()).collect();
            for x in vals {
                prepare_debug_in(host, &x, depth + 1, seen);
            }
        }
        Value::Object(h) => {
            let id = h.borrow().id;
            if !seen.insert(std::rc::Rc::as_ptr(h) as *const () as usize) {
                return;
            }
            if DEBUG_VIEWS.with(|m| m.borrow().contains_key(&id)) {
                return;
            }
            if is_sxe_object(v) {
                if let Some(props) = sxe_props(host, v, true) {
                    DEBUG_VIEWS.with(|m| m.borrow_mut().insert(id, props.clone()));
                    for (_, pv) in props {
                        prepare_debug_in(host, &pv, depth + 1, seen);
                    }
                }
            } else {
                let vals: Vec<Value> = h.borrow().properties.values().cloned().collect();
                for x in vals {
                    prepare_debug_in(host, &x, depth + 1, seen);
                }
            }
        }
        _ => {}
    }
}

/// The prepared debug view of object `id`, if it is a SimpleXMLElement.
pub fn debug_view(id: u32) -> Option<Vec<(PhpStr, Value)>> {
    DEBUG_VIEWS.with(|m| m.borrow().get(&id).cloned())
}

pub fn clear_debug() {
    DEBUG_VIEWS.with(|m| m.borrow_mut().clear());
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    m.insert(
        "__sxe",
        Builtin {
            name: "__sxe",
            func: fn_sxe,
            min_args: 1,
            max_args: None,
        },
    );
    m.insert(
        "simplexml_load_string",
        Builtin {
            name: "simplexml_load_string",
            func: fn_load_string,
            min_args: 1,
            max_args: Some(5),
        },
    );
    m.insert(
        "simplexml_load_file",
        Builtin {
            name: "simplexml_load_file",
            func: fn_load_file,
            min_args: 1,
            max_args: Some(5),
        },
    );
    m.insert(
        "simplexml_import_dom",
        Builtin {
            name: "simplexml_import_dom",
            func: fn_import_dom,
            min_args: 1,
            max_args: Some(2),
        },
    );
}

pub const PRELUDE_PHP: &str = r#"
class SimpleXMLElement implements Stringable, Countable, RecursiveIterator, ArrayAccess {
    public $__d; public $__n; public $__t; public $__nm; public $__ns; public $__p;
    public $__it; public $__i = 0; public $__xns = [];
    public function __construct(string $data, int $options = 0, bool $dataIsURL = false, string $namespaceOrPrefix = "", bool $isPrefix = false) {
        if (!__sxe('construct', $this, $data, $options, $dataIsURL, $namespaceOrPrefix, $isPrefix)) {
            throw new Exception('String could not be parsed as XML');
        }
    }
    public function __get($name) { return __sxe('get', $this, $name); }
    public function __set($name, $value) { __sxe('set', $this, $name, $value); }
    public function __isset($name) { return __sxe('isset', $this, $name); }
    public function __unset($name) { __sxe('unset', $this, $name); }
    public function offsetGet($k) { return __sxe('dimGet', $this, $k); }
    public function offsetExists($k) { return __sxe('dimIsset', $this, $k); }
    public function offsetSet($k, $v) { __sxe('dimSet', $this, $k, $v); }
    public function offsetUnset($k) { __sxe('dimUnset', $this, $k); }
    public function __toString(): string { return __sxe('string', $this); }
    public function count(): int { return __sxe('count', $this); }
    public function getName(): string { return __sxe('getName', $this); }
    public function children(?string $namespaceOrPrefix = null, bool $isPrefix = false): ?static { return __sxe('children', $this, $namespaceOrPrefix, $isPrefix); }
    public function attributes(?string $namespaceOrPrefix = null, bool $isPrefix = false): ?static { return __sxe('attributes', $this, $namespaceOrPrefix, $isPrefix); }
    public function asXML(?string $filename = null): string|bool { return __sxe('asXML', $this, $filename); }
    public function saveXML(?string $filename = null): string|bool { return __sxe('asXML', $this, $filename); }
    public function xpath(string $expression): array|null|false { return __sxe('xpath', $this, $expression, $this->__xns); }
    public function registerXPathNamespace(string $prefix, string $namespace): bool { $this->__xns[$prefix] = $namespace; return true; }
    public function addChild(string $qualifiedName, ?string $value = null, ?string $namespace = null): ?static { return __sxe('addChild', $this, $qualifiedName, $value, $namespace); }
    public function addAttribute(string $qualifiedName, string $value, ?string $namespace = null): void { __sxe('addAttribute', $this, $qualifiedName, $value, $namespace); }
    public function getNamespaces(bool $recursive = false): array { return __sxe('getNamespaces', $this, $recursive); }
    public function getDocNamespaces(bool $recursive = false, bool $fromRoot = true): array|false { return __sxe('getDocNamespaces', $this, $recursive, $fromRoot); }
    public function rewind(): void { $this->__it = __sxe('iter', $this); $this->__i = 0; }
    public function valid(): bool { if ($this->__it === null) { $this->rewind(); } return $this->__i < count($this->__it); }
    public function current(): ?static { if ($this->__it === null) { $this->rewind(); } return isset($this->__it[$this->__i]) ? __sxe('iterItem', $this, $this->__it[$this->__i]) : null; }
    public function key(): string { if ($this->__it === null) { $this->rewind(); } return isset($this->__it[$this->__i]) ? __sxe('iterKey', $this, $this->__it[$this->__i]) : ''; }
    public function next(): void { $this->__i++; }
    public function hasChildren(): bool { return isset($this->__it[$this->__i]) && __sxe('hasChildren', $this, $this->__it[$this->__i]); }
    public function getChildren(): ?static { $c = $this->current(); return $c === null ? null : $c->children(); }
    public function __sleep(): array { throw new Exception("Serialization of '" . static::class . "' is not allowed"); }
    public function __wakeup(): void { throw new Exception("Unserialization of '" . static::class . "' is not allowed"); }
    public function __clone() { __sxe('clone', $this); }
}

class SimpleXMLIterator extends SimpleXMLElement {}
"#;
