//! Dom\HTMLDocument's parsing and saveHtml() output (php-src
//! ext/dom/html_document.c, PHP License 3.01) over the C glue in
//! `html5c/` (lexbor's HTML5 parser, converted to a libxml2 tree, as php
//! does it).

use std::ffi::{c_char, c_int, c_long, c_void};
use std::ptr;

use super::nscompat::{self, DomPrivate};
use crate::ext::libxml::sys::{XmlDoc, XmlNode, XmlNs};

#[repr(C)]
pub struct Html5 {
    _p: [u8; 0],
}

type Sink = unsafe extern "C" fn(*mut c_void, *const c_char, usize);

extern "C" {
    fn rhp_html5_status_string(status: c_int) -> *const c_char;
    fn rhp_html5_encoding_name(name: *const c_char, len: usize) -> *const c_char;
    fn rhp_html5_new(
        input_name: *const c_char,
        no_implied: c_int,
        report: c_int,
        rust: *mut c_void,
    ) -> *mut Html5;
    fn rhp_html5_set_encoding(
        h: *mut Html5,
        buf_start: *const c_char,
        name: *const c_char,
        len: usize,
    ) -> c_int;
    fn rhp_html5_parse_string(h: *mut Html5, source: *const c_char, len: usize) -> c_int;
    fn rhp_html5_file_buf(h: *mut Html5) -> *mut c_char;
    fn rhp_html5_parse_file_chunk(h: *mut Html5, read: usize) -> c_int;
    fn rhp_html5_finish(h: *mut Html5) -> c_int;
    fn rhp_html5_convert(
        h: *mut Html5,
        options: c_long,
        compact: c_int,
        create_default_ns: c_int,
        private_data: *mut c_void,
        out: *mut *mut XmlDoc,
    ) -> c_int;
    fn rhp_html5_quirks(h: *mut Html5) -> c_int;
    fn rhp_html5_free(h: *mut Html5);
    fn rhp_html5_parse_fragment(
        private_data: *mut c_void,
        context: *mut XmlNode,
        input: *const c_char,
        len: usize,
        quirks: c_int,
        out: *mut *mut XmlNode,
    ) -> c_int;
    fn rhp_dom_xml_parse_fragment(
        context: *mut XmlNode,
        decls: *const c_char,
        decls_len: usize,
        input: *const c_char,
        len: usize,
        out: *mut *mut XmlDoc,
    ) -> c_int;
    fn rhp_dom_css(
        root: *const XmlNode,
        sel: *const c_char,
        len: usize,
        mode: c_int,
        quirks: c_int,
        sink: unsafe extern "C" fn(*mut c_void, *const XmlNode),
        ctx: *mut c_void,
        out: *mut *const XmlNode,
        err: *mut *mut c_char,
    ) -> c_int;
    fn rhp_html5_encode_output(
        encoding: *const c_char,
        utf8: *const c_char,
        len: usize,
        sink: Sink,
        ctx: *mut c_void,
    );
}

fn token(t: c_int) -> Option<&'static nscompat::Token> {
    Some(match t {
        1 => &nscompat::HTML,
        2 => &nscompat::MATHML,
        3 => &nscompat::SVG,
        4 => &nscompat::XLINK,
        5 => &nscompat::XML,
        6 => &nscompat::XMLNS,
        _ => return None,
    })
}

#[no_mangle]
unsafe extern "C" fn rhp_dom_ns_set_token(ns: *mut XmlNs, t: c_int) {
    if let Some(t) = token(t) {
        unsafe { (*ns)._private = t as *const nscompat::Token as *mut c_void };
    }
}

#[no_mangle]
unsafe extern "C" fn rhp_dom_ns_is_fast(node: *const XmlNode, t: c_int) -> bool {
    match token(t) {
        Some(t) => nscompat::ns_is_fast(node, t),
        None => false,
    }
}

#[no_mangle]
unsafe extern "C" fn rhp_dom_ns_is_html_and_document_is_html(node: *const XmlNode) -> bool {
    nscompat::ns_is_html_and_document_is_html(node)
}

#[no_mangle]
unsafe extern "C" fn rhp_dom_mapper_get_ns(
    private_data: *mut c_void,
    prefix: *const c_char,
    uri: *const c_char,
    t: c_int,
) -> *mut XmlNs {
    let p = unsafe { &mut *(private_data as *mut DomPrivate) };
    let ns = p.ns_mapper.get_ns_nullsafe(prefix.cast(), uri.cast());
    if !ns.is_null() {
        unsafe { rhp_dom_ns_set_token(ns, t) };
    }
    ns
}

