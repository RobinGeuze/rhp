//! The `libxml` extension: php's glue over libxml2 (ext/libxml/libxml.c):
//! the error store behind `libxml_use_internal_errors` / `libxml_get_errors`,
//! the `LibXMLError` class, the stream context and external entity loader,
//! php's stream-based I/O hooks ([`io`]), the `LIBXML_*` constants, and the
//! once-per-process and once-per-thread libxml2 setup.
//!
//! [`sys`] declares the libxml2 calls, [`ffi`] is how C callbacks reach the
//! VM, [`node`] is php's ownership of libxml2 trees. ext/xml and ext/xmlwriter run on libxml2. DOM, SimpleXML and XMLReader
//! still run on the hand-written parser and tree below (`parser`, `tree`,
//! `xpath`, `xsd`, `charset`) and report into the same error store.

pub mod charset;
pub mod ffi;
pub mod io;
pub(crate) mod node;
pub mod parser;
pub mod sys;
pub mod tree;
pub mod xpath;
pub mod xsd;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void};
use std::sync::{Once, OnceLock};

use rhp_values::{conv, PhpArray, Value};

use crate::{Builtin, Host, Thrown};
pub use parser::XmlError;

pub(crate) fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// How a diagnostic is shown as a php warning when internal errors are off.
#[derive(Clone, Copy, PartialEq)]
pub enum Ctx {
    /// A parser diagnostic: `msg in <file|Entity>, line: N`.
    Parser,
    /// Anything else (validation, I/O): the bare message.
    Plain,
    /// libxml2's generic error handler (no parser context registered, as
    /// SimpleXML loads): `Entity: line N: parser error : msg`, then the
    /// source line and a caret line, each its own warning.
    Generic,
}

/// php's LIBXML() globals.
struct State {
    /// `LIBXML(error_list)`: set while internal errors are on.
    error_list: Option<Vec<XmlError>>,
    /// `LIBXML(error_buffer)`: a message libxml2 delivers in pieces.
    error_buffer: Vec<u8>,
    /// The last error of the hand-written parser; libxml2 keeps its own
    /// (`xmlGetLastError`).
    last: Option<XmlError>,
    /// `LIBXML(stream_context)`.
    stream_context: Option<Value>,
    /// `LIBXML(entity_loader_callback)`.
    entity_loader: Option<Value>,
    /// `LIBXML(entity_loader_disabled)`.
    entity_loader_disabled: bool,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State {
        error_list: None,
        error_buffer: Vec::new(),
        last: None,
        stream_context: None,
        entity_loader: None,
        entity_loader_disabled: false,
    }) };
    /// libxml2's per-thread handlers are php's on this thread.
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// libxml2's external entity loader from before [`init`] replaced it.
static DEFAULT_LOADER: OnceLock<Option<sys::xmlExternalEntityLoader>> = OnceLock::new();

/// php_libxml_initialize: process-wide, once. The entity loader is a
/// process global in libxml2, so it is set once to a loader that checks the
/// calling thread.
pub fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        sys::rhp_xml_process_init();
        DEFAULT_LOADER.get_or_init(|| sys::xmlGetExternalEntityLoader());
        sys::xmlSetExternalEntityLoader(Some(io::entity_loader));
        sys::rhp_xml_set_msg_sink(Some(msg_sink));
    });
}

/// RINIT plus post_deactivate for the VM starting on this thread: libxml2's
/// error handlers and I/O hooks are per thread.
fn thread_init() {
    init();
    unsafe {
        sys::rhp_xml_set_generic_error(1);
        sys::xmlParserInputBufferCreateFilenameDefault(Some(io::input_buffer_create));
        sys::xmlOutputBufferCreateFilenameDefault(Some(io::output_buffer_create));
        sys::xmlSetStructuredErrorFunc(std::ptr::null_mut(), None);
        sys::xmlResetLastError();
    }
    INSTALLED.with(|i| i.set(true));
}

pub(crate) fn thread_installed() -> bool {
    INSTALLED.with(|i| i.get())
}

