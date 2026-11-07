//! The `xml` extension: php's expat-compatible SAX parser on libxml2's push
//! parser (php's `XML_SAX_IMPL` "libxml"). [`compat`] ports ext/xml/compat.c,
//! the libxml2 SAX handlers that imitate expat; this module ports
//! ext/xml/xml.c, the handlers that call PHP and the `xml_*` functions.
//! The `XMLParser` class and the `xml_*` functions are declared in PHP
//! ([`PRELUDE_PHP`]); the functions call [`FUNCTIONS`] through `__xml_fn`,
//! except the native `xml_parse_into_struct`.
//! A parser's state lives here, keyed by object id, and is freed with the
//! object.

mod compat;

use std::cell::RefCell;
use std::ffi::c_int;
use std::rc::{Rc, Weak};

use rhp_values::{conv, ArrayKey, ObjectData, PhpArray, PhpRef, PhpStr, Value};

use crate::ext::libxml::{ffi, sys};
use crate::zend::object_handlers::{ClassHandlers, ObjectHandlers};
use crate::zend::zpp::{deref, long_arg};
use crate::{Host, Thrown};

const XML_MAXLEVEL: i32 = 255;

// php_xml.h PHP_XML_OPTION_*
const OPT_CASE_FOLDING: i64 = 1;
const OPT_TARGET_ENCODING: i64 = 2;
const OPT_SKIP_TAGSTART: i64 = 3;
const OPT_SKIP_WHITE: i64 = 4;
const OPT_PARSE_HUGE: i64 = 5;

/// expat_compat.h `XML_ERROR_EXTERNAL_ENTITY_HANDLING`.
const XML_ERROR_EXTERNAL_ENTITY_HANDLING: c_int = 21;

/// The target encodings (xml.c `xml_encodings`); a decoder for the two that
/// are not UTF-8.
const ENCODINGS: &[&str] = &["ISO-8859-1", "US-ASCII", "UTF-8"];

/// A handler (php's `zend_fcall_info_cache`).
#[derive(Clone)]
enum Fcc {
    Callable(Value),
    /// A method named by a string, looked up on the `xml_set_object` object
    /// (php sets no calling scope for these).
    Method {
        object: Value,
        name: Vec<u8>,
    },
}

// Handler slots.
const H_START: usize = 0;
const H_END: usize = 1;
const H_CDATA: usize = 2;
const H_PI: usize = 3;
const H_DEFAULT: usize = 4;
const H_UNPARSED: usize = 5;
const H_NOTATION: usize = 6;
const H_EXTENT: usize = 7;
const H_START_NS: usize = 8;
const H_END_NS: usize = 9;

/// Which of compat.c's `h_*` C handlers xml.c installed (`XML_Set*Handler`).
#[derive(Default, Clone, Copy)]
struct CHandlers {
    element: bool,
    cdata: bool,
    pi: bool,
    default: bool,
    unparsed: bool,
    notation: bool,
    extent: bool,
    start_ns: bool,
    end_ns: bool,
}

/// xml.c `xml_parser` plus compat.c's `XML_Parser_Struct`.
pub(crate) struct Parser {
    ctxt: sys::Ctxt,
    use_namespace: bool,
    ns_separator: Vec<u8>,
    c: CHandlers,
    target_encoding: &'static str,
    /// The XMLParser object itself (php's `index`, not owned).
    index: Weak<RefCell<ObjectData>>,
    object: Option<Value>,
    h: [Option<Fcc>; 10],
    data: Option<PhpRef>,
    info: Option<PhpRef>,
    level: i32,
    toffset: i32,
    curtag: i64,
    ctag_index: i64,
    ltags: Option<Vec<Option<PhpStr>>>,
    lastwasopen: bool,
    skipwhite: bool,
    isparsing: bool,
    parsehuge: bool,
    case_folding: bool,
}

impl Drop for Parser {
    /// compat.c XML_ParserFree
    fn drop(&mut self) {
        if !self.ctxt.is_null() {
            unsafe {
                sys::rhp_xml_ctxt_free_doc(self.ctxt);
                sys::xmlFreeParserCtxt(self.ctxt);
            }
        }
    }
}

type ParserRef = Rc<RefCell<Parser>>;

/// `XMLParser`'s handlers: no clone, not comparable. The parser is the
/// object's payload.
pub(crate) const CLASSES: &[ClassHandlers] = &[ClassHandlers {
    class: "XMLParser",
    handlers: &ObjectHandlers::OPAQUE,
    create_object: true,
}];

thread_local! {
    /// The parsers created on this thread. A parser holds PHP values (the
    /// `xml_set_object` target, handlers) that often hold the parser's own
    /// object; php's cycle collector, or its end-of-request free of every
    /// object, breaks that cycle. rhp has neither, so a new request drops
    /// what the old request's parsers still hold.
    static LIVE: RefCell<Vec<Weak<RefCell<Parser>>>> = const { RefCell::new(Vec::new()) };
}

/// A new request: release the PHP values earlier requests' parsers hold.
pub(crate) fn reset_state() {
    let live = LIVE.with(|l| std::mem::take(&mut *l.borrow_mut()));
    for p in live.iter().filter_map(Weak::upgrade) {
        let held = p.try_borrow_mut().ok().map(|mut p| {
            (
                p.object.take(),
                std::mem::take(&mut p.h),
                p.data.take(),
                p.info.take(),
            )
        });
        drop(held);
    }
}

/// The XMLParser argument `i` of builtin `fname`, else php's TypeError.
fn parser_arg(host: &dyn Host, args: &[Value], i: usize, fname: &str) -> Result<ParserRef, Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    if let Some(p) = crate::zend::object_handlers::payload::<RefCell<Parser>>(&v) {
        return Ok(p);
    }
    Err(Thrown::type_error(format!(
        "{fname}(): Argument #{} ($parser) must be of type XMLParser, {} given",
        i + 1,
        crate::zend::zpp::zval_value_name(host, &v)
    )))
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(deref).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// encoding (xml.c)
// ---------------------------------------------------------------------------