#[no_mangle]
unsafe extern "C" fn rhp_dom_template_add(
    private_data: *mut c_void,
    template: *mut XmlNode,
    fragment: *mut XmlNode,
) {
    let p = unsafe { &mut *(private_data as *mut DomPrivate) };
    nscompat::add_element_ns_hook(p, template);
    nscompat::add_templated_content(p, template, fragment);
}

/// A parse error, reported after the parse.
pub struct ParseError {
    pub msg: Vec<u8>,
    pub line: usize,
    pub column: usize,
}

#[no_mangle]
unsafe extern "C" fn rhp_dom_html5_error(
    rust: *mut c_void,
    msg: *const c_char,
    len: usize,
    line: usize,
    column: usize,
) {
    if rust.is_null() {
        return;
    }
    let errors = unsafe { &mut *(rust as *mut Vec<ParseError>) };
    errors.push(ParseError {
        msg: unsafe { std::slice::from_raw_parts(msg.cast::<u8>(), len) }.to_vec(),
        line,
        column,
    });
}

/// lxb_encoding_data_by_name: the canonical name of an encoding lexbor
/// knows.
pub fn encoding_name(name: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let p = rhp_html5_encoding_name(name.as_ptr().cast(), name.len());
        (!p.is_null()).then(|| crate::ext::libxml::sys::cstr(p).to_vec())
    }
}

pub fn status_string(status: c_int) -> Vec<u8> {
    unsafe { crate::ext::libxml::sys::cstr(rhp_html5_status_string(status)).to_vec() }
}

/// One parse: the glue's state and the errors it reported.
pub struct Parser {
    h: *mut Html5,
    /// Boxed: the glue keeps a pointer to the Vec while it parses.
    #[allow(clippy::box_collection)]
    pub errors: Box<Vec<ParseError>>,
}

impl Parser {
    pub fn new(input_name: &[u8], no_implied: bool, report: bool) -> Parser {
        let mut errors = Box::new(Vec::new());
        let name = super::util::c(input_name);
        let h = unsafe {
            rhp_html5_new(
                name.as_ptr().cast(),
                no_implied as c_int,
                report as c_int,
                &mut *errors as *mut Vec<ParseError> as *mut c_void,
            )
        };
        Parser { h, errors }
    }

    /// dom_setup_parser_encoding_manually; false for an unknown encoding.
    /// `buf_start` is the string parsed (None: the file buffer).
    pub fn set_encoding(&mut self, buf_start: Option<&[u8]>, name: &[u8]) -> bool {
        let start = buf_start.map_or(ptr::null(), |b| b.as_ptr().cast());
        unsafe { rhp_html5_set_encoding(self.h, start, name.as_ptr().cast(), name.len()) != 0 }
    }

    pub fn parse_string(&mut self, source: &[u8]) -> bool {
        unsafe { rhp_html5_parse_string(self.h, source.as_ptr().cast(), source.len()) == 0 }
    }

    /// One read of createFromFile's loop.
    pub fn parse_file_chunk(&mut self, chunk: &[u8]) -> bool {
        debug_assert!(chunk.len() <= 4096);
        unsafe {
            ptr::copy_nonoverlapping(
                chunk.as_ptr(),
                rhp_html5_file_buf(self.h).cast(),
                chunk.len(),
            );
            rhp_html5_parse_file_chunk(self.h, chunk.len()) == 0
        }
    }

    pub fn finish(&mut self) -> bool {
        unsafe { rhp_html5_finish(self.h) == 0 }
    }

    /// The libxml2 document, or the bridge status.
    pub fn convert(
        &mut self,
        options: i64,
        compact: bool,
        default_ns: bool,
        private: *mut DomPrivate,
    ) -> Result<*mut XmlDoc, c_int> {
        let mut out: *mut XmlDoc = ptr::null_mut();
        let st = unsafe {
            rhp_html5_convert(
                self.h,
                options as c_long,
                compact as c_int,
                default_ns as c_int,
                private.cast(),
                &mut out,
            )
        };
        if st == 0 {
            Ok(out)
        } else {
            Err(st)
        }
    }

    pub fn quirks(&self) -> crate::ext::libxml::node::QuirksMode {
        use crate::ext::libxml::node::QuirksMode;
        match unsafe { rhp_html5_quirks(self.h) } {
            0 => QuirksMode::NoQuirks,
            1 => QuirksMode::LimitedQuirks,
            _ => QuirksMode::Quirks,
        }
    }
}

