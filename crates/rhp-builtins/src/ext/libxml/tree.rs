//! The in-memory XML tree shared by `dom`, `simplexml`, `xmlreader` and the
//! XSD / XPath engines: a libxml2-shaped node arena per document (doubly
//! linked siblings, attribute lists, namespace declarations), kept in a
//! thread-local registry keyed by document id, plus libxml2's serializer
//! (`xmlDocDumpFormatMemoryEnc` / `xmlNodeDump`) byte for byte.

use std::cell::RefCell;

pub type NodeId = usize;
pub type DocId = usize;

/// libxml2's `xmlElementType` numbering (also DOM's `nodeType`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Element = 1,
    Attr = 2,
    Text = 3,
    CData = 4,
    EntityRef = 5,
    Pi = 7,
    Comment = 8,
    Document = 9,
    DocType = 10,
    Fragment = 11,
}

#[derive(Clone, Debug, Default)]
pub struct DocTypeInfo {
    pub public_id: Option<Vec<u8>>,
    pub system_id: Option<Vec<u8>>,
    /// The internal subset re-serialized libxml2-style, one declaration per
    /// entry (`<!ENTITY e "v">`), or None when the doctype has no `[...]`.
    pub subset: Option<Vec<Vec<u8>>>,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: Kind,
    /// Local name (element / attribute), PI target, entity or doctype name.
    pub name: Vec<u8>,
    /// Namespace prefix (empty: none).
    pub prefix: Vec<u8>,
    /// Namespace URI.
    pub ns: Option<Vec<u8>>,
    /// Character data: text / CDATA / comment / PI data / attribute value.
    pub content: Vec<u8>,
    pub parent: Option<NodeId>,
    pub first: Option<NodeId>,
    pub last: Option<NodeId>,
    pub next: Option<NodeId>,
    pub prev: Option<NodeId>,
    /// Attribute nodes of an element, in order.
    pub attrs: Vec<NodeId>,
    /// Namespace declarations on an element: (prefix or empty, uri).
    pub ns_defs: Vec<(Vec<u8>, Vec<u8>)>,
    pub line: u32,
    pub doctype: Option<Box<DocTypeInfo>>,
    /// Parsed from an empty-element tag (`<a/>`; libxml2's NODE_IS_EMPTY).
    pub empty_tag: bool,
}

impl Node {
    pub fn new(kind: Kind) -> Node {
        Node {
            kind,
            name: Vec::new(),
            prefix: Vec::new(),
            ns: None,
            content: Vec::new(),
            parent: None,
            first: None,
            last: None,
            next: None,
            prev: None,
            attrs: Vec::new(),
            ns_defs: Vec::new(),
            line: 0,
            doctype: None,
            empty_tag: false,
        }
    }

    /// The qualified name (`prefix:local`).
    pub fn qname(&self) -> Vec<u8> {
        if self.prefix.is_empty() {
            self.name.clone()
        } else {
            let mut v = self.prefix.clone();
            v.push(b':');
            v.extend_from_slice(&self.name);
            v
        }
    }
}

#[derive(Clone, Debug)]
pub struct Doc {
    pub nodes: Vec<Node>,
    pub version: Option<Vec<u8>>,
    pub encoding: Option<Vec<u8>>,
    /// -1: not declared, 0: no, 1: yes.
    pub standalone: i8,
    pub url: Option<Vec<u8>>,
    /// Internal general entities declared in the DTD: (name, value).
    pub entities: Vec<(Vec<u8>, Vec<u8>)>,
    // DOMDocument settings.
    pub format_output: bool,
    pub preserve_ws: bool,
    pub validate_on_parse: bool,
    pub resolve_externals: bool,
    pub substitute_entities: bool,
    pub strict_error_checking: bool,
    pub recover: bool,
    /// A hidden document owning detached nodes created without a document
    /// (`new DOMElement(...)`).
    pub hidden: bool,
}

impl Doc {
    pub fn new() -> Doc {
        Doc {
            nodes: vec![Node::new(Kind::Document)],
            version: Some(b"1.0".to_vec()),
            encoding: None,
            standalone: -1,
            url: None,
            entities: Vec::new(),
            format_output: false,
            preserve_ws: true,
            validate_on_parse: false,
            resolve_externals: false,
            substitute_entities: false,
            strict_error_checking: true,
            recover: false,
            hidden: false,
        }
    }

    pub fn add(&mut self, n: Node) -> NodeId {
        self.nodes.push(n);
        self.nodes.len() - 1
    }