fn xml_get_encoding(name: &[u8]) -> Option<&'static str> {
    ENCODINGS
        .iter()
        .copied()
        .find(|e| e.as_bytes().eq_ignore_ascii_case(name))
}

/// ext/standard/html.c `php_next_utf8_char`: the code point at `*pos`, or
/// `None` for an invalid sequence; `*pos` moves past what was read.
fn next_utf8_char(s: &[u8], pos: &mut usize) -> Option<u32> {
    let lead = |c: u8| c < 0x80 || (0xC2..=0xF4).contains(&c);
    let trail = |c: u8| (0x80..=0xBF).contains(&c);
    let p = *pos;
    let avail = s.len() - p;
    let fail = |pos: &mut usize, n: usize| {
        *pos = p + n;
        None
    };
    let c = s[p];
    if c < 0x80 {
        *pos = p + 1;
        Some(c as u32)
    } else if c < 0xC2 {
        fail(pos, 1)
    } else if c < 0xE0 {
        if avail < 2 {
            return fail(pos, 1);
        }
        if !trail(s[p + 1]) {
            return fail(pos, if lead(s[p + 1]) { 1 } else { 2 });
        }
        let ch = ((c as u32 & 0x1f) << 6) | (s[p + 1] as u32 & 0x3f);
        if ch < 0x80 {
            return fail(pos, 2);
        }
        *pos = p + 2;
        Some(ch)
    } else if c < 0xF0 {
        if avail < 3 || !trail(s[p + 1]) || !trail(s[p + 2]) {
            return if avail < 2 || lead(s[p + 1]) {
                fail(pos, 1)
            } else if avail < 3 || lead(s[p + 2]) {
                fail(pos, 2)
            } else {
                fail(pos, 3)
            };
        }
        let ch =
            ((c as u32 & 0x0f) << 12) | ((s[p + 1] as u32 & 0x3f) << 6) | (s[p + 2] as u32 & 0x3f);
        if ch < 0x800 || (0xd800..=0xdfff).contains(&ch) {
            return fail(pos, 3);
        }
        *pos = p + 3;
        Some(ch)
    } else if c < 0xF5 {
        if avail < 4 || !trail(s[p + 1]) || !trail(s[p + 2]) || !trail(s[p + 3]) {
            return if avail < 2 || lead(s[p + 1]) {
                fail(pos, 1)
            } else if avail < 3 || lead(s[p + 2]) {
                fail(pos, 2)
            } else if avail < 4 || lead(s[p + 3]) {
                fail(pos, 3)
            } else {
                fail(pos, 4)
            };
        }
        let ch = ((c as u32 & 0x07) << 18)
            | ((s[p + 1] as u32 & 0x3f) << 12)
            | ((s[p + 2] as u32 & 0x3f) << 6)
            | (s[p + 3] as u32 & 0x3f);
        if !(0x10000..=0x10FFFF).contains(&ch) {
            return fail(pos, 4);
        }
        *pos = p + 4;
        Some(ch)
    } else {
        fail(pos, 1)
    }
}

/// xml.c xml_utf8_decode: UTF-8 to the target encoding, php's own
/// ISO-8859-1 and US-ASCII decoders ('?' for what they cannot hold).
fn xml_utf8_decode(s: &[u8], encoding: &str) -> Vec<u8> {
    let max = match encoding {
        "ISO-8859-1" => 0xFF,
        "US-ASCII" => 0x7F,
        _ => return s.to_vec(),
    };
    let mut out = Vec::with_capacity(s.len());
    let mut pos = 0;
    while pos < s.len() {
        let c = match next_utf8_char(s, &mut pos) {
            Some(c) if c <= 0xFF => c,
            _ => b'?' as u32,
        };
        out.push(if c > max { b'?' } else { c as u8 });
    }
    out
}

/// xml.c xml_xmlchar_zval: a string argument, or false for NULL.
fn xml_xmlchar_zval(s: Option<&[u8]>, encoding: &str) -> Value {
    match s {
        None => Value::Bool(false),
        Some(s) => Value::str(xml_utf8_decode(s, encoding)),
    }
}

/// xml.c xml_decode_tag
fn xml_decode_tag(p: &Parser, tag: &[u8]) -> Vec<u8> {
    let mut s = xml_utf8_decode(tag, p.target_encoding);
    if p.case_folding {
        s.make_ascii_uppercase();
    }
    s
}

/// xml.c xml_stripped_tag
fn xml_stripped_tag(tag: &[u8], offset: i32) -> Vec<u8> {
    let o = offset as usize;
    if o == 0 {
        tag.to_vec()
    } else if o >= tag.len() {
        Vec::new()
    } else {
        tag[o..].to_vec()
    }
}

// ---------------------------------------------------------------------------
// the handlers that call PHP (xml.c)
// ---------------------------------------------------------------------------

/// zend_call_known_fcc: a no-op while an exception is pending.
fn call_fcc(fcc: &Fcc, args: Vec<Value>) -> Option<Value> {
    if ffi::aborted() {
        return None;
    }
    ffi::call_host(|h| match fcc {
        Fcc::Callable(cb) => h.call_callable(cb, &args),
        Fcc::Method { object, name } => {
            let class = h.object_class_name(object).unwrap_or_default();
            h.reflect_invoke(object, &class, name, crate::host::list_value(args))
        }
    })
}

/// The handler in slot `slot` and the parser object, if the handler is set.
fn handler(p: &RefCell<Parser>, slot: usize) -> Option<(Fcc, Value)> {
    let s = p.borrow();
    let fcc = s.h[slot].clone()?;
    let index = s.index.upgrade().map(Value::Object)?;
    Some((fcc, index))
}

/// php_error_docref from a handler: the running xml_* function's warning.
fn handler_warning(msg: &str) {
    let fname = ffi::fname();
    ffi::call_host(|h| crate::warn(h, &fname, msg));
}

