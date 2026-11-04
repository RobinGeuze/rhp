//! A namespace-aware XML 1.0 parser shaped after libxml2 2.9's
//! `xmlParseDocument` (the SAX2 tree builder), reporting the same error
//! codes, messages, line and column numbers for the common malformations,
//! and applying libxml2's blank-node heuristic for `keepBlanks = 0`.

// Only SimpleXML and XMLReader still use this; it goes when they move to
// libxml2 trees.
#![allow(dead_code)]

use super::charset;
use super::tree::{Doc, DocTypeInfo, Kind, Node, NodeId, XML_NS};

#[derive(Clone, Debug)]
pub struct XmlError {
    pub domain: i32,
    pub level: i32,
    pub code: i32,
    pub line: i32,
    pub column: i32,
    pub message: String,
    pub file: Option<Vec<u8>>,
    /// libxml2's printed input context: the source line around the error
    /// and the caret line (xmlParserPrintFileContextInternal).
    pub context: Option<(Vec<u8>, Vec<u8>)>,
}

pub const LEVEL_WARNING: i32 = 1;
pub const LEVEL_ERROR: i32 = 2;
pub const LEVEL_FATAL: i32 = 3;

#[derive(Clone, Default)]
pub struct ParseOpts {
    pub keep_blanks: bool,
    pub noent: bool,
    pub nocdata: bool,
    pub huge: bool,
    pub recover: bool,
}

impl ParseOpts {
    /// Options from php's LIBXML_* flags (+ DOMDocument's preserveWhiteSpace /
    /// substituteEntities).
    pub fn from_flags(flags: i64, keep_blanks: bool, subst: bool) -> ParseOpts {
        ParseOpts {
            keep_blanks: keep_blanks && flags & 256 == 0,
            noent: subst || flags & 2 != 0,
            nocdata: flags & 16384 != 0,
            huge: flags & 524288 != 0,
            recover: flags & 1 != 0,
        }
    }
}

struct P<'o> {
    s: Vec<u8>,
    pos: usize,
    // (pos, line, col) of the last location computed
    lc: (usize, u32, u32),
    doc: Doc,
    errors: Vec<XmlError>,
    well_formed: bool,
    halted: bool,
    opts: &'o ParseOpts,
    url: Option<Vec<u8>>,
    /// In-scope namespace bindings (prefix, uri), innermost last.
    ns: Vec<(Vec<u8>, Vec<u8>)>,
    space: Vec<i8>,
    depth: usize,
    has_external_subset: bool,
    in_attr_entity: usize,
    /// First non-UTF-8 byte position and its hex dump (reported on reach).
    bad_utf8: Option<(usize, String)>,
    /// The input was UTF-16 (BOM), transcoded by `normalize_input`.
    utf16_input: bool,
}

/// An attribute as parsed: (prefix, local name, value, line).
type RawAttr = (Vec<u8>, Vec<u8>, Vec<u8>, u32);

fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b':' || c >= 0x80
}

fn is_name_char(c: u8) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == b'-' || c == b'.'
}

