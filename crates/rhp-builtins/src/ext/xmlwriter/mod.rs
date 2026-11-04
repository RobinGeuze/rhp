//! The `xmlwriter` extension on libxml2's xmlTextWriter: a port of
//! ext/xmlwriter/php_xmlwriter.c. The `XMLWriter` class is declared in PHP
//! ([`PRELUDE_PHP`]) and calls `__xmlwriter` for its methods, the
//! procedural `xmlwriter_*` functions `__xmlwriter_fn`. A writer is its
//! object's payload, freed by the class's `dtor_obj` or with the object.

use std::ffi::{c_char, c_int, c_void, OsStr};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::ptr;
use std::rc::Rc;

use rhp_values::{conv, Value};

use crate::ext::libxml::io::Io;
use crate::ext::libxml::sys::{self, cbytes};
use crate::ext::libxml::{ffi, io};
use crate::ext::zpp::deref;
use crate::object::{ClassHandlers, CloneObj, ObjectHandlers};
use crate::{Host, Thrown};

/// `ze_xmlwriter_object`: the writer and, for a memory writer, its buffer
/// (owned by the writer's output buffer, freed by its close callback).
struct Writer {
    ptr: *mut sys::xmlTextWriter,
    output: *mut Vec<u8>,
}

impl Drop for Writer {
    /// xmlwriter_destroy_libxml_objects: also frees the output. Callbacks
    /// that write to a php stream need a host; drop a writer inside
    /// [`ffi::with_host`].
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { sys::xmlFreeTextWriter(self.ptr) };
        }
    }
}

/// `XMLWriter`'s handlers: no clone; `dtor_obj` frees the writer.
pub(crate) const CLASSES: &[ClassHandlers] = &[ClassHandlers {
    class: "XMLWriter",
    handlers: &ObjectHandlers {
        clone_obj: CloneObj::None,
        dtor_obj: Some(object_dtor),
        ..ObjectHandlers::STD
    },
    create_object: true,
}];

/// xmlwriter_object_dtor: free the writer while a host can take its last
/// writes (a URI or stream writer flushes on free). Methods called later,
/// from a subclass's `__destruct`, find no writer, as in php. Without a
/// `dtor_obj` (a fatal error's bailout, a constructor that threw) the
/// writer goes with the object, with no host, and stream writes are lost.
fn object_dtor(host: &mut dyn Host, obj: &rhp_values::ObjectHandle) -> Result<(), Thrown> {
    let w = obj.borrow_mut().take_payload();
    if w.is_none() {
        return Ok(());
    }
    // php names the active function in diagnostics; no builtin is running.
    ffi::with_host(host, "Unknown", || drop(w))
}

/// Attach `w` to the object, freeing the writer it had (inside the caller's
/// host frame).
fn set_writer(obj: &Value, w: Option<Writer>) {
    let Some(h) = crate::object::object_of(obj) else {
        return;
    };
    let old = match w {
        Some(w) => h.borrow_mut().set_payload(Rc::new(w)),
        None => h.borrow_mut().take_payload(),
    };
    drop(old);
}

fn writer_of(obj: &Value) -> Option<Rc<Writer>> {
    crate::object::payload::<Writer>(obj)
}

unsafe extern "C" fn memory_write(ctx: *mut c_void, buf: *const c_char, len: c_int) -> c_int {
    ffi::guard(-1, || {
        let out = unsafe { &mut *(ctx as *mut Vec<u8>) };
        out.extend_from_slice(unsafe {
            std::slice::from_raw_parts(buf.cast::<u8>(), len.max(0) as usize)
        });
        len
    })
}

unsafe extern "C" fn memory_close(ctx: *mut c_void) -> c_int {
    ffi::guard(0, || {
        drop(unsafe { Box::from_raw(ctx as *mut Vec<u8>) });
        0
    })
}