    pub fn new_element(&mut self, qname: &[u8]) -> NodeId {
        let mut n = Node::new(Kind::Element);
        n.name = qname.to_vec();
        self.add(n)
    }

    pub fn new_text(&mut self, kind: Kind, content: &[u8]) -> NodeId {
        let mut n = Node::new(kind);
        n.content = content.to_vec();
        self.add(n)
    }

    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        let mut v = Vec::new();
        let mut c = self.nodes[id].first;
        while let Some(x) = c {
            v.push(x);
            c = self.nodes[x].next;
        }
        v
    }

    /// Detach `id` from its parent (attributes from their element).
    pub fn unlink(&mut self, id: NodeId) {
        let Some(p) = self.nodes[id].parent else {
            return;
        };
        if self.nodes[id].kind == Kind::Attr {
            self.nodes[p].attrs.retain(|&a| a != id);
            self.nodes[id].parent = None;
            return;
        }
        let (prev, next) = (self.nodes[id].prev, self.nodes[id].next);
        match prev {
            Some(pr) => self.nodes[pr].next = next,
            None => self.nodes[p].first = next,
        }
        match next {
            Some(nx) => self.nodes[nx].prev = prev,
            None => self.nodes[p].last = prev,
        }
        let n = &mut self.nodes[id];
        n.parent = None;
        n.prev = None;
        n.next = None;
    }

    /// Insert `id` (detached) into `parent` before `before` (None: append).
    pub fn insert(&mut self, parent: NodeId, id: NodeId, before: Option<NodeId>) {
        self.unlink(id);
        if self.nodes[id].kind == Kind::Attr {
            self.nodes[parent].attrs.push(id);
            self.nodes[id].parent = Some(parent);
            return;
        }
        self.nodes[id].parent = Some(parent);
        match before {
            None => {
                let last = self.nodes[parent].last;
                self.nodes[id].prev = last;
                self.nodes[id].next = None;
                match last {
                    Some(l) => self.nodes[l].next = Some(id),
                    None => self.nodes[parent].first = Some(id),
                }
                self.nodes[parent].last = Some(id);
            }
            Some(b) => {
                let prev = self.nodes[b].prev;
                self.nodes[id].prev = prev;
                self.nodes[id].next = Some(b);
                self.nodes[b].prev = Some(id);
                match prev {
                    Some(p) => self.nodes[p].next = Some(id),
                    None => self.nodes[parent].first = Some(id),
                }
            }
        }
    }

    pub fn append(&mut self, parent: NodeId, id: NodeId) {
        self.insert(parent, id, None);
    }

    pub fn remove_children(&mut self, id: NodeId) {
        for c in self.children(id) {
            self.unlink(c);
        }
    }

    /// The document element.
    pub fn root(&self) -> Option<NodeId> {
        let mut c = self.nodes[0].first;
        while let Some(x) = c {
            if self.nodes[x].kind == Kind::Element {
                return Some(x);
            }
            c = self.nodes[x].next;
        }
        None
    }

    pub fn doctype(&self) -> Option<NodeId> {
        let mut c = self.nodes[0].first;
        while let Some(x) = c {
            if self.nodes[x].kind == Kind::DocType {
                return Some(x);
            }
            c = self.nodes[x].next;
        }
        None
    }

    /// Whether `id` is (transitively) attached to the document node.
    pub fn connected(&self, mut id: NodeId) -> bool {
        loop {
            if id == 0 {
                return true;
            }
            match self.nodes[id].parent {
                Some(p) => id = p,
                None => return false,
            }
        }
    }

    pub fn is_ancestor(&self, anc: NodeId, mut id: NodeId) -> bool {
        loop {
            if id == anc {
                return true;
            }
            match self.nodes[id].parent {
                Some(p) => id = p,
                None => return false,
            }
        }
    }

    pub fn entity_value(&self, name: &[u8]) -> Option<&[u8]> {
        match name {
            b"lt" => Some(b"<"),
            b"gt" => Some(b">"),
            b"amp" => Some(b"&"),
            b"quot" => Some(b"\""),
            b"apos" => Some(b"'"),
            _ => self
                .entities
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_slice()),
        }
    }

    /// `xmlNodeGetContent`: the text content of a node.
    pub fn text_content(&self, id: NodeId) -> Vec<u8> {
        let n = &self.nodes[id];
        match n.kind {
            Kind::Text | Kind::CData | Kind::Comment | Kind::Pi | Kind::Attr => n.content.clone(),
            Kind::EntityRef => self.entity_value(&n.name).unwrap_or(b"").to_vec(),
            Kind::DocType => Vec::new(),
            _ => {
                let mut out = Vec::new();
                self.collect_text(id, &mut out);
                out
            }
        }
    }

    fn collect_text(&self, id: NodeId, out: &mut Vec<u8>) {
        let mut c = self.nodes[id].first;
        while let Some(x) = c {
            let n = &self.nodes[x];
            match n.kind {
                Kind::Text | Kind::CData => out.extend_from_slice(&n.content),
                Kind::EntityRef => out.extend_from_slice(self.entity_value(&n.name).unwrap_or(b"")),
                Kind::Element | Kind::Fragment => self.collect_text(x, out),
                _ => {}
            }
            c = n.next;
        }
    }

    /// Resolve a namespace prefix (empty: default namespace) in scope at `id`.
    pub fn lookup_ns(&self, mut id: NodeId, prefix: &[u8]) -> Option<Vec<u8>> {
        if prefix == b"xml" {
            return Some(XML_NS.to_vec());
        }
        loop {
            let n = &self.nodes[id];
            if n.kind == Kind::Element {
                for (p, u) in &n.ns_defs {
                    if p == prefix {
                        return Some(u.clone());
                    }
                }
            }
            id = n.parent?;
        }
    }

    /// A prefix bound to `uri` in scope at `id`.
    pub fn lookup_prefix(&self, mut id: NodeId, uri: &[u8]) -> Option<Vec<u8>> {
        if uri == XML_NS {
            return Some(b"xml".to_vec());
        }
        loop {
            let n = &self.nodes[id];
            if n.kind == Kind::Element {
                for (p, u) in &n.ns_defs {
                    if u == uri {
                        return Some(p.clone());
                    }
                }
            }
            id = n.parent?;
        }
    }

    pub fn find_attr_ns(&self, el: NodeId, ns: Option<&[u8]>, local: &[u8]) -> Option<NodeId> {
        self.nodes[el].attrs.iter().copied().find(|&a| {
            let n = &self.nodes[a];
            n.name == local && n.ns.as_deref() == ns
        })
    }

    /// Deep (or shallow) copy of `src` from `from` into this document,
    /// returning the new detached node.
    pub fn import(&mut self, from: &Doc, src: NodeId, deep: bool) -> NodeId {
        let s = &from.nodes[src];
        let mut n = Node::new(s.kind);
        n.name = s.name.clone();
        n.prefix = s.prefix.clone();
        n.ns = s.ns.clone();
        n.content = s.content.clone();
        n.ns_defs = s.ns_defs.clone();
        n.line = s.line;
        n.doctype = s.doctype.clone();
        n.empty_tag = s.empty_tag;
        let id = self.add(n);
        if s.kind == Kind::Element {
            for &a in &from.nodes[src].attrs {
                let na = self.import(from, a, true);
                self.nodes[na].parent = Some(id);
                self.nodes[id].attrs.push(na);
            }
        }
        if s.kind == Kind::EntityRef {
            if let Some(v) = from.entity_value(&s.name) {
                if !self.entities.iter().any(|(n, _)| *n == s.name)
                    && from.entities.iter().any(|(n, _)| *n == s.name)
                {
                    self.entities.push((s.name.clone(), v.to_vec()));
                }
            }
        }
        if deep {
            let mut c = from.nodes[src].first;
            while let Some(x) = c {
                let nc = self.import(from, x, true);
                self.append(id, nc);
                c = from.nodes[x].next;
            }
        }
        id
    }

    /// Copy within the same document.
    pub fn clone_node(&mut self, src: NodeId, deep: bool) -> NodeId {
        let s = &self.nodes[src];
        let mut n = Node::new(s.kind);
        n.name = s.name.clone();
        n.prefix = s.prefix.clone();
        n.ns = s.ns.clone();
        n.content = s.content.clone();
        n.ns_defs = s.ns_defs.clone();
        n.line = s.line;
        n.doctype = s.doctype.clone();
        n.empty_tag = s.empty_tag;
        let attrs = s.attrs.clone();
        let id = self.add(n);
        for a in attrs {
            let na = self.clone_node(a, false);
            self.nodes[na].parent = Some(id);
            self.nodes[id].attrs.push(na);
        }
        if deep {
            for c in self.children(src) {
                let nc = self.clone_node(c, true);
                self.append(id, nc);
            }
        }
        id
    }

    /// Nodes of the subtree under `id` in document order (excluding `id`).
    pub fn descendants(&self, id: NodeId, out: &mut Vec<NodeId>) {
        let mut c = self.nodes[id].first;
        while let Some(x) = c {
            out.push(x);
            self.descendants(x, out);
            c = self.nodes[x].next;
        }
    }
}

