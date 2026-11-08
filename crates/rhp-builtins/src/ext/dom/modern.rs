//! The `Dom\` API's own methods (php-src ext/dom: xml_document.c,
//! html_document.c, domimplementation.c and the modern halves of node.c,
//! document.c and element.c; PHP License 3.01). Methods php shares between
//! the two APIs stay in node.rs, element.rs and friends.

use std::ffi::c_int;
use std::ptr;
use std::rc::Rc;

use rhp_values::{conv, PhpArray, Value};

use super::nscompat::{self, DomPrivate, NsMapper};
use super::util::*;
use super::{
    arg, boolean, bytes, create_nullable, create_object, get_intern, get_obj, instantiate,
    intern_of, opt_bytes, throw, throw_msg, Cx, DomObject, INVALID_CHARACTER_ERR,
    INVALID_STATE_ERR, NOT_FOUND_ERR, NOT_SUPPORTED_ERR, SYNTAX_ERR, WRONG_DOCUMENT_ERR,
};
use crate::ext::libxml::node::{self as lxn, ClassType, PrivateHeader};
use crate::ext::libxml::sys::{self, XmlAttr, XmlDoc, XmlNode, XmlNs};
use crate::Thrown;

pub const INUSE_ATTRIBUTE_ERR: i64 = 10;

/// `Dom\HTML_NO_DEFAULT_NS`
pub const HTML_NO_DEFAULT_NS: i64 = 1 << 31;

// libxml2's parser options php checks.
const XML_PARSE_RECOVER: i64 = 1 << 0;
const XML_PARSE_NOENT: i64 = 1 << 1;
const XML_PARSE_DTDLOAD: i64 = 1 << 2;
const XML_PARSE_DTDATTR: i64 = 1 << 3;
const XML_PARSE_DTDVALID: i64 = 1 << 4;
const XML_PARSE_NOERROR: i64 = 1 << 5;
const XML_PARSE_NOWARNING: i64 = 1 << 6;
const XML_PARSE_PEDANTIC: i64 = 1 << 7;
const XML_PARSE_NOBLANKS: i64 = 1 << 8;
const XML_PARSE_XINCLUDE: i64 = 1 << 10;
const XML_PARSE_NONET: i64 = 1 << 11;
const XML_PARSE_NSCLEAN: i64 = 1 << 13;
const XML_PARSE_NOCDATA: i64 = 1 << 14;
const XML_PARSE_COMPACT: i64 = 1 << 16;
const XML_PARSE_IGNORE_ENC: i64 = 1 << 21;
const XML_PARSE_BIG_LINES: i64 = 1 << 22;
const XML_PARSE_NO_XXE: i64 = 1 << 23;
const XML_PARSE_HUGE: i64 = 1 << 19;
const HTML_PARSE_NOIMPLIED: i64 = 1 << 13;
const XML_SAVE_NO_DECL: i64 = 1 << 1;
const LIBXML_SAVE_NOEMPTYTAG: i64 = 1 << 2;

fn value_error(cx: &Cx, n: usize, pname: &str, msg: &str) -> Thrown {
    crate::zend::zpp::value_error(&cx.fname, n, pname, msg)
}

fn dom_err(cx: &mut Cx, code: i64) -> Thrown {
    match throw(cx, code, true) {
        Err(t) => t,
        Ok(_) => Thrown::error("Error", "DOM error"),
    }
}

fn dom_err_msg(cx: &mut Cx, code: i64, msg: &str) -> Thrown {
    match throw_msg(cx, code, msg, true) {
        Err(t) => t,
        Ok(_) => Thrown::error("Error", msg),
    }
}

/// The document's namespace mapper (php_dom_get_ns_mapper).
pub fn mapper(d: &DomObject) -> &'static mut NsMapper {
    nscompat::mapper(d.document()).expect("a modern document has private data")
}

pub fn private(d: &DomObject) -> Option<&'static mut DomPrivate> {
    nscompat::of(d.document())
}

pub fn is_modern(d: &DomObject) -> bool {
    nscompat::follow_spec(d.document())
}

/// php_dom_instantiate_object_helper + dom_set_xml_class + private data:
/// the object for a new modern document.
fn new_document_object(
    cx: &mut Cx,
    class: &[u8],
    doc: *mut XmlDoc,
    private: *mut PrivateHeader,
) -> Result<Value, Thrown> {
    let v = instantiate(cx, class, doc.cast(), ptr::null_mut())?;
    let d = intern_of(&v).expect("a DOM document");
    unsafe {
        let r = d.document();
        (*r).class_type = ClassType::Modern;
        (*r).private_data = private;
    }
    Ok(v)
}

// ---- Dom\XMLDocument ----

pub fn xml_create_empty(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let version = bytes(args, 0);
    let encoding = bytes(args, 1);
    if encoding.contains(&0) {
        return Err(value_error(
            cx,
            2,
            "encoding",
            "must not contain any null bytes",
        ));
    }
    let ce = c(&encoding);
    unsafe {
        let handler = sys::xmlFindCharEncodingHandler(ce.as_ptr().cast());
        if handler.is_null() {
            return Err(value_error(
                cx,
                2,
                "encoding",
                "is not a valid document encoding",
            ));
        }
        sys::xmlCharEncCloseFunc(handler);
        let cv = c(&version);
        let doc = sys::xmlNewDoc(cv.as_ptr());
        if doc.is_null() {
            return Err(dom_err(cx, INVALID_STATE_ERR));
        }
        (*doc).encoding = sys::xmlStrdup(ce.as_ptr());
        new_document_object(cx, b"Dom\\XMLDocument", doc, DomPrivate::create())
    }
}

const XML_VALID_OPTIONS: i64 = XML_PARSE_RECOVER
    | XML_PARSE_NOENT
    | XML_PARSE_NO_XXE
    | XML_PARSE_DTDLOAD
    | XML_PARSE_DTDATTR
    | XML_PARSE_DTDVALID
    | XML_PARSE_NOERROR
    | XML_PARSE_NOWARNING
    | XML_PARSE_NOBLANKS
    | XML_PARSE_XINCLUDE
    | XML_PARSE_NSCLEAN
    | XML_PARSE_NOCDATA
    | XML_PARSE_NONET
    | XML_PARSE_PEDANTIC
    | XML_PARSE_COMPACT
    | XML_PARSE_HUGE
    | XML_PARSE_BIG_LINES;

fn xml_load(cx: &mut Cx, args: &[Value], file: bool) -> Result<Value, Thrown> {
    let source = crate::ext::libxml::str_arg(args, 0);
    let source = source.as_bytes();
    let mut options = super::int(args, 1);
    let override_encoding = opt_bytes(args, 2);
    let pname = if file { "path" } else { "source" };
    if file && source.contains(&0) {
        return Err(value_error(cx, 1, pname, "must not contain any null bytes"));
    }
    if source.is_empty() {
        return Err(value_error(cx, 1, pname, "must not be empty"));
    }
    if source.len() > i32::MAX as usize {
        return Err(value_error(cx, 1, pname, "is too long"));
    }
    if file && source.windows(3).any(|w| w == b"%00") {
        return Err(value_error(
            cx,
            1,
            pname,
            "must not contain percent-encoded NUL bytes",
        ));
    }
    if options & !XML_VALID_OPTIONS != 0 {
        return Err(value_error(
            cx,
            2,
            "options",
            "contains invalid flags (allowed flags: LIBXML_RECOVER, LIBXML_NOENT, LIBXML_NO_XXE, LIBXML_DTDLOAD, LIBXML_DTDATTR, LIBXML_DTDVALID, LIBXML_NOERROR, LIBXML_NOWARNING, LIBXML_NOBLANKS, LIBXML_XINCLUDE, LIBXML_NSCLEAN, LIBXML_NOCDATA, LIBXML_NONET, LIBXML_PEDANTIC, LIBXML_COMPACT, LIBXML_PARSEHUGE, LIBXML_BIGLINES)",
        ));
    }
    let mut encoding: *mut sys::xmlCharEncodingHandler = ptr::null_mut();
    if let Some(e) = &override_encoding {
        if e.contains(&0) {
            return Err(value_error(
                cx,
                3,
                "overrideEncoding",
                "must not contain any null bytes",
            ));
        }
        let ce = c(e);
        encoding = unsafe { sys::xmlFindCharEncodingHandler(ce.as_ptr().cast()) };
        if encoding.is_null() {
            return Err(value_error(
                cx,
                3,
                "overrideEncoding",
                "must be a valid document encoding",
            ));
        }
        options |= XML_PARSE_IGNORE_ENC;
    }
    let mode = if file {
        super::document::Mode::File
    } else {
        super::document::Mode::String
    };
    let parsed = super::document::document_parser(
        cx,
        &lxn::DocProps::DEFAULT,
        mode,
        source,
        options,
        encoding,
    )?;
    let doc = match parsed {
        super::document::Parsed::Doc(d) if !d.is_null() => d,
        super::document::Parsed::Malformed => {
            return Err(dom_err_msg(
                cx,
                SYNTAX_ERR,
                "XML fragment is not well-formed",
            ));
        }
        _ => {
            if file {
                return Err(Thrown::error(
                    "Exception",
                    format!("Cannot open file '{}'", String::from_utf8_lossy(source)),
                ));
            }
            return Err(dom_err(cx, INVALID_STATE_ERR));
        }
    };
    unsafe {
        if (*doc).encoding.is_null() {
            let e = override_encoding.unwrap_or_else(|| b"UTF-8".to_vec());
            let ce = c(&e);
            (*doc).encoding = sys::xmlStrdup(ce.as_ptr());
        }
        if file && !(*doc).URL.is_null() {
            let url = b((*doc).URL).to_vec();
            if !is_stream_path(&url) && !url.starts_with(b"file:/") {
                let mut n = b"file://".to_vec();
                n.extend_from_slice(&url);
                let cn = c(&n);
                sys::xml_free((*doc).URL as *mut u8);
                (*doc).URL = sys::xmlStrdup(cn.as_ptr());
            }
        }
    }
    let v = instantiate(cx, b"Dom\\XMLDocument", doc.cast(), ptr::null_mut())?;
    let d = intern_of(&v).expect("a DOM document");
    unsafe { (*d.document()).class_type = ClassType::Modern };
    nscompat::convert_to_modern(d.document(), doc);
    Ok(v)
}