fn data_array<R>(p: &RefCell<Parser>, f: impl FnOnce(&mut PhpArray) -> R) -> Option<R> {
    let data = p.borrow().data.clone()?;
    let mut v = data.borrow_mut();
    match &mut *v {
        Value::Array(a) => Some(f(a)),
        _ => None,
    }
}

fn info_add(p: &RefCell<Parser>, name: &[u8]) {
    let (info, curtag) = {
        let s = p.borrow();
        (s.info.clone(), s.curtag)
    };
    let Some(info) = info else { return };
    {
        let mut v = info.borrow_mut();
        let Value::Array(a) = &mut *v else { return };
        let slot = a.get_or_insert_mut(ArrayKey::from_bytes(name));
        if !matches!(slot, Value::Array(_)) {
            *slot = Value::Array(PhpArray::new());
        }
        if let Value::Array(list) = slot {
            list.push(Value::Int(curtag));
        }
    }
    p.borrow_mut().curtag += 1;
}

fn str_key(k: &[u8]) -> ArrayKey {
    ArrayKey::from_bytes(k)
}

/// xml.c xml_startElementHandler
pub(crate) fn start_element_handler(
    p: &RefCell<Parser>,
    name: &[u8],
    attributes: &[(Vec<u8>, Vec<u8>)],
) {
    if ffi::aborted() {
        return;
    }
    let (tag_name, attrs, toffset) = {
        let mut s = p.borrow_mut();
        s.level += 1;
        let tag = xml_decode_tag(&s, name);
        let mut a = PhpArray::new();
        for (k, v) in attributes {
            a.insert(
                str_key(&xml_decode_tag(&s, k)),
                Value::str(xml_utf8_decode(v, s.target_encoding)),
            );
        }
        (tag, a, s.toffset)
    };
    if let Some((fcc, index)) = handler(p, H_START) {
        call_fcc(
            &fcc,
            vec![
                index,
                Value::str(xml_stripped_tag(&tag_name, toffset)),
                Value::Array(attrs.clone()),
            ],
        );
    }
    let has_data = p.borrow().data.is_some();
    if !has_data || ffi::aborted() {
        return;
    }
    let level = p.borrow().level;
    if level <= XML_MAXLEVEL {
        let toffset = p.borrow().toffset;
        let stripped = xml_stripped_tag(&tag_name, toffset);
        info_add(p, &stripped);
        let mut tag = PhpArray::new();
        tag.insert(str_key(b"tag"), Value::str(stripped));
        tag.insert(str_key(b"type"), Value::str("open"));
        tag.insert(str_key(b"level"), Value::Int(level as i64));
        {
            let mut s = p.borrow_mut();
            if let Some(lt) = s.ltags.as_mut() {
                lt[(level - 1) as usize] = Some(PhpStr::from_bytes(tag_name.clone()));
            }
            s.lastwasopen = true;
        }
        if !attrs.is_empty() {
            tag.insert(str_key(b"attributes"), Value::Array(attrs));
        }
        let idx = data_array(p, |a| {
            let before = a.len();
            a.push(Value::Array(tag));
            (a.len() > before)
                .then(|| a.nth(a.len() - 1).map(|(k, _)| k.clone()))
                .flatten()
        })
        .flatten();
        if let Some(ArrayKey::Int(i)) = idx {
            p.borrow_mut().ctag_index = i;
        }
    } else if level == XML_MAXLEVEL + 1 {
        handler_warning("Maximum depth exceeded - Results truncated");
    }
}

/// xml.c xml_endElementHandler
pub(crate) fn end_element_handler(p: &RefCell<Parser>, name: &[u8]) {
    if ffi::aborted() {
        return;
    }
    let (tag_name, toffset) = {
        let s = p.borrow();
        (xml_decode_tag(&s, name), s.toffset)
    };
    if let Some((fcc, index)) = handler(p, H_END) {
        call_fcc(
            &fcc,
            vec![index, Value::str(xml_stripped_tag(&tag_name, toffset))],
        );
    }
    let has_data = p.borrow().data.is_some();
    if has_data && !ffi::aborted() {
        let (lastwasopen, ctag, level, toffset) = {
            let s = p.borrow();
            (s.lastwasopen, s.ctag_index, s.level, s.toffset)
        };
        if lastwasopen {
            data_array(p, |a| {
                if let Some(Value::Array(t)) = a.get_mut(&ArrayKey::Int(ctag)) {
                    t.insert(str_key(b"type"), Value::str("complete"));
                }
            });
        } else {
            let stripped = xml_stripped_tag(&tag_name, toffset);
            info_add(p, &stripped);
            let mut tag = PhpArray::new();
            tag.insert(str_key(b"tag"), Value::str(stripped));
            tag.insert(str_key(b"type"), Value::str("close"));
            tag.insert(str_key(b"level"), Value::Int(level as i64));
            data_array(p, |a| a.push(Value::Array(tag)));
        }
        p.borrow_mut().lastwasopen = false;
    }
    let mut s = p.borrow_mut();
    let level = s.level;
    if (1..=XML_MAXLEVEL).contains(&level) {
        if let Some(lt) = s.ltags.as_mut() {
            lt[(level - 1) as usize] = None;
        }
    }
    s.level -= 1;
}