/// A new request on this thread.
pub fn reset_state() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.error_list = None;
        s.error_buffer.clear();
        s.last = None;
        s.stream_context = None;
        s.entity_loader = None;
        s.entity_loader_disabled = false;
    });
    thread_init();
    crate::ext::xml::reset_state();
}

/// php_libxml_issue_error's internal-errors half: record `msg` as an
/// XML_ERR_INTERNAL_ERROR when internal errors are on; false otherwise.
pub fn push_internal_error(msg: &str) -> bool {
    STATE.with(|s| match s.borrow_mut().error_list.as_mut() {
        Some(l) => {
            l.push(plain_error(
                sys::XML_ERR_ERROR,
                sys::XML_ERR_INTERNAL_ERROR,
                0,
                0,
                msg.to_string(),
            ));
            true
        }
        None => false,
    })
}

/// php_libxml_uses_internal_errors
pub fn use_internal() -> bool {
    STATE.with(|s| s.borrow().error_list.is_some())
}

pub(crate) fn entity_loader_disabled() -> bool {
    STATE.with(|s| s.borrow().entity_loader_disabled)
}

pub(crate) fn entity_loader_callback() -> Option<Value> {
    STATE.with(|s| s.borrow().entity_loader.clone())
}

/// The stream context `libxml_set_streams_context` set, if any.
pub fn stream_context() -> Option<Value> {
    STATE.with(|s| s.borrow().stream_context.clone())
}

/// A copy of libxml2's error record.
fn error_from_raw(e: &sys::XmlErrorRaw) -> XmlError {
    let text = |p: *mut c_char| unsafe { sys::cstr(p) }.to_vec();
    XmlError {
        domain: e.domain,
        level: e.level,
        code: e.code,
        line: e.line,
        column: e.int2,
        message: String::from_utf8_lossy(&text(e.message)).into_owned(),
        file: (!e.file.is_null()).then(|| text(e.file)),
        context: None,
    }
}

/// php_libxml_structured_error_handler: internal errors are on.
unsafe extern "C" fn structured_error(_user: *mut c_void, e: *const sys::XmlErrorRaw) {
    ffi::guard((), || {
        if let Some(e) = unsafe { e.as_ref() } {
            let e = error_from_raw(e);
            STATE.with(|s| {
                if let Some(l) = s.borrow_mut().error_list.as_mut() {
                    l.push(e);
                }
            });
        }
    })
}

// php_libxml_error_level
const PHP_LIBXML_ERROR: c_int = 0;
const PHP_LIBXML_CTX_ERROR: c_int = 1;
const PHP_LIBXML_CTX_WARNING: c_int = 2;

/// shim.c hands every printf-style libxml2 callback here, formatted:
/// php_libxml_error_handler_va.
unsafe extern "C" fn msg_sink(kind: c_int, ctx: *mut c_void, msg: *const c_char, len: usize) {
    ffi::guard((), || {
        let buf = unsafe { std::slice::from_raw_parts(msg.cast::<u8>(), len) };
        let (mut line, mut col) = (0, 0);
        // The context is no parser for PHP_LIBXML_ERROR.
        if kind != PHP_LIBXML_ERROR && !ctx.is_null() {
            let mut f = std::ptr::null();
            unsafe { sys::rhp_xml_ctxt_input_pos(ctx.cast(), &mut f, &mut line, &mut col) };
        }
        internal_error_handler(kind, ctx, buf, line, col);
    })
}

/// php_libxml_ctx_error with a formatted message.
pub(crate) fn ctx_error(ctx: *mut c_void, msg: &[u8]) {
    let (mut line, mut col) = (0, 0);
    if !ctx.is_null() {
        let mut f = std::ptr::null();
        unsafe { sys::rhp_xml_ctxt_input_pos(ctx.cast(), &mut f, &mut line, &mut col) };
    }
    internal_error_handler(PHP_LIBXML_CTX_ERROR, ctx, msg, line, col);
}