pub fn xml_create_from_string(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    xml_load(cx, args, false)
}

pub fn xml_create_from_file(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    xml_load(cx, args, true)
}

/// php_new_dom_dump_node_to_str and friends with the document's settings.
fn dump(
    d: &DomObject,
    node: *mut XmlNode,
    options: c_int,
    format: bool,
    encoding: *const u8,
) -> Result<Option<Vec<u8>>, Thrown> {
    let p = private(d).map(|p| &*p);
    super::xml_serializer::dump_to_str(node, options, format, encoding, false, p)
}

/// dom_document_save_xml (Dom\XMLDocument::saveXml, Dom\HTMLDocument::saveXml)
pub fn save_xml(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let nodev = arg(args, 0);
    let options = super::int(args, 1);
    let (intern, docn) = get_obj(cx, this)?;
    let docp: *mut XmlDoc = docn.cast();
    let format = lxn::doc_props_read_only(intern.document()).formatoutput;
    let no_empty = (options & LIBXML_SAVE_NOEMPTYTAG != 0) as c_int;
    let res = unsafe {
        if !matches!(nodev, Value::Null) {
            let (_, node) = get_obj(cx, &nodev)?;
            if (*node).doc != docp {
                return throw(cx, WRONG_DOCUMENT_ERR, intern.strict());
            }
            let old = sys::rhp_xml_set_save_no_empty_tags(no_empty);
            let r = dump(&intern, node, 0, format, (*docp).encoding);
            sys::rhp_xml_set_save_no_empty_tags(old);
            r?
        } else {
            let mut converted = super::xml_serializer::XML_SAVE_AS_XML;
            if options & XML_SAVE_NO_DECL != 0 {
                converted |= XML_SAVE_NO_DECL as c_int;
            }
            if format {
                converted |= super::xml_serializer::XML_SAVE_FORMAT;
            }
            let old = sys::rhp_xml_set_save_no_empty_tags(no_empty);
            let r = dump(&intern, docn, converted, format, (*docp).encoding);
            sys::rhp_xml_set_save_no_empty_tags(old);
            r?
        }
    };
    match res {
        Some(s) => Ok(Value::str(s)),
        None => {
            cx.warn("Could not save document")?;
            Ok(Value::Bool(false))
        }
    }
}

/// php_is_stream_path: "scheme://" (or "data:").
fn is_stream_path(p: &[u8]) -> bool {
    let n = p
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
        .count();
    n > 0
        && (p[n..].starts_with(b"://")
            || (n == 4 && p[..4].eq_ignore_ascii_case(b"data") && p.get(4) == Some(&b':')))
}

/// Write bytes to a file through libxml2's output layer (php's streams);
/// the byte count or None.
fn write_file(cx: &mut Cx, file: &[u8], data: &[u8]) -> Result<Option<i64>, Thrown> {
    let cf = c(file);
    let n = cx.with_libxml(|| unsafe {
        let out = sys::xmlOutputBufferCreateFilename(cf.as_ptr().cast(), ptr::null_mut(), 0);
        if out.is_null() {
            return -1;
        }
        sys::xmlOutputBufferWrite(out, data.len() as c_int, data.as_ptr().cast());
        sys::xmlOutputBufferClose(out)
    })?;
    Ok((n >= 0).then_some(n as i64))
}

/// Open `path` for reading through php's streams and read it all; the
/// data and, for a plain file, the path opened.
type FileData = (Vec<u8>, Option<Vec<u8>>);

fn read_file(cx: &mut Cx, path: &[u8]) -> Result<Option<FileData>, Thrown> {
    let fname = cx.fname();
    let ctx = crate::ext::libxml::stream_context().unwrap_or(Value::Null);
    let Some(h) =
        crate::ext::standard::file::open_stream(cx.host, &fname, path, b"rb", &ctx, false)?
    else {
        return Ok(None);
    };
    let mut data = Vec::new();
    while let Some(chunk) = cx.host.stream_read(&h, 4096) {
        if chunk.is_empty() {
            break;
        }
        data.extend_from_slice(&chunk);
    }
    let plain = cx
        .host
        .stream_meta(&h)
        .is_some_and(|m| m.wrapper_type == "plainfile");
    cx.host.stream_close(&h);
    let opened = if plain {
        std::fs::canonicalize(crate::zend::virtual_cwd::os_path(path))
            .ok()
            .map(|p| {
                use std::os::unix::ffi::OsStringExt;
                p.into_os_string().into_vec()
            })
    } else {
        None
    };
    Ok(Some((data, opened)))
}

/// Dom\XMLDocument::saveXmlFile and Dom\HTMLDocument::saveXmlFile
pub fn save_xml_file(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let file = crate::ext::libxml::str_arg(args, 0);
    let file = file.as_bytes();
    let options = super::int(args, 1);
    if file.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "filename",
            "must not contain any null bytes",
        ));
    }
    if file.is_empty() {
        return Err(value_error(cx, 1, "filename", "must not be empty"));
    }
    let (intern, docn) = get_obj(cx, this)?;
    let docp: *mut XmlDoc = docn.cast();
    let format = lxn::doc_props_read_only(intern.document()).formatoutput;
    let no_empty = (options & LIBXML_SAVE_NOEMPTYTAG != 0) as c_int;
    let data = unsafe {
        let old = sys::rhp_xml_set_save_no_empty_tags(no_empty);
        let r = dump(
            &intern,
            docn,
            super::xml_serializer::XML_SAVE_AS_XML,
            format,
            (*docp).encoding,
        );
        sys::rhp_xml_set_save_no_empty_tags(old);
        r
    };
    let Some(data) = data? else {
        return Ok(Value::Bool(false));
    };
    Ok(match write_file(cx, file, &data)? {
        Some(n) => Value::Int(n),
        None => Value::Bool(false),
    })
}

// ---- Dom\HTMLDocument ----

pub fn html_create_empty(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    rhp_csrc::lexbor::setup();
    let encoding = bytes(args, 0);
    if encoding.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "encoding",
            "must not contain any null bytes",
        ));
    }
    if super::html5::encoding_name(&encoding).is_none() {
        return Err(value_error(
            cx,
            1,
            "encoding",
            "must be a valid document encoding",
        ));
    }
    let doc = create_html_doc();
    if doc.is_null() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    let ce = c(&encoding);
    unsafe { (*doc).encoding = sys::xmlStrdup(ce.as_ptr()) };
    new_document_object(cx, b"Dom\\HTMLDocument", doc, DomPrivate::create())
}

/// php_dom_create_html_doc
pub fn create_html_doc() -> *mut XmlDoc {
    unsafe {
        let doc = sys::htmlNewDocNoDtD(ptr::null(), ptr::null());
        if !doc.is_null() {
            (*doc).dict = sys::xmlDictCreate();
        }
        doc
    }
}

const HTML_VALID_OPTIONS: i64 =
    XML_PARSE_NOERROR | XML_PARSE_COMPACT | HTML_PARSE_NOIMPLIED | HTML_NO_DEFAULT_NS;

fn html_check_options(cx: &Cx, options: i64) -> Result<(), Thrown> {
    if options & !HTML_VALID_OPTIONS != 0 {
        return Err(value_error(
            cx,
            2,
            "options",
            "contains invalid flags (allowed flags: LIBXML_NOERROR, LIBXML_COMPACT, LIBXML_HTML_NOIMPLIED, Dom\\HTML_NO_DEFAULT_NS)",
        ));
    }
    Ok(())
}

/// dom_should_register_error_handlers
fn report_errors(cx: &mut Cx, options: i64) -> bool {
    if options & XML_PARSE_NOERROR != 0 {
        return false;
    }
    crate::ext::libxml::use_internal()
        || crate::zend::builtin_functions::error_reporting_level(cx.host) & crate::E_WARNING != 0
}

/// After a parse: the errors as php_libxml_pretend_ctx_error_ex reports
/// them, the conversion, and the document object.
fn html_finish(
    cx: &mut Cx,
    mut p: super::html5::Parser,
    options: i64,
    input_name: &[u8],
    url: Option<Vec<u8>>,
) -> Result<Value, Thrown> {
    let errors = std::mem::take(&mut *p.errors);
    if !errors.is_empty() {
        let name = input_name.to_vec();
        cx.with_libxml(|| {
            for e in &errors {
                crate::ext::libxml::pretend_ctx_error(
                    &name,
                    e.line as i32,
                    e.column as i32,
                    &e.msg,
                );
            }
        })?;
    }
    let private = DomPrivate::create();
    let doc = match p.convert(
        options,
        options & XML_PARSE_COMPACT != 0,
        options & HTML_NO_DEFAULT_NS == 0,
        private.cast(),
    ) {
        Ok(d) => d,
        Err(st) => {
            nscompat::destroy(private);
            let mut msg = super::html5::status_string(st);
            msg.extend_from_slice(b" in ");
            msg.extend_from_slice(input_name);
            msg.push(b'\n');
            cx.with_libxml(|| crate::ext::libxml::ctx_error(ptr::null_mut(), &msg))?;
            return Ok(Value::Bool(false));
        }
    };
    let quirks = p.quirks();
    drop(p);
    if let Some(u) = url {
        let cu = c(&u);
        unsafe { (*doc).URL = sys::xmlStrdup(cu.as_ptr()) };
    }
    let v = new_document_object(cx, b"Dom\\HTMLDocument", doc, private)?;
    let d = intern_of(&v).expect("a DOM document");
    unsafe { (*d.document()).quirks_mode = quirks };
    Ok(v)
}

pub fn html_create_from_string(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    rhp_csrc::lexbor::setup();
    let source = crate::ext::libxml::str_arg(args, 0);
    let source = source.as_bytes();
    let options = super::int(args, 1);
    let override_encoding = opt_bytes(args, 2);
    html_check_options(cx, options)?;
    let report = report_errors(cx, options);
    let mut p = super::html5::Parser::new(b"Entity", options & HTML_PARSE_NOIMPLIED != 0, report);
    if let Some(e) = &override_encoding {
        if e.contains(&0) {
            return Err(value_error(
                cx,
                3,
                "overrideEncoding",
                "must not contain any null bytes",
            ));
        }
        if !p.set_encoding(Some(source), e) {
            return Err(value_error(
                cx,
                3,
                "overrideEncoding",
                "must be a valid document encoding",
            ));
        }
    }
    if !p.parse_string(source) || !p.finish() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    html_finish(cx, p, options, b"Entity", None)
}

