//! The `xmlreader` extension: libxml2's xmlTextReader pull API over the
//! shared tree. The document is parsed up front and walked as the node
//! sequence the reader would produce (element / end-element / text /
//! whitespace / …, with namespace declarations listed as attributes). A
//! malformed document fails on the first `read()`, the way libxml2's reader
//! fails for inputs that fit its first push chunk.

use std::cell::RefCell;
use std::collections::HashMap;

use rhp_values::{conv, Value};

use crate::ext::dom;
use crate::ext::libxml::parser::{self, ParseOpts, XmlError};
use crate::ext::libxml::tree::{self, DocId, Kind, NodeId, SaveOpts, XMLNS_NS};
use crate::ext::libxml::{self as lx, Ctx};
use crate::{Builtin, Host, Thrown};

/// `XMLReader`'s handlers (`xmlreader_object_handlers`: no clone).
pub(crate) static CLASSES: &[crate::object::ClassHandlers] = &[crate::object::ClassHandlers {
    class: "XMLReader",
    handlers: &crate::object::ObjectHandlers::UNCLONEABLE,
}];

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

fn s(b: impl Into<Vec<u8>>) -> Value {
    Value::str(b)
}

#[derive(Clone, Copy)]
struct Ev {
    node: NodeId,
    kind: i64,
    depth: i64,
}

struct Reader {
    doc: Option<DocId>,
    events: Vec<Ev>,
    /// Index of the current event (-1 before the first read).
    pos: isize,
    /// Current attribute (index into the node's attribute list).
    attr: Option<usize>,
    pending: Vec<XmlError>,
    done: bool,
}

thread_local! {
    static READERS: RefCell<HashMap<i64, Reader>> = RefCell::new(HashMap::new());
    static NEXT: RefCell<i64> = const { RefCell::new(1) };
}

/// The attribute list xmlTextReader exposes: namespace declarations first
/// (as `xmlns` / `xmlns:p`), then the attributes. (name, local, prefix, ns, value)
type AttrView = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);

fn attr_list(doc: &tree::Doc, n: NodeId) -> Vec<AttrView> {
    let node = &doc.nodes[n];
    let mut out = Vec::new();
    if node.kind != Kind::Element {
        return out;
    }
    for (p, u) in &node.ns_defs {
        if p.is_empty() {
            out.push((
                b"xmlns".to_vec(),
                b"xmlns".to_vec(),
                Vec::new(),
                XMLNS_NS.to_vec(),
                u.clone(),
            ));
        } else {
            out.push((
                [b"xmlns:".as_slice(), p].concat(),
                p.clone(),
                b"xmlns".to_vec(),
                XMLNS_NS.to_vec(),
                u.clone(),
            ));
        }
    }
    for &a in &node.attrs {
        let an = &doc.nodes[a];
        out.push((
            an.qname(),
            an.name.clone(),
            an.prefix.clone(),
            an.ns.clone().unwrap_or_default(),
            an.content.clone(),
        ));
    }
    out
}

fn is_blank(doc: &tree::Doc, n: NodeId) -> bool {
    doc.nodes[n]
        .content
        .iter()
        .all(|&c| matches!(c, b' ' | b'\t' | b'\n' | b'\r'))
}

/// xmlNodeGetSpacePreserve: 1 / 0 from the nearest xml:space, else -1.
fn space_preserve(doc: &tree::Doc, mut n: NodeId) -> i32 {
    loop {
        let node = &doc.nodes[n];
        if node.kind == Kind::Element {
            for &a in &node.attrs {
                let an = &doc.nodes[a];
                if an.prefix == b"xml" && an.name == b"space" {
                    return match &an.content[..] {
                        b"preserve" => 1,
                        b"default" => 0,
                        _ => -1,
                    };
                }
            }
        }
        match node.parent {
            Some(p) => n = p,
            None => return -1,
        }
    }
}