/// php_libxml_internal_error_handler_ex: collect pieces up to a trailing
/// newline, then record the message (internal errors on) or raise it.
fn internal_error_handler(kind: c_int, ctx: *mut c_void, buf: &[u8], line: c_int, column: c_int) {
    // php overwrites the trailing newlines with NULs and appends the whole
    // buffer; the message is read up to the first NUL.
    let mut piece = buf.to_vec();
    let mut output = false;
    let mut i = piece.len();
    while i > 0 && piece[i - 1] == b'\n' {
        i -= 1;
        piece[i] = 0;
        output = true;
    }
    let msg = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.error_buffer.extend_from_slice(&piece);
        if !output {
            return None;
        }
        let b = std::mem::take(&mut s.error_buffer);
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        let msg = b[..end].to_vec();
        if let Some(l) = s.error_list.as_mut() {
            l.push(XmlError {
                domain: 0,
                level: sys::XML_ERR_ERROR,
                code: sys::XML_ERR_INTERNAL_ERROR,
                line,
                column,
                message: String::from_utf8_lossy(&msg).into_owned(),
                file: None,
                context: None,
            });
            return None;
        }
        Some(msg)
    });
    // Don't add notices or warnings after an exception.
    let Some(msg) = msg else { return };
    if ffi::aborted() {
        return;
    }
    let msg = String::from_utf8_lossy(&msg).into_owned();
    let fname = ffi::fname();
    match kind {
        PHP_LIBXML_CTX_ERROR => ctx_error_level(crate::E_WARNING, ctx, &fname, &msg, line),
        PHP_LIBXML_CTX_WARNING => ctx_error_level(crate::E_NOTICE, ctx, &fname, &msg, line),
        _ => {
            ffi::call_host(|h| crate::warn(h, &fname, &msg));
        }
    }
}

/// php_libxml_ctx_error_level
fn ctx_error_level(level: i64, ctx: *mut c_void, fname: &str, msg: &str, line: c_int) {
    let mut file = std::ptr::null();
    let (mut l, mut c) = (0, 0);
    let has_input = !ctx.is_null()
        && unsafe { sys::rhp_xml_ctxt_input_pos(ctx.cast(), &mut file, &mut l, &mut c) } != 0;
    if has_input {
        let src = if file.is_null() {
            "Entity".to_string()
        } else {
            String::from_utf8_lossy(unsafe { sys::cstr(file) }).into_owned()
        };
        ffi::call_host(|h| {
            h.emit_diagnostic(level, &format!("{fname}(): {msg} in {src}, line: {line}"))
        });
    } else {
        ffi::call_host(|h| crate::warn(h, fname, msg));
    }
}

/// Record an error of the hand-written parser as the last error, in place
/// of libxml2's.
fn set_last(e: &XmlError) {
    unsafe { sys::xmlResetLastError() };
    STATE.with(|s| s.borrow_mut().last = Some(e.clone()));
}

/// Route libxml2 diagnostics the way php's libxml glue does: always record
/// the last error; collect into the error list when internal errors are on,
/// else raise each one as an E_WARNING from `fname`.
pub fn report(
    host: &mut dyn Host,
    fname: &str,
    errors: Vec<XmlError>,
    ctx: Ctx,
) -> Result<(), Thrown> {
    for e in errors {
        set_last(&e);
        let internal = STATE.with(|s| match s.borrow_mut().error_list.as_mut() {
            Some(l) => {
                l.push(e.clone());
                true
            }
            None => false,
        });
        if internal {
            continue;
        }
        let msg = e.message.trim_end_matches('\n');
        let text = match ctx {
            Ctx::Parser => {
                let file = match &e.file {
                    Some(f) => String::from_utf8_lossy(f).into_owned(),
                    None => "Entity".to_string(),
                };
                format!("{msg} in {file}, line: {}", e.line)
            }
            Ctx::Plain => msg.to_string(),
            Ctx::Generic => {
                let mut head = String::new();
                match &e.file {
                    Some(f) => {
                        head.push_str(&format!("{}:{}: ", String::from_utf8_lossy(f), e.line))
                    }
                    None if e.line != 0 && e.domain == 1 => {
                        head.push_str(&format!("Entity: line {}: ", e.line))
                    }
                    None => {}
                }
                head.push_str(match e.domain {
                    1 => "parser ",
                    3 => "namespace ",
                    _ => "",
                });
                head.push_str(if e.level == 1 {
                    "warning : "
                } else {
                    "error : "
                });
                head.push_str(msg);
                crate::warn(host, fname, &head)?;
                if let Some((content, caret)) = &e.context {
                    crate::warn(host, fname, &String::from_utf8_lossy(content))?;
                    crate::warn(host, fname, &String::from_utf8_lossy(caret))?;
                }
                continue;
            }
        };
        crate::warn(host, fname, &text)?;
    }
    Ok(())
}