/// xml_writer_create_in_memory
fn create_in_memory() -> Option<Writer> {
    let output = Box::into_raw(Box::new(Vec::<u8>::new()));
    unsafe {
        let buf = sys::xmlOutputBufferCreateIO(
            Some(memory_write),
            Some(memory_close),
            output.cast(),
            ptr::null_mut(),
        );
        if buf.is_null() {
            drop(Box::from_raw(output));
            return None;
        }
        let w = sys::xmlNewTextWriter(buf);
        if w.is_null() {
            // Closing the buffer frees the output too.
            sys::xmlOutputBufferClose(buf);
            return None;
        }
        Some(Writer { ptr: w, output })
    }
}

/// xml_writer_stream_close: the stream is not ours to close; drop the
/// reference.
unsafe extern "C" fn stream_close(ctx: *mut c_void) -> c_int {
    ffi::guard(0, || {
        drop(unsafe { Box::from_raw(ctx as *mut Io) });
        0
    })
}

/// zend_dirname
fn zend_dirname(path: &[u8]) -> Vec<u8> {
    let mut end = path.len();
    while end > 1 && path[end - 1] == b'/' {
        end -= 1;
    }
    match path[..end].iter().rposition(|&b| b == b'/') {
        None => b".".to_vec(),
        Some(0) => b"/".to_vec(),
        Some(i) => {
            let mut j = i;
            while j > 1 && path[j - 1] == b'/' {
                j -= 1;
            }
            path[..j].to_vec()
        }
    }
}

/// _xmlwriter_get_valid_file_path
fn valid_file_path(source: &[u8]) -> Option<Vec<u8>> {
    let mut source = source;
    let has_scheme = unsafe {
        let uri = sys::xmlCreateURI();
        if uri.is_null() {
            return None;
        }
        let c = cbytes(source);
        let esc = sys::xmlURIEscapeStr(c.as_ptr(), c":".as_ptr().cast());
        sys::xmlParseURIReference(uri, esc as *const c_char);
        sys::rhp_xml_free(esc.cast());
        let s = !sys::rhp_xml_uri_scheme(uri).is_null();
        sys::xmlFreeURI(uri);
        s
    };
    let mut is_file_uri = false;
    if has_scheme {
        let lower = source.to_ascii_lowercase();
        if lower.starts_with(b"file:///") {
            if source.len() == 8 {
                return None;
            }
            is_file_uri = true;
            source = &source[7..];
        } else if lower.starts_with(b"file://localhost/") {
            if source.len() == 17 {
                return None;
            }
            is_file_uri = true;
            source = &source[16..];
        }
    }
    if has_scheme && !is_file_uri {
        return Some(source.to_vec());
    }
    // VCWD_REALPATH, else expand_filepath; paths stay bytes.
    let resolved = match std::fs::canonicalize(OsStr::from_bytes(source)) {
        Ok(p) => p.into_os_string().into_vec(),
        Err(_) => crate::ext::dom::expand_filepath(source),
    };
    // expand_filepath fails on a path longer than MAXPATHLEN.
    if resolved.len() >= libc::PATH_MAX as usize {
        return None;
    }
    let dir = zend_dirname(source);
    if !dir.is_empty() && std::fs::metadata(OsStr::from_bytes(&dir)).is_err() {
        return None;
    }
    Some(resolved)
}

/// XMLW_NAME_CHK: xmlValidateName, else php's ValueError naming argument
/// `arg_no` the way zend_argument_value_error does.
fn name_chk(call: &Call, arg_no: usize, subject: &str, name: &[u8]) -> Result<(), Thrown> {
    let c = cbytes(name);
    if unsafe { sys::xmlValidateName(c.as_ptr(), 0) } == 0 {
        return Ok(());
    }
    let shown = String::from_utf8_lossy(&c[..c.len() - 1]).into_owned();
    Err(Thrown::error(
        "ValueError",
        format!(
            "{}(): Argument #{arg_no}{} must be a valid {subject}, \"{shown}\" given",
            call.fname,
            call.param(arg_no)
        ),
    ))
}