fn build_events(doc: &tree::Doc) -> Vec<Ev> {
    fn walk(doc: &tree::Doc, n: NodeId, depth: i64, out: &mut Vec<Ev>) {
        let mut c = doc.nodes[n].first;
        while let Some(x) = c {
            let node = &doc.nodes[x];
            match node.kind {
                Kind::Element => {
                    out.push(Ev {
                        node: x,
                        kind: 1,
                        depth,
                    });
                    if node.first.is_some() || !node.empty_tag {
                        walk(doc, x, depth + 1, out);
                        out.push(Ev {
                            node: x,
                            kind: 15,
                            depth,
                        });
                    }
                }
                Kind::Text => {
                    let kind = if is_blank(doc, x) {
                        if space_preserve(doc, x) != 0 {
                            14
                        } else {
                            13
                        }
                    } else {
                        3
                    };
                    out.push(Ev {
                        node: x,
                        kind,
                        depth,
                    });
                }
                Kind::CData => out.push(Ev {
                    node: x,
                    kind: 4,
                    depth,
                }),
                Kind::Comment => out.push(Ev {
                    node: x,
                    kind: 8,
                    depth,
                }),
                Kind::Pi => out.push(Ev {
                    node: x,
                    kind: 7,
                    depth,
                }),
                Kind::DocType => out.push(Ev {
                    node: x,
                    kind: 10,
                    depth,
                }),
                Kind::EntityRef => out.push(Ev {
                    node: x,
                    kind: 5,
                    depth,
                }),
                _ => {}
            }
            c = node.next;
        }
    }
    let mut out = Vec::new();
    walk(doc, 0, 0, &mut out);
    out
}

fn with_r<R>(id: i64, f: impl FnOnce(&mut Reader) -> R) -> Option<R> {
    READERS.with(|r| r.borrow_mut().get_mut(&id).map(f))
}

fn open_reader(host: &mut dyn Host, data: Vec<u8>, url: Option<Vec<u8>>, flags: i64) -> i64 {
    let opts = ParseOpts::from_flags(flags, true, flags & 4 != 0);
    let (doc, errs) = parser::parse(&data, &opts, url.as_deref());
    let mut r = Reader {
        doc: None,
        events: Vec::new(),
        pos: -1,
        attr: None,
        pending: Vec::new(),
        done: false,
    };
    match doc {
        Some(mut d) => {
            if url.is_none() {
                if let Ok(cwd) = std::env::current_dir() {
                    let mut u = cwd.to_string_lossy().into_owned();
                    if !u.ends_with('/') {
                        u.push('/');
                    }
                    d.url = Some(u.into_bytes());
                }
            }
            r.events = build_events(&d);
            r.doc = Some(tree::register(d));
        }
        None => {
            // the push parser reports a premature end as "Extra content"
            let base = url.clone().or_else(|| {
                std::env::current_dir().ok().map(|c| {
                    let mut u = c.to_string_lossy().into_owned();
                    if !u.ends_with('/') {
                        u.push('/');
                    }
                    u.into_bytes()
                })
            });
            let mut pend = Vec::new();
            for mut e in errs {
                e.file = base.clone();
                if e.level == 3
                    && matches!(e.code, 77 | 73)
                    && e.message.starts_with(if e.code == 77 {
                        "Premature"
                    } else {
                        "Couldn't"
                    })
                {
                    e.code = 5;
                    e.message = "Extra content at the end of the document\n".into();
                }
                let fatal = e.level == 3;
                pend.push(e);
                if fatal {
                    break;
                }
            }
            r.pending = pend;
        }
    }
    let _ = host;
    let id = NEXT.with(|n| {
        let mut n = n.borrow_mut();
        let v = *n;
        *n += 1;
        v
    });
    READERS.with(|m| m.borrow_mut().insert(id, r));
    id
}

