//! The `xmlwriter` extension: a port of libxml2's `xmlTextWriter` state
//! machine (indentation, escaping and namespace-declaration output rules),
//! with php's `XMLWriter` class (PHP-source, [`PRELUDE_PHP`]) and the
//! procedural `xmlwriter_*` functions over it.

use std::cell::RefCell;
use std::collections::HashMap;

use rhp_values::{conv, Value};

use crate::ext::libxml::tree::escape_attr;
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

#[derive(Clone, Copy, PartialEq, Debug)]
enum St {
    Name,
    Attribute,
    Text,
    Pi,
    PiText,
    Cdata,
    Comment,
    Dtd,
    DtdText,
    DtdElem,
    DtdElemText,
    DtdAttl,
    DtdAttlText,
    DtdEnty,
    DtdEntyText,
}

struct Entry {
    name: Vec<u8>,
    state: St,
}

enum Target {
    Memory,
    Uri(Vec<u8>),
    Stream(Value),
}

struct Writer {
    out: Vec<u8>,
    target: Target,
    nodes: Vec<Entry>, // innermost last
    /// Pending namespace declarations of the open start tag: (prefix, uri).
    nsstack: Vec<(Vec<u8>, Vec<u8>)>,
    indent: bool,
    ichar: Vec<u8>,
    doindent: bool,
    qchar: u8,
    started_doc: bool,
    /// libxml2 sets writer->doc->encoding from startDocument; without one,
    /// attribute values escape non-ASCII as character references.
    doc_encoding: bool,
    /// The output encoder startDocument installed (php hands the encoding
    /// to xmlFindCharEncodingHandler): applied when output leaves the buffer.
    out_enc: Option<Vec<u8>>,
}

impl Writer {
    fn new(target: Target) -> Writer {
        Writer {
            out: Vec::new(),
            target,
            nodes: Vec::new(),
            nsstack: Vec::new(),
            indent: false,
            ichar: b" ".to_vec(),
            doindent: true,
            qchar: b'"',
            started_doc: false,
            doc_encoding: false,
            out_enc: None,
        }
    }

    /// Take (or copy) the buffered output through the output encoder.
    fn take_out(&mut self, take: bool) -> Vec<u8> {
        let data = if take {
            std::mem::take(&mut self.out)
        } else {
            self.out.clone()
        };
        match &self.out_enc {
            Some(e) => crate::ext::libxml::tree::encode_output(data, e),
            None => data,
        }
    }

    fn top(&self) -> Option<St> {
        self.nodes.last().map(|e| e.state)
    }

    fn set_top(&mut self, s: St) {
        if let Some(e) = self.nodes.last_mut() {
            e.state = s;
        }
    }

    fn write_indent(&mut self) {
        let n = self.nodes.len();
        if n >= 1 {
            for _ in 0..n - 1 {
                let ic = self.ichar.clone();
                self.out.extend_from_slice(&ic);
            }
        }
    }

    fn output_ns_decl(&mut self) {
        let decls = std::mem::take(&mut self.nsstack);
        // libxml2 keeps them in a push-front list
        for (p, u) in decls.into_iter().rev() {
            self.out.push(b' ');
            self.out.extend_from_slice(b"xmlns");
            if !p.is_empty() {
                self.out.push(b':');
                self.out.extend_from_slice(&p);
            }
            self.out.push(b'=');
            self.out.push(self.qchar);
            escape_attr(&mut self.out, &u, self.doc_encoding);
            self.out.push(self.qchar);
        }
    }

    /// xmlTextWriterHandleStateDependencies.
    fn handle_state(&mut self) {
        match self.top() {
            Some(St::Name) => {
                self.output_ns_decl();
                self.out.push(b'>');
                self.set_top(St::Text);
            }
            Some(St::Pi) => {
                self.out.push(b' ');
                self.set_top(St::PiText);
            }
            Some(St::Dtd) => {
                self.out.extend_from_slice(b" [");
                self.set_top(St::DtdText);
            }
            Some(St::DtdElem) => {
                self.out.push(b' ');
                self.set_top(St::DtdElemText);
            }
            Some(St::DtdAttl) => {
                self.out.push(b' ');
                self.set_top(St::DtdAttlText);
            }
            Some(St::DtdEnty) => {
                self.out.push(b' ');
                self.out.push(self.qchar);
                self.set_top(St::DtdEntyText);
            }
            _ => {}
        }
    }