/// One XMLWriter call: the method (`XMLWriter::m`) or the procedural
/// function that shares its C implementation.
struct Call {
    fname: String,
    /// The parameter names as zend_argument_error counts them: the method's,
    /// or the function's (with `$writer` first).
    params: Vec<&'static str>,
}

impl Call {
    fn param(&self, n: usize) -> String {
        match self.params.get(n - 1) {
            Some(p) => format!(" (${p})"),
            None => String::new(),
        }
    }
}

/// (method, function, method parameters)
const OPS: &[(&str, &str, &[&str])] = &[
    ("openUri", "xmlwriter_open_uri", &["uri"]),
    ("openMemory", "xmlwriter_open_memory", &[]),
    ("setIndent", "xmlwriter_set_indent", &["enable"]),
    (
        "setIndentString",
        "xmlwriter_set_indent_string",
        &["indentation"],
    ),
    ("startComment", "xmlwriter_start_comment", &[]),
    ("endComment", "xmlwriter_end_comment", &[]),
    ("startAttribute", "xmlwriter_start_attribute", &["name"]),
    ("endAttribute", "xmlwriter_end_attribute", &[]),
    (
        "writeAttribute",
        "xmlwriter_write_attribute",
        &["name", "value"],
    ),
    (
        "startAttributeNs",
        "xmlwriter_start_attribute_ns",
        &["prefix", "name", "namespace"],
    ),
    (
        "writeAttributeNs",
        "xmlwriter_write_attribute_ns",
        &["prefix", "name", "namespace", "value"],
    ),
    ("startElement", "xmlwriter_start_element", &["name"]),
    ("endElement", "xmlwriter_end_element", &[]),
    ("fullEndElement", "xmlwriter_full_end_element", &[]),
    (
        "startElementNs",
        "xmlwriter_start_element_ns",
        &["prefix", "name", "namespace"],
    ),
    (
        "writeElement",
        "xmlwriter_write_element",
        &["name", "content"],
    ),
    (
        "writeElementNs",
        "xmlwriter_write_element_ns",
        &["prefix", "name", "namespace", "content"],
    ),
    ("startPi", "xmlwriter_start_pi", &["target"]),
    ("endPi", "xmlwriter_end_pi", &[]),
    ("writePi", "xmlwriter_write_pi", &["target", "content"]),
    ("startCdata", "xmlwriter_start_cdata", &[]),
    ("endCdata", "xmlwriter_end_cdata", &[]),
    ("writeCdata", "xmlwriter_write_cdata", &["content"]),
    ("text", "xmlwriter_text", &["content"]),
    ("writeRaw", "xmlwriter_write_raw", &["content"]),
    (
        "startDocument",
        "xmlwriter_start_document",
        &["version", "encoding", "standalone"],
    ),
    ("endDocument", "xmlwriter_end_document", &[]),
    ("writeComment", "xmlwriter_write_comment", &["content"]),
    (
        "startDtd",
        "xmlwriter_start_dtd",
        &["qualifiedName", "publicId", "systemId"],
    ),
    ("endDtd", "xmlwriter_end_dtd", &[]),
    (
        "writeDtd",
        "xmlwriter_write_dtd",
        &["name", "publicId", "systemId", "content"],
    ),
    (
        "startDtdElement",
        "xmlwriter_start_dtd_element",
        &["qualifiedName"],
    ),
    ("endDtdElement", "xmlwriter_end_dtd_element", &[]),
    (
        "writeDtdElement",
        "xmlwriter_write_dtd_element",
        &["name", "content"],
    ),
    ("startDtdAttlist", "xmlwriter_start_dtd_attlist", &["name"]),
    ("endDtdAttlist", "xmlwriter_end_dtd_attlist", &[]),
    (
        "writeDtdAttlist",
        "xmlwriter_write_dtd_attlist",
        &["name", "content"],
    ),
    (
        "startDtdEntity",
        "xmlwriter_start_dtd_entity",
        &["name", "isParam"],
    ),
    ("endDtdEntity", "xmlwriter_end_dtd_entity", &[]),
    (
        "writeDtdEntity",
        "xmlwriter_write_dtd_entity",
        &[
            "name",
            "content",
            "isParam",
            "publicId",
            "systemId",
            "notationData",
        ],
    ),
    ("outputMemory", "xmlwriter_output_memory", &["flush"]),
    ("flush", "xmlwriter_flush", &["empty"]),
];

