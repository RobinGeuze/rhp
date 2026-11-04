//! libxml2's file and URL I/O through php's streams, and the external
//! entity loader: ports of ext/libxml/libxml.c's
//! `php_libxml_streams_IO_*`, `php_libxml_input_buffer_create_filename`,
//! `php_libxml_output_buffer_create_filename` and
//! `php_libxml_external_entity_loader`.

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use super::ffi::{call_host, guard};
use super::sys::{self, cbytes, cstr};
use crate::{Host, Thrown};

/// A php stream libxml2 reads or writes (the `void *context` of its I/O
/// callbacks).
pub struct Io {
    pub res: Value,
}

impl Io {
    pub fn into_raw(self) -> *mut c_void {
        Box::into_raw(Box::new(self)).cast()
    }
}

/// php_libxml_streams_IO_read
pub unsafe extern "C" fn io_read(ctx: *mut c_void, buf: *mut c_char, len: c_int) -> c_int {
    guard(-1, || {
        let io = unsafe { &*(ctx as *const Io) };
        let want = len.max(0) as usize;
        match call_host(|h| Ok(h.stream_read(&io.res, want as i64))) {
            Some(Some(data)) => {
                let n = data.len().min(want);
                unsafe { ptr::copy_nonoverlapping(data.as_ptr(), buf.cast::<u8>(), n) };
                n as c_int
            }
            _ => -1,
        }
    })
}

/// php_libxml_streams_IO_write
pub unsafe extern "C" fn io_write(ctx: *mut c_void, buf: *const c_char, len: c_int) -> c_int {
    guard(-1, || {
        let io = unsafe { &*(ctx as *const Io) };
        let data = unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), len.max(0) as usize) };
        match call_host(|h| Ok(h.stream_write(&io.res, data))) {
            Some(Some(n)) => n as c_int,
            _ => -1,
        }
    })
}

/// php_libxml_streams_IO_close
pub unsafe extern "C" fn io_close(ctx: *mut c_void) -> c_int {
    guard(-1, || {
        let io = unsafe { Box::from_raw(ctx as *mut Io) };
        // Without a host (libxml2 freed the buffer outside a builtin), the
        // last reference to the resource frees the stream.
        match call_host(|h| Ok(h.stream_close(&io.res))) {
            Some(true) | None => 0,
            Some(false) => -1,
        }
    })
}

/// php_libxml_streams_IO_open_wrapper: open `filename` (a path or URI as
/// libxml2 passes it) through php's streams.
fn open_wrapper(
    h: &mut dyn Host,
    filename: &[u8],
    mode: &[u8],
    read_only: bool,
) -> Result<Option<Value>, Thrown> {
    let fname = super::ffi::fname();
    if filename.windows(3).any(|w| w == b"%00") {
        crate::warn(h, &fname, "URI must not contain percent-encoded NUL bytes")?;
        return Ok(None);
    }
    let c = cbytes(filename);
    let resolved: Vec<u8> = unsafe {
        let uri = sys::xmlParseURI(c.as_ptr().cast());
        let local = !uri.is_null() && {
            let scheme = sys::rhp_xml_uri_scheme(uri);
            scheme.is_null() || cstr(scheme).starts_with(b"file")
        };
        let out = if local {
            let p = sys::xmlURIUnescapeString(c.as_ptr().cast(), 0, ptr::null_mut());
            if p.is_null() {
                None
            } else {
                let v = cstr(p).to_vec();
                sys::rhp_xml_free(p.cast());
                Some(v)
            }
        } else {
            Some(filename.to_vec())
        };
        if !uri.is_null() {
            sys::xmlFreeURI(uri);
        }
        match out {
            Some(v) => v,
            None => return Ok(None),
        }
    };
    // php_stream_locate_url_wrapper: a file:// URL opens as a plain path.
    let (scheme, _) = crate::main::streams::split_scheme(&resolved);
    let is_url = resolved.len() > scheme.len() && resolved[scheme.len()..].starts_with(b"://");
    let plain = !is_url || scheme.eq_ignore_ascii_case(b"file");
    let path: Vec<u8> = if is_url && scheme.eq_ignore_ascii_case(b"file") {
        let localhost =
            resolved.len() >= 17 && resolved[..17].eq_ignore_ascii_case(b"file://localhost/");
        if !localhost
            && resolved.get(7).is_some_and(|&c| c != b'/')
            && resolved.get(6) == Some(&b'/')
        {
            let msg = format!(
                "Remote host file access not supported, {}",
                String::from_utf8_lossy(&resolved)
            );
            crate::warn(h, &fname, &msg)?;
            return Ok(None);
        }
        let mut i = 5 + if localhost { 11 } else { 0 } + 1;
        while resolved.get(i) == Some(&b'/') {
            i += 1;
        }
        resolved[i - 1..].to_vec()
    } else {
        resolved
    };
    // Only fail quietly when a stat says the file is not there: libxml2
    // probes for files (DTDs) whose absence is no error.
    if read_only && plain && h.stream_stat(&path).is_none() {
        return Ok(None);
    }
    // php_libxml_get_stream_context: without libxml's own context the open
    // uses (and first allocates) the default one.
    if super::stream_context().is_none() {
        h.default_stream_context();
    }
    crate::main::streams::clear_errno();
    match h.stream_open(&path, mode) {
        Some(res) => Ok(Some(res)),
        None => {
            crate::open_failed(h, &fname, &path)?;
            Ok(None)
        }
    }
}