    fn start_document(
        &mut self,
        version: Option<Vec<u8>>,
        encoding: Option<Vec<u8>>,
        standalone: Option<Vec<u8>>,
    ) -> bool {
        if !self.nodes.is_empty() || self.started_doc {
            return false;
        }
        self.started_doc = true;
        self.out.extend_from_slice(b"<?xml version=");
        self.out.push(self.qchar);
        self.out
            .extend_from_slice(&version.unwrap_or_else(|| b"1.0".to_vec()));
        self.out.push(self.qchar);
        if let Some(e) = encoding.filter(|e| !e.is_empty()) {
            self.doc_encoding = true;
            self.out_enc = Some(e.clone());
            self.out.extend_from_slice(b" encoding=");
            self.out.push(self.qchar);
            self.out.extend_from_slice(&e);
            self.out.push(self.qchar);
        }
        if let Some(s) = standalone.filter(|e| !e.is_empty()) {
            self.out.extend_from_slice(b" standalone=");
            self.out.push(self.qchar);
            self.out.extend_from_slice(&s);
            self.out.push(self.qchar);
        }
        self.out.extend_from_slice(b"?>\n");
        true
    }

    fn end_document(&mut self) -> bool {
        while let Some(st) = self.top() {
            let ok = match st {
                St::Name | St::Attribute | St::Text => self.end_element(),
                St::Pi | St::PiText => self.end_pi(),
                St::Cdata => self.end_cdata(),
                St::Comment => self.end_comment(),
                St::Dtd
                | St::DtdText
                | St::DtdElem
                | St::DtdElemText
                | St::DtdAttl
                | St::DtdAttlText
                | St::DtdEnty
                | St::DtdEntyText => self.end_dtd(),
            };
            if !ok {
                return false;
            }
        }
        if !self.indent {
            self.out.push(b'\n');
        }
        true
    }

    fn close_open_tag_for_child(&mut self) -> bool {
        match self.top() {
            Some(St::Pi) | Some(St::PiText) => return false,
            Some(St::Attribute) => {
                self.end_attribute();
                self.output_ns_decl();
                self.out.push(b'>');
                if self.indent {
                    self.out.push(b'\n');
                }
                self.set_top(St::Text);
            }
            Some(St::Name) => {
                self.output_ns_decl();
                self.out.push(b'>');
                if self.indent {
                    self.out.push(b'\n');
                }
                self.set_top(St::Text);
            }
            _ => {}
        }
        true
    }

    fn start_element(&mut self, name: &[u8]) -> bool {
        if name.is_empty() {
            return false;
        }
        if !self.close_open_tag_for_child() {
            return false;
        }
        self.nodes.push(Entry {
            name: name.to_vec(),
            state: St::Name,
        });
        if self.indent {
            self.write_indent();
        }
        self.out.push(b'<');
        self.out.extend_from_slice(name);
        true
    }

    fn start_element_ns(
        &mut self,
        prefix: Option<Vec<u8>>,
        name: &[u8],
        uri: Option<Vec<u8>>,
    ) -> bool {
        let q = match &prefix {
            Some(p) if !p.is_empty() => [&p[..], b":", name].concat(),
            _ => name.to_vec(),
        };
        if !self.start_element(&q) {
            return false;
        }
        if let Some(u) = uri {
            self.nsstack.push((prefix.unwrap_or_default(), u));
        }
        true
    }