fn op(method: &str) -> Option<&'static (&'static str, &'static str, &'static [&'static str])> {
    OPS.iter().find(|o| o.0 == method)
}

fn s(a: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&a.get(i).map(deref).unwrap_or(Value::Null))
}

fn os(a: &[Value], i: usize) -> Option<Vec<u8>> {
    match a.get(i).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(conv::to_string_bytes(&v)),
    }
}

fn b(a: &[Value], i: usize, default: bool) -> bool {
    a.get(i)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(default)
}

fn cp(v: &Option<Vec<u8>>) -> *const u8 {
    v.as_ref().map_or(ptr::null(), |v| v.as_ptr())
}

/// The XMLWriter operation `method` on `this` with the method's arguments.
fn run(
    host: &mut dyn Host,
    call: &Call,
    method: &str,
    this: &Value,
    a: &[Value],
) -> Result<Value, Thrown> {
    match method {
        "openUri" => return open_uri(host, call, Some(this), a),
        "openMemory" => return open_memory(host, call, Some(this)),
        _ => {}
    }
    // XMLWRITER_FROM_OBJECT
    let Some(w) = writer_of(this) else {
        return Err(Thrown::error(
            "Error",
            "Invalid or uninitialized XMLWriter object",
        ));
    };
    let p = w.ptr;
    let cs = |i: usize| Some(cbytes(&s(a, i)));
    let cos = |i: usize| os(a, i).map(|v| cbytes(&v));
    // php_xmlwriter_string_arg / php_xmlwriter_end: true unless -1.
    let ok = |r: c_int| Ok(Value::Bool(r != -1));
    macro_rules! c {
        ($e:expr) => {{
            let r = ffi::with_host(host, &call.fname, || unsafe { $e });
            r?
        }};
    }
    match method {
        "setIndent" => {
            let on = b(a, 0, false);
            Ok(Value::Bool(
                c!(sys::xmlTextWriterSetIndent(p, on as c_int)) == 0,
            ))
        }
        "setIndentString" => {
            let v = cs(0);
            ok(c!(sys::xmlTextWriterSetIndentString(p, cp(&v))))
        }
        "startComment" => ok(c!(sys::xmlTextWriterStartComment(p))),
        "endComment" => ok(c!(sys::xmlTextWriterEndComment(p))),
        "startAttribute" => {
            name_chk(call, 2, "attribute name", &s(a, 0))?;
            let n = cs(0);
            ok(c!(sys::xmlTextWriterStartAttribute(p, cp(&n))))
        }
        "endAttribute" => ok(c!(sys::xmlTextWriterEndAttribute(p))),
        "writeAttribute" => {
            name_chk(call, 2, "attribute name", &s(a, 0))?;
            let (n, v) = (cs(0), cs(1));
            ok(c!(sys::xmlTextWriterWriteAttribute(p, cp(&n), cp(&v))))
        }
        "startAttributeNs" => {
            name_chk(call, 3, "attribute name", &s(a, 1))?;
            let (pre, n, uri) = (cos(0), cs(1), cos(2));
            ok(c!(sys::xmlTextWriterStartAttributeNS(
                p,
                cp(&pre),
                cp(&n),
                cp(&uri)
            )))
        }
        "writeAttributeNs" => {
            name_chk(call, 3, "attribute name", &s(a, 1))?;
            let (pre, n, uri, v) = (cos(0), cs(1), cos(2), cs(3));
            ok(c!(sys::xmlTextWriterWriteAttributeNS(
                p,
                cp(&pre),
                cp(&n),
                cp(&uri),
                cp(&v)
            )))
        }
        "startElement" => {
            name_chk(call, 2, "element name", &s(a, 0))?;
            let n = cs(0);
            ok(c!(sys::xmlTextWriterStartElement(p, cp(&n))))
        }
        "endElement" => ok(c!(sys::xmlTextWriterEndElement(p))),
        "fullEndElement" => ok(c!(sys::xmlTextWriterFullEndElement(p))),
        "startElementNs" => {
            name_chk(call, 3, "element name", &s(a, 1))?;
            let (pre, n, uri) = (cos(0), cs(1), cos(2));
            ok(c!(sys::xmlTextWriterStartElementNS(
                p,
                cp(&pre),
                cp(&n),
                cp(&uri)
            )))
        }
        "writeElement" => {
            name_chk(call, 2, "element name", &s(a, 0))?;
            let (n, content) = (cs(0), cos(1));
            let r = c!(match &content {
                None => {
                    if sys::xmlTextWriterStartElement(p, cp(&n)) == -1 {
                        -1
                    } else {
                        sys::xmlTextWriterEndElement(p)
                    }
                }
                Some(_) => sys::xmlTextWriterWriteElement(p, cp(&n), cp(&content)),
            });
            ok(r)
        }
        "writeElementNs" => {
            name_chk(call, 3, "element name", &s(a, 1))?;
            let (pre, n, uri, content) = (cos(0), cs(1), cos(2), cos(3));
            let r = c!(match &content {
                None => {
                    if sys::xmlTextWriterStartElementNS(p, cp(&pre), cp(&n), cp(&uri)) == -1 {
                        -1
                    } else {
                        sys::xmlTextWriterEndElement(p)
                    }
                }
                Some(_) =>
                    sys::xmlTextWriterWriteElementNS(p, cp(&pre), cp(&n), cp(&uri), cp(&content)),
            });
            ok(r)
        }
        "startPi" => {
            name_chk(call, 2, "PI target", &s(a, 0))?;
            let n = cs(0);
            ok(c!(sys::xmlTextWriterStartPI(p, cp(&n))))
        }
        "endPi" => ok(c!(sys::xmlTextWriterEndPI(p))),
        "writePi" => {
            name_chk(call, 2, "PI target", &s(a, 0))?;
            let (n, v) = (cs(0), cs(1));
            ok(c!(sys::xmlTextWriterWritePI(p, cp(&n), cp(&v))))
        }
        "startCdata" => ok(c!(sys::xmlTextWriterStartCDATA(p))),
        "endCdata" => ok(c!(sys::xmlTextWriterEndCDATA(p))),
        "writeCdata" => {
            let v = cs(0);
            ok(c!(sys::xmlTextWriterWriteCDATA(p, cp(&v))))
        }
        "writeRaw" => {
            let v = cs(0);
            ok(c!(sys::xmlTextWriterWriteRaw(p, cp(&v))))
        }
        "text" => {
            let v = cs(0);
            ok(c!(sys::xmlTextWriterWriteString(p, cp(&v))))
        }
        "writeComment" => {
            let v = cs(0);
            ok(c!(sys::xmlTextWriterWriteComment(p, cp(&v))))
        }
        "startDocument" => {
            let (version, enc, alone) = (cos(0), os(a, 1), cos(2));
            // `p!`: a path, no NUL bytes.
            if enc.as_ref().is_some_and(|e| e.contains(&0)) {
                return Err(Thrown::error(
                    "ValueError",
                    format!(
                        "{}(): Argument #{}{} must not contain any null bytes",
                        call.fname,
                        call.params.len() - 1,
                        call.param(call.params.len() - 1)
                    ),
                ));
            }
            let enc = enc.map(|e| cbytes(&e));
            ok(c!(sys::xmlTextWriterStartDocument(
                p,
                cp(&version).cast(),
                cp(&enc).cast(),
                cp(&alone).cast()
            )))
        }
        "endDocument" => ok(c!(sys::xmlTextWriterEndDocument(p))),
        "startDtd" => {
            let (n, pubid, sysid) = (cs(0), cos(1), cos(2));
            ok(c!(sys::xmlTextWriterStartDTD(
                p,
                cp(&n),
                cp(&pubid),
                cp(&sysid)
            )))
        }
        "endDtd" => ok(c!(sys::xmlTextWriterEndDTD(p))),
        "writeDtd" => {
            let (n, pubid, sysid, subset) = (cs(0), cos(1), cos(2), cos(3));
            ok(c!(sys::xmlTextWriterWriteDTD(
                p,
                cp(&n),
                cp(&pubid),
                cp(&sysid),
                cp(&subset)
            )))
        }
        "startDtdElement" => {
            name_chk(call, 2, "element name", &s(a, 0))?;
            let n = cs(0);
            ok(c!(sys::xmlTextWriterStartDTDElement(p, cp(&n))))
        }
        "endDtdElement" => ok(c!(sys::xmlTextWriterEndDTDElement(p))),
        "writeDtdElement" => {
            name_chk(call, 2, "element name", &s(a, 0))?;
            let (n, v) = (cs(0), cs(1));
            ok(c!(sys::xmlTextWriterWriteDTDElement(p, cp(&n), cp(&v))))
        }
        "startDtdAttlist" => {
            name_chk(call, 2, "element name", &s(a, 0))?;
            let n = cs(0);
            ok(c!(sys::xmlTextWriterStartDTDAttlist(p, cp(&n))))
        }
        "endDtdAttlist" => ok(c!(sys::xmlTextWriterEndDTDAttlist(p))),
        "writeDtdAttlist" => {
            name_chk(call, 2, "element name", &s(a, 0))?;
            let (n, v) = (cs(0), cs(1));
            ok(c!(sys::xmlTextWriterWriteDTDAttlist(p, cp(&n), cp(&v))))
        }
        "startDtdEntity" => {
            name_chk(call, 2, "attribute name", &s(a, 0))?;
            let (n, pe) = (cs(0), b(a, 1, false));
            ok(c!(sys::xmlTextWriterStartDTDEntity(p, pe as c_int, cp(&n))))
        }
        "endDtdEntity" => ok(c!(sys::xmlTextWriterEndDTDEntity(p))),
        "writeDtdEntity" => {
            name_chk(call, 2, "element name", &s(a, 0))?;
            let (n, content, pe) = (cs(0), cs(1), b(a, 2, false));
            let (pubid, sysid, ndata) = (cos(3), cos(4), cos(5));
            ok(c!(sys::xmlTextWriterWriteDTDEntity(
                p,
                pe as c_int,
                cp(&n),
                cp(&pubid),
                cp(&sysid),
                cp(&ndata),
                cp(&content)
            )))
        }
        "outputMemory" => flush(host, call, &w, b(a, 0, true), true),
        "flush" => flush(host, call, &w, b(a, 0, true), false),
        _ => Err(Thrown::error(
            "Error",
            format!("Call to undefined method XMLWriter::{method}()"),
        )),
    }
}