fn open_read(filename: &[u8]) -> Option<Value> {
    call_host(|h| open_wrapper(h, filename, b"rb", true)).flatten()
}

fn open_write(filename: &[u8]) -> Option<Value> {
    call_host(|h| open_wrapper(h, filename, b"wb", false)).flatten()
}

/// php_libxml_input_buffer_create_filename (libxml2's per-thread
/// xmlParserInputBufferCreateFilenameDefault hook).
pub unsafe extern "C" fn input_buffer_create(
    uri: *const c_char,
    enc: c_int,
) -> *mut sys::xmlParserInputBuffer {
    guard(ptr::null_mut(), || {
        if super::entity_loader_disabled() || uri.is_null() {
            return ptr::null_mut();
        }
        let Some(res) = open_read(unsafe { cstr(uri) }) else {
            return ptr::null_mut();
        };
        // php sniffs a charset from http response headers here; rhp's
        // streams do not expose them yet.
        let ctx = Io { res }.into_raw();
        let ret =
            unsafe { sys::xmlParserInputBufferCreateIO(Some(io_read), Some(io_close), ctx, enc) };
        if ret.is_null() {
            unsafe { io_close(ctx) };
        }
        ret
    })
}

/// php_libxml_output_buffer_create_filename (the per-thread
/// xmlOutputBufferCreateFilenameDefault hook).
pub unsafe extern "C" fn output_buffer_create(
    uri: *const c_char,
    encoder: *mut sys::xmlCharEncodingHandler,
    _compression: c_int,
) -> *mut sys::xmlOutputBuffer {
    guard(ptr::null_mut(), || {
        let fail = || {
            unsafe { sys::rhp_xml_output_hook_failed(encoder) };
            ptr::null_mut()
        };
        if uri.is_null() {
            return fail();
        }
        let raw = unsafe { cstr(uri) }.to_vec();
        if raw.windows(3).any(|w| w == b"%00") {
            call_host(|h| {
                crate::warn(
                    h,
                    &super::ffi::fname(),
                    "URI must not contain percent-encoded NUL bytes",
                )
            });
            return fail();
        }
        let mut unescaped = None;
        unsafe {
            let puri = sys::xmlParseURI(uri);
            if !puri.is_null() {
                if !sys::rhp_xml_uri_scheme(puri).is_null() {
                    let p = sys::xmlURIUnescapeString(uri, 0, ptr::null_mut());
                    if !p.is_null() {
                        unescaped = Some(cstr(p).to_vec());
                        sys::rhp_xml_free(p.cast());
                    }
                }
                sys::xmlFreeURI(puri);
            }
        }
        let mut res = unescaped.and_then(|u| open_write(&u));
        // Try the URI as written: it may be a strange file name.
        if res.is_none() {
            res = open_write(&raw);
        }
        let Some(res) = res else {
            return fail();
        };
        let ctx = Io { res }.into_raw();
        let ret =
            unsafe { sys::xmlOutputBufferCreateIO(Some(io_write), Some(io_close), ctx, encoder) };
        if ret.is_null() {
            unsafe { io_close(ctx) };
            return fail();
        }
        ret
    })
}

/// libxml2's own loader, saved before rhp installs [`entity_loader`].
pub fn default_entity_loader(
    url: *const c_char,
    id: *const c_char,
    ctxt: sys::Ctxt,
) -> *mut sys::xmlParserInput {
    match super::DEFAULT_LOADER.get().copied().flatten() {
        Some(f) => unsafe { f(url, id, ctxt) },
        None => ptr::null_mut(),
    }
}

/// php_libxml_pre_ext_ent_loader: the process-wide loader. It runs php's
/// loader for a thread whose VM installed the libxml glue, while a builtin
/// is running, and libxml2's own otherwise.
pub unsafe extern "C" fn entity_loader(
    url: *const c_char,
    id: *const c_char,
    ctxt: sys::Ctxt,
) -> *mut sys::xmlParserInput {
    guard(ptr::null_mut(), || {
        if super::thread_installed() && super::ffi::has_host() {
            external_entity_loader(url, id, ctxt)
        } else {
            default_entity_loader(url, id, ctxt)
        }
    })
}

fn opt_str(p: *const c_char) -> Value {
    if p.is_null() {
        Value::Null
    } else {
        Value::str(unsafe { cstr(p) }.to_vec())
    }
}