/// xml.c xml_characterDataHandler
pub(crate) fn character_data_handler(p: &RefCell<Parser>, s: &[u8]) {
    if ffi::aborted() {
        return;
    }
    let enc = p.borrow().target_encoding;
    if let Some((fcc, index)) = handler(p, H_CDATA) {
        call_fcc(&fcc, vec![index, xml_xmlchar_zval(Some(s), enc)]);
    }
    let has_data = p.borrow().data.is_some();
    if !has_data || ffi::aborted() {
        return;
    }
    let decoded = xml_utf8_decode(s, enc);
    let (skipwhite, lastwasopen, ctag, level, toffset) = {
        let s = p.borrow();
        (s.skipwhite, s.lastwasopen, s.ctag_index, s.level, s.toffset)
    };
    let doprint = skipwhite && decoded.iter().any(|c| !matches!(c, b' ' | b'\t' | b'\n'));
    if lastwasopen {
        data_array(p, |a| {
            let Some(Value::Array(t)) = a.get_mut(&ArrayKey::Int(ctag)) else {
                return;
            };
            match t.get_mut(&str_key(b"value")) {
                // The current tag already has a value: append to it.
                Some(Value::Str(v)) => v.make_mut().extend_from_slice(&decoded),
                _ => {
                    if doprint || !skipwhite {
                        t.insert(str_key(b"value"), Value::str(decoded.clone()));
                    }
                }
            }
        });
        return;
    }
    // Append to a trailing cdata entry.
    let appended = data_array(p, |a| {
        let n = a.len();
        if n == 0 {
            return false;
        }
        let key = a.nth(n - 1).map(|(k, _)| k.clone());
        let Some(key) = key else { return false };
        let Some(Value::Array(t)) = a.get_mut(&key) else {
            return false;
        };
        if !matches!(t.get(&str_key(b"type")), Some(Value::Str(ty)) if ty.as_bytes() == b"cdata") {
            return false;
        }
        match t.get_mut(&str_key(b"value")) {
            Some(Value::Str(v)) => {
                v.make_mut().extend_from_slice(&decoded);
                true
            }
            _ => false,
        }
    });
    match appended {
        None | Some(true) => return,
        Some(false) => {}
    }
    if level <= XML_MAXLEVEL && level > 0 && (doprint || !skipwhite) {
        let ltag = p
            .borrow()
            .ltags
            .as_ref()
            .and_then(|lt| lt[(level - 1) as usize].clone())
            .unwrap_or_default();
        let stripped = xml_stripped_tag(ltag.as_bytes(), toffset);
        info_add(p, &stripped);
        let mut tag = PhpArray::new();
        tag.insert(str_key(b"tag"), Value::str(stripped));
        tag.insert(str_key(b"value"), Value::str(decoded));
        tag.insert(str_key(b"type"), Value::str("cdata"));
        tag.insert(str_key(b"level"), Value::Int(level as i64));
        data_array(p, |a| a.push(Value::Array(tag)));
    } else if level == XML_MAXLEVEL + 1 {
        handler_warning("Maximum depth exceeded - Results truncated");
    }
}

/// xml.c xml_processingInstructionHandler
pub(crate) fn processing_instruction_handler(
    p: &RefCell<Parser>,
    target: Option<&[u8]>,
    data: Option<&[u8]>,
) {
    let Some((fcc, index)) = handler(p, H_PI) else {
        return;
    };
    let enc = p.borrow().target_encoding;
    call_fcc(
        &fcc,
        vec![
            index,
            xml_xmlchar_zval(target, enc),
            xml_xmlchar_zval(data, enc),
        ],
    );
}

/// xml.c xml_defaultHandler
pub(crate) fn default_handler(p: &RefCell<Parser>, s: &[u8]) {
    let Some((fcc, index)) = handler(p, H_DEFAULT) else {
        return;
    };
    let enc = p.borrow().target_encoding;
    call_fcc(&fcc, vec![index, xml_xmlchar_zval(Some(s), enc)]);
}

/// xml.c xml_unparsedEntityDeclHandler
pub(crate) fn unparsed_entity_decl_handler(p: &RefCell<Parser>, f: [Option<&[u8]>; 5]) {
    let Some((fcc, index)) = handler(p, H_UNPARSED) else {
        return;
    };
    let enc = p.borrow().target_encoding;
    let mut args = vec![index];
    args.extend(f.iter().map(|s| xml_xmlchar_zval(*s, enc)));
    call_fcc(&fcc, args);
}

/// xml.c xml_notationDeclHandler
pub(crate) fn notation_decl_handler(p: &RefCell<Parser>, f: [Option<&[u8]>; 4]) {
    let Some((fcc, index)) = handler(p, H_NOTATION) else {
        return;
    };
    let enc = p.borrow().target_encoding;
    let mut args = vec![index];
    args.extend(f.iter().map(|s| xml_xmlchar_zval(*s, enc)));
    call_fcc(&fcc, args);
}

/// xml.c xml_externalEntityRefHandler: nonzero to continue.
pub(crate) fn external_entity_ref_handler(p: &RefCell<Parser>, f: [Option<&[u8]>; 4]) -> i64 {
    let Some((fcc, index)) = handler(p, H_EXTENT) else {
        return 0;
    };
    let enc = p.borrow().target_encoding;
    let mut args = vec![index];
    args.extend(f.iter().map(|s| xml_xmlchar_zval(*s, enc)));
    match call_fcc(&fcc, args) {
        Some(v) => conv::to_int(&deref(&v)),
        None => 0,
    }
}

/// xml.c xml_startNamespaceDeclHandler
pub(crate) fn start_namespace_decl_handler(
    p: &RefCell<Parser>,
    prefix: Option<&[u8]>,
    uri: Option<&[u8]>,
) {
    let Some((fcc, index)) = handler(p, H_START_NS) else {
        return;
    };
    let enc = p.borrow().target_encoding;
    call_fcc(
        &fcc,
        vec![
            index,
            xml_xmlchar_zval(prefix, enc),
            xml_xmlchar_zval(uri, enc),
        ],
    );
}

// ---------------------------------------------------------------------------
// the functions
// ---------------------------------------------------------------------------