/// php_xmlwriter_flush
fn flush(
    host: &mut dyn Host,
    call: &Call,
    w: &Writer,
    empty: bool,
    force_string: bool,
) -> Result<Value, Thrown> {
    if force_string && w.output.is_null() {
        return Ok(Value::str(Vec::new()));
    }
    let p = w.ptr;
    let bytes = ffi::with_host(host, &call.fname, || unsafe { sys::xmlTextWriterFlush(p) })?;
    if w.output.is_null() {
        return Ok(Value::Int(bytes as i64));
    }
    // The buffer lives as long as the writer.
    let out = unsafe { &mut *w.output };
    Ok(Value::str(if empty {
        std::mem::take(out)
    } else {
        out.clone()
    }))
}

/// xmlwriter_open_uri / XMLWriter::openUri
fn open_uri(
    host: &mut dyn Host,
    call: &Call,
    this: Option<&Value>,
    a: &[Value],
) -> Result<Value, Thrown> {
    let source = s(a, 0);
    if source.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{}(): Argument #1 ($uri) must not contain any null bytes",
                call.fname
            ),
        ));
    }
    if source.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            format!("{}(): Argument #1 ($uri) must not be empty", call.fname),
        ));
    }
    let Some(valid) = valid_file_path(&source) else {
        crate::warn(host, &call.fname, "Unable to resolve file path")?;
        return Ok(Value::Bool(false));
    };
    let c = cbytes(&valid);
    let ptr = ffi::with_host(host, &call.fname, || unsafe {
        sys::xmlNewTextWriterFilename(c.as_ptr().cast(), 0)
    })?;
    if ptr.is_null() {
        return Ok(Value::Bool(false));
    }
    attach(
        host,
        call,
        this,
        Writer {
            ptr,
            output: ptr::null_mut(),
        },
    )
}