/// Report the serializer's diagnostics (see `tree::take_save_diags`). The
/// output-module ones are structured errors (they also become the last
/// error); code 1 comes through libxml2's generic channel, which php turns
/// into an XML_ERR_INTERNAL_ERROR record without touching the last error.
pub fn report_save_diags(host: &mut dyn Host, fname: &str) -> Result<(), Thrown> {
    for (code, msg) in tree::take_save_diags() {
        if code == 1 {
            let internal = STATE.with(|s| match s.borrow_mut().error_list.as_mut() {
                Some(l) => {
                    l.push(plain_error(2, 1, 0, 0, msg.to_string()));
                    true
                }
                None => false,
            });
            if !internal {
                crate::warn(host, fname, msg)?;
            }
        } else {
            report(
                host,
                fname,
                vec![plain_error(2, code, 7, 0, msg.to_string())],
                Ctx::Plain,
            )?;
        }
    }
    Ok(())
}

/// A libxml2 tree-module error (`xmlTreeErr`, e.g. XML_TREE_UNTERMINATED_ENTITY
/// = 1302): an XML_FROM_TREE record with line -1, or a warning from `fname`.
pub fn report_tree_error(host: &mut dyn Host, fname: &str, msg: &str) -> Result<(), Thrown> {
    report(
        host,
        fname,
        vec![plain_error(2, 1302, 2, -1, format!("{msg}\n"))],
        Ctx::Plain,
    )
}

/// The user function a property hook runs on behalf of (what php's
/// `get_active_function_name` reports for a DOM property write): the frame
/// below the prelude's `__set`, or `main` at top level.
pub fn active_user_function(host: &dyn Host) -> String {
    let st = host.call_stack();
    let mut it = st.iter().skip_while(|(f, ..)| f.as_slice() != b"__set");
    if it.next().is_none() {
        return "main".into();
    }
    match it.next() {
        Some((f, c, ..)) if !f.is_empty() => match c {
            Some(c) => format!(
                "{}::{}",
                String::from_utf8_lossy(c),
                String::from_utf8_lossy(f)
            ),
            None => String::from_utf8_lossy(f).into_owned(),
        },
        _ => "main".into(),
    }
}

/// Write serialized output to `path` through the stream layer (libxml's
/// php_libxml_streams_IO_open_write_wrapper): the byte count, or `None` after
/// php's "Failed to open stream" warning from `fname`.
pub fn save_to_uri(
    host: &mut dyn Host,
    fname: &str,
    path: &[u8],
    data: &[u8],
) -> Result<Option<usize>, Thrown> {
    crate::streams::clear_errno();
    match host.stream_put_contents(path, data) {
        Some(_) => Ok(Some(data.len())),
        None => {
            crate::open_failed(host, fname, path)?;
            Ok(None)
        }
    }
}

/// A non-parser error record (validation, I/O): no file/column.
pub fn plain_error(level: i32, code: i32, domain: i32, line: i32, msg: String) -> XmlError {
    XmlError {
        domain,
        level,
        code,
        line,
        column: 0,
        message: msg,
        file: None,
        context: None,
    }
}