/// php_xml_parser_create_impl
fn create(host: &mut dyn Host, args: &[Value], ns_support: bool) -> Result<Value, Thrown> {
    let fname = if ns_support {
        "xml_parser_create_ns"
    } else {
        "xml_parser_create"
    };
    let encoding = match arg(args, 0) {
        Value::Null => "UTF-8",
        v => {
            let e = conv::to_string_bytes(&v);
            if e.is_empty() {
                // Auto-detect: the target stays UTF-8.
                "UTF-8"
            } else {
                match xml_get_encoding(&e) {
                    Some(e) => e,
                    None => {
                        return Err(crate::zend::zpp::value_error(
                            fname,
                            1,
                            "encoding",
                            "is not a supported source encoding",
                        ))
                    }
                }
            }
        }
    };
    let separator = ns_support.then(|| match args.get(1) {
        Some(v) => conv::to_string_bytes(&deref(v)),
        None => b":".to_vec(),
    });
    let Some(obj) = host.instantiate(b"XMLParser", Vec::new()) else {
        return Err(Thrown::error("Error", "XMLParser is not available"));
    };
    let Value::Object(h) = &obj else {
        return Err(Thrown::error("Error", "XMLParser is not available"));
    };
    let parser = Rc::new(RefCell::new(Parser {
        ctxt: std::ptr::null_mut(),
        use_namespace: false,
        ns_separator: Vec::new(),
        c: CHandlers::default(),
        target_encoding: encoding,
        index: Rc::downgrade(h),
        object: None,
        h: Default::default(),
        data: None,
        info: None,
        level: 0,
        toffset: 0,
        curtag: 0,
        ctag_index: 0,
        ltags: None,
        lastwasopen: false,
        skipwhite: false,
        isparsing: false,
        parsehuge: false,
        case_folding: true,
    }));
    compat::parser_create(&parser, separator);
    LIVE.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|p| p.strong_count() > 0);
        l.push(Rc::downgrade(&parser));
    });
    crate::zend::object_handlers::set_payload(h, parser);
    Ok(obj)
}

fn fn_parser_create(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    create(host, args, false)
}

fn fn_parser_create_ns(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    create(host, args, true)
}

/// php_xml_check_string_method_arg: a method named by a string on the
/// `xml_set_object` object. `Ok(None)` unsets the handler.
fn check_string_method_arg(
    host: &mut dyn Host,
    fname: &str,
    arg_num: usize,
    pname: &str,
    object: Option<&Value>,
    method: &[u8],
) -> Result<Option<Fcc>, Thrown> {
    if method.is_empty() {
        return Ok(None);
    }
    let Some(object) = object else {
        return Err(crate::zend::zpp::value_error(
            fname,
            arg_num,
            pname,
            "an object must be set via xml_set_object() to be able to lookup method",
        ));
    };
    if !object_has_method(host, object, method) {
        let class = host.object_class_name(object).unwrap_or_default();
        return Err(crate::zend::zpp::value_error(
            fname,
            arg_num,
            pname,
            &format!(
                "method {}::{}() does not exist",
                String::from_utf8_lossy(&class),
                String::from_utf8_lossy(method)
            ),
        ));
    }
    Ok(Some(Fcc::Method {
        object: object.clone(),
        name: method.to_vec(),
    }))
}

fn object_has_method(host: &dyn Host, object: &Value, method: &[u8]) -> bool {
    let Value::Object(h) = object else {
        return false;
    };
    host.class_has_method(h.borrow().class, &method.to_ascii_lowercase())
}

/// zpp `F!`: a callable or null (`Some(None)`), else `None`.
fn parse_callable(host: &mut dyn Host, v: &Value) -> Result<Option<Option<Fcc>>, Thrown> {
    if matches!(v, Value::Null) {
        return Ok(Some(None));
    }
    Ok(host
        .check_callable(v)?
        .is_none()
        .then(|| Some(Fcc::Callable(v.clone()))))
}

/// zpp `S`: a string, else `None`. Coercive mode also takes scalars, and
/// null as "" with php's deprecation.
fn parse_string(
    host: &mut dyn Host,
    fname: &str,
    n: usize,
    pname: &str,
    v: &Value,
) -> Result<Option<Vec<u8>>, Thrown> {
    if host.caller_strict_types() {
        return Ok(match v {
            Value::Str(s) => Some(s.as_bytes().to_vec()),
            _ => None,
        });
    }
    Ok(match v {
        Value::Str(s) => Some(s.as_bytes().to_vec()),
        Value::Int(_) | Value::Float(_) | Value::Bool(_) => Some(conv::to_string_bytes(v)),
        Value::Null => {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                format!("{fname}(): Passing null to parameter #{n} (${pname}) of type string is deprecated").as_bytes(),
            )?;
            Some(Vec::new())
        }
        Value::Object(_) => crate::zend::object_handlers::to_string(host, v)?,
        _ => None,
    })
}

const NON_CALLABLE_DEPRECATION: &str = "Passing non-callable strings is deprecated since 8.4";

/// xml_set_element_handler: php tries the parameter forms "OF!F!", "OF!S",
/// "OSF!" and "OSS" in turn.
fn fn_set_element_handler(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const F: &str = "xml_set_element_handler";
    const START: &str = "start_handler";
    const END: &str = "end_handler";
    let p = parser_arg(host, args, 0, F)?;
    let (start, end) = (arg(args, 1), arg(args, 2));
    let object = p.borrow().object.clone();
    let start_callable = parse_callable(host, &start)?;
    let both_callable = match &start_callable {
        Some(s) => parse_callable(host, &end)?.map(|e| (s.clone(), e)),
        None => None,
    };
    let (start_fcc, end_fcc) = if let Some(pair) = both_callable {
        pair
    } else if let Some((s, e)) = match &start_callable {
        Some(s) => parse_string(host, F, 3, END, &end)?.map(|e| (s.clone(), e)),
        None => None,
    } {
        crate::deprecated(host, F, NON_CALLABLE_DEPRECATION)?;
        (
            s,
            check_string_method_arg(host, F, 3, END, object.as_ref(), &e)?,
        )
    } else if let Some((s, e)) = match parse_string(host, F, 2, START, &start)? {
        Some(s) => parse_callable(host, &end)?.map(|e| (s, e)),
        None => None,
    } {
        crate::deprecated(host, F, NON_CALLABLE_DEPRECATION)?;
        (
            check_string_method_arg(host, F, 2, START, object.as_ref(), &s)?,
            e,
        )
    } else if let Some((s, e)) = match parse_string(host, F, 2, START, &start)? {
        Some(s) => parse_string(host, F, 3, END, &end)?.map(|e| (s, e)),
        None => None,
    } {
        crate::deprecated(host, F, NON_CALLABLE_DEPRECATION)?;
        let s = check_string_method_arg(host, F, 2, START, object.as_ref(), &s)?;
        (
            s,
            check_string_method_arg(host, F, 3, END, object.as_ref(), &e)?,
        )
    } else if !matches!(start, Value::Null | Value::Str(_)) {
        return Err(Thrown::type_error(format!(
            "{F}(): Argument #2 ($start_handler) must be of type callable|string|null"
        )));
    } else {
        return Err(Thrown::type_error(format!(
            "{F}(): Argument #3 ($end_handler) must be of type callable|string|null"
        )));
    };
    let mut s = p.borrow_mut();
    if s.isparsing {
        return Err(Thrown::error(
            "Error",
            "Cannot change handlers while parsing",
        ));
    }
    s.h[H_START] = start_fcc;
    s.h[H_END] = end_fcc;
    s.c.element = true;
    Ok(Value::Bool(true))
}