fn fn_xr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = String::from_utf8_lossy(&bytes(args, 0)).into_owned();
    let id = conv::to_int(&arg(args, 1));
    match op.as_str() {
        "xml" => {
            let data = bytes(args, 2);
            if data.is_empty() {
                return Err(Thrown::Error {
                    class: "ValueError",
                    message: "XMLReader::XML(): Argument #1 ($source) cannot be empty".into(),
                });
            }
            let flags = conv::to_int(&arg(args, 4));
            Ok(Value::Int(open_reader(host, data, None, flags)))
        }
        "open" => {
            let uri = bytes(args, 2);
            if uri.is_empty() {
                return Err(Thrown::Error {
                    class: "ValueError",
                    message: "XMLReader::open(): Argument #1 ($uri) cannot be empty".into(),
                });
            }
            let flags = conv::to_int(&arg(args, 4));
            match dom::read_source(host, &uri) {
                Some((data, url)) => Ok(Value::Int(open_reader(host, data, Some(url), flags))),
                None => {
                    crate::warn(host, "XMLReader::open", "Unable to open source data")?;
                    Ok(Value::Bool(false))
                }
            }
        }
        "stream" => {
            let stream = arg(args, 2);
            let mut data = Vec::new();
            while let Some(chunk) = host.stream_read(&stream, 8192) {
                if chunk.is_empty() {
                    break;
                }
                data.extend_from_slice(&chunk);
            }
            let flags = conv::to_int(&arg(args, 4));
            let url = match arg(args, 5) {
                Value::Null => None,
                v => Some(conv::to_string_bytes(&v)),
            };
            Ok(Value::Int(open_reader(host, data, url, flags)))
        }
        "close" => {
            READERS.with(|m| m.borrow_mut().remove(&id));
            Ok(Value::Bool(true))
        }
        "read" => {
            let pending = with_r(id, |r| std::mem::take(&mut r.pending)).unwrap_or_default();
            if !pending.is_empty() {
                with_r(id, |r| r.done = true);
                lx::report(host, "XMLReader::read", pending, Ctx::Generic)?;
                return Ok(Value::Bool(false));
            }
            let ok = with_r(id, |r| {
                if r.done {
                    return false;
                }
                r.attr = None;
                r.pos += 1;
                if r.pos as usize >= r.events.len() {
                    r.done = true;
                    r.pos = r.events.len() as isize;
                    return false;
                }
                true
            })
            .unwrap_or(false);
            Ok(Value::Bool(ok))
        }
        "next" => {
            let name = match arg(args, 2) {
                Value::Null => None,
                v => Some(conv::to_string_bytes(&v)),
            };
            let pending = with_r(id, |r| std::mem::take(&mut r.pending)).unwrap_or_default();
            if !pending.is_empty() {
                with_r(id, |r| r.done = true);
                lx::report(host, "XMLReader::next", pending, Ctx::Generic)?;
                return Ok(Value::Bool(false));
            }
            let ok = with_r(id, |r| {
                if r.done {
                    return false;
                }
                let doc = r.doc;
                r.attr = None;
                // skip the current element's subtree
                if r.pos >= 0 && (r.pos as usize) < r.events.len() {
                    let cur = r.events[r.pos as usize];
                    if cur.kind == 1 {
                        let mut i = r.pos as usize + 1;
                        while i < r.events.len()
                            && !(r.events[i].node == cur.node && r.events[i].kind == 15)
                        {
                            if r.events[i].depth <= cur.depth && r.events[i].kind != 15 {
                                i -= 1;
                                break;
                            }
                            i += 1;
                        }
                        r.pos = i as isize;
                    }
                }
                loop {
                    r.pos += 1;
                    if r.pos as usize >= r.events.len() {
                        r.done = true;
                        r.pos = r.events.len() as isize;
                        return false;
                    }
                    let e = r.events[r.pos as usize];
                    let Some(nm) = &name else {
                        return true;
                    };
                    if e.kind == 1 {
                        let qn = doc
                            .and_then(|d| tree::with_doc(d, |dd| dd.nodes[e.node].qname()))
                            .unwrap_or_default();
                        if qn == *nm {
                            return true;
                        }
                        // skip this element's subtree
                        let mut i = r.pos as usize + 1;
                        while i < r.events.len()
                            && !(r.events[i].node == e.node && r.events[i].kind == 15)
                        {
                            i += 1;
                        }
                        if i < r.events.len() && r.events[i].node == e.node {
                            r.pos = i as isize;
                        }
                    }
                }
            })
            .unwrap_or(false);
            Ok(Value::Bool(ok))
        }
        _ => node_op(host, &op, id, args),
    }
}