    fn end_element(&mut self) -> bool {
        let Some(st) = self.top() else {
            return false;
        };
        match st {
            St::Attribute | St::Name => {
                if st == St::Attribute {
                    self.end_attribute();
                }
                self.output_ns_decl();
                if self.indent {
                    self.doindent = true;
                }
                self.out.extend_from_slice(b"/>");
            }
            St::Text => {
                if self.indent && self.doindent {
                    self.write_indent();
                }
                self.doindent = true;
                let name = self.nodes.last().unwrap().name.clone();
                self.out.extend_from_slice(b"</");
                self.out.extend_from_slice(&name);
                self.out.push(b'>');
            }
            _ => return false,
        }
        if self.indent {
            self.out.push(b'\n');
        }
        self.nodes.pop();
        true
    }

    fn full_end_element(&mut self) -> bool {
        let Some(st) = self.top() else {
            return false;
        };
        match st {
            St::Attribute | St::Name | St::Text => {
                if st == St::Attribute {
                    self.end_attribute();
                }
                if st != St::Text {
                    self.output_ns_decl();
                    self.out.push(b'>');
                    if self.indent {
                        self.doindent = false;
                    }
                }
                if self.indent && self.doindent {
                    self.write_indent();
                }
                self.doindent = true;
                let name = self.nodes.last().unwrap().name.clone();
                self.out.extend_from_slice(b"</");
                self.out.extend_from_slice(&name);
                self.out.push(b'>');
            }
            _ => return false,
        }
        if self.indent {
            self.out.push(b'\n');
        }
        self.nodes.pop();
        true
    }

    fn start_attribute(&mut self, name: &[u8]) -> bool {
        if name.is_empty() {
            return false;
        }
        match self.top() {
            Some(St::Attribute) => {
                self.end_attribute();
            }
            Some(St::Name) => {}
            _ => return false,
        }
        self.out.push(b' ');
        self.out.extend_from_slice(name);
        self.out.push(b'=');
        self.out.push(self.qchar);
        self.set_top(St::Attribute);
        true
    }

    fn start_attribute_ns(
        &mut self,
        prefix: Option<Vec<u8>>,
        name: &[u8],
        uri: Option<Vec<u8>>,
    ) -> bool {
        let q = match &prefix {
            Some(p) if !p.is_empty() => [&p[..], b":", name].concat(),
            _ => name.to_vec(),
        };
        if let Some(u) = uri {
            let p = prefix.clone().unwrap_or_default();
            if !self.nsstack.iter().any(|(x, _)| *x == p) {
                self.nsstack.push((p, u));
            }
        }
        self.start_attribute(&q)
    }

    fn end_attribute(&mut self) -> bool {
        if self.top() != Some(St::Attribute) {
            return false;
        }
        self.set_top(St::Name);
        self.out.push(self.qchar);
        true
    }

    fn write_raw(&mut self, content: &[u8]) -> bool {
        if !self.nodes.is_empty() {
            self.handle_state();
        }
        if self.indent {
            self.doindent = false;
        }
        self.out.extend_from_slice(content);
        true
    }

    fn write_string(&mut self, content: &[u8]) -> bool {
        match self.top() {
            Some(St::Attribute) => {
                escape_attr(&mut self.out, content, self.doc_encoding);
                true
            }
            Some(St::Name) | Some(St::Text) => {
                let enc = encode_special(content);
                self.write_raw(&enc)
            }
            _ => self.write_raw(content),
        }
    }

    fn start_cdata(&mut self) -> bool {
        match self.top() {
            Some(St::Text) | None => {}
            Some(St::Attribute) => {
                self.end_attribute();
                self.output_ns_decl();
                self.out.push(b'>');
                self.set_top(St::Text);
            }
            Some(St::Name) => {
                self.output_ns_decl();
                self.out.push(b'>');
                self.set_top(St::Text);
            }
            Some(St::Cdata) => return false,
            _ => return false,
        }
        self.nodes.push(Entry {
            name: Vec::new(),
            state: St::Cdata,
        });
        self.out.extend_from_slice(b"<![CDATA[");
        true
    }

    fn end_cdata(&mut self) -> bool {
        if self.top() != Some(St::Cdata) {
            return false;
        }
        self.out.extend_from_slice(b"]]>");
        self.nodes.pop();
        true
    }