pub fn html_create_from_file(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    rhp_csrc::lexbor::setup();
    let path = crate::ext::libxml::str_arg(args, 0);
    let path = path.as_bytes().to_vec();
    let options = super::int(args, 1);
    let override_encoding = opt_bytes(args, 2);
    if path.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "path",
            "must not contain any null bytes",
        ));
    }
    if path.windows(3).any(|w| w == b"%00") {
        return Err(value_error(
            cx,
            1,
            "path",
            "must not contain percent-encoded NUL bytes",
        ));
    }
    html_check_options(cx, options)?;
    let report = report_errors(cx, options);
    let mut p = super::html5::Parser::new(&path, options & HTML_PARSE_NOIMPLIED != 0, report);
    if let Some(e) = &override_encoding {
        if e.contains(&0) {
            return Err(value_error(
                cx,
                3,
                "overrideEncoding",
                "must not contain any null bytes",
            ));
        }
        if !p.set_encoding(None, e) {
            return Err(value_error(
                cx,
                3,
                "overrideEncoding",
                "must be a valid document encoding",
            ));
        }
    }
    let Some((data, opened)) = read_file(cx, &path)? else {
        return Err(Thrown::error(
            "Exception",
            format!("Cannot open file '{}'", String::from_utf8_lossy(&path)),
        ));
    };
    for chunk in data.chunks(4096) {
        if !p.parse_file_chunk(chunk) {
            return Err(dom_err(cx, INVALID_STATE_ERR));
        }
    }
    if !p.finish() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    let url = match opened {
        Some(opened) => unsafe {
            let co = c(&opened);
            let conv = sys::take_string(sys::xmlPathToURI(co.as_ptr())).unwrap_or_default();
            if conv.starts_with(b"file:/") {
                Some(conv)
            } else {
                let mut u = b"file://".to_vec();
                u.extend_from_slice(&conv);
                Some(u)
            }
        },
        None => Some(path.clone()),
    };
    html_finish(cx, p, options, &path, url)
}

/// The HTML serialization of `node` in the document's encoding
/// (dom_common_save).
fn html_save(d: &DomObject, docp: *mut XmlDoc, node: *mut XmlNode) -> Vec<u8> {
    rhp_csrc::lexbor::setup();
    let mut utf8 = Vec::new();
    super::html5_serializer::serialize_outer(&mut utf8, private(d).map(|p| &*p), node);
    let enc = unsafe { b((*docp).encoding) }.to_vec();
    super::html5::encode_output(&enc, &utf8)
}

pub fn save_html(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let nodev = arg(args, 0);
    let (intern, docn) = get_obj(cx, this)?;
    let docp: *mut XmlDoc = docn.cast();
    let node = if matches!(nodev, Value::Null) {
        docn
    } else {
        let (_, node) = get_obj(cx, &nodev)?;
        if unsafe { (*node).doc } != docp {
            return Err(dom_err(cx, WRONG_DOCUMENT_ERR));
        }
        node
    };
    Ok(Value::str(html_save(&intern, docp, node)))
}

pub fn save_html_file(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let file = crate::ext::libxml::str_arg(args, 0);
    let file = file.as_bytes();
    if file.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "filename",
            "must not contain any null bytes",
        ));
    }
    if file.is_empty() {
        return Err(value_error(cx, 1, "filename", "must not be empty"));
    }
    let (intern, docn) = get_obj(cx, this)?;
    let data = html_save(&intern, docn.cast(), docn);
    Ok(match write_file(cx, file, &data)? {
        Some(n) => Value::Int(n),
        None => Value::Bool(false),
    })
}

// ---- Dom\Implementation ----

pub fn impl_create_document_type(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let public_id = bytes(args, 1);
    let system_id = bytes(args, 2);
    for (i, (v, p)) in [
        (&name, "qualifiedName"),
        (&public_id, "publicId"),
        (&system_id, "systemId"),
    ]
    .into_iter()
    .enumerate()
    {
        if v.contains(&0) {
            return Err(value_error(cx, i + 1, p, "must not contain any null bytes"));
        }
    }
    if !valid_qname(&name) {
        return Err(dom_err(cx, super::NAMESPACE_ERR));
    }
    let cn = c(&name);
    let cp = c(&public_id);
    let cs = c(&system_id);
    let dtd = unsafe {
        sys::xmlCreateIntSubset(
            ptr::null_mut(),
            cn.as_ptr(),
            if public_id.is_empty() {
                ptr::null()
            } else {
                cp.as_ptr()
            },
            if system_id.is_empty() {
                ptr::null()
            } else {
                cs.as_ptr()
            },
        )
    };
    if dtd.is_null() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    instantiate(cx, b"Dom\\DocumentType", dtd.cast(), ptr::null_mut())
}

pub fn impl_create_document(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0);
    let qname = bytes(args, 1);
    let dtdv = arg(args, 2);
    let mut doctype: *mut XmlNode = ptr::null_mut();
    if !matches!(dtdv, Value::Null) {
        doctype = get_obj(cx, &dtdv)?.1;
    }
    let h = DomPrivate::create();
    let pm = unsafe { &mut (*(h as *mut DomPrivate)).ns_mapper };
    let document = unsafe { sys::xmlNewDoc(c"1.0".as_ptr().cast()) };
    if document.is_null() {
        nscompat::destroy(h);
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    unsafe { (*document).encoding = sys::xmlStrdup(c"UTF-8".as_ptr().cast()) };
    let mut element: *mut XmlNode = ptr::null_mut();
    if !qname.is_empty() {
        match validate_and_extract(uri.as_deref(), &qname) {
            Ok((local, prefix)) => {
                let ns = pm.get_ns(
                    prefix.as_deref().unwrap_or(b""),
                    uri.as_deref().unwrap_or(b""),
                );
                let cl = c(&local);
                element = unsafe { sys::xmlNewDocNode(document, ns, cl.as_ptr(), ptr::null()) };
            }
            Err(code) => {
                unsafe { sys::xmlFreeDoc(document) };
                nscompat::destroy(h);
                return Err(dom_err(cx, code));
            }
        }
    }
    let v = new_document_object(cx, b"Dom\\XMLDocument", document, h)?;
    let intern = intern_of(&v).expect("a DOM document");
    unsafe {
        if !doctype.is_null() {
            adopt(&intern, doctype, document);
            sys::xmlAddChild(document.cast(), doctype);
            (*doctype).doc = document;
            (*document).intSubset = doctype.cast();
        }
        if !element.is_null() {
            sys::xmlAddChild(document.cast(), element);
        }
    }
    Ok(v)
}

pub fn impl_create_html_document(cx: &mut Cx, _: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let title = opt_bytes(args, 0);
    if title.as_ref().is_some_and(|t| t.contains(&0)) {
        return Err(value_error(
            cx,
            1,
            "title",
            "must not contain any null bytes",
        ));
    }
    let doc = create_html_doc();
    if doc.is_null() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    let h = DomPrivate::create();
    unsafe {
        (*doc).encoding = sys::xmlStrdup(c"UTF-8".as_ptr().cast());
        sys::xmlCreateIntSubset(doc, c"html".as_ptr().cast(), ptr::null(), ptr::null());
        let html_ns = (*(h as *mut DomPrivate)).ns_mapper.ensure_html_ns();
        let html = sys::xmlNewDocRawNode(doc, html_ns, c"html".as_ptr().cast(), ptr::null());
        sys::xmlAddChild(doc.cast(), html);
        let head = sys::xmlNewDocRawNode(doc, html_ns, c"head".as_ptr().cast(), ptr::null());
        sys::xmlAddChild(html, head);
        if let Some(t) = &title {
            let ct = c(t);
            let te = sys::xmlNewDocRawNode(doc, html_ns, c"title".as_ptr().cast(), ct.as_ptr());
            sys::xmlAddChild(head, te);
        }
        let body = sys::xmlNewDocRawNode(doc, html_ns, c"body".as_ptr().cast(), ptr::null());
        sys::xmlAddChild(html, body);
    }
    new_document_object(cx, b"Dom\\HTMLDocument", doc, h)
}

// ---- Dom\Document ----

/// php_dom_adopt_node
pub fn adopt(intern: &DomObject, nodep: *mut XmlNode, new_document: *mut XmlDoc) -> bool {
    unsafe {
        let old_doc = (*nodep).doc;
        lxn::invalidate_node_list_cache_from_doc(old_doc);
        if old_doc != new_document {
            lxn::invalidate_node_list_cache(intern.document());
            if is_modern(intern) {
                sys::xmlUnlinkNode(nodep);
                sys::xmlSetTreeDoc(nodep, new_document);
                nscompat::reconcile_modern(mapper(intern), nodep);
            } else if sys::xmlDOMWrapAdoptNode(
                ptr::null_mut(),
                old_doc,
                nodep,
                new_document,
                ptr::null_mut(),
                0,
            ) != 0
            {
                return false;
            }
            super::document::transfer_document_ref(nodep, intern.document());
        } else {
            sys::xmlUnlinkNode(nodep);
        }
    }
    true
}

pub fn doc_create_element(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let (intern, docn) = get_obj(cx, this)?;
    let docp: *mut XmlDoc = docn.cast();
    if !valid_name(&name) {
        return Err(dom_err(cx, INVALID_CHARACTER_ERR));
    }
    let node = unsafe {
        if (*docp).type_ == sys::XML_HTML_DOCUMENT_NODE {
            let ns = mapper(&intern).ensure_html_ns();
            let lower = c(&name.to_ascii_lowercase());
            sys::xmlNewDocRawNode(docp, ns, lower.as_ptr(), ptr::null())
        } else {
            let cn = c(&name);
            sys::xmlNewDocNode(docp, ptr::null_mut(), cn.as_ptr(), ptr::null())
        }
    };
    if node.is_null() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    create_object(cx, node, Some(&intern))
}

pub fn doc_create_element_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0);
    let name = bytes(args, 1);
    let (intern, docn) = get_obj(cx, this)?;
    let docp: *mut XmlDoc = docn.cast();
    match validate_and_extract(uri.as_deref(), &name) {
        Ok((local, prefix)) => {
            let ns = mapper(&intern).get_ns(
                prefix.as_deref().unwrap_or(b""),
                uri.as_deref().unwrap_or(b""),
            );
            let cl = c(&local);
            let node = unsafe { sys::xmlNewDocNode(docp, ns, cl.as_ptr(), ptr::null()) };
            if node.is_null() {
                return Err(dom_err(cx, INVALID_STATE_ERR));
            }
            create_object(cx, node, Some(&intern))
        }
        Err(code) => throw(cx, code, intern.strict()),
    }
}