fn current(id: i64) -> Option<(DocId, Ev, Option<usize>)> {
    with_r(id, |r| {
        if r.pos < 0 || r.pos as usize >= r.events.len() {
            return None;
        }
        Some((r.doc?, r.events[r.pos as usize], r.attr))
    })
    .flatten()
}

fn node_op(host: &mut dyn Host, op: &str, id: i64, args: &[Value]) -> Result<Value, Thrown> {
    let cur = current(id);
    match op {
        "prop" => {
            let name = bytes(args, 2);
            let Some((d, ev, attr)) = cur else {
                return Ok(match &name[..] {
                    b"attributeCount" | b"depth" | b"nodeType" => Value::Int(0),
                    b"hasAttributes" | b"hasValue" | b"isDefault" | b"isEmptyElement" => {
                        Value::Bool(false)
                    }
                    _ => s(Vec::new()),
                });
            };
            let v = tree::with_doc(d, |doc| {
                let n = &doc.nodes[ev.node];
                let attrs = attr_list(doc, ev.node);
                if let Some(ai) = attr {
                    let (an, al, ap, ans, av) = attrs[ai].clone();
                    return match &name[..] {
                        b"attributeCount" => Value::Int(0),
                        b"depth" => Value::Int(ev.depth + 1),
                        b"nodeType" => Value::Int(2),
                        b"hasAttributes" | b"isDefault" | b"isEmptyElement" => Value::Bool(false),
                        b"hasValue" => Value::Bool(true),
                        b"name" => s(an),
                        b"localName" => s(al),
                        b"prefix" => s(ap),
                        b"namespaceURI" => s(ans),
                        b"value" => s(av),
                        b"baseURI" => s(doc.url.clone().unwrap_or_default()),
                        _ => s(Vec::new()),
                    };
                }
                let (name_s, local_s): (Vec<u8>, Vec<u8>) = match n.kind {
                    Kind::Element => (n.qname(), n.name.clone()),
                    Kind::Text => (b"#text".to_vec(), b"#text".to_vec()),
                    Kind::CData => (b"#cdata-section".to_vec(), b"#cdata-section".to_vec()),
                    Kind::Comment => (b"#comment".to_vec(), b"#comment".to_vec()),
                    _ => (n.name.clone(), n.name.clone()),
                };
                let has_value =
                    matches!(n.kind, Kind::Text | Kind::CData | Kind::Comment | Kind::Pi);
                match &name[..] {
                    b"attributeCount" => {
                        Value::Int(if ev.kind == 1 { attrs.len() as i64 } else { 0 })
                    }
                    b"depth" => Value::Int(ev.depth),
                    b"nodeType" => Value::Int(ev.kind),
                    b"hasAttributes" => Value::Bool(ev.kind == 1 && !attrs.is_empty()),
                    b"hasValue" => Value::Bool(has_value),
                    b"isDefault" => Value::Bool(false),
                    b"isEmptyElement" => {
                        Value::Bool(ev.kind == 1 && n.first.is_none() && n.empty_tag)
                    }
                    b"name" => s(name_s),
                    b"localName" => s(local_s),
                    b"prefix" => s(if n.kind == Kind::Element {
                        n.prefix.clone()
                    } else {
                        Vec::new()
                    }),
                    b"namespaceURI" => s(if n.kind == Kind::Element {
                        n.ns.clone().unwrap_or_default()
                    } else {
                        Vec::new()
                    }),
                    b"value" => s(if has_value {
                        n.content.clone()
                    } else {
                        Vec::new()
                    }),
                    b"baseURI" => s(doc.url.clone().unwrap_or_default()),
                    b"xmlLang" => {
                        let mut c = Some(ev.node);
                        let mut lang = Vec::new();
                        while let Some(x) = c {
                            if let Some(&a) = doc.nodes[x].attrs.iter().find(|&&a| {
                                doc.nodes[a].prefix == b"xml" && doc.nodes[a].name == b"lang"
                            }) {
                                lang = doc.nodes[a].content.clone();
                                break;
                            }
                            c = doc.nodes[x].parent;
                        }
                        s(lang)
                    }
                    _ => s(Vec::new()),
                }
            });
            Ok(v.unwrap_or(Value::Null))
        }
        "getAttribute"
        | "getAttributeNo"
        | "getAttributeNs"
        | "moveToAttribute"
        | "moveToAttributeNo"
        | "moveToAttributeNs"
        | "moveToFirstAttribute"
        | "moveToNextAttribute"
        | "moveToElement" => {
            let Some((d, ev, attr)) = cur else {
                return Ok(if op.starts_with("get") {
                    Value::Null
                } else {
                    Value::Bool(false)
                });
            };
            let attrs = tree::with_doc(d, |doc| attr_list(doc, ev.node)).unwrap_or_default();
            let find = |op: &str| -> Option<usize> {
                match op {
                    "getAttribute" | "moveToAttribute" => {
                        let nm = bytes(args, 2);
                        attrs.iter().position(|a| a.0 == nm)
                    }
                    "getAttributeNo" | "moveToAttributeNo" => {
                        let i = conv::to_int(&arg(args, 2));
                        (i >= 0 && (i as usize) < attrs.len()).then_some(i as usize)
                    }
                    _ => {
                        let local = bytes(args, 2);
                        let ns = bytes(args, 3);
                        attrs.iter().position(|a| a.1 == local && a.3 == ns)
                    }
                }
            };
            match op {
                "getAttribute" | "getAttributeNo" | "getAttributeNs" => {
                    if ev.kind != 1 {
                        return Ok(Value::Null);
                    }
                    Ok(match find(op) {
                        Some(i) => s(attrs[i].4.clone()),
                        None => Value::Null,
                    })
                }
                "moveToElement" => {
                    let had = attr.is_some();
                    with_r(id, |r| r.attr = None);
                    Ok(Value::Bool(had))
                }
                "moveToFirstAttribute" => {
                    if attrs.is_empty() || ev.kind != 1 {
                        return Ok(Value::Bool(false));
                    }
                    with_r(id, |r| r.attr = Some(0));
                    Ok(Value::Bool(true))
                }
                "moveToNextAttribute" => {
                    if ev.kind != 1 {
                        return Ok(Value::Bool(false));
                    }
                    let next = attr.map(|a| a + 1).unwrap_or(0);
                    if next >= attrs.len() {
                        return Ok(Value::Bool(false));
                    }
                    with_r(id, |r| r.attr = Some(next));
                    Ok(Value::Bool(true))
                }
                _ => {
                    if ev.kind != 1 {
                        return Ok(Value::Bool(false));
                    }
                    match find(op) {
                        Some(i) => {
                            with_r(id, |r| r.attr = Some(i));
                            Ok(Value::Bool(true))
                        }
                        None => Ok(Value::Bool(false)),
                    }
                }
            }
        }
        "readString" | "readInnerXml" | "readOuterXml" => {
            let Some((d, ev, attr)) = cur else {
                return Ok(s(Vec::new()));
            };
            let out = tree::with_doc(d, |doc| {
                if let Some(ai) = attr {
                    let attrs = attr_list(doc, ev.node);
                    let (an, _, _, _, av) = attrs[ai].clone();
                    return match op {
                        "readOuterXml" => {
                            let mut o = an;
                            o.extend_from_slice(b"=\"");
                            tree::escape_attr(&mut o, &av, true);
                            o.push(b'"');
                            o
                        }
                        _ => av,
                    };
                }
                if ev.kind == 15 {
                    return Vec::new();
                }
                let opts = SaveOpts {
                    format: false,
                    no_empty_tag: false,
                };
                match op {
                    "readString" => doc.text_content(ev.node),
                    "readOuterXml" => tree::dump_node(doc, ev.node, &opts),
                    _ => {
                        let mut o = Vec::new();
                        if doc.nodes[ev.node].kind == Kind::Element {
                            for c in doc.children(ev.node) {
                                o.extend(tree::dump_node(doc, c, &opts));
                            }
                        }
                        o
                    }
                }
            })
            .unwrap_or_default();
            Ok(s(out))
        }
        "expand" => {
            let Some((d, ev, _)) = cur else {
                crate::warn(
                    host,
                    "XMLReader::expand",
                    "An Error Occurred while expanding",
                )?;
                return Ok(Value::Bool(false));
            };
            Ok(dom::wrap(host, d, ev.node))
        }
        "lookupNamespace" => {
            let Some((d, ev, _)) = cur else {
                return Ok(Value::Null);
            };
            let p = bytes(args, 2);
            let r = tree::with_doc(d, |doc| doc.lookup_ns(ev.node, &p)).flatten();
            Ok(r.map(s).unwrap_or(Value::Null))
        }
        _ => Err(Thrown::Error {
            class: "Error",
            message: format!("__xr: unknown op {op}"),
        }),
    }
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    m.insert(
        "__xr",
        Builtin {
            name: "__xr",
            func: fn_xr,
            min_args: 1,
            max_args: None,
        },
    );
}