    fn start_comment(&mut self) -> bool {
        match self.top() {
            Some(St::Text) | None => {}
            Some(St::Name) | Some(St::Attribute) => {
                if self.top() == Some(St::Attribute) {
                    self.end_attribute();
                }
                self.output_ns_decl();
                self.out.push(b'>');
                if self.indent {
                    self.out.push(b'\n');
                }
                self.set_top(St::Text);
            }
            _ => return false,
        }
        self.nodes.push(Entry {
            name: Vec::new(),
            state: St::Comment,
        });
        if self.indent {
            self.write_indent();
        }
        self.out.extend_from_slice(b"<!--");
        true
    }

    fn end_comment(&mut self) -> bool {
        if self.top() != Some(St::Comment) {
            return false;
        }
        self.out.extend_from_slice(b"-->");
        if self.indent {
            self.out.push(b'\n');
        }
        self.nodes.pop();
        true
    }

    fn start_pi(&mut self, target: &[u8]) -> bool {
        if target.eq_ignore_ascii_case(b"xml") {
            return false;
        }
        match self.top() {
            Some(St::Attribute) | Some(St::Name) => {
                if self.top() == Some(St::Attribute) {
                    self.end_attribute();
                }
                self.output_ns_decl();
                self.out.push(b'>');
                self.set_top(St::Text);
            }
            Some(St::Text) | Some(St::Dtd) | None => {}
            _ => return false,
        }
        self.nodes.push(Entry {
            name: target.to_vec(),
            state: St::Pi,
        });
        self.out.extend_from_slice(b"<?");
        self.out.extend_from_slice(target);
        true
    }

    fn end_pi(&mut self) -> bool {
        match self.top() {
            Some(St::Pi) | Some(St::PiText) => {
                self.out.extend_from_slice(b"?>");
            }
            _ => return false,
        }
        if self.indent {
            self.out.push(b'\n');
        }
        self.nodes.pop();
        true
    }

    fn start_dtd(&mut self, name: &[u8], pubid: Option<Vec<u8>>, sysid: Option<Vec<u8>>) -> bool {
        if !self.nodes.is_empty() {
            return false;
        }
        self.nodes.push(Entry {
            name: name.to_vec(),
            state: St::Dtd,
        });
        self.out.extend_from_slice(b"<!DOCTYPE ");
        self.out.extend_from_slice(name);
        if let Some(p) = pubid {
            self.out
                .extend_from_slice(if self.indent { b"\n" } else { b" " });
            self.out.extend_from_slice(b"PUBLIC ");
            self.out.push(self.qchar);
            self.out.extend_from_slice(&p);
            self.out.push(self.qchar);
            if let Some(s) = &sysid {
                self.out
                    .extend_from_slice(if self.indent { b"\n       " } else { b" " });
                self.out.push(self.qchar);
                self.out.extend_from_slice(s);
                self.out.push(self.qchar);
            }
        } else if let Some(s) = sysid {
            self.out
                .extend_from_slice(if self.indent { b"\n" } else { b" " });
            self.out.extend_from_slice(b"SYSTEM ");
            self.out.push(self.qchar);
            self.out.extend_from_slice(&s);
            self.out.push(self.qchar);
        }
        true
    }

    fn end_dtd(&mut self) -> bool {
        loop {
            match self.top() {
                Some(St::DtdText) => {
                    self.out.push(b']');
                    self.out.push(b'>');
                    if self.indent {
                        self.out.push(b'\n');
                    }
                    self.nodes.pop();
                    return true;
                }
                Some(St::Dtd) => {
                    self.out.push(b'>');
                    if self.indent {
                        self.out.push(b'\n');
                    }
                    self.nodes.pop();
                    return true;
                }
                Some(St::DtdElem)
                | Some(St::DtdElemText)
                | Some(St::DtdAttl)
                | Some(St::DtdAttlText) => {
                    self.out.push(b'>');
                    self.nodes.pop();
                }
                Some(St::DtdEnty) | Some(St::DtdEntyText) => {
                    if self.top() == Some(St::DtdEntyText) {
                        self.out.push(self.qchar);
                    }
                    self.out.push(b'>');
                    self.nodes.pop();
                }
                _ => return false,
            }
        }
    }