/// xmlwriter_open_memory / XMLWriter::openMemory
fn open_memory(host: &mut dyn Host, call: &Call, this: Option<&Value>) -> Result<Value, Thrown> {
    let Some(w) = create_in_memory() else {
        return Ok(Value::Bool(false));
    };
    attach(host, call, this, w)
}

/// A new writer for `this` (true), or for a new XMLWriter object.
fn attach(
    host: &mut dyn Host,
    call: &Call,
    this: Option<&Value>,
    w: Writer,
) -> Result<Value, Thrown> {
    match this {
        Some(obj) => {
            ffi::with_host(host, &call.fname, || set_writer(obj, Some(w)))?;
            Ok(Value::Bool(true))
        }
        None => {
            let Some(obj) = host.instantiate(b"XMLWriter", Vec::new()) else {
                return Ok(Value::Bool(false));
            };
            set_writer(&obj, Some(w));
            Ok(obj)
        }
    }
}

/// xml_writer_create_static: a new object of the called class, its
/// constructor run, holding `w`.
fn create_static(
    host: &mut dyn Host,
    fname: &str,
    class: &[u8],
    w: Writer,
) -> Result<Value, Thrown> {
    match host.reflect_new_instance(class, Vec::new(), true) {
        Ok(obj) => {
            set_writer(&obj, Some(w));
            Ok(obj)
        }
        Err(e) => {
            ffi::with_host(host, fname, || drop(w))?;
            Err(e)
        }
    }
}