pub const PRELUDE_PHP: &str = r#"
class XMLReader {
    const NONE = 0; const ELEMENT = 1; const ATTRIBUTE = 2; const TEXT = 3; const CDATA = 4;
    const ENTITY_REF = 5; const ENTITY = 6; const PI = 7; const COMMENT = 8; const DOC = 9;
    const DOC_TYPE = 10; const DOC_FRAGMENT = 11; const NOTATION = 12; const WHITESPACE = 13;
    const SIGNIFICANT_WHITESPACE = 14; const END_ELEMENT = 15; const END_ENTITY = 16;
    const XML_DECLARATION = 17; const LOADDTD = 1; const DEFAULTATTRS = 2; const VALIDATE = 3;
    const SUBST_ENTITIES = 4;
    public $__r = null; public $__props = [];
    public function __get($name) {
        static $known = ['attributeCount', 'baseURI', 'depth', 'hasAttributes', 'hasValue', 'isDefault',
            'isEmptyElement', 'localName', 'name', 'namespaceURI', 'nodeType', 'prefix', 'value', 'xmlLang'];
        if (in_array($name, $known, true)) { return __xr('prop', $this->__r ?? 0, $name); }
        return $this->$name;
    }
    public function __set($name, $value) {
        static $known = ['attributeCount', 'baseURI', 'depth', 'hasAttributes', 'hasValue', 'isDefault',
            'isEmptyElement', 'localName', 'name', 'namespaceURI', 'nodeType', 'prefix', 'value', 'xmlLang'];
        if (in_array($name, $known, true)) { throw new Error('Cannot modify readonly property XMLReader::$' . $name); }
        $this->$name = $value;
    }
    public function __isset($name) { return $this->__get($name) !== null; }
    public function close(): true { if ($this->__r !== null) { __xr('close', $this->__r); $this->__r = null; } return true; }
    public function getAttribute(string $name): ?string { return __xr('getAttribute', $this->__r ?? 0, $name); }
    public function getAttributeNo(int $index): ?string { return __xr('getAttributeNo', $this->__r ?? 0, $index); }
    public function getAttributeNs(string $name, string $namespace): ?string { return __xr('getAttributeNs', $this->__r ?? 0, $name, $namespace); }
    public function getParserProperty(int $property): bool { return (bool) ($this->__props[$property] ?? false); }
    public function isValid(): bool { return $this->__r !== null; }
    public function lookupNamespace(string $prefix): ?string { return __xr('lookupNamespace', $this->__r ?? 0, $prefix); }
    public function moveToAttribute(string $name): bool { return __xr('moveToAttribute', $this->__r ?? 0, $name); }
    public function moveToAttributeNo(int $index): bool { return __xr('moveToAttributeNo', $this->__r ?? 0, $index); }
    public function moveToAttributeNs(string $name, string $namespace): bool { return __xr('moveToAttributeNs', $this->__r ?? 0, $name, $namespace); }
    public function moveToElement(): bool { return __xr('moveToElement', $this->__r ?? 0); }
    public function moveToFirstAttribute(): bool { return __xr('moveToFirstAttribute', $this->__r ?? 0); }
    public function moveToNextAttribute(): bool { return __xr('moveToNextAttribute', $this->__r ?? 0); }
    public function read(): bool {
        if ($this->__r === null) { throw new Error('Data must be loaded before reading'); }
        return __xr('read', $this->__r);
    }
    public function next(?string $name = null): bool {
        if ($this->__r === null) { throw new Error('Data must be loaded before reading'); }
        return __xr('next', $this->__r, $name);
    }
    public function readInnerXml(): string { return __xr('readInnerXml', $this->__r ?? 0); }
    public function readOuterXml(): string { return __xr('readOuterXml', $this->__r ?? 0); }
    public function readString(): string { return __xr('readString', $this->__r ?? 0); }
    public function setSchema(?string $filename): bool { return true; }
    public function setParserProperty(int $property, bool $value): true { $this->__props[$property] = $value; return true; }
    public function setRelaxNGSchema(?string $filename): bool { return true; }
    public function setRelaxNGSchemaSource(?string $source): bool { return true; }
    public function expand(?DOMNode $baseNode = null): DOMNode|false { return __xr('expand', $this->__r ?? 0); }
    // open() / XML() work both statically (returning a new reader) and on
    // an instance (loading into it): routed through the magic call hooks.
    private function __load($op, $args) {
        $src = (string) ($args[0] ?? '');
        $id = __xr($op, 0, $src, $args[1] ?? null, (int) ($args[2] ?? 0));
        if ($id === false) { return false; }
        if ($this->__r !== null) { __xr('close', $this->__r); }
        $this->__r = $id;
        return true;
    }
    public function __call($name, $args) {
        $l = strtolower($name);
        if ($l === 'open' || $l === 'xml') { return $this->__load($l === 'xml' ? 'xml' : 'open', $args); }
        throw new Error('Call to undefined method ' . static::class . '::' . $name . '()');
    }
    public static function __callStatic($name, $args) {
        $l = strtolower($name);
        if ($l === 'open' || $l === 'xml') {
            $r = new static();
            return $r->__load($l === 'xml' ? 'xml' : 'open', $args) ? $r : false;
        }
        throw new Error('Call to undefined method ' . static::class . '::' . $name . '()');
    }
    public static function fromString(string $source, ?string $encoding = null, int $flags = 0): static {
        $r = new static(); $r->__r = __xr('xml', 0, $source, $encoding, $flags); return $r;
    }
    public static function fromStream($stream, ?string $encoding = null, int $flags = 0, ?string $documentUri = null): static {
        if (!is_resource($stream)) { throw new TypeError('XMLReader::fromStream(): Argument #1 ($stream) must be of type resource, ' . get_debug_type($stream) . ' given'); }
        $r = new static(); $r->__r = __xr('stream', 0, $stream, $encoding, $flags, $documentUri); return $r;
    }
    public static function fromUri(string $uri, ?string $encoding = null, int $flags = 0): static {
        $r = new static(); $id = __xr('open', 0, $uri, $encoding, $flags);
        if ($id === false) { throw new Error('Unable to open source data'); }
        $r->__r = $id; return $r;
    }
}
"#;