fn is_xml_char(c: u32) -> bool {
    matches!(c, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

fn push_utf8(out: &mut Vec<u8>, c: u32) {
    if let Some(ch) = char::from_u32(c) {
        let mut b = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
    }
}

/// First invalid UTF-8 position, if any.
fn utf8_error_at(s: &[u8]) -> Option<usize> {
    match std::str::from_utf8(s) {
        Ok(_) => None,
        Err(e) => Some(e.valid_up_to()),
    }
}

impl P<'_> {
    fn cur(&self) -> u8 {
        self.s.get(self.pos).copied().unwrap_or(0)
    }
    fn nxt(&self, k: usize) -> u8 {
        self.s.get(self.pos + k).copied().unwrap_or(0)
    }
    fn starts(&self, lit: &[u8]) -> bool {
        self.s[self.pos.min(self.s.len())..].starts_with(lit)
    }
    fn eof(&self) -> bool {
        self.pos >= self.s.len()
    }

    fn loc(&mut self) -> (u32, u32) {
        let (mut p, mut line, mut col) = self.lc;
        if p > self.pos {
            p = 0;
            line = 1;
            col = 1;
        }
        let end = self.pos.min(self.s.len());
        while p < end {
            let c = self.s[p];
            if c == b'\n' {
                line += 1;
                col = 1;
            } else if !(0x80..0xC0).contains(&c) {
                col += 1;
            }
            p += 1;
        }
        self.lc = (p, line, col);
        (line, col)
    }

    fn line(&mut self) -> u32 {
        self.loc().0
    }

    fn err(&mut self, level: i32, code: i32, msg: String) {
        let domain = if (200..300).contains(&code) || code == 100 {
            3
        } else {
            1
        };
        let (line, col) = self.loc();
        if level == LEVEL_FATAL {
            self.well_formed = false;
        }
        let context = Some(self.context());
        self.errors.push(XmlError {
            domain,
            level,
            code,
            line: line as i32,
            column: col as i32,
            message: msg,
            file: self.url.clone(),
            context,
        });
    }

    /// xmlParserPrintFileContextInternal.
    fn context(&self) -> (Vec<u8>, Vec<u8>) {
        let s = &self.s;
        let at = |i: usize| s.get(i).copied().unwrap_or(0);
        let mut cur = self.pos.min(s.len());
        while cur > 0 && (at(cur) == b'\n' || at(cur) == b'\r') {
            cur -= 1;
        }
        let mut n = 0;
        loop {
            let ok = n < 80;
            n += 1;
            if !(ok && cur > 0 && at(cur) != b'\n' && at(cur) != b'\r') {
                break;
            }
            cur -= 1;
        }
        if at(cur) == b'\n' || at(cur) == b'\r' {
            cur += 1;
        }
        let col = self.pos.min(s.len()) - cur;
        let mut content = Vec::new();
        let mut i = cur;
        while i < s.len() && at(i) != b'\n' && at(i) != b'\r' && content.len() < 80 {
            content.push(s[i]);
            i += 1;
        }
        let mut caret = Vec::new();
        let mut n = 0;
        let mut k = 0;
        loop {
            if n >= col {
                break;
            }
            let ok = n < 79;
            n += 1;
            if !ok || k >= content.len() {
                break;
            }
            caret.push(if content[k] == b'\t' { b'\t' } else { b' ' });
            k += 1;
        }
        caret.push(b'^');
        (content, caret)
    }

    fn fatal(&mut self, code: i32, msg: &str) {
        self.err(LEVEL_FATAL, code, msg.to_string());
    }

    fn halt(&mut self) {
        self.halted = true;
        self.pos = self.s.len();
    }

    fn skip_blanks(&mut self) -> usize {
        let st = self.pos;
        while !self.eof() && is_blank(self.cur()) {
            self.pos += 1;
        }
        self.pos - st
    }

    fn parse_name(&mut self) -> Option<Vec<u8>> {
        if !is_name_start(self.cur()) {
            return None;
        }
        let st = self.pos;
        while !self.eof() && is_name_char(self.cur()) {
            self.pos += 1;
        }
        Some(self.s[st..self.pos].to_vec())
    }

    fn parse_ncname(&mut self) -> Option<Vec<u8>> {
        let c = self.cur();
        if !(c.is_ascii_alphabetic() || c == b'_' || c >= 0x80) {
            return None;
        }
        let st = self.pos;
        while !self.eof() && is_name_char(self.cur()) && self.cur() != b':' {
            self.pos += 1;
        }
        Some(self.s[st..self.pos].to_vec())
    }

    /// `xmlParseQName` (libxml2 2.9.13): (prefix, local). A malformed QName
    /// raises XML_NS_ERR_QNAME and is repaired the way libxml2 does it.
    fn parse_qname(&mut self) -> Option<(Vec<u8>, Vec<u8>)> {
        let Some(l) = self.parse_ncname() else {
            if self.cur() == b':' {
                // e.g. ":a": a plain Name
                let n = self.parse_name()?;
                let msg = format!("Failed to parse QName '{}'\n", String::from_utf8_lossy(&n));
                self.err(LEVEL_ERROR, 202, msg);
                return Some((Vec::new(), n));
            }
            return None;
        };
        if self.cur() != b':' {
            return Some((Vec::new(), l));
        }
        self.pos += 1;
        let p = l;
        let Some(l) = self.parse_ncname() else {
            // "a:" — prefix, then an Nmtoken (or nothing)
            let msg = format!("Failed to parse QName '{}:'\n", String::from_utf8_lossy(&p));
            self.err(LEVEL_ERROR, 202, msg);
            let st = self.pos;
            while !self.eof() && is_name_char(self.cur()) {
                self.pos += 1;
            }
            let tok = &self.s[st..self.pos];
            return Some((Vec::new(), [&p[..], b":", tok].concat()));
        };
        if self.cur() == b':' {
            // "a:b:c" — prefix a, local "b:c"
            let msg = format!(
                "Failed to parse QName '{}:{}:'\n",
                String::from_utf8_lossy(&p),
                String::from_utf8_lossy(&l)
            );
            self.err(LEVEL_ERROR, 202, msg);
            self.pos += 1;
            let rest = self.parse_name().unwrap_or_default();
            return Some((p, [&l[..], b":", &rest[..]].concat()));
        }
        Some((p, l))
    }

    // ------------------------------------------------------------ document

    fn document(&mut self) {
        if self.s.is_empty() {
            self.fatal(4, "Document is empty\n");
            return;
        }
        if self.starts(b"<?xml") && self.s.len() > 5 && is_blank(self.nxt(5)) {
            self.xml_decl();
            if self.halted {
                return;
            }
        }
        self.misc();
        if self.starts(b"<!DOCTYPE") {
            self.doctype();
            if self.halted {
                return;
            }
            self.misc();
        }
        if self.cur() != b'<' || self.eof() {
            self.fatal(4, "Start tag expected, '<' not found\n");
            return;
        }
        self.element(0);
        if self.halted {
            return;
        }
        self.misc();
        if !self.eof() {
            self.fatal(5, "Extra content at the end of the document\n");
        }
    }

    fn misc(&mut self) {
        loop {
            if self.halted {
                return;
            }
            self.skip_blanks();
            if self.starts(b"<?") {
                self.pi(0);
            } else if self.starts(b"<!--") {
                self.comment(0);
            } else {
                return;
            }
        }
    }

    fn quoted(&mut self) -> Option<Vec<u8>> {
        let q = self.cur();
        if q != b'"' && q != b'\'' {
            return None;
        }
        self.pos += 1;
        let st = self.pos;
        while !self.eof() && self.cur() != q {
            self.pos += 1;
        }
        if self.eof() {
            return None;
        }
        let v = self.s[st..self.pos].to_vec();
        self.pos += 1;
        Some(v)
    }

    fn decl_attr(&mut self, name: &[u8]) -> Option<Option<Vec<u8>>> {
        let save = self.pos;
        self.skip_blanks();
        if !self.starts(name) {
            self.pos = save;
            return Some(None);
        }
        self.pos += name.len();
        self.skip_blanks();
        if self.cur() != b'=' {
            self.pos = save;
            return Some(None);
        }
        self.pos += 1;
        self.skip_blanks();
        self.quoted().map(Some)
    }

    /// `xmlParseVersionInfo` / `xmlParseVersionNum`: `version = '1.x'`.
    fn version_info(&mut self) -> Option<Vec<u8>> {
        if !self.starts(b"version") {
            return None;
        }
        self.pos += 7;
        self.skip_blanks();
        if self.cur() != b'=' {
            self.fatal(75, "Specification mandates value for attribute version\n");
            return None;
        }
        self.pos += 1;
        self.skip_blanks();
        let q = self.cur();
        if q != b'"' && q != b'\'' {
            self.fatal(33, "String not started expecting ' or \"\n");
            return None;
        }
        self.pos += 1;
        // VersionNum ::= [0-9] '.' [0-9]*
        let st = self.pos;
        let mut version = None;
        if self.cur().is_ascii_digit() {
            self.pos += 1;
            if self.cur() == b'.' {
                self.pos += 1;
                while self.cur().is_ascii_digit() {
                    self.pos += 1;
                }
                version = Some(self.s[st..self.pos].to_vec());
            }
        }
        if self.cur() != q {
            self.fatal(34, "String not closed expecting \" or '\n");
        } else {
            self.pos += 1;
        }
        version
    }

    fn xml_decl(&mut self) {
        self.xml_decl_inner();
        self.check_utf8();
    }

    /// `xmlParseXMLDecl`.
    fn xml_decl_inner(&mut self) {
        self.pos += 5;
        self.skip_blanks();
        match self.version_info() {
            None => self.fatal(96, "Malformed declaration expecting version\n"),
            Some(v) => {
                if v != b"1.0" {
                    let msg = format!("Unsupported version '{}'\n", String::from_utf8_lossy(&v));
                    if v.starts_with(b"1.") {
                        self.err(LEVEL_WARNING, 97, msg);
                    } else {
                        self.err(LEVEL_FATAL, 108, msg);
                    }
                }
                self.doc.version = Some(v);
            }
        }
        if !is_blank(self.cur()) {
            if self.starts(b"?>") {
                self.pos += 2;
                return;
            }
            self.fatal(65, "Blank needed here\n");
        }
        match self.decl_attr(b"encoding") {
            Some(Some(e)) => {
                let up = e.to_ascii_uppercase();
                let single = charset::single_byte(&e);
                let utf16 = matches!(&up[..], b"UTF-16" | b"UTF16");
                let known = matches!(&up[..], b"UTF-8" | b"UTF8") || utf16 || single.is_some();
                if !known {
                    self.err(
                        LEVEL_FATAL,
                        32,
                        format!("Unsupported encoding {}\n", String::from_utf8_lossy(&e)),
                    );
                    self.halt();
                    return;
                }
                if utf16 && !self.utf16_input {
                    self.fatal(81, "Document labelled UTF-16 but has UTF-8 content\n");
                    self.halt();
                    return;
                }
                if let Some(cs) = single.filter(|c| *c != charset::SingleByte::Ascii) {
                    // transcode the rest of the input to UTF-8
                    let mut conv = self.s[..self.pos].to_vec();
                    conv.extend(cs.decode(&self.s[self.pos..]));
                    self.s = conv;
                }
                self.doc.encoding = Some(e);
            }
            Some(None) => {}
            None => {
                self.fatal(34, "String not closed expecting \" or '\n");
                self.halt();
                return;
            }
        }
        // (libxml2 only switches — and then checks — for a non-UTF-8 encoding)
        let switched = self.doc.encoding.as_ref().is_some_and(|e| {
            !matches!(
                &e.to_ascii_uppercase()[..],
                b"UTF-8" | b"UTF8" | b"UTF-16" | b"UTF16"
            )
        });
        if switched && !is_blank(self.cur()) {
            if self.starts(b"?>") {
                self.pos += 2;
                return;
            }
            self.fatal(65, "Blank needed here\n");
        }
        // xmlParseSDDecl
        self.skip_blanks();
        if self.starts(b"standalone") {
            self.pos += 10;
            self.skip_blanks();
            if self.cur() != b'=' {
                self.fatal(
                    75,
                    "Specification mandates value for attribute standalone\n",
                );
            } else {
                self.pos += 1;
                self.skip_blanks();
                let q = self.cur();
                if q == b'"' || q == b'\'' {
                    self.pos += 1;
                    if self.starts(b"no") {
                        self.doc.standalone = 0;
                        self.pos += 2;
                    } else if self.starts(b"yes") {
                        self.doc.standalone = 1;
                        self.pos += 3;
                    } else {
                        self.fatal(78, "standalone accepts only 'yes' or 'no'\n");
                    }
                    if self.cur() != q {
                        self.fatal(34, "String not closed expecting \" or '\n");
                    } else {
                        self.pos += 1;
                    }
                } else {
                    self.fatal(33, "String not started expecting ' or \"\n");
                }
            }
        }
        self.skip_blanks();
        if self.starts(b"?>") {
            self.pos += 2;
        } else if self.cur() == b'>' {
            self.fatal(57, "parsing XML declaration: '?>' expected\n");
            self.pos += 1;
        } else {
            self.fatal(57, "parsing XML declaration: '?>' expected\n");
            while !self.eof() && self.cur() != b'>' {
                self.pos += 1;
            }
            if !self.eof() {
                self.pos += 1;
            }
        }
    }

    /// libxml2 halts on the first byte sequence that isn't UTF-8.
    fn check_utf8(&mut self) {
        if let Some(bad) = utf8_error_at(&self.s[self.pos..]) {
            let at = self.pos + bad;
            // reported when the parser reaches it; approximate by truncating
            // the input there and recording the error position.
            let bytes: Vec<String> = self.s[at..(at + 4).min(self.s.len())]
                .iter()
                .map(|b| format!("0x{b:02X}"))
                .collect();
            self.bad_utf8 = Some((at, bytes.join(" ")));
        }
    }

    fn doctype(&mut self) {
        self.pos += 9;
        self.skip_blanks();
        let Some(name) = self.parse_name() else {
            self.fatal(68, "xmlParseDocTypeDecl : no DOCTYPE name !\n");
            self.halt();
            return;
        };
        let mut info = DocTypeInfo::default();
        self.skip_blanks();
        if self.starts(b"PUBLIC") {
            self.pos += 6;
            self.skip_blanks();
            info.public_id = self.quoted();
            self.skip_blanks();
            info.system_id = self.quoted();
        } else if self.starts(b"SYSTEM") {
            self.pos += 6;
            self.skip_blanks();
            info.system_id = self.quoted();
        }
        if info.system_id.is_some() || info.public_id.is_some() {
            self.has_external_subset = true;
        }
        self.skip_blanks();
        if self.cur() == b'[' {
            self.pos += 1;
            let mut decls = Vec::new();
            loop {
                self.skip_blanks();
                if self.eof() {
                    self.fatal(
                        61,
                        "xmlParseInternalSubset: error detected in Markup declaration\n",
                    );
                    self.halt();
                    return;
                }
                if self.cur() == b']' {
                    self.pos += 1;
                    break;
                }
                if self.starts(b"<!--") {
                    let st = self.pos;
                    self.comment_raw();
                    decls.push(self.s[st..self.pos].to_vec());
                } else if self.starts(b"<!ENTITY") {
                    self.pos += 8;
                    self.skip_blanks();
                    let pe = if self.cur() == b'%' {
                        self.pos += 1;
                        self.skip_blanks();
                        true
                    } else {
                        false
                    };
                    let ename = self.parse_name().unwrap_or_default();
                    self.skip_blanks();
                    let mut out = b"<!ENTITY ".to_vec();
                    if pe {
                        out.extend_from_slice(b"% ");
                    }
                    out.extend_from_slice(&ename);
                    if self.cur() == b'"' || self.cur() == b'\'' {
                        let v = self.quoted().unwrap_or_default();
                        out.push(b' ');
                        super::tree::quote(&mut out, &v);
                        if !pe {
                            let val = self.expand_char_refs(&v);
                            self.doc.entities.push((ename.clone(), val));
                        }
                    } else {
                        // external entity: keep the declaration as written
                        let st = self.pos;
                        while !self.eof() && self.cur() != b'>' {
                            self.pos += 1;
                        }
                        let raw = String::from_utf8_lossy(&self.s[st..self.pos]).into_owned();
                        let norm: Vec<&str> = raw.split_whitespace().collect();
                        out.push(b' ');
                        out.extend_from_slice(norm.join(" ").as_bytes());
                    }
                    self.skip_blanks();
                    if self.cur() == b'>' {
                        self.pos += 1;
                    }
                    out.push(b'>');
                    decls.push(out);
                } else if self.starts(b"<!") || self.starts(b"<?") {
                    let st = self.pos;
                    let mut q = 0u8;
                    while !self.eof() {
                        let c = self.cur();
                        if q != 0 {
                            if c == q {
                                q = 0;
                            }
                        } else if c == b'"' || c == b'\'' {
                            q = c;
                        } else if c == b'>' {
                            break;
                        }
                        self.pos += 1;
                    }
                    if !self.eof() {
                        self.pos += 1;
                    }
                    let raw = String::from_utf8_lossy(&self.s[st..self.pos]).into_owned();
                    let norm: Vec<&str> = raw.split_whitespace().collect();
                    decls.push(norm.join(" ").into_bytes());
                } else if self.cur() == b'%' {
                    while !self.eof() && self.cur() != b';' {
                        self.pos += 1;
                    }
                    self.pos += 1;
                } else {
                    self.fatal(
                        61,
                        "xmlParseInternalSubset: error detected in Markup declaration\n",
                    );
                    self.halt();
                    return;
                }
            }
            info.subset = Some(decls);
            self.skip_blanks();
        }
        if self.cur() != b'>' {
            self.fatal(61, "DOCTYPE improperly terminated\n");
            self.halt();
            return;
        }
        self.pos += 1;
        let mut n = Node::new(Kind::DocType);
        n.name = name;
        n.doctype = Some(Box::new(info));
        let id = self.doc.add(n);
        self.doc.append(0, id);
    }

    fn expand_char_refs(&self, v: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < v.len() {
            if v[i] == b'&' && v.get(i + 1) == Some(&b'#') {
                if let Some(end) = v[i..].iter().position(|&c| c == b';') {
                    let body = &v[i + 2..i + end];
                    let cp = if body.first() == Some(&b'x') {
                        u32::from_str_radix(std::str::from_utf8(&body[1..]).unwrap_or(""), 16).ok()
                    } else {
                        std::str::from_utf8(body).ok().and_then(|s| s.parse().ok())
                    };
                    if let Some(cp) = cp {
                        push_utf8(&mut out, cp);
                        i += end + 1;
                        continue;
                    }
                }
            }
            out.push(v[i]);
            i += 1;
        }
        out
    }

    // ------------------------------------------------------------ content

    fn add_child(&mut self, parent: NodeId, n: Node) -> NodeId {
        let id = self.doc.add(n);
        self.doc.append(parent, id);
        id
    }

    /// Append character data to `parent`, coalescing with a trailing text node.
    fn characters(&mut self, parent: NodeId, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if let Some(l) = self.doc.nodes[parent].last {
            if self.doc.nodes[l].kind == Kind::Text {
                self.doc.nodes[l].content.extend_from_slice(data);
                return;
            }
        }
        let line = self.line();
        let mut n = Node::new(Kind::Text);
        n.content = data.to_vec();
        n.line = line;
        self.add_child(parent, n);
    }

    /// libxml2's `areBlanks` for keepBlanks = 0 (no DTD).
    fn are_blanks(&self, parent: NodeId) -> bool {
        if self.opts.keep_blanks {
            return false;
        }
        if matches!(self.space.last(), Some(1) | Some(-2)) {
            return false;
        }
        let c = self.cur();
        if c != b'<' && c != b'\r' {
            return false;
        }
        let n = &self.doc.nodes[parent];
        if n.first.is_none() && c == b'<' && self.nxt(1) == b'/' {
            return false;
        }
        match n.last {
            None => {}
            Some(l) => {
                if self.doc.nodes[l].kind == Kind::Text {
                    return false;
                }
                if let Some(f) = n.first {
                    if self.doc.nodes[f].kind == Kind::Text {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn content(&mut self, parent: NodeId, depth: usize) {
        while !self.eof() && !self.halted {
            if let Some((at, _)) = &self.bad_utf8 {
                if self.pos >= *at {
                    self.report_bad_utf8();
                    return;
                }
            }
            let c = self.cur();
            if c == b'<' {
                match self.nxt(1) {
                    b'/' => return,
                    b'?' => self.pi(parent),
                    b'!' => {
                        if self.starts(b"<!--") {
                            self.comment(parent);
                        } else if self.starts(b"<![CDATA[") {
                            self.cdata(parent);
                        } else {
                            self.fatal(68, "StartTag: invalid element name\n");
                            self.halt();
                        }
                    }
                    _ => {
                        self.element_in(parent, depth + 1);
                    }
                }
            } else if c == b'&' {
                self.reference(parent);
            } else {
                self.char_data(parent);
            }
        }
    }

    fn report_bad_utf8(&mut self) {
        if let Some((at, bytes)) = self.bad_utf8.take() {
            self.pos = at;
            self.err(
                LEVEL_FATAL,
                9,
                format!("Input is not proper UTF-8, indicate encoding !\nBytes: {bytes}\n"),
            );
            self.halt();
        }
    }

    fn char_data(&mut self, parent: NodeId) {
        let st = self.pos;
        let mut stop = self.s.len();
        if let Some((at, _)) = &self.bad_utf8 {
            stop = stop.min(*at);
        }
        let mut blank = true;
        while self.pos < stop {
            let c = self.cur();
            if c == b'<' || c == b'&' {
                break;
            }
            if c == b']' && self.starts(b"]]>") {
                let text = self.s[st..self.pos].to_vec();
                self.emit_text(parent, &text, false);
                self.fatal(62, "Sequence ']]>' not allowed in content\n");
                self.pos += 3;
                let t2 = b"]]>".to_vec();
                self.emit_text(parent, &t2, false);
                return;
            }
            if c < 0x20 && !is_blank(c) {
                // xmlParseCharDataComplex: report, skip the character, go on
                let text = self.s[st..self.pos].to_vec();
                self.emit_text(parent, &text, blank);
                self.err(LEVEL_FATAL, 9, format!("PCDATA invalid Char value {c}\n"));
                self.pos += 1;
                return;
            }
            if !is_blank(c) {
                blank = false;
            }
            self.pos += 1;
        }
        let text = self.s[st..self.pos].to_vec();
        self.emit_text(parent, &text, blank);
        if self.pos == stop && self.bad_utf8.is_some() {
            self.report_bad_utf8();
        }
    }

    fn emit_text(&mut self, parent: NodeId, text: &[u8], blank: bool) {
        if text.is_empty() || parent == 0 {
            return;
        }
        if blank && self.are_blanks(parent) {
            return;
        }
        self.characters(parent, text);
    }

    /// `&...;` in content.
    fn reference(&mut self, parent: NodeId) {
        if self.nxt(1) == b'#' {
            if let Some(cp) = self.char_ref() {
                let mut b = Vec::new();
                push_utf8(&mut b, cp);
                if parent != 0 {
                    self.characters(parent, &b);
                }
            }
            return;
        }
        self.pos += 1;
        let Some(name) = self.parse_name() else {
            self.fatal(68, "xmlParseEntityRef: no name\n");
            return;
        };
        if self.cur() != b';' {
            self.fatal(23, "EntityRef: expecting ';'\n");
            return;
        }
        self.pos += 1;
        let predefined: Option<&[u8]> = match &name[..] {
            b"lt" => Some(b"<"),
            b"gt" => Some(b">"),
            b"amp" => Some(b"&"),
            b"quot" => Some(b"\""),
            b"apos" => Some(b"'"),
            _ => None,
        };
        if let Some(v) = predefined {
            self.characters(parent, v);
            return;
        }
        let val = self
            .doc
            .entities
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone());
        match val {
            Some(v) => {
                if self.opts.noent {
                    self.characters(parent, &v);
                } else {
                    let mut n = Node::new(Kind::EntityRef);
                    n.name = name;
                    self.add_child(parent, n);
                }
            }
            None => {
                let msg = format!("Entity '{}' not defined\n", String::from_utf8_lossy(&name));
                if self.has_external_subset && self.doc.standalone != 1 {
                    self.err(LEVEL_ERROR, 27, msg);
                    let mut n = Node::new(Kind::EntityRef);
                    n.name = name;
                    self.add_child(parent, n);
                } else {
                    self.err(LEVEL_FATAL, 26, msg);
                }
            }
        }
    }

    /// `&#...;` — returns the code point (errors reported).
    fn char_ref(&mut self) -> Option<u32> {
        self.pos += 2;
        let hex = self.cur() == b'x';
        if hex {
            self.pos += 1;
        }
        let st = self.pos;
        while !self.eof()
            && (if hex {
                self.cur().is_ascii_hexdigit()
            } else {
                self.cur().is_ascii_digit()
            })
        {
            self.pos += 1;
        }
        let digits = std::str::from_utf8(&self.s[st..self.pos])
            .unwrap_or("")
            .to_string();
        if self.cur() != b';' || digits.is_empty() {
            if hex {
                self.fatal(6, "xmlParseCharRef: invalid hexadecimal value\n");
            } else {
                self.fatal(7, "xmlParseCharRef: invalid decimal value\n");
            }
            return None;
        }
        self.pos += 1;
        let v = u32::from_str_radix(&digits, if hex { 16 } else { 10 }).unwrap_or(0x110000);
        if !is_xml_char(v) {
            self.err(
                LEVEL_FATAL,
                9,
                format!("xmlParseCharRef: invalid xmlChar value {v}\n"),
            );
            return None;
        }
        Some(v)
    }

    fn comment_raw(&mut self) -> Option<Vec<u8>> {
        self.pos += 4;
        let st = self.pos;
        while self.pos < self.s.len() {
            if self.starts(b"-->") {
                let v = self.s[st..self.pos].to_vec();
                self.pos += 3;
                return Some(v);
            }
            self.pos += 1;
        }
        None
    }

    /// The character at `at` (decoded UTF-8) and its length; `(0, 0)` at the
    /// end of the input or at the first byte that isn't UTF-8 (`stop`).
    fn char_at(&self, at: usize, stop: usize) -> (u32, usize) {
        if at >= stop {
            return (0, 0);
        }
        let c = self.s[at];
        let n = match c {
            0..=0x7F => return (c as u32, 1),
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            _ => 4,
        };
        let end = (at + n).min(stop);
        match std::str::from_utf8(&self.s[at..end])
            .ok()
            .and_then(|t| t.chars().next())
        {
            Some(ch) => (ch as u32, ch.len_utf8()),
            None => (c as u32, 1),
        }
    }

    /// `xmlParseComment` + `xmlParseCommentComplex` (libxml2 2.9.13): the
    /// ASCII fast path reports `--` with the text so far, the general path
    /// with the generic message; a bad character ends the comment early.
    fn comment(&mut self, parent: NodeId) {
        self.pos += 4;
        let st = self.pos;
        let stop = self.bad_utf8.as_ref().map_or(self.s.len(), |b| b.0);
        loop {
            while self.pos < stop {
                let c = self.s[self.pos];
                if (c > b'-' && c <= 0x7F) || (0x20..b'-').contains(&c) || c == b'\t' || c == b'\n'
                {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos < stop && self.s[self.pos] == b'-' {
                if self.nxt(1) == b'-' {
                    if self.nxt(2) == b'>' {
                        let v = self.s[st..self.pos].to_vec();
                        self.pos += 3;
                        self.add_comment(parent, v);
                        return;
                    }
                    if self.pos > st {
                        let shown = &self.s[st..self.pos.min(st + 50)];
                        let msg = format!(
                            "Double hyphen within comment: <!--{}\n",
                            String::from_utf8_lossy(shown)
                        );
                        self.err(LEVEL_FATAL, 80, msg);
                    } else {
                        self.fatal(80, "Double hyphen within comment\n");
                    }
                    self.pos += 2;
                    continue;
                }
                self.pos += 1;
                continue;
            }
            break;
        }
        // xmlParseCommentComplex
        let mut buf = self.s[st..self.pos].to_vec();
        let (q, ql) = self.char_at(self.pos, stop);
        if q == 0 {
            return self.comment_not_terminated(stop);
        }
        if !is_xml_char(q) {
            let msg = format!("xmlParseComment: invalid xmlChar value {q}\n");
            self.err(LEVEL_FATAL, 9, msg);
            return;
        }
        self.pos += ql;
        let (r, rl) = self.char_at(self.pos, stop);
        if r == 0 {
            return self.comment_not_terminated(stop);
        }
        if !is_xml_char(r) {
            // (libxml2 reports the previous character here)
            let msg = format!("xmlParseComment: invalid xmlChar value {q}\n");
            self.err(LEVEL_FATAL, 9, msg);
            return;
        }
        self.pos += rl;
        let (mut q, mut r) = (q, r);
        let (mut cur, mut l) = self.char_at(self.pos, stop);
        if cur == 0 {
            return self.comment_not_terminated(stop);
        }
        while is_xml_char(cur) && (cur != b'>' as u32 || r != b'-' as u32 || q != b'-' as u32) {
            if r == b'-' as u32 && q == b'-' as u32 {
                self.fatal(80, "Comment must not contain '--' (double-hyphen)\n");
            }
            push_utf8(&mut buf, q);
            q = r;
            r = cur;
            self.pos += l;
            (cur, l) = self.char_at(self.pos, stop);
        }
        if cur == 0 {
            if self.pos >= stop && self.bad_utf8.is_some() {
                self.report_bad_utf8();
                return;
            }
            let shown = &buf[..buf.len().min(50)];
            let msg = format!(
                "Comment not terminated \n<!--{}\n",
                String::from_utf8_lossy(shown)
            );
            self.err(LEVEL_FATAL, 45, msg);
        } else if !is_xml_char(cur) {
            let msg = format!("xmlParseComment: invalid xmlChar value {cur}\n");
            self.err(LEVEL_FATAL, 9, msg);
        } else {
            self.pos += 1;
            self.add_comment(parent, buf);
        }
    }

    fn comment_not_terminated(&mut self, stop: usize) {
        if self.pos >= stop && self.bad_utf8.is_some() {
            self.report_bad_utf8();
            return;
        }
        self.fatal(45, "Comment not terminated\n");
    }

    fn add_comment(&mut self, parent: NodeId, v: Vec<u8>) {
        let line = self.line();
        let mut n = Node::new(Kind::Comment);
        n.content = v;
        n.line = line;
        self.add_child(parent, n);
    }

    fn pi(&mut self, parent: NodeId) {
        self.pos += 2;
        let Some(target) = self.parse_name() else {
            self.fatal(46, "xmlParsePI : no target name\n");
            return;
        };
        if target.eq_ignore_ascii_case(b"xml") {
            if target == b"xml" {
                self.fatal(
                    64,
                    "XML declaration allowed only at the start of the document\n",
                );
            } else {
                self.fatal(64, "xmlParsePITarget: invalid name prefix 'xml'\n");
            }
        }
        if self.starts(b"?>") {
            self.pos += 2;
            let mut n = Node::new(Kind::Pi);
            n.name = target;
            self.add_child(parent, n);
            return;
        }
        if self.skip_blanks() == 0 {
            self.err(
                LEVEL_FATAL,
                65,
                format!(
                    "ParsePI: PI {} space expected\n",
                    String::from_utf8_lossy(&target)
                ),
            );
        }
        let st = self.pos;
        while self.pos < self.s.len() {
            let c = self.cur();
            if c < 0x20 && !is_blank(c) {
                break; // not a Char: the PI never ends
            }
            if self.starts(b"?>") {
                let data = self.s[st..self.pos].to_vec();
                self.pos += 2;
                let mut n = Node::new(Kind::Pi);
                n.name = target;
                n.content = data;
                self.add_child(parent, n);
                return;
            }
            self.pos += 1;
        }
        self.err(
            LEVEL_FATAL,
            47,
            format!(
                "ParsePI: PI {} never end ...\n",
                String::from_utf8_lossy(&target)
            ),
        );
    }

    fn cdata(&mut self, parent: NodeId) {
        self.pos += 9;
        let st = self.pos;
        while self.pos < self.s.len() {
            let c = self.cur();
            if c < 0x20 && !is_blank(c) {
                // xmlParseCDSect: a non-Char among the first two characters
                // is a bare XML_ERR_CDATA_NOT_FINISHED
                if self.pos - st < 2 {
                    self.fatal(63, "Unregistered error message\n");
                    return;
                }
                break;
            }
            if self.starts(b"]]>") {
                let data = self.s[st..self.pos].to_vec();
                self.pos += 3;
                if self.opts.nocdata {
                    self.characters(parent, &data);
                } else {
                    let line = self.line();
                    let mut n = Node::new(Kind::CData);
                    n.content = data;
                    n.line = line;
                    self.add_child(parent, n);
                }
                return;
            }
            self.pos += 1;
        }
        let got = &self.s[st..self.pos];
        if got.len() < 2 {
            self.fatal(63, "CData section not finished\n");
        } else {
            let shown = String::from_utf8_lossy(&got[..(got.len() - 2).min(50)]).into_owned();
            self.err(
                LEVEL_FATAL,
                63,
                format!("CData section not finished\n{shown}\n"),
            );
        }
    }

    // ------------------------------------------------------------ elements

    fn element(&mut self, depth: usize) {
        self.element_in(0, depth);
    }

    /// Attribute value (`xmlParseAttValueInternal`), normalized.
    fn att_value(&mut self) -> Option<Vec<u8>> {
        let q = self.cur();
        if q != b'"' && q != b'\'' {
            self.fatal(39, "AttValue: \" or ' expected\n");
            return None;
        }
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            if self.eof() {
                self.fatal(40, "AttValue: ' expected\n");
                return None;
            }
            let c = self.cur();
            if c == q {
                self.pos += 1;
                return Some(out);
            }
            match c {
                b'<' => {
                    self.fatal(38, "Unescaped '<' not allowed in attributes values\n");
                    return None;
                }
                b'&' => {
                    if self.nxt(1) == b'#' {
                        if let Some(cp) = self.char_ref() {
                            push_utf8(&mut out, cp);
                        }
                        continue;
                    }
                    self.pos += 1;
                    let Some(name) = self.parse_name() else {
                        self.fatal(68, "xmlParseEntityRef: no name\n");
                        continue;
                    };
                    if self.cur() != b';' {
                        self.fatal(23, "EntityRef: expecting ';'\n");
                        continue;
                    }
                    self.pos += 1;
                    match self.doc.entity_value(&name).map(|v| v.to_vec()) {
                        Some(v) => out.extend_from_slice(&v),
                        None => {
                            let msg = format!(
                                "Entity '{}' not defined\n",
                                String::from_utf8_lossy(&name)
                            );
                            if self.has_external_subset && self.doc.standalone != 1 {
                                self.err(LEVEL_ERROR, 27, msg);
                            } else {
                                self.err(LEVEL_FATAL, 26, msg);
                            }
                        }
                    }
                }
                b'\t' | b'\n' | b'\r' => {
                    out.push(b' ');
                    self.pos += 1;
                }
                _ if c < 0x20 => {
                    self.fatal(9, "invalid character in attribute value\n");
                    return None;
                }
                _ => {
                    if self.bad_utf8.as_ref().is_some_and(|b| b.0 == self.pos) {
                        self.report_bad_utf8();
                        return None;
                    }
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn element_in(&mut self, parent: NodeId, depth: usize) {
        let max_depth = if self.opts.huge { 2048 } else { 256 };
        if depth > max_depth {
            self.err(
                LEVEL_FATAL,
                1,
                format!("Excessive depth in document: {max_depth} use XML_PARSE_HUGE option\n"),
            );
            self.halt();
            return;
        }
        let start_line = self.line();
        self.pos += 1; // '<'
        let Some((prefix, local)) = self.parse_qname() else {
            self.fatal(68, "StartTag: invalid element name\n");
            return;
        };
        let qname = if prefix.is_empty() {
            local.clone()
        } else {
            [&prefix[..], b":", &local[..]].concat()
        };
        // attributes: (prefix, local, value)
        let mut attrs: Vec<RawAttr> = Vec::new();
        let ns_mark = self.ns.len();
        let mut ns_defs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        self.skip_blanks();
        while !self.eof()
            && self.cur() != b'>'
            && !(self.cur() == b'/' && self.nxt(1) == b'>')
            && !self.halted
        {
            let Some((ap, al)) = self.parse_qname() else {
                self.fatal(68, "error parsing attribute name\n");
                self.fatal(65, "attributes construct error\n");
                break;
            };
            self.skip_blanks();
            let mut val = None;
            if self.cur() == b'=' {
                self.pos += 1;
                self.skip_blanks();
                val = self.att_value();
                if self.halted {
                    self.ns.truncate(ns_mark);
                    return;
                }
            } else {
                let an = if ap.is_empty() {
                    al.clone()
                } else {
                    [&ap[..], b":", &al[..]].concat()
                };
                self.err(
                    LEVEL_FATAL,
                    41,
                    format!(
                        "Specification mandates value for attribute {}\n",
                        String::from_utf8_lossy(&an)
                    ),
                );
            }
            if let Some(v) = val {
                if ap.is_empty() && al == b"xmlns" {
                    self.check_ns_uri(None, &v);
                    if !ns_defs.iter().any(|(p, _)| p.is_empty()) {
                        ns_defs.push((Vec::new(), v.clone()));
                        self.ns.push((Vec::new(), v));
                    } else {
                        self.err(LEVEL_FATAL, 42, "Attribute xmlns redefined\n".into());
                    }
                } else if ap == b"xmlns" {
                    if v.is_empty() {
                        self.err(
                            LEVEL_ERROR,
                            200,
                            format!(
                                "xmlns:{}: Empty XML namespace is not allowed\n",
                                String::from_utf8_lossy(&al)
                            ),
                        );
                    } else if al == b"xml" {
                        if v != XML_NS {
                            self.err(
                                LEVEL_ERROR,
                                200,
                                "xml namespace prefix mapped to wrong URI\n".into(),
                            );
                        }
                    } else {
                        if ns_defs.iter().any(|(p, _)| *p == al) {
                            self.err(
                                LEVEL_FATAL,
                                42,
                                format!(
                                    "Attribute xmlns:{} redefined\n",
                                    String::from_utf8_lossy(&al)
                                ),
                            );
                        } else {
                            ns_defs.push((al.clone(), v.clone()));
                            self.ns.push((al, v));
                        }
                    }
                } else {
                    let line = self.line();
                    attrs.push((ap, al, v, line));
                }
            }
            if self.cur() == b'>' || (self.cur() == b'/' && self.nxt(1) == b'>') {
                break;
            }
            if self.skip_blanks() == 0 {
                self.fatal(65, "attributes construct error\n");
                break;
            }
        }
        // duplicate attributes
        let mut seen: Vec<Vec<u8>> = Vec::new();
        let mut dedup = Vec::new();
        for a in attrs {
            let q = if a.0.is_empty() {
                a.1.clone()
            } else {
                [&a.0[..], b":", &a.1[..]].concat()
            };
            if seen.contains(&q) {
                self.err(
                    LEVEL_FATAL,
                    42,
                    format!("Attribute {} redefined\n", String::from_utf8_lossy(&q)),
                );
                continue;
            }
            seen.push(q);
            dedup.push(a);
        }
        // namespace resolution
        let lookup = |ns: &Vec<(Vec<u8>, Vec<u8>)>, p: &[u8]| -> Option<Vec<u8>> {
            if p == b"xml" {
                return Some(XML_NS.to_vec());
            }
            ns.iter()
                .rev()
                .find(|(x, _)| x == p)
                .map(|(_, u)| u.clone())
        };
        let line = self.line();
        let mut el = Node::new(Kind::Element);
        el.line = line;
        el.ns_defs = ns_defs;
        if prefix.is_empty() {
            el.name = local.clone();
            el.ns = lookup(&self.ns, b"").filter(|u| !u.is_empty());
        } else {
            match lookup(&self.ns, &prefix) {
                Some(u) => {
                    el.name = local.clone();
                    el.prefix = prefix.clone();
                    el.ns = Some(u);
                }
                None => {
                    self.err(
                        LEVEL_ERROR,
                        201,
                        format!(
                            "Namespace prefix {} on {} is not defined\n",
                            String::from_utf8_lossy(&prefix),
                            String::from_utf8_lossy(&local)
                        ),
                    );
                    el.name = qname.clone();
                }
            }
        }
        let id = self.add_child(parent, el);
        let mut xml_space: Option<i8> = None;
        for (ap, al, v, aline) in dedup {
            let mut a = Node::new(Kind::Attr);
            a.line = aline;
            if ap.is_empty() {
                a.name = al;
            } else {
                match lookup(&self.ns, &ap) {
                    Some(u) => {
                        if ap == b"xml" && al == b"space" {
                            xml_space = Some(if v == b"preserve" { 1 } else { 0 });
                        }
                        a.name = al;
                        a.prefix = ap;
                        a.ns = Some(u);
                    }
                    None => {
                        self.err(
                            LEVEL_ERROR,
                            201,
                            format!(
                                "Namespace prefix {} for {} on {} is not defined\n",
                                String::from_utf8_lossy(&ap),
                                String::from_utf8_lossy(&al),
                                String::from_utf8_lossy(&local)
                            ),
                        );
                        a.name = [&ap[..], b":", &al[..]].concat();
                    }
                }
            }
            a.content = v;
            a.parent = Some(id);
            let aid = self.doc.add(a);
            self.doc.nodes[id].attrs.push(aid);
        }
        let ended = self.cur() == b'/' && self.nxt(1) == b'>';
        if ended {
            self.pos += 2;
            self.doc.nodes[id].empty_tag = true;
            self.ns.truncate(ns_mark);
            return;
        }
        if self.cur() != b'>' {
            self.err(
                LEVEL_FATAL,
                73,
                format!(
                    "Couldn't find end of Start Tag {} line {}\n",
                    String::from_utf8_lossy(&qname),
                    start_line
                ),
            );
            self.doc.unlink(id);
            self.ns.truncate(ns_mark);
            return;
        }
        self.pos += 1;
        let sp = xml_space.unwrap_or_else(|| *self.space.last().unwrap_or(&-1));
        self.space.push(sp);
        self.content(id, depth);
        self.space.pop();
        if self.halted {
            self.ns.truncate(ns_mark);
            return;
        }
        if self.eof() {
            self.err(
                LEVEL_FATAL,
                77,
                format!(
                    "Premature end of data in tag {} line {}\n",
                    String::from_utf8_lossy(&qname),
                    start_line
                ),
            );
            self.ns.truncate(ns_mark);
            return;
        }
        // end tag (xmlParseQNameAndCompare: a matching name raises nothing)
        self.pos += 2;
        let n_err = self.errors.len();
        let got = self.parse_qname();
        self.errors.truncate(n_err);
        let got = got.map(|(p, l)| {
            if p.is_empty() {
                l
            } else {
                [&p[..], b":", &l[..]].concat()
            }
        });
        self.skip_blanks();
        if self.cur() != b'>' {
            self.fatal(73, "expected '>'\n");
        } else {
            self.pos += 1;
        }
        let got = got.unwrap_or_default();
        if got != qname {
            self.err(
                LEVEL_FATAL,
                76,
                format!(
                    "Opening and ending tag mismatch: {} line {} and {}\n",
                    String::from_utf8_lossy(&qname),
                    start_line,
                    String::from_utf8_lossy(&got)
                ),
            );
        }
        self.ns.truncate(ns_mark);
    }

    fn check_ns_uri(&mut self, prefix: Option<&[u8]>, uri: &[u8]) {
        if uri.is_empty() {
            return;
        }
        // absolute iff it starts with a URI scheme
        let mut has_scheme = false;
        if uri[0].is_ascii_alphabetic() {
            for &c in &uri[1..] {
                if c == b':' {
                    has_scheme = true;
                    break;
                }
                if !(c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.') {
                    break;
                }
            }
        }
        if !has_scheme {
            let msg = match prefix {
                None => format!(
                    "xmlns: URI {} is not absolute\n",
                    String::from_utf8_lossy(uri)
                ),
                Some(p) => format!(
                    "xmlns:{}: URI {} is not absolute\n",
                    String::from_utf8_lossy(p),
                    String::from_utf8_lossy(uri)
                ),
            };
            self.err(LEVEL_WARNING, 100, msg);
        }
    }
}

/// Decode a UTF-16 / UTF-8-BOM input to plain UTF-8.
fn normalize_input(input: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = if input.starts_with(&[0xEF, 0xBB, 0xBF]) {
        input[3..].to_vec()
    } else if input.starts_with(&[0xFF, 0xFE]) || input.starts_with(&[0xFE, 0xFF]) {
        let le = input[0] == 0xFF;
        let units: Vec<u16> = input[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units).into_bytes()
    } else {
        input.to_vec()
    };
    // end-of-line handling: \r\n and lone \r become \n
    if s.contains(&b'\r') {
        let mut out = Vec::with_capacity(s.len());
        let mut i = 0;
        while i < s.len() {
            if s[i] == b'\r' {
                out.push(b'\n');
                if s.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            } else {
                out.push(s[i]);
            }
            i += 1;
        }
        s = out;
    }
    s
}

/// Parse a document. Returns the tree when the input is well-formed (or the
/// parse recovered), plus every diagnostic raised.
pub fn parse(input: &[u8], opts: &ParseOpts, url: Option<&[u8]>) -> (Option<Doc>, Vec<XmlError>) {
    let s = normalize_input(input);
    let mut p = P {
        s,
        pos: 0,
        lc: (0, 1, 1),
        doc: Doc::new(),
        errors: Vec::new(),
        well_formed: true,
        halted: false,
        opts,
        url: url.map(|u| u.to_vec()),
        ns: Vec::new(),
        space: vec![-1],
        depth: 0,
        has_external_subset: false,
        in_attr_entity: 0,
        bad_utf8: None,
        utf16_input: input.starts_with(&[0xFF, 0xFE]) || input.starts_with(&[0xFE, 0xFF]),
    };
    p.doc.version = None;
    // no XML declaration: the encoding is undeclared; check UTF-8 up front
    if !(p.starts(b"<?xml") && p.s.len() > 5 && is_blank(p.nxt(5))) {
        p.check_utf8();
    }
    p.document();
    let _ = (p.depth, p.in_attr_entity);
    let mut doc = p.doc;
    if doc.version.is_none() {
        doc.version = Some(b"1.0".to_vec());
    }
    doc.url = url.map(|u| u.to_vec());
    if p.well_formed || opts.recover {
        (Some(doc), p.errors)
    } else {
        (None, p.errors)
    }
}

/// Parse a well-balanced chunk (`DOMDocumentFragment::appendXML`) in the
/// context of `doc`'s entities; returns the top-level nodes in a fresh
/// fragment document.
pub fn parse_fragment(chunk: &[u8], opts: &ParseOpts) -> (Option<Doc>, Vec<XmlError>) {
    let mut wrapped = b"<__frag__>".to_vec();
    wrapped.extend_from_slice(chunk);
    wrapped.extend_from_slice(b"</__frag__>");
    parse(&wrapped, opts, None)
}