/// `__xmlwriter_to($method, $class, ...$args)`: XMLWriter::toMemory,
/// toUri and toStream.
fn fn_to(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = s(args, 0);
    let class = s(args, 1);
    let a = &args[2.min(args.len())..];
    match &method[..] {
        b"toMemory" => {
            let Some(w) = create_in_memory() else {
                return Err(Thrown::error("Error", "Could not construct libxml writer"));
            };
            create_static(host, "XMLWriter::toMemory", &class, w)
        }
        b"toUri" => {
            const F: &str = "XMLWriter::toUri";
            let source = s(a, 0);
            if source.contains(&0) {
                return Err(Thrown::error(
                    "ValueError",
                    format!("{F}(): Argument #1 ($uri) must not contain any null bytes"),
                ));
            }
            if source.is_empty() {
                return Err(Thrown::error(
                    "ValueError",
                    format!("{F}(): Argument #1 ($uri) must not be empty"),
                ));
            }
            let Some(valid) = valid_file_path(&source) else {
                return Err(Thrown::error(
                    "ValueError",
                    format!("{F}(): Argument #1 ($uri) must resolve to a valid file path"),
                ));
            };
            let c = cbytes(&valid);
            let ptr = ffi::with_host(host, F, || unsafe {
                sys::xmlNewTextWriterFilename(c.as_ptr().cast(), 0)
            })?;
            if ptr.is_null() {
                return Err(Thrown::error("Error", "Could not construct libxml writer"));
            }
            create_static(
                host,
                F,
                &class,
                Writer {
                    ptr,
                    output: ptr::null_mut(),
                },
            )
        }
        _ => {
            const F: &str = "XMLWriter::toStream";
            let stream = a.first().map(deref).unwrap_or(Value::Null);
            if !matches!(stream, Value::Resource(_)) {
                return Err(Thrown::type_error(format!(
                    "{F}(): Argument #1 ($stream) must be of type resource, {} given",
                    crate::argparse::zval_value_name(host, &stream)
                )));
            }
            if host.stream_kind(&stream).is_none() {
                return Err(Thrown::type_error(format!(
                    "{F}(): supplied resource is not a valid stream resource"
                )));
            }
            let ctx = Io { res: stream }.into_raw();
            let ptr = unsafe {
                let buf = sys::xmlOutputBufferCreateIO(
                    Some(io::io_write),
                    Some(stream_close),
                    ctx,
                    ptr::null_mut(),
                );
                if buf.is_null() {
                    stream_close(ctx);
                    return Err(Thrown::error(
                        "Error",
                        "Could not construct libxml output buffer",
                    ));
                }
                let w = sys::xmlNewTextWriter(buf);
                if w.is_null() {
                    sys::xmlOutputBufferClose(buf);
                    return Err(Thrown::error("Error", "Could not construct libxml writer"));
                }
                w
            };
            create_static(
                host,
                F,
                &class,
                Writer {
                    ptr,
                    output: ptr::null_mut(),
                },
            )
        }
    }
}