    fn start_dtd_part(&mut self, kind: &[u8], name: &[u8], st: St) -> bool {
        match self.top() {
            Some(St::Dtd) => {
                self.out.extend_from_slice(b" [");
                if self.indent {
                    self.out.push(b'\n');
                }
                self.set_top(St::DtdText);
            }
            Some(St::DtdText) | None => {}
            _ => return false,
        }
        self.nodes.push(Entry {
            name: name.to_vec(),
            state: st,
        });
        if self.indent {
            self.write_indent();
        }
        self.out.extend_from_slice(b"<!");
        self.out.extend_from_slice(kind);
        self.out.push(b' ');
        self.out.extend_from_slice(name);
        true
    }

    fn end_dtd_part(&mut self) -> bool {
        match self.top() {
            Some(St::DtdEntyText) => {
                self.out.push(self.qchar);
                self.out.push(b'>');
            }
            Some(St::DtdElem)
            | Some(St::DtdElemText)
            | Some(St::DtdAttl)
            | Some(St::DtdAttlText)
            | Some(St::DtdEnty) => {
                self.out.push(b'>');
            }
            _ => return false,
        }
        if self.indent {
            self.out.push(b'\n');
        }
        self.nodes.pop();
        true
    }
}

/// xmlEncodeSpecialChars.
fn encode_special(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        match c {
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'\r' => out.extend_from_slice(b"&#13;"),
            _ => out.push(c),
        }
    }
    out
}

thread_local! {
    static WRITERS: RefCell<HashMap<i64, Writer>> = RefCell::new(HashMap::new());
    static NEXT: RefCell<i64> = const { RefCell::new(1) };
}

fn with_w<R>(id: i64, f: impl FnOnce(&mut Writer) -> R) -> Option<R> {
    WRITERS.with(|w| w.borrow_mut().get_mut(&id).map(f))
}

fn flush_target(host: &mut dyn Host, id: i64, empty: bool) -> Value {
    let r = with_w(id, |w| {
        let data = w.take_out(empty);
        let target = match &w.target {
            Target::Memory => None,
            Target::Uri(u) => Some(Err(u.clone())),
            Target::Stream(v) => Some(Ok(v.clone())),
        };
        if target.is_some() {
            w.out.clear();
        }
        (data, target)
    });
    let Some((data, target)) = r else {
        return Value::Bool(false);
    };
    match target {
        None => Value::str(data),
        Some(t) => {
            let n = data.len();
            if !data.is_empty() {
                match t {
                    Err(uri) => write_uri(host, &uri, &data),
                    Ok(stream) => {
                        host.stream_write(&stream, &data);
                    }
                }
            }
            Value::Int(n as i64)
        }
    }
}

fn write_uri(host: &mut dyn Host, uri: &[u8], data: &[u8]) {
    match uri {
        b"php://output" | b"php://stdout" => host.write_out(data),
        _ => {
            use std::io::Write;
            let path =
                String::from_utf8_lossy(uri.strip_prefix(b"file://").unwrap_or(uri)).into_owned();
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(path)
            {
                let _ = f.write_all(data);
            }
        }
    }
}