impl Default for Doc {
    fn default() -> Self {
        Doc::new()
    }
}

pub const XML_NS: &[u8] = b"http://www.w3.org/XML/1998/namespace";
pub const XMLNS_NS: &[u8] = b"http://www.w3.org/2000/xmlns/";

thread_local! {
    static DOCS: RefCell<Vec<Option<Doc>>> = const { RefCell::new(Vec::new()) };
}

/// Register a document, returning its id.
pub fn register(doc: Doc) -> DocId {
    DOCS.with(|d| {
        let mut d = d.borrow_mut();
        d.push(Some(doc));
        d.len() - 1
    })
}

pub fn with_doc<R>(id: DocId, f: impl FnOnce(&mut Doc) -> R) -> Option<R> {
    DOCS.with(|d| {
        let mut d = d.borrow_mut();
        d.get_mut(id).and_then(|x| x.as_mut()).map(f)
    })
}

/// Run `f` with two distinct documents borrowed (`a` mutably).
pub fn with_two<R>(a: DocId, b: DocId, f: impl FnOnce(&mut Doc, &Doc) -> R) -> Option<R> {
    if a == b {
        let snap = with_doc(b, |d| d.clone())?;
        return with_doc(a, |d| f(d, &snap));
    }
    // (a != b below)
    DOCS.with(|d| {
        let mut d = d.borrow_mut();
        let (lo, hi) = (a.min(b), a.max(b));
        if hi >= d.len() {
            return None;
        }
        let (l, r) = d.split_at_mut(hi);
        let (x, y) = (l[lo].as_mut()?, r[0].as_mut()?);
        Some(if a < b { f(x, y) } else { f(y, x) })
    })
}