/// php_xml_set_handler_parse_callable plus XML_SET_HANDLER_PHP_FUNCTION.
fn set_handler(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    slot: usize,
) -> Result<Value, Thrown> {
    let p = parser_arg(host, args, 0, fname)?;
    let v = arg(args, 1);
    let fcc = if let Some(f) = parse_callable(host, &v)? {
        f
    } else if let Some(name) = match &v {
        Value::Null => None,
        other => parse_string(host, fname, 2, "handler", other)?,
    } {
        crate::deprecated(host, fname, NON_CALLABLE_DEPRECATION)?;
        let object = p.borrow().object.clone();
        check_string_method_arg(host, fname, 2, "handler", object.as_ref(), &name)?
    } else {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #2 ($handler) must be of type callable|string|null"
        )));
    };
    let mut s = p.borrow_mut();
    if s.isparsing {
        return Err(Thrown::error(
            "Error",
            "Cannot change handlers while parsing",
        ));
    }
    s.h[slot] = fcc;
    match slot {
        H_CDATA => s.c.cdata = true,
        H_PI => s.c.pi = true,
        H_DEFAULT => s.c.default = true,
        H_UNPARSED => s.c.unparsed = true,
        H_NOTATION => s.c.notation = true,
        H_EXTENT => s.c.extent = true,
        H_START_NS => s.c.start_ns = true,
        H_END_NS => s.c.end_ns = true,
        _ => {}
    }
    Ok(Value::Bool(true))
}

macro_rules! setter {
    ($fn:ident, $name:literal, $slot:expr) => {
        fn $fn(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            set_handler(host, args, $name, $slot)
        }
    };
}
setter!(
    fn_set_character_data_handler,
    "xml_set_character_data_handler",
    H_CDATA
);
setter!(
    fn_set_processing_instruction_handler,
    "xml_set_processing_instruction_handler",
    H_PI
);
setter!(fn_set_default_handler, "xml_set_default_handler", H_DEFAULT);
setter!(
    fn_set_unparsed_entity_decl_handler,
    "xml_set_unparsed_entity_decl_handler",
    H_UNPARSED
);
setter!(
    fn_set_notation_decl_handler,
    "xml_set_notation_decl_handler",
    H_NOTATION
);
setter!(
    fn_set_external_entity_ref_handler,
    "xml_set_external_entity_ref_handler",
    H_EXTENT
);
setter!(
    fn_set_start_namespace_decl_handler,
    "xml_set_start_namespace_decl_handler",
    H_START_NS
);
setter!(
    fn_set_end_namespace_decl_handler,
    "xml_set_end_namespace_decl_handler",
    H_END_NS
);

/// xml_set_object, with PHP_XML_CHECK_NEW_THIS_METHODS.
fn fn_set_object(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const F: &str = "xml_set_object";
    // Reached through the prelude's wrapper, not the VM's builtin call.
    crate::deprecated_function(host, F)?;
    let p = parser_arg(host, args, 0, F)?;
    let new_this = arg(args, 1);
    if !matches!(new_this, Value::Object(_)) {
        return Err(Thrown::type_error(format!(
            "{F}(): Argument #2 ($object) must be of type object, {} given",
            crate::zend::zpp::zval_value_name(host, &new_this)
        )));
    }
    const SET_VIA: [&str; 10] = [
        "xml_set_element_handler()",
        "xml_set_element_handler()",
        "xml_set_character_data_handler()",
        "xml_set_processing_instruction_handler()",
        "xml_set_default_handler()",
        "xml_set_unparsed_entity_decl_handler()",
        "xml_set_notation_decl_handler()",
        "xml_set_external_entity_ref_handler()",
        "xml_set_start_namespace_decl_handler()",
        "xml_set_end_namespace_decl_handler()",
    ];
    let old = p.borrow().object.clone();
    if let Some(Value::Object(old_h)) = &old {
        for (slot, via) in SET_VIA.iter().enumerate() {
            let fcc = p.borrow().h[slot].clone();
            let Some(Fcc::Method {
                object: Value::Object(o),
                name,
            }) = fcc
            else {
                continue;
            };
            if !Rc::ptr_eq(&o, old_h) {
                continue;
            }
            if !object_has_method(host, &new_this, &name) {
                // zend_fcc_dtor ran before the check: the slot stays empty.
                p.borrow_mut().h[slot] = None;
                let class = host.object_class_name(&new_this).unwrap_or_default();
                return Err(crate::zend::zpp::value_error(
                    F,
                    2,
                    "object",
                    &format!(
                        "cannot safely swap to object of class {} as method \"{}\" does not exist, which was set via {via}",
                        String::from_utf8_lossy(&class),
                        String::from_utf8_lossy(&name)
                    ),
                ));
            }
            p.borrow_mut().h[slot] = Some(Fcc::Method {
                object: new_this.clone(),
                name,
            });
        }
    }
    p.borrow_mut().object = Some(new_this);
    Ok(Value::Bool(true))
}