fn fn_xw(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = String::from_utf8_lossy(&bytes(args, 0)).into_owned();
    let id = conv::to_int(&arg(args, 1));
    let b = |x: bool| Ok(Value::Bool(x));
    match op.as_str() {
        "openMemory" | "openUri" => {
            let target = if op == "openMemory" {
                Target::Memory
            } else {
                let uri = bytes(args, 2);
                let fname = match arg(args, 3) {
                    Value::Null => "XMLWriter::openUri".to_string(),
                    v => String::from_utf8_lossy(&conv::to_string_bytes(&v)).into_owned(),
                };
                if uri.is_empty() {
                    return Err(Thrown::Error {
                        class: "ValueError",
                        message: format!("{fname}(): Argument #1 ($uri) cannot be empty"),
                    });
                }
                if !matches!(&uri[..], b"php://output" | b"php://stdout") {
                    let path =
                        String::from_utf8_lossy(uri.strip_prefix(b"file://").unwrap_or(&uri))
                            .into_owned();
                    if std::fs::write(&path, b"").is_err() {
                        crate::warn(host, &fname, "Unable to resolve file path")?;
                        return Ok(Value::Bool(false));
                    }
                }
                Target::Uri(uri)
            };
            let new_id = NEXT.with(|n| {
                let mut n = n.borrow_mut();
                let v = *n;
                *n += 1;
                v
            });
            WRITERS.with(|w| w.borrow_mut().insert(new_id, Writer::new(target)));
            Ok(Value::Int(new_id))
        }
        "openStream" => {
            let stream = arg(args, 2);
            let new_id = NEXT.with(|n| {
                let mut n = n.borrow_mut();
                let v = *n;
                *n += 1;
                v
            });
            WRITERS.with(|w| {
                w.borrow_mut()
                    .insert(new_id, Writer::new(Target::Stream(stream)))
            });
            Ok(Value::Int(new_id))
        }
        "chkName" => {
            // XMLW_NAME_CHK: (op, 0, name, "fn(): Argument #N ($p)", subject)
            let name = bytes(args, 2);
            if crate::ext::dom::valid_name(&name) {
                return Ok(Value::Null);
            }
            Err(Thrown::Error {
                class: "ValueError",
                message: format!(
                    "{} must be a valid {}, \"{}\" given",
                    String::from_utf8_lossy(&bytes(args, 3)),
                    String::from_utf8_lossy(&bytes(args, 4)),
                    String::from_utf8_lossy(&name)
                ),
            })
        }
        "free" => {
            WRITERS.with(|w| w.borrow_mut().remove(&id));
            Ok(Value::Null)
        }
        "flush" => Ok(flush_target(host, id, conv::to_bool(&arg(args, 2)))),
        "outputMemory" => {
            let flush = conv::to_bool(&arg(args, 2));
            Ok(with_w(id, |w| Value::str(w.take_out(flush))).unwrap_or(Value::str(Vec::new())))
        }
        "setIndent" => {
            let on = conv::to_bool(&arg(args, 2));
            b(with_w(id, |w| {
                w.indent = on;
                w.doindent = true;
                true
            })
            .unwrap_or(false))
        }
        "setIndentString" => {
            let s = bytes(args, 2);
            b(with_w(id, |w| {
                w.ichar = s;
                true
            })
            .unwrap_or(false))
        }
        _ => {
            let r = with_w(id, |w| -> bool {
                match op.as_str() {
                    "startDocument" => {
                        w.start_document(opt_bytes(args, 2), opt_bytes(args, 3), opt_bytes(args, 4))
                    }
                    "endDocument" => w.end_document(),
                    "startElement" => w.start_element(&bytes(args, 2)),
                    "startElementNs" => {
                        w.start_element_ns(opt_bytes(args, 2), &bytes(args, 3), opt_bytes(args, 4))
                    }
                    "endElement" => w.end_element(),
                    "fullEndElement" => w.full_end_element(),
                    "startAttribute" => w.start_attribute(&bytes(args, 2)),
                    "startAttributeNs" => w.start_attribute_ns(
                        opt_bytes(args, 2),
                        &bytes(args, 3),
                        opt_bytes(args, 4),
                    ),
                    "endAttribute" => w.end_attribute(),
                    "writeAttribute" => {
                        w.start_attribute(&bytes(args, 2))
                            && w.write_string(&bytes(args, 3))
                            && w.end_attribute()
                    }
                    "writeAttributeNs" => {
                        w.start_attribute_ns(
                            opt_bytes(args, 2),
                            &bytes(args, 3),
                            opt_bytes(args, 4),
                        ) && w.write_string(&bytes(args, 5))
                            && w.end_attribute()
                    }
                    "writeElement" => {
                        let ok = w.start_element(&bytes(args, 2));
                        if !ok {
                            return false;
                        }
                        if let Some(c) = opt_bytes(args, 3) {
                            w.write_string(&c);
                        }
                        w.end_element()
                    }
                    "writeElementNs" => {
                        let ok = w.start_element_ns(
                            opt_bytes(args, 2),
                            &bytes(args, 3),
                            opt_bytes(args, 4),
                        );
                        if !ok {
                            return false;
                        }
                        if let Some(c) = opt_bytes(args, 5) {
                            w.write_string(&c);
                        }
                        w.end_element()
                    }
                    "text" => w.write_string(&bytes(args, 2)),
                    "writeRaw" => w.write_raw(&bytes(args, 2)),
                    "startCdata" => w.start_cdata(),
                    "endCdata" => w.end_cdata(),
                    "writeCdata" => {
                        w.start_cdata() && w.write_raw(&bytes(args, 2)) && w.end_cdata()
                    }
                    "startComment" => w.start_comment(),
                    "endComment" => w.end_comment(),
                    "writeComment" => {
                        w.start_comment() && w.write_raw(&bytes(args, 2)) && w.end_comment()
                    }
                    "startPi" => w.start_pi(&bytes(args, 2)),
                    "endPi" => w.end_pi(),
                    "writePi" => {
                        w.start_pi(&bytes(args, 2)) && w.write_raw(&bytes(args, 3)) && w.end_pi()
                    }
                    "startDtd" => {
                        w.start_dtd(&bytes(args, 2), opt_bytes(args, 3), opt_bytes(args, 4))
                    }
                    "endDtd" => w.end_dtd(),
                    "writeDtd" => {
                        let ok =
                            w.start_dtd(&bytes(args, 2), opt_bytes(args, 3), opt_bytes(args, 4));
                        if !ok {
                            return false;
                        }
                        if let Some(sub) = opt_bytes(args, 5) {
                            w.write_raw(&sub);
                        }
                        w.end_dtd()
                    }
                    "startDtdElement" => w.start_dtd_part(b"ELEMENT", &bytes(args, 2), St::DtdElem),
                    "startDtdAttlist" => w.start_dtd_part(b"ATTLIST", &bytes(args, 2), St::DtdAttl),
                    "startDtdEntity" => {
                        let pe = conv::to_bool(&arg(args, 3));
                        let name = if pe {
                            [b"% ".as_slice(), &bytes(args, 2)].concat()
                        } else {
                            bytes(args, 2)
                        };
                        w.start_dtd_part(b"ENTITY", &name, St::DtdEnty)
                    }
                    "endDtdElement" | "endDtdAttlist" | "endDtdEntity" => w.end_dtd_part(),
                    "writeDtdElement" => {
                        w.start_dtd_part(b"ELEMENT", &bytes(args, 2), St::DtdElem)
                            && w.write_raw(&bytes(args, 3))
                            && w.end_dtd_part()
                    }
                    "writeDtdAttlist" => {
                        w.start_dtd_part(b"ATTLIST", &bytes(args, 2), St::DtdAttl)
                            && w.write_raw(&bytes(args, 3))
                            && w.end_dtd_part()
                    }
                    "writeDtdEntity" => {
                        let pe = conv::to_bool(&arg(args, 4));
                        let name = if pe {
                            [b"% ".as_slice(), &bytes(args, 2)].concat()
                        } else {
                            bytes(args, 2)
                        };
                        w.start_dtd_part(b"ENTITY", &name, St::DtdEnty)
                            && w.write_raw(&bytes(args, 3))
                            && w.end_dtd_part()
                    }
                    _ => false,
                }
            });
            if op == "endDocument" && r == Some(true) {
                // xmlTextWriterEndDocument flushes the output buffer
                if with_w(id, |w| !matches!(w.target, Target::Memory)).unwrap_or(false) {
                    flush_target(host, id, true);
                }
            }
            match r {
                Some(x) => b(x),
                None => Err(Thrown::Error {
                    class: "Error",
                    message: "Invalid or uninitialized XMLWriter object".into(),
                }),
            }
        }
    }
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    m.insert(
        "__xw",
        Builtin {
            name: "__xw",
            func: fn_xw,
            min_args: 1,
            max_args: None,
        },
    );
}

pub const PRELUDE_PHP: &str = include_str!("prelude.php");