pub fn clone_doc(id: DocId) -> Option<Doc> {
    with_doc(id, |d| d.clone())
}

// ---------------------------------------------------------------- serializer

/// How character data is escaped (libxml2's `ctxt->escape`).
#[derive(Clone, Copy, PartialEq)]
enum Esc {
    /// `xmlEscapeEntities`: document saved without an encoding — non-ASCII
    /// and control characters become hex character references.
    Entities,
    /// `xmlEscapeContent`: `<`, `>`, `&`, `\r` only.
    Content,
}

pub struct SaveOpts {
    pub format: bool,
    pub no_empty_tag: bool,
}

struct Ser<'a> {
    doc: &'a Doc,
    out: Vec<u8>,
    esc: Esc,
    opts: &'a SaveOpts,
}

fn utf8_char(s: &[u8]) -> Option<(u32, usize)> {
    let b = s[0];
    let (len, init) = match b {
        0xC0..=0xDF => (2, (b & 0x1F) as u32),
        0xE0..=0xEF => (3, (b & 0x0F) as u32),
        0xF0..=0xF7 => (4, (b & 0x07) as u32),
        _ => return None,
    };
    if s.len() < len {
        return None;
    }
    let mut c = init;
    for &x in &s[1..len] {
        if x & 0xC0 != 0x80 {
            return None;
        }
        c = (c << 6) | (x & 0x3F) as u32;
    }
    Some((c, len))
}

fn escape_text(out: &mut Vec<u8>, s: &[u8], esc: Esc) {
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        match c {
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            b'\r' if esc == Esc::Content => out.extend_from_slice(b"&#13;"),
            _ if esc == Esc::Content => out.push(c),
            0x20..=0x7F | b'\n' | b'\t' => out.push(c),
            0x80.. => match utf8_char(&s[i..]) {
                Some((cp, len)) => {
                    out.extend_from_slice(format!("&#x{cp:X};").as_bytes());
                    i += len;
                    continue;
                }
                None => {
                    out.extend_from_slice(format!("&#x{c:X};").as_bytes());
                }
            },
            _ => out.extend_from_slice(format!("&#x{c:X};").as_bytes()),
        }
        i += 1;
    }
}