impl Drop for Parser {
    fn drop(&mut self) {
        unsafe { rhp_html5_free(self.h) };
    }
}

unsafe extern "C" fn sink_vec(ctx: *mut c_void, buf: *const c_char, len: usize) {
    let v = unsafe { &mut *(ctx as *mut Vec<u8>) };
    v.extend_from_slice(unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), len) });
}

/// dom_common_save's output encoding: the serializer's UTF-8 in the
/// document's encoding.
pub fn encode_output(encoding: &[u8], utf8: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(utf8.len());
    let enc = super::util::c(encoding);
    unsafe {
        rhp_html5_encode_output(
            enc.as_ptr().cast(),
            utf8.as_ptr().cast(),
            utf8.len(),
            sink_vec,
            &mut out as *mut Vec<u8> as *mut c_void,
        )
    };
    out
}

/// dom_html_fragment_parsing_algorithm: the fragment, or the glue's
/// status (-1: lexbor failed).
pub fn parse_html_fragment(
    private: *mut DomPrivate,
    context: *mut XmlNode,
    input: &[u8],
    quirks: crate::ext::libxml::node::QuirksMode,
) -> Result<*mut XmlNode, c_int> {
    rhp_csrc::lexbor::setup();
    use crate::ext::libxml::node::QuirksMode;
    let q = match quirks {
        QuirksMode::NoQuirks => 0,
        QuirksMode::LimitedQuirks => 1,
        QuirksMode::Quirks => 2,
    };
    let mut out: *mut XmlNode = ptr::null_mut();
    let st = unsafe {
        rhp_html5_parse_fragment(
            private.cast(),
            context,
            input.as_ptr().cast(),
            input.len(),
            q,
            &mut out,
        )
    };
    if st == 0 {
        Ok(out)
    } else {
        Err(st)
    }
}

/// dom_xml_fragment_parsing_algorithm's parse: the parsed document
/// (Ok(None): not well-formed).
pub fn parse_xml_fragment(
    context: *mut XmlNode,
    decls: &[u8],
    input: &[u8],
) -> Result<Option<*mut XmlDoc>, ()> {
    let mut out: *mut XmlDoc = ptr::null_mut();
    let r = unsafe {
        rhp_dom_xml_parse_fragment(
            context,
            decls.as_ptr().cast(),
            decls.len(),
            input.as_ptr().cast(),
            input.len(),
            &mut out,
        )
    };
    match r {
        1 => Ok(Some(out)),
        0 => Ok(None),
        _ => Err(()),
    }
}

/// How a selector query failed.
pub enum CssError {
    /// The selector did not parse (lexbor's first message, if any).
    Syntax(Option<Vec<u8>>),
    Unsupported,
    /// The :blank pseudo-class.
    Blank,
}

unsafe extern "C" fn css_sink(ctx: *mut c_void, node: *const XmlNode) {
    let v = unsafe { &mut *(ctx as *mut Vec<*mut XmlNode>) };
    v.push(node as *mut XmlNode);
}

/// A selector query (parentnode/css_selectors.c): mode 0 querySelector,
/// 1 querySelectorAll, 2 matches, 3 closest. The nodes found.
pub fn css(
    root: *mut XmlNode,
    sel: &[u8],
    mode: c_int,
    quirks: bool,
) -> Result<Vec<*mut XmlNode>, CssError> {
    rhp_csrc::lexbor::setup();
    let mut nodes: Vec<*mut XmlNode> = Vec::new();
    let mut out: *const XmlNode = ptr::null();
    let mut err: *mut c_char = ptr::null_mut();
    let r = unsafe {
        rhp_dom_css(
            root,
            sel.as_ptr().cast(),
            sel.len(),
            mode,
            quirks as c_int,
            css_sink,
            &mut nodes as *mut Vec<*mut XmlNode> as *mut c_void,
            &mut out,
            &mut err,
        )
    };
    let msg = (!err.is_null()).then(|| unsafe {
        let m = crate::ext::libxml::sys::cstr(err).to_vec();
        libc::free(err.cast());
        m
    });
    match r {
        0 => {
            if mode != 1 && !out.is_null() {
                nodes.push(out as *mut XmlNode);
            }
            Ok(nodes)
        }
        1 => Err(CssError::Syntax(msg)),
        3 => Err(CssError::Blank),
        _ => Err(CssError::Unsupported),
    }
}