pub fn doc_create_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    if name.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "localName",
            "must not contain any null bytes",
        ));
    }
    let (intern, docn) = get_obj(cx, this)?;
    let docp: *mut XmlDoc = docn.cast();
    if !valid_name(&name) {
        return throw(cx, INVALID_CHARACTER_ERR, intern.strict());
    }
    let n = if unsafe { (*docp).type_ } == sys::XML_HTML_DOCUMENT_NODE {
        name.to_ascii_lowercase()
    } else {
        name
    };
    let cn = c(&n);
    let node = unsafe { sys::xmlNewDocProp(docp, cn.as_ptr(), ptr::null()) };
    if node.is_null() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    create_object(cx, node.cast(), Some(&intern))
}

pub fn doc_create_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0);
    let name = bytes(args, 1);
    let (intern, docn) = get_obj(cx, this)?;
    let docp: *mut XmlDoc = docn.cast();
    let (local, prefix) = match validate_and_extract(uri.as_deref(), &name) {
        Ok(p) => p,
        Err(code) => return throw(cx, code, intern.strict()),
    };
    let cl = c(&local);
    let node: *mut XmlNode = unsafe { sys::xmlNewDocProp(docp, cl.as_ptr(), ptr::null()).cast() };
    if node.is_null() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    if let Some(u) = uri.as_ref().filter(|u| !u.is_empty()) {
        let ns = mapper(&intern).get_ns(prefix.as_deref().unwrap_or(b""), u);
        unsafe { (*node).ns = ns };
    }
    create_object(cx, node, Some(&intern))
}

pub fn doc_create_cdata_section(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let value = bytes(args, 0);
    let (intern, docn) = get_obj(cx, this)?;
    let docp: *mut XmlDoc = docn.cast();
    if unsafe { (*docp).type_ } == sys::XML_HTML_DOCUMENT_NODE {
        return Err(dom_err_msg(
            cx,
            NOT_SUPPORTED_ERR,
            "This operation is not supported for HTML documents",
        ));
    }
    if value.windows(3).any(|w| w == b"]]>") {
        return Err(dom_err_msg(
            cx,
            INVALID_CHARACTER_ERR,
            "Invalid character sequence \"]]>\" in CDATA section",
        ));
    }
    let node = unsafe { sys::xmlNewCDataBlock(docp, value.as_ptr(), value.len() as c_int) };
    if node.is_null() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    create_object(cx, node, Some(&intern))
}

pub fn doc_create_processing_instruction(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let value = bytes(args, 1);
    let (intern, docn) = get_obj(cx, this)?;
    if !valid_name(&name) {
        return throw(cx, INVALID_CHARACTER_ERR, intern.strict());
    }
    if value.windows(2).any(|w| w == b"?>") {
        return Err(dom_err_msg(
            cx,
            INVALID_CHARACTER_ERR,
            "Invalid character sequence \"?>\" in processing instruction",
        ));
    }
    let cn = c(&name);
    let cv = c(&value);
    let node = unsafe { sys::xmlNewDocPI(docn.cast(), cn.as_ptr(), cv.as_ptr()) };
    if node.is_null() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    create_object(cx, node, Some(&intern))
}

/// dom_modern_document_import_node (importNode, importLegacyNode)
pub fn doc_import_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, docn) = get_obj(cx, this)?;
    let docp: *mut XmlDoc = docn.cast();
    let nodev = arg(args, 0);
    let (_, nodep) = get_obj(cx, &nodev)?;
    let recursive = boolean(args, 1);
    unsafe {
        let t = (*nodep).type_;
        if t == sys::XML_HTML_DOCUMENT_NODE || t == sys::XML_DOCUMENT_NODE {
            return Err(dom_err(cx, NOT_SUPPORTED_ERR));
        }
        let ret = if (*nodep).doc == docp {
            nodep
        } else {
            let r = clone_node_modern(Some(mapper(&intern)), nodep, docp, recursive);
            if r.is_null() {
                return Err(dom_err(cx, INVALID_STATE_ERR));
            }
            r
        };
        create_object(cx, ret, Some(&intern))
    }
}

pub fn doc_adopt_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let nodev = arg(args, 0);
    let (_, nodep) = get_obj(cx, &nodev)?;
    let (intern, docn) = get_obj(cx, this)?;
    unsafe {
        let t = (*nodep).type_;
        if t == sys::XML_DOCUMENT_NODE || t == sys::XML_HTML_DOCUMENT_NODE {
            return Err(dom_err(cx, NOT_SUPPORTED_ERR));
        }
        if t == sys::XML_ENTITY_REF_NODE && !(*nodep).parent.is_null() {
            // php reports an error for adopting an entity reference's child too
        }
    }
    if !adopt(&intern, nodep, docn.cast()) {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    Ok(nodev)
}

/// HTMLCollection from getElementsByTagName(NS) and getElementsByClassName.
pub fn get_elements_by_tag_name(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    if name.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "qualifiedName",
            "must not contain any null bytes",
        ));
    }
    let intern = get_intern(cx, this)?;
    super::lists::new_map_named(
        cx,
        &intern,
        super::lists::Handler::ByTagName,
        b"Dom\\HTMLCollection",
        Some(&name),
        None,
    )
}

pub fn get_elements_by_tag_name_ns(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0).unwrap_or_default();
    let name = bytes(args, 1);
    if uri.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "namespace",
            "must not contain any null bytes",
        ));
    }
    if name.contains(&0) {
        return Err(value_error(
            cx,
            2,
            "localName",
            "must not contain any null bytes",
        ));
    }
    let intern = get_intern(cx, this)?;
    super::lists::new_map_named(
        cx,
        &intern,
        super::lists::Handler::ByTagName,
        b"Dom\\HTMLCollection",
        Some(&name),
        Some(&uri),
    )
}

pub fn get_elements_by_class_name(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let names = bytes(args, 0);
    if names.contains(&0) {
        return Err(value_error(
            cx,
            1,
            "classNames",
            "must not contain any null bytes",
        ));
    }
    let intern = get_intern(cx, this)?;
    let quirks = unsafe { intern.document().as_ref() }
        .is_some_and(|d| d.quirks_mode == lxn::QuirksMode::Quirks);
    let tokens = ordered_set_parser(&names, quirks);
    if tokens.is_empty() {
        return super::lists::new_map(
            cx,
            &intern,
            super::lists::Handler::Noop,
            b"Dom\\HTMLCollection",
        );
    }
    let v = super::lists::new_map(
        cx,
        &intern,
        super::lists::Handler::ByClassName,
        b"Dom\\HTMLCollection",
    )?;
    if let Some(d) = intern_of(&v) {
        if let super::Kind::Map(m) = &d.kind {
            m.borrow_mut().class_names = tokens;
        }
    }
    Ok(v)
}

pub const ASCII_WHITESPACE: &[u8] = b"\x09\x0a\x0c\x0d\x20";