/// `xmlBufAttrSerializeTxtContent`.
pub fn escape_attr(out: &mut Vec<u8>, s: &[u8], has_encoding: bool) {
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        match c {
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'\n' => out.extend_from_slice(b"&#10;"),
            b'\r' => out.extend_from_slice(b"&#13;"),
            b'\t' => out.extend_from_slice(b"&#9;"),
            0x80.. if !has_encoding => match utf8_char(&s[i..]) {
                Some((cp, len)) => {
                    out.extend_from_slice(format!("&#x{cp:X};").as_bytes());
                    i += len;
                    continue;
                }
                None => out.extend_from_slice(format!("&#{c};").as_bytes()),
            },
            _ => out.push(c),
        }
        i += 1;
    }
}

const MAX_INDENT_LEVELS: usize = 30;

impl Ser<'_> {
    fn indent(&mut self, level: usize) {
        for _ in 0..level.min(MAX_INDENT_LEVELS) {
            self.out.extend_from_slice(b"  ");
        }
    }

    fn node(&mut self, id: NodeId, level: usize, format: bool) {
        let doc = self.doc;
        let n = &doc.nodes[id];
        match n.kind {
            Kind::Element => {
                self.out.push(b'<');
                self.out.extend_from_slice(&n.qname());
                for (p, u) in &n.ns_defs {
                    self.out.extend_from_slice(b" xmlns");
                    if !p.is_empty() {
                        self.out.push(b':');
                        self.out.extend_from_slice(p);
                    }
                    self.out.extend_from_slice(b"=\"");
                    escape_attr(&mut self.out, u, doc.encoding.is_some());
                    self.out.push(b'"');
                }
                for &a in &n.attrs {
                    self.attr(a);
                }
                if n.first.is_none() && !self.opts.no_empty_tag {
                    self.out.extend_from_slice(b"/>");
                    return;
                }
                self.out.push(b'>');
                let mut cf = format;
                if cf {
                    let mut c = n.first;
                    while let Some(x) = c {
                        if matches!(
                            doc.nodes[x].kind,
                            Kind::Text | Kind::CData | Kind::EntityRef
                        ) {
                            cf = false;
                            break;
                        }
                        c = doc.nodes[x].next;
                    }
                }
                if cf {
                    self.out.push(b'\n');
                }
                self.list(n.first, level + 1, cf);
                if cf {
                    self.indent(level);
                }
                self.out.extend_from_slice(b"</");
                self.out.extend_from_slice(&n.qname());
                self.out.push(b'>');
            }
            Kind::Text => escape_text(&mut self.out, &n.content, self.esc),
            Kind::CData => {
                // A CDATA section can't contain "]]>": libxml2 splits it.
                let s = &n.content;
                if s.is_empty() {
                    self.out.extend_from_slice(b"<![CDATA[]]>");
                } else {
                    let mut start = 0;
                    let mut i = 0;
                    while i + 3 <= s.len() {
                        if &s[i..i + 3] == b"]]>" {
                            self.out.extend_from_slice(b"<![CDATA[");
                            self.out.extend_from_slice(&s[start..i + 2]);
                            self.out.extend_from_slice(b"]]>");
                            start = i + 2;
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    if start < s.len() {
                        self.out.extend_from_slice(b"<![CDATA[");
                        self.out.extend_from_slice(&s[start..]);
                        self.out.extend_from_slice(b"]]>");
                    }
                }
            }
            Kind::Comment => {
                self.out.extend_from_slice(b"<!--");
                self.out.extend_from_slice(&n.content);
                self.out.extend_from_slice(b"-->");
            }
            Kind::Pi => {
                self.out.extend_from_slice(b"<?");
                self.out.extend_from_slice(&n.name);
                if !n.content.is_empty() {
                    self.out.push(b' ');
                    self.out.extend_from_slice(&n.content);
                }
                self.out.extend_from_slice(b"?>");
            }
            Kind::EntityRef => {
                self.out.push(b'&');
                self.out.extend_from_slice(&n.name);
                self.out.push(b';');
            }
            Kind::Attr => self.attr(id),
            Kind::DocType => self.doctype(id),
            Kind::Document | Kind::Fragment => {
                self.list(n.first, level, format);
            }
        }
    }

    fn list(&mut self, first: Option<NodeId>, level: usize, format: bool) {
        let mut c = first;
        while let Some(x) = c {
            let k = self.doc.nodes[x].kind;
            if format && matches!(k, Kind::Element | Kind::Comment | Kind::Pi) {
                self.indent(level);
            }
            self.node(x, level, format);
            if format {
                self.out.push(b'\n');
            }
            c = self.doc.nodes[x].next;
        }
    }

    fn attr(&mut self, a: NodeId) {
        let an = &self.doc.nodes[a];
        self.out.push(b' ');
        self.out.extend_from_slice(&an.qname());
        self.out.extend_from_slice(b"=\"");
        escape_attr(&mut self.out, &an.content, self.doc.encoding.is_some());
        self.out.push(b'"');
    }

    fn doctype(&mut self, id: NodeId) {
        let n = &self.doc.nodes[id];
        self.out.extend_from_slice(b"<!DOCTYPE ");
        self.out.extend_from_slice(&n.name);
        let info = n.doctype.as_deref().cloned().unwrap_or_default();
        if let Some(p) = &info.public_id {
            self.out.extend_from_slice(b" PUBLIC ");
            quote(&mut self.out, p);
            if let Some(s) = &info.system_id {
                self.out.push(b' ');
                quote(&mut self.out, s);
            }
        } else if let Some(s) = &info.system_id {
            self.out.extend_from_slice(b" SYSTEM ");
            quote(&mut self.out, s);
        }
        if let Some(sub) = &info.subset {
            self.out.extend_from_slice(b" [\n");
            for d in sub {
                self.out.extend_from_slice(d);
                self.out.push(b'\n');
            }
            self.out.push(b']');
        }
        self.out.push(b'>');
    }
}

/// libxml2's `xmlBufWriteQuotedString`.
pub fn quote(out: &mut Vec<u8>, s: &[u8]) {
    if s.contains(&b'"') {
        if s.contains(&b'\'') {
            out.push(b'"');
            for &c in s {
                if c == b'"' {
                    out.extend_from_slice(b"&quot;");
                } else {
                    out.push(c);
                }
            }
            out.push(b'"');
        } else {
            out.push(b'\'');
            out.extend_from_slice(s);
            out.push(b'\'');
        }
    } else {
        out.push(b'"');
        out.extend_from_slice(s);
        out.push(b'"');
    }
}

/// Transcode UTF-8 output into the document's declared encoding (libxml2
/// writes unrepresentable characters as decimal character references).
fn encode_output(s: Vec<u8>, enc: &[u8]) -> Vec<u8> {
    let e = enc.to_ascii_uppercase();
    let limit: u32 = match &e[..] {
        b"ISO-8859-1" | b"LATIN1" | b"ISO_8859-1" | b"ISO-LATIN-1" => 0xFF,
        b"US-ASCII" | b"ASCII" => 0x7F,
        _ => return s,
    };
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] < 0x80 {
            out.push(s[i]);
            i += 1;
            continue;
        }
        match utf8_char(&s[i..]) {
            Some((cp, len)) => {
                if cp <= limit {
                    out.push(cp as u8);
                } else {
                    out.extend_from_slice(format!("&#{cp};").as_bytes());
                }
                i += len;
            }
            None => {
                out.push(s[i]);
                i += 1;
            }
        }
    }
    out
}