/// xml_parse_helper
fn parse_helper(
    host: &mut dyn Host,
    fname: &str,
    p: &ParserRef,
    data: &[u8],
    is_final: bool,
) -> Result<i64, Thrown> {
    let ctxt = {
        let mut s = p.borrow_mut();
        unsafe { sys::rhp_xml_ctxt_set_huge(s.ctxt, s.parsehuge as c_int) };
        s.isparsing = true;
        s.ctxt
    };
    let r = ffi::with_host(host, fname, || {
        ffi::set_stop(move || unsafe { sys::xmlStopParser(ctxt) });
        compat::xml_parse(ctxt, data, is_final)
    });
    p.borrow_mut().isparsing = false;
    r.map(|v| v as i64)
}

fn fn_parse(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const F: &str = "xml_parse";
    let p = parser_arg(host, args, 0, F)?;
    let data = conv::to_string_bytes(&arg(args, 1));
    let is_final = conv::to_bool(&arg(args, 2));
    if p.borrow().isparsing {
        return Err(Thrown::error(
            "Error",
            "Parser must not be called recursively",
        ));
    }
    Ok(Value::Int(parse_helper(host, F, &p, &data, is_final)?))
}

/// zend_try_array_init on a by-reference argument.
fn try_array_init(args: &[Value], i: usize) -> PhpRef {
    match args.get(i) {
        Some(Value::Ref(r)) => {
            *r.borrow_mut() = Value::Array(PhpArray::new());
            r.clone()
        }
        _ => Rc::new(RefCell::new(Value::Array(PhpArray::new()))),
    }
}

/// Native, unlike the other `xml_*` functions: the VM does not bind a PHP
/// function's by-reference parameters passed by name, so a wrapper would
/// drop `values:` and `index:` silently, where a builtin refuses named
/// arguments.
fn fn_parse_into_struct(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const F: &str = "xml_parse_into_struct";
    let p = parser_arg(host, args, 0, F)?;
    let data = conv::to_string_bytes(&arg(args, 1));
    if p.borrow().isparsing {
        crate::warn(host, F, "Parser must not be called recursively")?;
        return Ok(Value::Bool(false));
    }
    let info = (args.len() > 3).then(|| try_array_init(args, 3));
    let values = try_array_init(args, 2);
    {
        let mut s = p.borrow_mut();
        s.data = Some(values);
        if info.is_some() {
            s.info = info;
        }
        s.level = 0;
        s.ltags = Some(vec![None; XML_MAXLEVEL as usize]);
        s.c.element = true;
        s.c.cdata = true;
    }
    Ok(Value::Int(parse_helper(host, F, &p, &data, true)?))
}

fn fn_get_error_code(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let p = parser_arg(host, args, 0, "xml_get_error_code")?;
    let ctxt = p.borrow().ctxt;
    Ok(Value::Int(unsafe { sys::rhp_xml_ctxt_errno(ctxt) } as i64))
}

fn fn_error_string(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = long_arg(host, args, 0, "xml_error_string", "error_code")?;
    Ok(Value::str(compat::error_string(code)))
}

fn position(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    which: usize,
) -> Result<Value, Thrown> {
    let p = parser_arg(host, args, 0, fname)?;
    let ctxt = p.borrow().ctxt;
    let (mut file, mut line, mut col) = (std::ptr::null(), 0, 0);
    Ok(Value::Int(unsafe {
        match which {
            0 | 1 => {
                sys::rhp_xml_ctxt_input_pos(ctxt, &mut file, &mut line, &mut col);
                if which == 0 {
                    line as i64
                } else {
                    col as i64
                }
            }
            _ => sys::rhp_xml_ctxt_byte_index(ctxt) as i64,
        }
    }))
}

fn fn_get_current_line_number(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    position(host, args, "xml_get_current_line_number", 0)
}

fn fn_get_current_column_number(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    position(host, args, "xml_get_current_column_number", 1)
}

fn fn_get_current_byte_index(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    position(host, args, "xml_get_current_byte_index", 2)
}