/// dom_ordered_set_parser: split on ASCII whitespace, without duplicates.
pub fn ordered_set_parser(s: &[u8], to_lowercase: bool) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    for tok in s
        .split(|c| ASCII_WHITESPACE.contains(c))
        .filter(|t| !t.is_empty())
    {
        let t = if to_lowercase {
            tok.to_ascii_lowercase()
        } else {
            tok.to_vec()
        };
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

// ---- Dom\Node ----

/// dom_clone_node with a namespace mapper (the modern half).
pub fn clone_node_modern(
    m: Option<&mut NsMapper>,
    node: *mut XmlNode,
    doc: *mut XmlDoc,
    recursive: bool,
) -> *mut XmlNode {
    unsafe {
        if (*node).type_ == sys::XML_DTD_NODE {
            let dtd = sys::xmlCopyDtd(node.cast());
            sys::xmlSetTreeDoc(dtd.cast(), doc);
            return dtd.cast();
        }
        let Some(m) = m else {
            return super::util::clone_node(node, doc, recursive);
        };
        let clone = clone_helper(m, node, doc, recursive);
        if !clone.is_null() && (*clone).doc != (*node).doc {
            let t = (*clone).type_;
            if t == sys::XML_DOCUMENT_NODE
                || t == sys::XML_HTML_DOCUMENT_NODE
                || t == sys::XML_DOCUMENT_FRAG_NODE
            {
                let mut child = (*clone).children;
                while !child.is_null() {
                    nscompat::reconcile_modern(m, child);
                    child = (*child).next;
                }
            } else {
                nscompat::reconcile_modern(m, clone);
            }
        }
        clone
    }
}

/// dom_clone_container_helper
unsafe fn clone_container(
    m: &mut NsMapper,
    src: *mut XmlNode,
    dst_doc: *mut XmlDoc,
) -> *mut XmlNode {
    unsafe {
        let clone = sys::xmlDocCopyNode(src, dst_doc, 0);
        if clone.is_null() {
            return clone;
        }
        (*clone).ns = (*src).ns;
        if (*src).type_ == sys::XML_ELEMENT_NODE {
            let mut last: *mut XmlAttr = ptr::null_mut();
            if !(*src).nsDef.is_null() {
                let mut ns = (*src).nsDef;
                while !ns.is_null() {
                    nscompat::mark_attribute(m, clone, ns);
                    ns = (*ns).next;
                }
                last = (*clone).properties;
                while !(*last).next.is_null() {
                    last = (*last).next;
                }
            }
            let mut attr = (*src).properties;
            while !attr.is_null() {
                let new_attr = sys::xmlDocCopyNode(attr.cast(), dst_doc, 0) as *mut XmlAttr;
                if new_attr.is_null() {
                    sys::xmlFreeNode(clone);
                    return ptr::null_mut();
                }
                if last.is_null() {
                    (*clone).properties = new_attr;
                } else {
                    (*new_attr).prev = last;
                    (*last).next = new_attr;
                }
                (*new_attr).parent = clone;
                last = new_attr;
                (*new_attr).ns = (*attr).ns;
                attr = (*attr).next;
            }
        }
        clone
    }
}

/// dom_clone_helper
unsafe fn clone_helper(
    m: &mut NsMapper,
    src_node: *mut XmlNode,
    mut dst_doc: *mut XmlDoc,
    recursive: bool,
) -> *mut XmlNode {
    unsafe {
        let outer = clone_container(m, src_node, dst_doc);
        let t = (*src_node).type_;
        if !recursive
            || !matches!(
                t,
                sys::XML_ELEMENT_NODE
                    | sys::XML_DOCUMENT_FRAG_NODE
                    | sys::XML_DOCUMENT_NODE
                    | sys::XML_HTML_DOCUMENT_NODE
            )
            || outer.is_null()
        {
            return outer;
        }
        if t == sys::XML_DOCUMENT_NODE || t == sys::XML_HTML_DOCUMENT_NODE {
            dst_doc = outer.cast();
            let sub = (*(src_node as *mut XmlDoc)).intSubset;
            if !sub.is_null() {
                let copy = sys::xmlCopyDtd(sub);
                if copy.is_null() {
                    sys::xmlFreeNode(outer);
                    return ptr::null_mut();
                }
                (*dst_doc).intSubset = copy;
                (*copy).parent = dst_doc;
                sys::xmlSetTreeDoc(copy.cast(), dst_doc);
                (*dst_doc).children = copy.cast();
                (*dst_doc).last = copy.cast();
            }
        }
        let mut cloned_parent = outer;
        let base = src_node;
        let mut src = (*src_node).children;
        while !src.is_null() {
            let cloned = match (*src).type_ {
                sys::XML_ELEMENT_NODE => clone_container(m, src, dst_doc),
                sys::XML_DTD_NODE => ptr::null_mut(),
                _ => sys::xmlDocCopyNode(src, dst_doc, 1),
            };
            if !cloned.is_null() {
                if (*cloned_parent).children.is_null() {
                    (*cloned_parent).children = cloned;
                } else {
                    (*cloned).prev = (*cloned_parent).last;
                    (*(*cloned_parent).last).next = cloned;
                }
                (*cloned).parent = cloned_parent;
                (*cloned_parent).last = cloned;
            }
            if (*src).type_ == sys::XML_ELEMENT_NODE
                && !(*src).children.is_null()
                && !cloned.is_null()
            {
                cloned_parent = cloned;
                src = (*src).children;
            } else if !(*src).next.is_null() {
                src = (*src).next;
            } else {
                loop {
                    src = (*src).parent;
                    if src == base {
                        return outer;
                    }
                    cloned_parent = (*cloned_parent).parent;
                    if !(*src).next.is_null() {
                        break;
                    }
                }
                src = (*src).next;
            }
        }
        outer
    }
}

pub fn clone_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, n) = get_obj(cx, this)?;
    let recursive = boolean(args, 0);
    let t = unsafe { (*n).type_ };
    let clone_document = t == sys::XML_DOCUMENT_NODE || t == sys::XML_HTML_DOCUMENT_NODE;
    if clone_document {
        let h = DomPrivate::create();
        let m = unsafe { &mut (*(h as *mut DomPrivate)).ns_mapper };
        let node = clone_node_modern(Some(m), n, unsafe { (*n).doc }, recursive);
        if node.is_null() {
            nscompat::destroy(h);
            return Ok(Value::Bool(false));
        }
        let class: &[u8] = if t == sys::XML_DOCUMENT_NODE {
            b"Dom\\XMLDocument"
        } else {
            b"Dom\\HTMLDocument"
        };
        let v = instantiate(cx, class, node, ptr::null_mut())?;
        if let Some(ni) = intern_of(&v) {
            super::node::update_document_after_clone(&intern, &ni);
            unsafe {
                (*ni.document()).private_data = h;
                (*ni.document()).quirks_mode = (*intern.document()).quirks_mode;
            }
        }
        Ok(v)
    } else {
        let node = clone_node_modern(Some(mapper(&intern)), n, unsafe { (*n).doc }, recursive);
        if node.is_null() {
            return Ok(Value::Bool(false));
        }
        unsafe {
            if (*node).type_ == sys::XML_ATTRIBUTE_NODE
                && !(*n).ns.is_null()
                && (*node).ns.is_null()
            {
                (*node).ns = (*n).ns;
            }
        }
        create_object(cx, node, Some(&intern))
    }
}

pub fn normalize(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, n) = get_obj(cx, this)?;
    normalize_modern(n);
    Ok(Value::Null)
}

/// php_dom_normalize_modern
fn normalize_modern(this: *mut XmlNode) {
    unsafe {
        let mut node = (*this).children;
        while !node.is_null() {
            if (*node).type_ == sys::XML_TEXT_NODE {
                let empty = (*node).content.is_null() || *(*node).content == 0;
                if empty {
                    let next = (*node).next;
                    sys::xmlUnlinkNode(node);
                    if (*node)._private.is_null() {
                        sys::xmlFreeNode(node);
                    }
                    node = next;
                    continue;
                }
                // dom_merge_adjacent_exclusive_text_nodes
                let mut nextp = (*node).next;
                while !nextp.is_null() && (*nextp).type_ == sys::XML_TEXT_NODE {
                    let next = (*nextp).next;
                    if !(*nextp).content.is_null() {
                        sys::xmlNodeAddContent(node, (*nextp).content);
                    }
                    sys::xmlUnlinkNode(nextp);
                    if (*nextp)._private.is_null() {
                        sys::xmlFreeNode(nextp);
                    }
                    nextp = next;
                }
            } else if (*node).type_ == sys::XML_ELEMENT_NODE {
                normalize_modern(node);
            }
            node = (*node).next;
        }
    }
}

/// php_dom_pre_insert_is_parent_invalid
fn parent_invalid(parent: *mut XmlNode) -> bool {
    !matches!(
        unsafe { (*parent).type_ },
        sys::XML_DOCUMENT_NODE
            | sys::XML_HTML_DOCUMENT_NODE
            | sys::XML_ELEMENT_NODE
            | sys::XML_DOCUMENT_FRAG_NODE
    )
}

pub fn append_child(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let childv = arg(args, 0);
    let (_, child) = get_obj(cx, &childv)?;
    if parent_invalid(nodep) {
        return Err(dom_err(cx, super::HIERARCHY_REQUEST_ERR));
    }
    lxn::invalidate_node_list_cache(intern.document());
    super::element::pre_insert(cx, intern.document(), child, nodep, ptr::null_mut())?;
    create_object(cx, child, Some(&intern))
}

pub fn insert_before(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, parentp) = get_obj(cx, this)?;
    let (_, child) = get_obj(cx, &arg(args, 0))?;
    if parent_invalid(parentp) {
        return Err(dom_err(cx, super::HIERARCHY_REQUEST_ERR));
    }
    let refv = arg(args, 1);
    let refp = if matches!(refv, Value::Null) {
        ptr::null_mut()
    } else {
        get_obj(cx, &refv)?.1
    };
    lxn::invalidate_node_list_cache(intern.document());
    super::element::pre_insert(cx, intern.document(), child, parentp, refp)?;
    create_object(cx, child, Some(&intern))
}

pub fn replace_child(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let (_, newchild) = get_obj(cx, &arg(args, 0))?;
    let (_, oldchild) = get_obj(cx, &arg(args, 1))?;
    unsafe {
        if (*newchild).doc != (*nodep).doc && !(*newchild).doc.is_null() {
            return throw(cx, WRONG_DOCUMENT_ERR, true);
        }
        super::element::replace_validity_checks(cx, nodep, newchild, oldchild)?;
        if (*newchild).doc.is_null() && !(*nodep).doc.is_null() {
            sys::xmlSetTreeDoc(newchild, (*nodep).doc);
            set_document_ref_pointers(newchild, intern.document());
        }
        if (*newchild).type_ == sys::XML_DOCUMENT_FRAG_NODE {
            let prevsib = (*oldchild).prev;
            let nextsib = (*oldchild).next;
            sys::xmlUnlinkNode(oldchild);
            super::node::insert_fragment(nodep, prevsib, nextsib, newchild);
        } else if oldchild != newchild {
            let int_subset = sys::xmlGetIntSubset((*nodep).doc);
            let replacedoctype = int_subset.cast::<XmlNode>() == oldchild;
            sys::xmlReplaceNode(oldchild, newchild);
            if replacedoctype {
                (*(*nodep).doc).intSubset = newchild.cast();
            }
        }
    }
    lxn::invalidate_node_list_cache(intern.document());
    create_object(cx, oldchild, Some(&intern))
}

/// dom_locate_a_namespace_prefix
fn locate_prefix(mut elem: *mut XmlNode, uri: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        while !elem.is_null() && (*elem).type_ == sys::XML_ELEMENT_NODE {
            let ns = (*elem).ns;
            if !ns.is_null() && !(*ns).prefix.is_null() && str_eq_b((*ns).href, uri) {
                return Some(b((*ns).prefix).to_vec());
            }
            let mut attr = (*elem).properties;
            while !attr.is_null() {
                if !(*attr).ns.is_null()
                    && !(*attr).children.is_null()
                    && str_eq_b((*(*attr).ns).prefix, b"xmlns")
                    && str_eq_b((*(*attr).children).content, uri)
                {
                    return Some(b((*attr).name).to_vec());
                }
                attr = (*attr).next;
            }
            elem = (*elem).parent;
        }
    }
    None
}

pub fn lookup_prefix(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0).unwrap_or_default();
    let (_, nodep) = get_obj(cx, this)?;
    if uri.is_empty() {
        return Ok(Value::Null);
    }
    unsafe {
        let lookupp = match (*nodep).type_ {
            sys::XML_ELEMENT_NODE => nodep,
            sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE => {
                sys::xmlDocGetRootElement(nodep.cast())
            }
            sys::XML_ENTITY_NODE
            | sys::XML_NOTATION_NODE
            | sys::XML_DOCUMENT_FRAG_NODE
            | sys::XML_DOCUMENT_TYPE_NODE
            | sys::XML_DTD_NODE => return Ok(Value::Null),
            _ => (*nodep).parent,
        };
        if lookupp.is_null() {
            return Ok(Value::Null);
        }
        Ok(locate_prefix(lookupp, &uri).map_or(Value::Null, Value::str))
    }
}