/// php_libxml_external_entity_loader
fn external_entity_loader(
    url: *const c_char,
    id: *const c_char,
    ctxt: sys::Ctxt,
) -> *mut sys::xmlParserInput {
    let Some(cb) = super::entity_loader_callback() else {
        return default_entity_loader(url, id, ctxt);
    };
    let mut info = PhpArray::new();
    unsafe {
        let (mut dir, mut isn, mut esu, mut ess) =
            (ptr::null(), ptr::null(), ptr::null(), ptr::null());
        if !ctxt.is_null() {
            sys::rhp_xml_ctxt_loader_info(ctxt, &mut dir, &mut isn, &mut esu, &mut ess);
        }
        info.insert(ArrayKey::from_bytes(b"directory"), opt_str(dir));
        info.insert(ArrayKey::from_bytes(b"intSubName"), opt_str(isn.cast()));
        info.insert(ArrayKey::from_bytes(b"extSubURI"), opt_str(esu.cast()));
        info.insert(ArrayKey::from_bytes(b"extSubSystem"), opt_str(ess.cast()));
    }
    let params = [opt_str(id), opt_str(url), Value::Array(info)];
    let mut ret: *mut sys::xmlParserInput = ptr::null_mut();
    let mut resource: Option<Vec<u8>> = None;
    match call_host(|h| h.call_callable(&cb, &params)) {
        None => {
            let name = call_host(|h| Ok(function_name(h, &cb))).unwrap_or_default();
            super::ctx_error(
                ctxt.cast(),
                format!("Call to user entity loader callback '{name}' has failed").as_bytes(),
            );
        }
        Some(Value::Null) => {}
        Some(Value::Resource(r)) => {
            let v = Value::Resource(r);
            let is_stream = call_host(|h| Ok(h.stream_kind(&v).is_some())).unwrap_or(false);
            if is_stream {
                let ctx = Io { res: v }.into_raw();
                ret = unsafe { sys::rhp_xml_input_from_io(ctxt, io_read, io_close, ctx) };
            } else {
                call_host(|h| {
                    let name = callable_name(h, &cb);
                    Err::<(), _>(Thrown::type_error(format!(
                        "{}(): The user entity loader callback \"{}\" has returned a resource, but it is not a stream",
                        super::ffi::fname(),
                        String::from_utf8_lossy(&name)
                    )))
                });
            }
        }
        Some(v) => {
            resource = call_host(|h| to_string(h, &v));
        }
    }
    if ret.is_null() {
        match resource {
            None => {
                let msg = if id.is_null() {
                    b"Failed to load external entity because the resolver function returned null\n"
                        .to_vec()
                } else {
                    [
                        b"Failed to load external entity \"".as_slice(),
                        unsafe { cstr(id) },
                        b"\"\n",
                    ]
                    .concat()
                };
                super::ctx_error(ctxt.cast(), &msg);
            }
            Some(path) => {
                let c = cbytes(&path);
                ret = unsafe { sys::xmlNewInputFromFile(ctxt, c.as_ptr().cast()) };
            }
        }
    }
    ret
}

/// try_convert_to_string
pub(crate) fn to_string(h: &mut dyn Host, v: &Value) -> Result<Vec<u8>, Thrown> {
    match v {
        Value::Array(_) => {
            h.emit_diagnostic(crate::E_WARNING, "Array to string conversion")?;
            Ok(b"Array".to_vec())
        }
        Value::Object(_) => match crate::zend::object_handlers::to_string(h, v)? {
            Some(s) => Ok(s),
            None => {
                let cls = h.object_class_name(v).unwrap_or_default();
                Err(Thrown::error(
                    "Error",
                    format!(
                        "Object of class {} could not be converted to string",
                        String::from_utf8_lossy(&cls)
                    ),
                ))
            }
        },
        Value::Closure(_) => Err(Thrown::error(
            "Error",
            "Object of class Closure could not be converted to string",
        )),
        other => Ok(conv::to_string_bytes(other)),
    }
}

/// zend_get_callable_name: `Class::method`, a function's or closure's name.
pub fn callable_name(h: &dyn Host, cb: &Value) -> Vec<u8> {
    match cb {
        Value::Array(a) if a.len() == 2 => {
            let parts: Vec<Value> = a.iter().map(|(_, x)| super::deref(x)).collect();
            let class = match &parts[0] {
                Value::Object(_) => h.object_class_name(&parts[0]).unwrap_or_default(),
                other => conv::to_string_bytes(other),
            };
            [&class[..], b"::", &conv::to_string_bytes(&parts[1])[..]].concat()
        }
        Value::Closure(_) => h.closure_name(cb).unwrap_or_else(|| b"{closure}".to_vec()),
        Value::Object(_) => [
            &h.object_class_name(cb).unwrap_or_default()[..],
            b"::__invoke",
        ]
        .concat(),
        other => conv::to_string_bytes(other),
    }
}

/// The function's own name (`function_handler->common.function_name`).
fn function_name(h: &dyn Host, cb: &Value) -> String {
    let full = callable_name(h, cb);
    let name = match full.windows(2).rposition(|w| w == b"::") {
        Some(i) if !matches!(cb, Value::Closure(_)) => full[i + 2..].to_vec(),
        _ => full,
    };
    String::from_utf8_lossy(&name).into_owned()
}