fn fn_parser_free(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const F: &str = "xml_parser_free";
    crate::deprecated_function(host, F)?;
    let p = parser_arg(host, args, 0, F)?;
    if p.borrow().isparsing {
        crate::warn(host, F, "Parser cannot be freed while it is parsing")?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_parser_set_option(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const F: &str = "xml_parser_set_option";
    let p = parser_arg(host, args, 0, F)?;
    let opt = long_arg(host, args, 1, F, "option")?;
    let value = arg(args, 2);
    if !matches!(value, Value::Bool(_) | Value::Int(_) | Value::Str(_)) {
        let msg = format!(
            "Argument #3 ($value) must be of type string|int|bool, {} given",
            crate::zend::zpp::zval_type_name(host, &value)
        );
        crate::warn(host, F, &msg)?;
    }
    match opt {
        OPT_CASE_FOLDING => p.borrow_mut().case_folding = conv::to_bool(&value),
        OPT_SKIP_WHITE => p.borrow_mut().skipwhite = conv::to_bool(&value),
        OPT_PARSE_HUGE => {
            if p.borrow().isparsing {
                return Err(Thrown::error(
                    "Error",
                    "Cannot change option XML_OPTION_PARSE_HUGE while parsing",
                ));
            }
            p.borrow_mut().parsehuge = conv::to_bool(&value);
        }
        OPT_SKIP_TAGSTART => {
            let v = crate::zend::zpp::zval_get_long(host, &value)?;
            if !(0..=i32::MAX as i64).contains(&v) {
                crate::warn(
                    host,
                    F,
                    &format!(
                        "Argument #3 ($value) must be between 0 and {} for option XML_OPTION_SKIP_TAGSTART",
                        i32::MAX
                    ),
                )?;
                return Ok(Value::Bool(false));
            }
            p.borrow_mut().toffset = v as i32;
        }
        OPT_TARGET_ENCODING => {
            let s = crate::ext::libxml::io::to_string(host, &value)?;
            match xml_get_encoding(&s) {
                Some(e) => p.borrow_mut().target_encoding = e,
                None => {
                    return Err(crate::zend::zpp::value_error(
                        F,
                        3,
                        "value",
                        "is not a supported target encoding",
                    ))
                }
            }
        }
        _ => {
            return Err(crate::zend::zpp::value_error(
                F,
                2,
                "option",
                "must be a XML_OPTION_* constant",
            ))
        }
    }
    Ok(Value::Bool(true))
}

fn fn_parser_get_option(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const F: &str = "xml_parser_get_option";
    let p = parser_arg(host, args, 0, F)?;
    let opt = long_arg(host, args, 1, F, "option")?;
    let s = p.borrow();
    Ok(match opt {
        OPT_CASE_FOLDING => Value::Bool(s.case_folding),
        OPT_SKIP_TAGSTART => Value::Int(s.toffset as i64),
        OPT_SKIP_WHITE => Value::Bool(s.skipwhite),
        OPT_PARSE_HUGE => Value::Bool(s.parsehuge),
        OPT_TARGET_ENCODING => Value::str(s.target_encoding),
        _ => {
            return Err(crate::zend::zpp::value_error(
                F,
                2,
                "option",
                "must be a XML_OPTION_* constant",
            ))
        }
    })
}

/// The `xml_*` functions the prelude's PHP wrappers call through `__xml_fn`
/// (so that named arguments and typed parameters work); all but
/// `xml_parse_into_struct`.
const FUNCTIONS: &[(&str, crate::NativeFn)] = &[
    ("xml_parser_create", fn_parser_create),
    ("xml_parser_create_ns", fn_parser_create_ns),
    ("xml_set_object", fn_set_object),
    ("xml_set_element_handler", fn_set_element_handler),
    (
        "xml_set_character_data_handler",
        fn_set_character_data_handler,
    ),
    (
        "xml_set_processing_instruction_handler",
        fn_set_processing_instruction_handler,
    ),
    ("xml_set_default_handler", fn_set_default_handler),
    (
        "xml_set_unparsed_entity_decl_handler",
        fn_set_unparsed_entity_decl_handler,
    ),
    (
        "xml_set_notation_decl_handler",
        fn_set_notation_decl_handler,
    ),
    (
        "xml_set_external_entity_ref_handler",
        fn_set_external_entity_ref_handler,
    ),
    (
        "xml_set_start_namespace_decl_handler",
        fn_set_start_namespace_decl_handler,
    ),
    (
        "xml_set_end_namespace_decl_handler",
        fn_set_end_namespace_decl_handler,
    ),
    ("xml_parse", fn_parse),
    ("xml_get_error_code", fn_get_error_code),
    ("xml_error_string", fn_error_string),
    ("xml_get_current_line_number", fn_get_current_line_number),
    (
        "xml_get_current_column_number",
        fn_get_current_column_number,
    ),
    ("xml_get_current_byte_index", fn_get_current_byte_index),
    ("xml_parser_free", fn_parser_free),
    ("xml_parser_set_option", fn_parser_set_option),
    ("xml_parser_get_option", fn_parser_get_option),
];

/// `__xml_fn($function, ...$args)`.
fn fn_dispatch(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&arg(args, 0));
    let Some(&(_, f)) = FUNCTIONS.iter().find(|(n, _)| n.as_bytes() == name) else {
        return Err(Thrown::error(
            "Error",
            format!(
                "Call to undefined function {}()",
                String::from_utf8_lossy(&name)
            ),
        ));
    };
    f(host, &args[1..])
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("xml_parse_into_struct", fn_parse_into_struct)]);
    crate::register_helpers(m, &[("__xml_fn", fn_dispatch, 1, None)]);
}

pub fn constants() -> Vec<(&'static str, Value)> {
    let i = Value::Int;
    vec![
        ("XML_ERROR_NONE", i(0)),
        ("XML_ERROR_NO_MEMORY", i(1)),
        ("XML_ERROR_SYNTAX", i(2)),
        ("XML_ERROR_NO_ELEMENTS", i(3)),
        ("XML_ERROR_INVALID_TOKEN", i(4)),
        ("XML_ERROR_UNCLOSED_TOKEN", i(5)),
        ("XML_ERROR_PARTIAL_CHAR", i(6)),
        ("XML_ERROR_TAG_MISMATCH", i(7)),
        ("XML_ERROR_DUPLICATE_ATTRIBUTE", i(8)),
        ("XML_ERROR_JUNK_AFTER_DOC_ELEMENT", i(9)),
        ("XML_ERROR_PARAM_ENTITY_REF", i(10)),
        ("XML_ERROR_UNDEFINED_ENTITY", i(11)),
        ("XML_ERROR_RECURSIVE_ENTITY_REF", i(12)),
        ("XML_ERROR_ASYNC_ENTITY", i(13)),
        ("XML_ERROR_BAD_CHAR_REF", i(14)),
        ("XML_ERROR_BINARY_ENTITY_REF", i(15)),
        ("XML_ERROR_ATTRIBUTE_EXTERNAL_ENTITY_REF", i(16)),
        ("XML_ERROR_MISPLACED_XML_PI", i(17)),
        ("XML_ERROR_UNKNOWN_ENCODING", i(18)),
        ("XML_ERROR_INCORRECT_ENCODING", i(19)),
        ("XML_ERROR_UNCLOSED_CDATA_SECTION", i(20)),
        (
            "XML_ERROR_EXTERNAL_ENTITY_HANDLING",
            i(XML_ERROR_EXTERNAL_ENTITY_HANDLING as i64),
        ),
        ("XML_OPTION_CASE_FOLDING", i(OPT_CASE_FOLDING)),
        ("XML_OPTION_TARGET_ENCODING", i(OPT_TARGET_ENCODING)),
        ("XML_OPTION_SKIP_TAGSTART", i(OPT_SKIP_TAGSTART)),
        ("XML_OPTION_SKIP_WHITE", i(OPT_SKIP_WHITE)),
        ("XML_OPTION_PARSE_HUGE", i(OPT_PARSE_HUGE)),
        ("XML_SAX_IMPL", Value::str("libxml")),
    ]
}

pub const PRELUDE_PHP: &str = include_str!("prelude.php");