/// dom_locate_a_namespace
pub fn locate_namespace(mut node: *const XmlNode, prefix: Option<&[u8]>) -> Option<Vec<u8>> {
    unsafe {
        match (*node).type_ {
            sys::XML_ELEMENT_NODE => {
                if let Some(p) = prefix {
                    if p.eq_ignore_ascii_case(b"xml") {
                        return Some(nscompat::XML_NS_URI.to_vec());
                    }
                    if p.eq_ignore_ascii_case(b"xmlns") {
                        return Some(nscompat::XMLNS_NS_URI.to_vec());
                    }
                }
                loop {
                    let ns = (*node).ns;
                    let pmatch = |p: *const u8| match prefix {
                        None => p.is_null(),
                        Some(x) => !p.is_null() && b(p) == x,
                    };
                    if !ns.is_null() && pmatch((*ns).prefix) {
                        return Some(b((*ns).href).to_vec());
                    }
                    let mut attr = (*node).properties;
                    while !attr.is_null() {
                        let ans = (*attr).ns;
                        if !ans.is_null() && nscompat::ns_is_fast_ex(ans, &nscompat::XMLNS) {
                            let hit = match prefix {
                                Some(p) => {
                                    str_eq_b((*ans).prefix, b"xmlns") && b((*attr).name) == p
                                }
                                None => (*ans).prefix.is_null() && b((*attr).name) == b"xmlns",
                            };
                            if hit {
                                let c = (*attr).children;
                                if !c.is_null() && !(*c).content.is_null() && *(*c).content != 0 {
                                    return Some(b((*c).content).to_vec());
                                }
                                return None;
                            }
                        }
                        attr = (*attr).next;
                    }
                    let parent = (*node).parent;
                    if parent.is_null() || (*parent).type_ != sys::XML_ELEMENT_NODE {
                        return None;
                    }
                    node = parent;
                }
            }
            sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE => {
                let root = sys::xmlDocGetRootElement(node as *mut XmlDoc);
                if root.is_null() {
                    return None;
                }
                locate_namespace(root, prefix)
            }
            sys::XML_DTD_NODE | sys::XML_DOCUMENT_FRAG_NODE => None,
            _ => {
                let parent = (*node).parent;
                if parent.is_null() || (*parent).type_ != sys::XML_ELEMENT_NODE {
                    return None;
                }
                locate_namespace(parent, prefix)
            }
        }
    }
}

pub fn lookup_namespace_uri(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let prefix = opt_bytes(args, 0).filter(|p| !p.is_empty());
    let (_, nodep) = get_obj(cx, this)?;
    Ok(locate_namespace(nodep, prefix.as_deref()).map_or(Value::Null, Value::str))
}

pub fn is_default_namespace(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0).filter(|u| !u.is_empty());
    let (_, nodep) = get_obj(cx, this)?;
    Ok(Value::Bool(locate_namespace(nodep, None) == uri))
}

pub fn get_node_path(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, n) = get_obj(cx, this)?;
    match unsafe { sys::take_string(sys::xmlGetNodePath(n)) } {
        Some(s) => Ok(Value::str(s)),
        None => Err(dom_err(cx, INVALID_STATE_ERR)),
    }
}

// ---- Dom\Element attributes ----

/// php_dom_get_attribute_node: by qualified name (lowercased for HTML
/// elements in HTML documents).
pub fn get_attribute_node(elem: *mut XmlNode, name: &[u8]) -> *mut XmlAttr {
    let name = if nscompat::ns_is_html_and_document_is_html(elem) {
        name.to_ascii_lowercase()
    } else {
        name.to_vec()
    };
    unsafe {
        let mut attr = (*elem).properties;
        while !attr.is_null() {
            if match_qualified_name(&name, attr.cast()) {
                return attr;
            }
            attr = (*attr).next;
        }
    }
    ptr::null_mut()
}

/// dom_match_qualified_name_according_to_spec
pub fn match_qualified_name(qname: &[u8], node: *const XmlNode) -> bool {
    unsafe {
        let local = b((*node).name);
        let ns = (*node).ns;
        if !ns.is_null() && !(*ns).prefix.is_null() {
            let p = b((*ns).prefix);
            qname.len() > p.len()
                && qname.starts_with(p)
                && qname[p.len()] == b':'
                && &qname[p.len() + 1..] == local
        } else {
            qname == local
        }
    }
}

fn attr_string(attr: *mut XmlAttr) -> Vec<u8> {
    unsafe {
        sys::take_string(sys::xmlNodeListGetString((*attr).doc, (*attr).children, 1))
            .unwrap_or_default()
    }
}

pub fn el_get_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let (_, nodep) = get_obj(cx, this)?;
    let attr = get_attribute_node(nodep, &name);
    if attr.is_null() {
        return Ok(Value::Null);
    }
    Ok(Value::str(attr_string(attr)))
}

pub fn el_get_attribute_names(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (_, nodep) = get_obj(cx, this)?;
    let mut arr = PhpArray::new();
    unsafe {
        let mut attr = (*nodep).properties;
        while !attr.is_null() {
            arr.push(Value::str(qualified_name(attr.cast())));
            attr = (*attr).next;
        }
    }
    Ok(Value::Array(arr))
}

/// dom_check_register_attribute_id
fn check_register_attribute_id(attr: *mut XmlAttr, document: *mut lxn::RefObj) {
    mark_ids_modified(document);
    unsafe {
        if (*attr).atype != sys::XML_ATTRIBUTE_ID
            && (*(*attr).doc).type_ == sys::XML_HTML_DOCUMENT_NODE
            && (*attr).ns.is_null()
            && b((*attr).name) == b"id"
        {
            (*attr).atype = sys::XML_ATTRIBUTE_ID;
        }
    }
}

pub fn el_set_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let value = bytes(args, 1);
    if name.is_empty() {
        return Err(value_error(cx, 1, "qualifiedName", "must not be empty"));
    }
    if !valid_name(&name) {
        return Err(dom_err(cx, INVALID_CHARACTER_ERR));
    }
    let (intern, nodep) = get_obj(cx, this)?;
    let processed = if nscompat::ns_is_html_and_document_is_html(nodep) {
        name.to_ascii_lowercase()
    } else {
        name.clone()
    };
    let attr = get_attribute_node(nodep, &name);
    let cv = c(&value);
    unsafe {
        if !attr.is_null() {
            attr_value_will_change(intern.document(), attr);
            remove_all_children(attr.cast());
            let node = sys::xmlNewDocText((*attr).doc, cv.as_ptr());
            sys::xmlAddChild(attr.cast(), node);
        } else {
            let cn = c(&processed);
            let a = sys::xmlSetNsProp(nodep, ptr::null_mut(), cn.as_ptr(), cv.as_ptr());
            if !a.is_null() {
                check_register_attribute_id(a, intern.document());
            }
        }
    }
    Ok(Value::Null)
}

/// dom_remove_attribute for an attribute node.
fn remove_attribute(attrp: *mut XmlAttr, document: *mut lxn::RefObj) {
    unsafe {
        if lxn::object_of_node(attrp.cast()).is_none() {
            node_list_unlink((*attrp).children);
            sys::xmlUnlinkNode(attrp.cast());
            sys::xmlFreeProp(attrp);
        } else {
            sys::xmlUnlinkNode(attrp.cast());
        }
    }
    lxn::invalidate_node_list_cache(document);
}

pub fn el_remove_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let (intern, nodep) = get_obj(cx, this)?;
    let attr = get_attribute_node(nodep, &name);
    if !attr.is_null() {
        remove_attribute(attr, intern.document());
    }
    Ok(Value::Null)
}

pub fn el_get_attribute_node(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let (intern, nodep) = get_obj(cx, this)?;
    let attr = get_attribute_node(nodep, &name);
    create_nullable(cx, attr.cast(), Some(&intern))
}

pub fn el_has_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let (_, nodep) = get_obj(cx, this)?;
    Ok(Value::Bool(!get_attribute_node(nodep, &name).is_null()))
}

fn ns_prop(nodep: *mut XmlNode, name: &[u8], uri: Option<&[u8]>) -> *mut XmlAttr {
    let cn = c(name);
    let cu = uri.map(c);
    unsafe {
        sys::xmlHasNsProp(
            nodep,
            cn.as_ptr(),
            cu.as_ref().map_or(ptr::null(), |u| u.as_ptr()),
        )
    }
}

pub fn el_get_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0).filter(|u| !u.is_empty());
    let name = bytes(args, 1);
    let (_, nodep) = get_obj(cx, this)?;
    let cn = c(&name);
    let cu = uri.as_deref().map(c);
    let v = unsafe {
        sys::take_string(sys::xmlGetNsProp(
            nodep,
            cn.as_ptr(),
            cu.as_ref().map_or(ptr::null(), |u| u.as_ptr()),
        ))
    };
    Ok(v.map_or(Value::Null, Value::str))
}

pub fn el_has_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(!matches!(
        el_get_attribute_ns(cx, this, args)?,
        Value::Null
    )))
}

pub fn el_set_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0);
    let name = bytes(args, 1);
    let value = bytes(args, 2);
    let (intern, elemp) = get_obj(cx, this)?;
    match validate_and_extract(uri.as_deref(), &name) {
        Ok((local, prefix)) => {
            let ns = mapper(&intern).get_ns(
                prefix.as_deref().unwrap_or(b""),
                uri.as_deref().unwrap_or(b""),
            );
            let href = if ns.is_null() {
                None
            } else {
                Some(unsafe { b((*ns).href) }.to_vec())
            };
            let existing = ns_prop(elemp, &local, href.as_deref());
            unsafe {
                if !existing.is_null() && (*existing).type_ != sys::XML_ATTRIBUTE_DECL {
                    node_list_unlink((*existing).children);
                }
                let cl = c(&local);
                let cv = c(&value);
                let attr = sys::xmlSetNsProp(elemp, ns, cl.as_ptr(), cv.as_ptr());
                if attr.is_null() {
                    return Err(dom_err(cx, INVALID_STATE_ERR));
                }
                check_register_attribute_id(attr, intern.document());
            }
            Ok(Value::Null)
        }
        Err(code) => Err(dom_err(cx, code)),
    }
}

pub fn el_remove_attribute_ns(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0).filter(|u| !u.is_empty());
    let name = bytes(args, 1);
    let (intern, nodep) = get_obj(cx, this)?;
    let attrp = ns_prop(nodep, &name, uri.as_deref());
    if !attrp.is_null() && unsafe { (*attrp).type_ } != sys::XML_ATTRIBUTE_DECL {
        remove_attribute(attrp, intern.document());
    }
    Ok(Value::Null)
}