/// `__xmlwriter($method, $this, ...$args)`: an XMLWriter method.
fn fn_method(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = String::from_utf8_lossy(&s(args, 0)).into_owned();
    let this = args.get(1).map(deref).unwrap_or(Value::Null);
    let a = &args[2.min(args.len())..];
    let Some(&(m, _, params)) = op(&method) else {
        return Err(Thrown::error(
            "Error",
            format!("Call to undefined method XMLWriter::{method}()"),
        ));
    };
    let call = Call {
        fname: format!("XMLWriter::{m}"),
        params: params.to_vec(),
    };
    run(host, &call, m, &this, a)
}

/// `__xmlwriter_fn($function, ...$args)`: a procedural `xmlwriter_*`
/// function (PHP-source wrappers in the prelude, so that named arguments
/// work), the writer first.
fn fn_procedural(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let func = String::from_utf8_lossy(&s(args, 0)).into_owned();
    let args = &args[1.min(args.len())..];
    let Some(&(m, f, params)) = OPS.iter().find(|o| o.1 == func) else {
        return Err(Thrown::error(
            "Error",
            format!("Call to undefined function {func}()"),
        ));
    };
    let mut names = vec!["writer"];
    names.extend_from_slice(params);
    let call = Call {
        fname: f.to_string(),
        params: names,
    };
    match m {
        "openUri" => return open_uri(host, &call, None, args),
        "openMemory" => return open_memory(host, &call, None),
        _ => {}
    }
    let this = args.first().map(deref).unwrap_or(Value::Null);
    run(host, &call, m, &this, &args[1.min(args.len())..])
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(
        m,
        &[
            ("__xmlwriter_fn", fn_procedural, 1, None),
            ("__xmlwriter", fn_method, 2, None),
            ("__xmlwriter_to", fn_to, 2, None),
        ],
    );
}

pub const PRELUDE_PHP: &str = include_str!("prelude.php");