/// Report php's "I/O warning : failed to load external entity" for `path`.
pub fn report_io(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<(), Thrown> {
    let p = String::from_utf8_lossy(path).into_owned();
    let e = plain_error(
        1,
        1549,
        8,
        0,
        format!("failed to load external entity \"{p}\"\n"),
    );
    set_last(&e);
    let internal = STATE.with(|s| match s.borrow_mut().error_list.as_mut() {
        Some(l) => {
            l.push(e.clone());
            true
        }
        None => false,
    });
    if !internal {
        crate::warn(
            host,
            fname,
            &format!("I/O warning : failed to load external entity \"{p}\""),
        )?;
    }
    Ok(())
}

/// php_libxml_create_error_object
fn error_object(host: &mut dyn Host, e: &XmlError) -> Value {
    let props = vec![
        (b"level".to_vec(), Value::Int(e.level as i64)),
        (b"code".to_vec(), Value::Int(e.code as i64)),
        (b"column".to_vec(), Value::Int(e.column as i64)),
        (
            b"message".to_vec(),
            Value::str(e.message.as_bytes().to_vec()),
        ),
        (
            b"file".to_vec(),
            Value::str(e.file.clone().unwrap_or_default()),
        ),
        (b"line".to_vec(), Value::Int(e.line as i64)),
    ];
    host.instantiate(b"LibXMLError", props)
        .unwrap_or(Value::Null)
}

fn fn_use_internal_errors(_h: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arg = args.first().map(deref);
    let prev = use_internal();
    match arg {
        None | Some(Value::Null) => {}
        Some(v) => {
            let on = conv::to_bool(&v);
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                if on {
                    unsafe {
                        sys::xmlSetStructuredErrorFunc(std::ptr::null_mut(), Some(structured_error))
                    };
                    s.error_list.get_or_insert_with(Vec::new);
                } else {
                    unsafe { sys::xmlSetStructuredErrorFunc(std::ptr::null_mut(), None) };
                    s.error_list = None;
                }
            });
        }
    }
    Ok(Value::Bool(prev))
}

fn fn_get_errors(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let errs = STATE.with(|s| s.borrow().error_list.clone().unwrap_or_default());
    let mut a = PhpArray::new();
    for e in &errs {
        let o = error_object(host, e);
        a.push(o);
    }
    Ok(Value::Array(a))
}

/// The last error: the newest in the list while internal errors are on,
/// else libxml2's, else the hand-written parser's.
fn last_error() -> Option<XmlError> {
    if let Some(l) = STATE.with(|s| s.borrow().error_list.as_ref().map(|l| l.last().cloned())) {
        return l;
    }
    if let Some(e) = unsafe { sys::xmlGetLastError().as_ref() } {
        return Some(error_from_raw(e));
    }
    STATE.with(|s| s.borrow().last.clone())
}