pub fn el_get_attribute_node_ns(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0).filter(|u| !u.is_empty());
    let name = bytes(args, 1);
    let (intern, nodep) = get_obj(cx, this)?;
    let attrp = ns_prop(nodep, &name, uri.as_deref());
    create_nullable(cx, attrp.cast(), Some(&intern))
}

/// dom_element_set_attribute_node_common, modern
fn set_attribute_node_common(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
    use_ns: bool,
) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let (_, attrn) = get_obj(cx, &arg(args, 0))?;
    let attrp: *mut XmlAttr = attrn.cast();
    unsafe {
        if !(*attrp).parent.is_null() && (*attrp).parent != nodep {
            return Err(dom_err(cx, INUSE_ATTRIBUTE_ERR));
        }
        if !(*attrp).doc.is_null() && (*attrp).doc != (*nodep).doc {
            adopt(&intern, attrn, (*nodep).doc);
        }
        let nsp = (*attrp).ns;
        let mut existattrp = if use_ns && !nsp.is_null() {
            sys::xmlHasNsProp(nodep, (*attrp).name, (*nsp).href)
        } else if nsp.is_null() {
            sys::xmlHasNsProp(nodep, (*attrp).name, ptr::null())
        } else {
            sys::xmlHasProp(nodep, (*attrp).name)
        };
        if !existattrp.is_null() && (*existattrp).type_ == sys::XML_ATTRIBUTE_DECL {
            existattrp = ptr::null_mut();
        }
        if !existattrp.is_null() {
            if existattrp == attrp {
                return Ok(Value::Null);
            }
            sys::xmlUnlinkNode(existattrp.cast());
        }
        if !(*attrp).parent.is_null() {
            sys::xmlUnlinkNode(attrn);
        }
        if (*attrp).doc.is_null() && !(*nodep).doc.is_null() && !intern.document().is_null() {
            set_document_ref_pointers_attr(attrp, intern.document());
        }
        sys::xmlAddChild(nodep, attrn);
        check_register_attribute_id(attrp, intern.document());
        create_nullable(cx, existattrp.cast(), Some(&intern))
    }
}

pub fn el_set_attribute_node_ns(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    set_attribute_node_common(cx, this, args, true)
}

pub fn el_remove_attribute_node(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let attrv = arg(args, 0);
    let (_, attrp) = get_obj(cx, &attrv)?;
    unsafe {
        if (*attrp).type_ != sys::XML_ATTRIBUTE_NODE || (*attrp).parent != nodep {
            return Err(dom_err(cx, NOT_FOUND_ERR));
        }
        sys::xmlUnlinkNode(attrp);
    }
    lxn::invalidate_node_list_cache(intern.document());
    create_object(cx, attrp, Some(&intern))
}

pub fn el_toggle_attribute(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let name = bytes(args, 0);
    let force = super::node::opt_bool(args, 1);
    let (intern, thisp) = get_obj(cx, this)?;
    if !valid_name(&name) {
        return Err(dom_err(cx, INVALID_CHARACTER_ERR));
    }
    unsafe {
        let ns = (*thisp).ns;
        let qname = if (*(*thisp).doc).type_ == sys::XML_HTML_DOCUMENT_NODE
            && !ns.is_null()
            && str_eq_b((*ns).href, nscompat::XHTML_NS_URI)
        {
            name.to_ascii_lowercase()
        } else {
            name.clone()
        };
        let attr = get_attribute_node(thisp, &qname);
        if attr.is_null() {
            if force.unwrap_or(true) {
                let cq = c(&qname);
                let a = sys::xmlSetNsProp(thisp, ptr::null_mut(), cq.as_ptr(), c"".as_ptr().cast());
                if !a.is_null() {
                    check_register_attribute_id(a, intern.document());
                }
                return Ok(Value::Bool(true));
            }
            return Ok(Value::Bool(false));
        }
        if force != Some(true) {
            remove_attribute(attr, intern.document());
            return Ok(Value::Bool(false));
        }
        Ok(Value::Bool(true))
    }
}

/// dom_element_get_in_scope_namespace_info
fn namespace_infos(
    cx: &mut Cx,
    intern: &Rc<DomObject>,
    nodep: *mut XmlNode,
    out: &mut PhpArray,
) -> Result<(), Thrown> {
    let m = mapper(intern);
    // prefix ("" for the default) to namespace, first (innermost) wins
    let mut table: Vec<(Vec<u8>, *mut XmlNs)> = Vec::new();
    let mut cur = nodep as *const XmlNode;
    unsafe {
        while !cur.is_null() {
            if (*cur).type_ == sys::XML_ELEMENT_NODE {
                let mut last = (*cur).properties;
                while !last.is_null() && !(*last).next.is_null() {
                    last = (*last).next;
                }
                let mut attr = last;
                while !attr.is_null() {
                    if !(*attr).ns.is_null()
                        && nscompat::ns_is_fast_ex((*attr).ns, &nscompat::XMLNS)
                        && !(*attr).children.is_null()
                        && !(*(*attr).children).content.is_null()
                    {
                        let prefix: *const u8 = if (*(*attr).ns).prefix.is_null() {
                            ptr::null()
                        } else {
                            (*attr).name
                        };
                        let key = if prefix.is_null() {
                            Vec::new()
                        } else {
                            b(prefix).to_vec()
                        };
                        let ns = m.get_ns_nullsafe(prefix, (*(*attr).children).content);
                        if !table.iter().any(|(k, _)| *k == key) {
                            table.push((key, ns));
                        }
                    }
                    attr = (*attr).prev;
                }
            }
            cur = (*cur).parent;
        }
    }
    for (prefix, ns) in table.into_iter().rev() {
        let has_href = !ns.is_null() && unsafe { !(*ns).href.is_null() && *(*ns).href != 0 };
        if prefix.is_empty() && !has_href {
            continue;
        }
        let element = create_object(cx, nodep, Some(intern))?;
        let p = if prefix.is_empty() {
            Value::Null
        } else {
            Value::str(prefix)
        };
        let u = if has_href {
            super::xs(unsafe { (*ns).href })
        } else {
            Value::Null
        };
        let info = cx
            .host
            .instantiate(
                b"Dom\\NamespaceInfo",
                vec![
                    (b"prefix".to_vec(), p),
                    (b"namespaceURI".to_vec(), u),
                    (b"element".to_vec(), element),
                ],
            )
            .unwrap_or(Value::Null);
        out.push(info);
    }
    Ok(())
}

pub fn el_get_in_scope_namespaces(cx: &mut Cx, this: &Value, _: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let mut out = PhpArray::new();
    namespace_infos(cx, &intern, nodep, &mut out)?;
    Ok(Value::Array(out))
}

pub fn el_get_descendant_namespaces(
    cx: &mut Cx,
    this: &Value,
    _: &[Value],
) -> Result<Value, Thrown> {
    let (intern, nodep) = get_obj(cx, this)?;
    let mut out = PhpArray::new();
    namespace_infos(cx, &intern, nodep, &mut out)?;
    let base = nodep;
    let mut cur = unsafe { (*nodep).children };
    while !cur.is_null() {
        if unsafe { (*cur).type_ } == sys::XML_ELEMENT_NODE {
            namespace_infos(cx, &intern, cur, &mut out)?;
        }
        cur = next_in_tree_order(cur, base);
    }
    Ok(Value::Array(out))
}

/// Dom\\Element::rename and Dom\\Attr::rename
pub fn rename(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let uri = opt_bytes(args, 0);
    let name = bytes(args, 1);
    let (intern, nodep) = get_obj(cx, this)?;
    let (local, prefix) = match validate_and_extract(uri.as_deref(), &name) {
        Ok(p) => p,
        Err(code) => return Err(dom_err(cx, code)),
    };
    unsafe {
        if (*nodep).type_ == sys::XML_ATTRIBUTE_NODE {
            let u = uri.as_deref().filter(|u| !u.is_empty());
            let existing = ns_prop((*nodep).parent, &local, u);
            if !existing.is_null() && existing.cast::<XmlNode>() != nodep {
                return Err(dom_err_msg(
                    cx,
                    super::INVALID_MODIFICATION_ERR,
                    "An attribute with the given name in the given namespace already exists",
                ));
            }
        } else {
            let is_html = nscompat::ns_is_fast(nodep, &nscompat::HTML);
            let will_be_html = uri.as_deref() == Some(nscompat::XHTML_NS_URI);
            if is_html != will_be_html {
                let msg = if is_html {
                    "It is not possible to move an element out of the HTML namespace because the HTML namespace is tied to the HTMLElement class"
                } else {
                    "It is not possible to move an element into the HTML namespace because the HTML namespace is tied to the HTMLElement class"
                };
                return Err(dom_err_msg(cx, super::INVALID_MODIFICATION_ERR, msg));
            }
            if is_html && b((*nodep).name) == b"template" && local != b"template" {
                return Err(dom_err_msg(
                    cx,
                    super::INVALID_MODIFICATION_ERR,
                    "It is not possible to rename the template element because it hosts a document fragment",
                ));
            }
        }
        lxn::invalidate_node_list_cache(intern.document());
        (*nodep).ns = mapper(&intern).get_ns(
            prefix.as_deref().unwrap_or(b""),
            uri.as_deref().unwrap_or(b""),
        );
        let dict = (*(*nodep).doc).dict;
        if sys::xmlDictOwns(dict, (*nodep).name) != 1 {
            sys::xml_free((*nodep).name as *mut u8);
        }
        let cl = c(&local);
        let copy = sys::xmlDictLookup(dict, cl.as_ptr(), -1);
        (*nodep).name = if copy.is_null() {
            sys::xmlStrdup(cl.as_ptr())
        } else {
            copy
        };
    }
    Ok(Value::Null)
}

/// Element::insertAdjacentElement with a Dom\AdjacentPosition.
pub fn insert_adjacent_element(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let where_ = enum_value(cx, &arg(args, 0));
    let mut a = vec![Value::str(where_)];
    a.extend(args.iter().skip(1).cloned());
    super::element::insert_adjacent_element(cx, this, &a)
}

pub fn insert_adjacent_text(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let where_ = enum_value(cx, &arg(args, 0));
    let mut a = vec![Value::str(where_)];
    a.extend(args.iter().skip(1).cloned());
    super::element::insert_adjacent_text(cx, this, &a)?;
    Ok(Value::Null)
}