/// `xmlDocDumpFormatMemoryEnc`: the whole document with its XML declaration.
pub fn dump_doc(doc: &Doc, opts: &SaveOpts) -> Vec<u8> {
    let esc = if doc.encoding.is_some() {
        Esc::Content
    } else {
        Esc::Entities
    };
    let mut s = Ser {
        doc,
        out: Vec::new(),
        esc,
        opts,
    };
    s.out.extend_from_slice(b"<?xml version=");
    let v = doc.version.clone().unwrap_or_else(|| b"1.0".to_vec());
    quote(&mut s.out, &v);
    if let Some(e) = &doc.encoding {
        s.out.extend_from_slice(b" encoding=");
        quote(&mut s.out, e);
    }
    match doc.standalone {
        0 => s.out.extend_from_slice(b" standalone=\"no\""),
        1 => s.out.extend_from_slice(b" standalone=\"yes\""),
        _ => {}
    }
    s.out.extend_from_slice(b"?>\n");
    let mut c = doc.nodes[0].first;
    while let Some(x) = c {
        s.node(x, 0, opts.format);
        s.out.push(b'\n');
        c = doc.nodes[x].next;
    }
    match &doc.encoding {
        Some(e) => encode_output(s.out, e),
        None => s.out,
    }
}

/// `xmlNodeDump`: one node (no declaration), UTF-8, content escaping.
pub fn dump_node(doc: &Doc, id: NodeId, opts: &SaveOpts) -> Vec<u8> {
    let mut s = Ser {
        doc,
        out: Vec::new(),
        esc: Esc::Content,
        opts,
    };
    if doc.nodes[id].kind == Kind::Document {
        return dump_doc(doc, opts);
    }
    s.node(id, 0, opts.format);
    s.out
}