fn fn_get_last_error(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    match last_error() {
        Some(e) => Ok(error_object(host, &e)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_clear_errors(_h: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    unsafe { sys::xmlResetLastError() };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(l) = s.error_list.as_mut() {
            l.clear();
        }
        s.last = None;
    });
    Ok(Value::Null)
}

fn fn_disable_entity_loader(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    crate::deprecated_fn(
        host,
        "libxml_disable_entity_loader",
        "8.0",
        "as external entity loading is disabled by default",
    )?;
    let on = args
        .first()
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(true);
    let prev = STATE.with(|s| std::mem::replace(&mut s.borrow_mut().entity_loader_disabled, on));
    Ok(Value::Bool(prev))
}

fn fn_set_streams_context(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arg = args.first().map(deref).unwrap_or(Value::Null);
    let Value::Resource(r) = &arg else {
        return Err(Thrown::type_error(format!(
            "libxml_set_streams_context(): Argument #1 ($context) must be of type resource, {} given",
            crate::ext::zpp::given_type(host, &arg)
        )));
    };
    if r.type_name != "stream-context" {
        return Err(Thrown::type_error(
            "libxml_set_streams_context(): supplied resource is not a valid Stream-Context resource",
        ));
    }
    STATE.with(|s| s.borrow_mut().stream_context = Some(arg.clone()));
    Ok(Value::Null)
}

fn fn_set_external_entity_loader(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cb = args.first().map(deref).unwrap_or(Value::Null);
    if !matches!(cb, Value::Null) {
        if let Some(why) = host.check_callable(&cb)? {
            return Err(Thrown::type_error(format!(
                "libxml_set_external_entity_loader(): Argument #1 ($resolver_function) must be a valid callback or null, {why}"
            )));
        }
    }
    let cb = (!matches!(cb, Value::Null)).then_some(cb);
    STATE.with(|s| s.borrow_mut().entity_loader = cb);
    Ok(Value::Bool(true))
}

fn fn_get_external_entity_loader(_h: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(entity_loader_callback().unwrap_or(Value::Null))
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(
        m,
        "libxml_use_internal_errors",
        fn_use_internal_errors,
        0,
        Some(1),
    );
    put(m, "libxml_get_errors", fn_get_errors, 0, Some(0));
    put(m, "libxml_get_last_error", fn_get_last_error, 0, Some(0));
    put(m, "libxml_clear_errors", fn_clear_errors, 0, Some(0));
    put(
        m,
        "libxml_disable_entity_loader",
        fn_disable_entity_loader,
        0,
        Some(1),
    );
    put(
        m,
        "libxml_set_streams_context",
        fn_set_streams_context,
        1,
        Some(1),
    );
    put(
        m,
        "libxml_set_external_entity_loader",
        fn_set_external_entity_loader,
        1,
        Some(1),
    );
    put(
        m,
        "libxml_get_external_entity_loader",
        fn_get_external_entity_loader,
        0,
        Some(0),
    );
}

/// `LIBXML_VERSION`: the header php was built against.
pub fn header_version() -> i64 {
    unsafe { sys::rhp_xml_header_version() as i64 }
}

/// `LIBXML_DOTTED_VERSION`.
pub fn header_dotted_version() -> &'static str {
    std::str::from_utf8(unsafe { sys::cstr(sys::rhp_xml_header_dotted_version()) }).unwrap_or("")
}

/// `LIBXML_LOADED_VERSION`: `xmlParserVersion` of the library loaded.
pub fn loaded_version() -> &'static str {
    init();
    std::str::from_utf8(unsafe { sys::cstr(sys::rhp_xml_loaded_version()) }).unwrap_or("")
}

/// libxml.stub.php's constants, values from the libxml2 headers.
pub fn constants() -> Vec<(&'static str, Value)> {
    let mut out = vec![
        ("LIBXML_VERSION", Value::Int(header_version())),
        ("LIBXML_DOTTED_VERSION", Value::str(header_dotted_version())),
        ("LIBXML_LOADED_VERSION", Value::str(loaded_version())),
    ];
    let mut p = unsafe { sys::rhp_xml_constants() };
    loop {
        let c = unsafe { &*p };
        if c.name.is_null() {
            break;
        }
        // shim.c's table is static.
        let name: &'static [u8] = unsafe { sys::cstr(c.name) };
        out.push((std::str::from_utf8(name).unwrap_or(""), Value::Int(c.value)));
        p = unsafe { p.add(1) };
    }
    out
}

pub const PRELUDE_PHP: &str = r#"
class LibXMLError {
    public int $level;
    public int $code;
    public int $column;
    public string $message;
    public string $file;
    public int $line;
}
"#;

/// Drop diagnostics suppressed by LIBXML_NOWARNING / LIBXML_NOERROR.
pub fn filter_by_flags(errors: Vec<XmlError>, flags: i64) -> Vec<XmlError> {
    errors
        .into_iter()
        .filter(|e| !((e.level == 1 && flags & 64 != 0) || (e.level >= 2 && flags & 32 != 0)))
        .collect()
}