fn enum_value(cx: &mut Cx, v: &Value) -> Vec<u8> {
    cx.host
        .object_prop(v, b"value")
        .map(|v| conv::to_string_bytes(&v))
        .unwrap_or_default()
}

// ---- fragment parsing (inner_outer_html_mixin.c) ----

/// dom_parse_fragment: the parsed children of `context` as a fragment, or
/// null with an exception thrown.
pub fn parse_fragment(
    cx: &mut Cx,
    d: &DomObject,
    context: *mut XmlNode,
    input: &[u8],
) -> Result<*mut XmlNode, Thrown> {
    unsafe {
        if (*(*context).doc).type_ == sys::XML_DOCUMENT_NODE {
            return xml_fragment(cx, d, context, input);
        }
        let quirks = (*d.document()).quirks_mode;
        let p = (*d.document()).private_data as *mut DomPrivate;
        match super::html5::parse_html_fragment(p, context, input, quirks) {
            Ok(f) => Ok(f),
            Err(_) => Err(dom_err(cx, INVALID_STATE_ERR)),
        }
    }
}

/// dom_xml_fragment_parsing_algorithm
fn xml_fragment(
    cx: &mut Cx,
    d: &DomObject,
    context: *mut XmlNode,
    input: &[u8],
) -> Result<*mut XmlNode, Thrown> {
    let m = mapper(d);
    let mut decls = Vec::new();
    for ns in nscompat::in_scope_ns(m, context, true) {
        unsafe {
            decls.extend_from_slice(b" xmlns:");
            decls.extend_from_slice(b((*ns).prefix));
            decls.extend_from_slice(b"=\"");
            decls.extend_from_slice(b((*ns).href));
            decls.push(b'"');
        }
    }
    if let Some(dflt) = locate_namespace(context, None) {
        decls.extend_from_slice(b" xmlns=\"");
        decls.extend_from_slice(&dflt);
        decls.push(b'"');
    }
    let doc = match super::html5::parse_xml_fragment(context, &decls, input) {
        Err(()) => return Err(dom_err(cx, INVALID_STATE_ERR)),
        Ok(None) => {
            return Err(dom_err_msg(
                cx,
                SYNTAX_ERR,
                "XML fragment is not well-formed",
            ))
        }
        Ok(Some(doc)) => doc,
    };
    unsafe {
        if doc.is_null() {
            return Ok(ptr::null_mut());
        }
        (*doc).dict = ptr::null_mut();
        let document_element = (*doc).children;
        if document_element.is_null() || !(*document_element).next.is_null() {
            sys::xmlFreeDoc(doc);
            return Err(dom_err_msg(
                cx,
                SYNTAX_ERR,
                "XML fragment is not well-formed",
            ));
        }
        let ctxdoc = (*context).doc;
        let fragment = sys::xmlNewDocFragment(ctxdoc);
        if !fragment.is_null() {
            let child = (*document_element).children;
            sys::xmlSetTreeDoc(document_element, ctxdoc);
            sys::xmlAddChildList(fragment, child);
            nscompat::mark_namespaces_as_attributes_too(m, doc);
            (*document_element).children = ptr::null_mut();
            (*document_element).last = ptr::null_mut();
        }
        sys::xmlFreeDoc(doc);
        Ok(fragment)
    }
}

/// dom_element_inner_html_write
pub fn inner_html_write(cx: &mut Cx, d: &Rc<DomObject>, input: &[u8]) -> Result<(), Thrown> {
    let mut context = d.node();
    if context.is_null() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    let fragment = parse_fragment(cx, d, context, input)?;
    if fragment.is_null() {
        return Ok(());
    }
    unsafe {
        if nscompat::ns_is_fast(context, &nscompat::HTML) && b((*context).name) == b"template" {
            let p = private(d).expect("modern private data");
            context = nscompat::ensure_templated_content(p, context);
            if context.is_null() {
                sys::xmlFreeNode(fragment);
                return Ok(());
            }
        }
    }
    lxn::invalidate_node_list_cache(d.document());
    remove_all_children(context);
    super::element::pre_insert(cx, d.document(), fragment, context, ptr::null_mut())?;
    Ok(())
}

/// dom_element_outer_html_write
pub fn outer_html_write(cx: &mut Cx, d: &Rc<DomObject>, input: &[u8]) -> Result<(), Thrown> {
    let this = d.node();
    if this.is_null() {
        return Err(dom_err(cx, INVALID_STATE_ERR));
    }
    unsafe {
        let mut parent = (*this).parent;
        if parent.is_null() {
            return Ok(());
        }
        if matches!(
            (*parent).type_,
            sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE
        ) {
            return Err(dom_err(cx, super::INVALID_MODIFICATION_ERR));
        }
        let mut created_parent = false;
        if (*parent).type_ == sys::XML_DOCUMENT_FRAG_NODE {
            let html_ns = mapper(d).ensure_html_ns();
            parent =
                sys::xmlNewDocNode((*parent).doc, html_ns, c"body".as_ptr().cast(), ptr::null());
            created_parent = true;
            if parent.is_null() {
                return Err(dom_err(cx, INVALID_STATE_ERR));
            }
        }
        let fragment = match parse_fragment(cx, d, parent, input) {
            Ok(f) if !f.is_null() => f,
            other => {
                if created_parent {
                    sys::xmlFreeNode(parent);
                }
                return other.map(|_| ());
            }
        };
        lxn::invalidate_node_list_cache(d.document());
        let ok = super::element::pre_insert(cx, d.document(), fragment, (*this).parent, this);
        match ok {
            Ok(true) => {
                sys::xmlUnlinkNode(this);
                if created_parent {
                    sys::xmlFreeNode(parent);
                }
                Ok(())
            }
            other => {
                sys::xmlFreeNode(fragment);
                if created_parent {
                    sys::xmlFreeNode(parent);
                }
                other.map(|_| ())
            }
        }
    }
}

/// Dom\Element::insertAdjacentHTML
pub fn insert_adjacent_html(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let where_ = enum_value(cx, &arg(args, 0));
    let string = bytes(args, 1);
    let (intern, thisp) = get_obj(cx, this)?;
    unsafe {
        let before_or_after = matches!(where_.as_slice(), b"beforebegin" | b"afterend");
        let mut context = if before_or_after {
            let c = (*thisp).parent;
            if c.is_null()
                || matches!(
                    (*c).type_,
                    sys::XML_DOCUMENT_NODE | sys::XML_HTML_DOCUMENT_NODE
                )
            {
                return Err(dom_err(cx, super::NO_MODIFICATION_ALLOWED_ERR));
            }
            c
        } else {
            thisp
        };
        let mut created = false;
        if (*context).type_ != sys::XML_ELEMENT_NODE
            || (nscompat::ns_is_html_and_document_is_html(context) && b((*context).name) == b"html")
        {
            let html_ns = mapper(&intern).ensure_html_ns();
            context =
                sys::xmlNewDocNode((*thisp).doc, html_ns, c"body".as_ptr().cast(), ptr::null());
            created = true;
            if context.is_null() {
                return Err(dom_err(cx, INVALID_STATE_ERR));
            }
        }
        let r = (|| -> Result<(), Thrown> {
            let fragment = parse_fragment(cx, &intern, context, &string)?;
            if fragment.is_null() {
                return Ok(());
            }
            lxn::invalidate_node_list_cache(intern.document());
            let doc = intern.document();
            match where_.as_slice() {
                b"beforebegin" => {
                    super::element::pre_insert(cx, doc, fragment, (*thisp).parent, thisp)?
                }
                b"afterend" => {
                    super::element::pre_insert(cx, doc, fragment, (*thisp).parent, (*thisp).next)?
                }
                b"afterbegin" => {
                    super::element::pre_insert(cx, doc, fragment, thisp, (*thisp).children)?
                }
                _ => super::element::pre_insert(cx, doc, fragment, thisp, ptr::null_mut())?,
            };
            Ok(())
        })();
        if created {
            sys::xmlFreeNode(context);
        }
        r?;
    }
    Ok(Value::Null)
}

// ---- CSS selectors (parentnode/css_selectors.c) ----

fn css(
    cx: &mut Cx,
    this: &Value,
    args: &[Value],
    mode: c_int,
) -> Result<(Rc<DomObject>, Vec<*mut XmlNode>), Thrown> {
    let sel = bytes(args, 0);
    let (intern, thisp) = get_obj(cx, this)?;
    let quirks = unsafe { intern.document().as_ref() }
        .is_some_and(|d| d.quirks_mode != lxn::QuirksMode::NoQuirks);
    match super::html5::css(thisp, &sel, mode, quirks) {
        Ok(nodes) => Ok((intern, nodes)),
        Err(super::html5::CssError::Syntax(msg)) => {
            let m = msg.map_or_else(|| "Invalid selector".to_string(), |m| String::from_utf8_lossy(&m).into_owned());
            Err(dom_err_msg(cx, SYNTAX_ERR, &m))
        }
        Err(super::html5::CssError::Blank) => Err(dom_err_msg(
            cx,
            NOT_SUPPORTED_ERR,
            ":blank selector is not implemented because CSSWG has not yet decided its semantics (https://github.com/w3c/csswg-drafts/issues/1967)",
        )),
        Err(super::html5::CssError::Unsupported) => Err(value_error(cx, 1, "selectors", "contains an unsupported selector")),
    }
}

pub fn query_selector(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodes) = css(cx, this, args, 0)?;
    create_nullable(
        cx,
        nodes.first().copied().unwrap_or(ptr::null_mut()),
        Some(&intern),
    )
}

pub fn query_selector_all(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodes) = css(cx, this, args, 1)?;
    let mut values = Vec::with_capacity(nodes.len());
    for n in nodes {
        values.push(create_object(cx, n, Some(&intern))?);
    }
    super::lists::nodeset_list(cx, b"Dom\\NodeList", values)
}

pub fn matches(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (_, nodes) = css(cx, this, args, 2)?;
    Ok(Value::Bool(!nodes.is_empty()))
}

pub fn closest(cx: &mut Cx, this: &Value, args: &[Value]) -> Result<Value, Thrown> {
    let (intern, nodes) = css(cx, this, args, 3)?;
    create_nullable(
        cx,
        nodes.first().copied().unwrap_or(ptr::null_mut()),
        Some(&intern),
    )
}
