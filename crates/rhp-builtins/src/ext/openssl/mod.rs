//! ext/openssl on the OpenSSL php links: a port of php-src's
//! ext/openssl/openssl.c, openssl_backend_common.c and openssl_backend_v3.c.
//!
//! - Every call php makes with a library context gets php's own
//!   `OSSL_LIB_CTX` (one per thread, as php's module globals are under
//!   ZTS), loaded from the default openssl.cnf; `openssl.libctx=default`
//!   switches to OpenSSL's global one. Calls php makes without one
//!   (`EVP_PKEY_CTX_new`, `X509_new`, `X509_REQ_new`) stay without.
//! - OpenSSL's error queue is copied into php's 16-entry ring
//!   (`php_openssl_store_errors`), which `openssl_error_string` reads.
//! - `OpenSSLCertificate`, `OpenSSLAsymmetricKey` and
//!   `OpenSSLCertificateSigningRequest` are prelude classes; their OpenSSL
//!   object lives in a per-thread table keyed by the object id and is freed
//!   with the object.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_ulong, CStr, CString};
use std::ptr;
use std::sync::{Once, OnceLock};

use rhp_values::{conv, Value};

use crate::ext::zpp::deref;
use crate::{warn, Host, Thrown};

/// The key and certificate classes' handlers: no clone, not comparable.
pub(crate) static CLASSES: &[crate::object::ClassHandlers] = &[
    crate::object::ClassHandlers {
        class: "OpenSSLAsymmetricKey",
        handlers: &crate::object::ObjectHandlers::OPAQUE,
        create_object: true,
    },
    crate::object::ClassHandlers {
        class: "OpenSSLCertificate",
        handlers: &crate::object::ObjectHandlers::OPAQUE,
        create_object: true,
    },
    crate::object::ClassHandlers {
        class: "OpenSSLCertificateSigningRequest",
        handlers: &crate::object::ObjectHandlers::OPAQUE,
        create_object: true,
    },
];

mod backend;
mod cipher;
mod csr;
mod pkcs;
mod pkey;
mod smime;
pub(crate) mod sys;
mod x509;
pub(crate) mod xp_ssl;

pub(crate) use pkey::verify_with_pem;

// ---------------------------------------------------------------------------
// library state
// ---------------------------------------------------------------------------

/// OPENSSL_VERSION_TEXT / OPENSSL_VERSION_NUMBER: the headers rhp was
/// compiled against, as php reports its own.
pub const VERSION_TEXT: &str = env!("RHP_OPENSSL_VERSION_TEXT");

fn version_number() -> i64 {
    i64::from_str_radix(env!("RHP_OPENSSL_VERSION_NUMBER"), 16).unwrap_or(0)
}

/// `OpenSSL_version(OPENSSL_VERSION)`: the library linked.
pub fn library_version() -> String {
    init();
    cstr_lossy(unsafe { sys::OpenSSL_version(sys::OPENSSL_VERSION) })
}

/// php_openssl_backend_init: once per process.
fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        sys::OPENSSL_init_ssl(sys::OPENSSL_INIT_LOAD_CONFIG, ptr::null_mut());
    });
}

/// `default_ssl_conf_filename`: `$OPENSSL_CONF`, `$SSLEAY_CONF`, else
/// openssl.cnf in OpenSSL's certificate area.
fn conf_filename() -> &'static CStr {
    static NAME: OnceLock<CString> = OnceLock::new();
    NAME.get_or_init(|| {
        init();
        let name = std::env::var_os("OPENSSL_CONF")
            .or_else(|| std::env::var_os("SSLEAY_CONF"))
            .map(|v| std::os::unix::ffi::OsStrExt::as_bytes(v.as_os_str()).to_vec())
            .unwrap_or_else(|| {
                let area = cstr_bytes(unsafe { sys::X509_get_default_cert_area() });
                let mut v = area.to_vec();
                v.extend_from_slice(b"/openssl.cnf");
                v
            });
        let name: Vec<u8> = name.into_iter().take_while(|&b| b != 0).collect();
        CString::new(name).unwrap_or_default()
    })
}

/// php_openssl_backend_init_libctx / destroy_libctx, per thread.
struct LibCtx(*mut sys::OSSL_LIB_CTX);

impl Drop for LibCtx {
    fn drop(&mut self) {
        // The thread's keys and certificates go first: they hold the context.
        let objects = NATIVES
            .try_with(|m| m.try_borrow_mut().map(|mut m| std::mem::take(&mut *m)).ok())
            .ok()
            .flatten();
        drop(objects);
        if !self.0.is_null() {
            unsafe { sys::OSSL_LIB_CTX_free(self.0) };
        }
    }
}

thread_local! {
    static CUSTOM_LIBCTX: LibCtx = {
        init();
        let ctx = unsafe { sys::OSSL_LIB_CTX_new() };
        if !ctx.is_null() {
            unsafe {
                sys::CONF_modules_load_file_ex(
                    ctx,
                    ptr::null(),
                    ptr::null(),
                    sys::CONF_MFLAGS_DEFAULT_SECTION
                        | sys::CONF_MFLAGS_IGNORE_MISSING_FILE
                        | sys::CONF_MFLAGS_IGNORE_RETURN_CODES,
                );
            }
        }
        LibCtx(ctx)
    };
}

/// `PHP_OPENSSL_LIBCTX`.
pub(super) fn libctx() -> *mut sys::OSSL_LIB_CTX {
    init();
    let default = crate::runtime_fns::ini_value(b"openssl.libctx")
        .flatten()
        .is_some_and(|v| v == b"default");
    let custom = CUSTOM_LIBCTX.with(|c| c.0);
    if default || custom.is_null() {
        unsafe { sys::OSSL_LIB_CTX_get0_global_default() }
    } else {
        custom
    }
}

/// `PHP_OPENSSL_PROPQ`: php never sets one.
pub(super) const PROPQ: *const c_char = ptr::null();

// ---------------------------------------------------------------------------
// exceptions raised while a function keeps running
// ---------------------------------------------------------------------------

thread_local! {
    static PENDING: RefCell<Option<Thrown>> = const { RefCell::new(None) };
}

/// `zend_throw_error` in C code that then returns NULL and lets its caller
/// carry on (warnings included) until the function returns: the first one
/// is thrown when the builtin finishes (see [`wrapped`]).
pub(super) fn throw(e: Thrown) {
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        if p.is_none() {
            *p = Some(e);
        }
    });
}

/// `EG(exception)`.
pub(super) fn has_exception() -> bool {
    PENDING.with(|p| p.borrow().is_some())
}

/// The end of every builtin: a pending exception replaces its result.
pub(super) fn finish(r: Result<Value, Thrown>) -> Result<Value, Thrown> {
    let pending = PENDING.with(|p| p.borrow_mut().take());
    match pending {
        Some(e) => Err(e),
        None => r,
    }
}

/// php_error_docref(E_WARNING) unless an exception is pending.
pub(super) fn warn_unless_thrown(
    host: &mut dyn Host,
    fname: &str,
    msg: &str,
) -> Result<(), Thrown> {
    if has_exception() {
        return Ok(());
    }
    warn(host, fname, msg)
}

/// A builtin that goes through [`finish`].
macro_rules! wrapped {
    ($f:path) => {
        (|h: &mut dyn crate::Host, a: &[rhp_values::Value]| super::finish($f(h, a)))
            as crate::NativeFn
    };
}
pub(super) use wrapped;

// ---------------------------------------------------------------------------
// errors (php_openssl_store_errors)
// ---------------------------------------------------------------------------

const ERR_BUFFER_SIZE: usize = 16;

#[derive(Clone, Copy, Default)]
struct Errors {
    buffer: [i32; ERR_BUFFER_SIZE],
    top: usize,
    bottom: usize,
}

thread_local! {
    static ERRORS: Cell<Option<Errors>> = const { Cell::new(None) };
    static ERRORS_MARK: Cell<Option<Errors>> = const { Cell::new(None) };
}

/// php_openssl_store_errors: move OpenSSL's error queue into the ring.
pub(super) fn store_errors() {
    let mut code = unsafe { sys::ERR_get_error() };
    if code == 0 {
        return;
    }
    let mut e = ERRORS.with(|c| c.get()).unwrap_or_default();
    while code != 0 {
        e.top = (e.top + 1) % ERR_BUFFER_SIZE;
        if e.top == e.bottom {
            e.bottom = (e.bottom + 1) % ERR_BUFFER_SIZE;
        }
        // php's ring holds ints.
        e.buffer[e.top] = code as i32;
        code = unsafe { sys::ERR_get_error() };
    }
    ERRORS.with(|c| c.set(Some(e)));
}

/// php_openssl_errors_set_mark.
pub(super) fn errors_set_mark() {
    if let Some(e) = ERRORS.with(|c| c.get()) {
        ERRORS_MARK.with(|c| c.set(Some(e)));
    }
}

/// php_openssl_errors_restore_mark.
pub(super) fn errors_restore_mark() {
    let Some(mut e) = ERRORS.with(|c| c.get()) else {
        return;
    };
    match ERRORS_MARK.with(|c| c.get()) {
        Some(m) => e = m,
        None => {
            e.top = 0;
            e.bottom = 0;
        }
    }
    ERRORS.with(|c| c.set(Some(e)));
}

fn fn_error_string(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    store_errors();
    let Some(mut e) = ERRORS.with(|c| c.get()) else {
        return Ok(Value::Bool(false));
    };
    if e.top == e.bottom {
        return Ok(Value::Bool(false));
    }
    e.bottom = (e.bottom + 1) % ERR_BUFFER_SIZE;
    let val = e.buffer[e.bottom];
    ERRORS.with(|c| c.set(Some(e)));
    if val == 0 {
        return Ok(Value::Bool(false));
    }
    let mut buf = [0 as c_char; 256];
    unsafe { sys::ERR_error_string_n(val as c_ulong, buf.as_mut_ptr(), buf.len()) };
    Ok(Value::str(cstr_bytes(buf.as_ptr()).to_vec()))
}

// ---------------------------------------------------------------------------
// the native objects
// ---------------------------------------------------------------------------

pub(super) const CERT_CLASS: &str = "OpenSSLCertificate";
pub(super) const KEY_CLASS: &str = "OpenSSLAsymmetricKey";
pub(super) const CSR_CLASS: &str = "OpenSSLCertificateSigningRequest";

enum Native {
    Cert(*mut sys::X509),
    Key(*mut sys::EVP_PKEY, bool),
    Csr(*mut sys::X509_REQ),
}

impl Drop for Native {
    fn drop(&mut self) {
        unsafe {
            match *self {
                Native::Cert(p) => sys::X509_free(p),
                Native::Key(p, _) => sys::EVP_PKEY_free(p),
                Native::Csr(p) => sys::X509_REQ_free(p),
            }
        }
    }
}

thread_local! {
    static NATIVES: RefCell<HashMap<u32, Native>> = RefCell::new(HashMap::new());
}

fn obj_id(v: &Value) -> Option<u32> {
    match v {
        Value::Object(h) => Some(h.borrow().id),
        Value::Ref(r) => obj_id(&r.borrow()),
        _ => None,
    }
}

/// A new object of `class` owning `native`.
fn new_object(host: &mut dyn Host, class: &str, native: Native) -> Result<Value, Thrown> {
    let Some(obj) = host.instantiate(class.as_bytes(), Vec::new()) else {
        // a host without native object construction (a unit-test host)
        return Err(Thrown::unsupported(
            class,
            "host cannot instantiate internal classes",
        ));
    };
    if let Some(id) = obj_id(&obj) {
        let old = NATIVES.with(|m| m.borrow_mut().insert(id, native));
        drop(old);
    }
    Ok(obj)
}

/// php_openssl_certificate_object for an `X509` the object now owns.
pub(super) fn cert_object(host: &mut dyn Host, x: *mut sys::X509) -> Result<Value, Thrown> {
    new_object(host, CERT_CLASS, Native::Cert(x))
}

/// php_openssl_pkey_object_init.
pub(super) fn key_object(
    host: &mut dyn Host,
    k: *mut sys::EVP_PKEY,
    is_private: bool,
) -> Result<Value, Thrown> {
    new_object(host, KEY_CLASS, Native::Key(k, is_private))
}

pub(super) fn csr_object(host: &mut dyn Host, r: *mut sys::X509_REQ) -> Result<Value, Thrown> {
    new_object(host, CSR_CLASS, Native::Csr(r))
}

/// The certificate an `OpenSSLCertificate` holds (borrowed).
pub(super) fn cert_of(v: &Value) -> Option<*mut sys::X509> {
    let id = obj_id(v)?;
    NATIVES.with(|m| match m.borrow().get(&id) {
        Some(Native::Cert(p)) => Some(*p),
        _ => None,
    })
}

/// The key and its privacy an `OpenSSLAsymmetricKey` holds (borrowed).
pub(super) fn key_of(v: &Value) -> Option<(*mut sys::EVP_PKEY, bool)> {
    let id = obj_id(v)?;
    NATIVES.with(|m| match m.borrow().get(&id) {
        Some(Native::Key(p, private)) => Some((*p, *private)),
        _ => None,
    })
}

/// The request an `OpenSSLCertificateSigningRequest` holds (borrowed).
pub(super) fn csr_of(v: &Value) -> Option<*mut sys::X509_REQ> {
    let id = obj_id(v)?;
    NATIVES.with(|m| match m.borrow().get(&id) {
        Some(Native::Csr(p)) => Some(*p),
        _ => None,
    })
}

/// The object free hook.
pub(crate) fn object_freed(id: u32) {
    let old = NATIVES
        .try_with(|m| m.try_borrow_mut().ok().and_then(|mut m| m.remove(&id)))
        .ok()
        .flatten();
    drop(old);
}

/// A new request: object ids restart.
pub(crate) fn reset_state() {
    let old = NATIVES.with(|m| m.try_borrow_mut().map(|mut m| std::mem::take(&mut *m)).ok());
    drop(old);
}

pub const PRELUDE_PHP: &str = r#"
final class OpenSSLCertificate {
    public function __construct() { throw new Error("Cannot directly construct OpenSSLCertificate, use openssl_x509_read() instead"); }
}
final class OpenSSLCertificateSigningRequest {
    public function __construct() { throw new Error("Cannot directly construct OpenSSLCertificateSigningRequest, use openssl_csr_new() instead"); }
}
final class OpenSSLAsymmetricKey {
    public function __construct() { throw new Error("Cannot directly construct OpenSSLAsymmetricKey, use openssl_pkey_new() instead"); }
}
"#;

// ---------------------------------------------------------------------------
// callbacks from OpenSSL
// ---------------------------------------------------------------------------

thread_local! {
    static CALLBACK_PANIC: RefCell<Option<Box<dyn std::any::Any + Send>>> =
        const { RefCell::new(None) };
}

/// The body of an `extern "C"` callback OpenSSL calls: a panic must not
/// unwind into C. The first one is kept and the callback (and any later one)
/// returns `default`; [`with_callbacks`] resumes it once the C call returns.
/// OpenSSL calls these without a host, so `ffi::guard` has no frame to keep
/// the panic in.
pub(super) fn guard<R>(default: R, f: impl FnOnce() -> R) -> R {
    if CALLBACK_PANIC.with(|p| p.borrow().is_some()) {
        return default;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => {
            CALLBACK_PANIC.with(|s| *s.borrow_mut() = Some(p));
            default
        }
    }
}

/// Run a C call that may call back into Rust, then resume the panic one of
/// its callbacks caught.
pub(super) fn with_callbacks<R>(f: impl FnOnce() -> R) -> R {
    let r = f();
    if let Some(p) = CALLBACK_PANIC.with(|s| s.borrow_mut().take()) {
        std::panic::resume_unwind(p);
    }
    r
}

// ---------------------------------------------------------------------------
// owned OpenSSL pointers
// ---------------------------------------------------------------------------

/// A pointer freed on drop unless borrowed (`free` is `None`).
pub(super) struct Own<T> {
    pub p: *mut T,
    free: Option<unsafe extern "C" fn(*mut T)>,
}

impl<T> Own<T> {
    pub fn new(p: *mut T, free: unsafe extern "C" fn(*mut T)) -> Own<T> {
        Own {
            p,
            free: Some(free),
        }
    }
    pub fn borrowed(p: *mut T) -> Own<T> {
        Own { p, free: None }
    }
    pub fn null() -> Own<T> {
        Own {
            p: ptr::null_mut(),
            free: None,
        }
    }
    pub fn is_null(&self) -> bool {
        self.p.is_null()
    }
    pub fn is_owned(&self) -> bool {
        self.free.is_some()
    }
    /// Give up ownership.
    pub fn release(mut self) -> *mut T {
        self.free = None;
        self.p
    }
}

impl<T> Drop for Own<T> {
    fn drop(&mut self) {
        if let (Some(f), false) = (self.free, self.p.is_null()) {
            unsafe { f(self.p) };
        }
    }
}

pub(super) unsafe extern "C" fn free_bio(b: *mut sys::BIO) {
    unsafe { sys::BIO_free_all(b) };
}

/// A read-only memory BIO over `data`, which must outlive it.
pub(super) fn mem_buf(data: &[u8]) -> Own<sys::BIO> {
    let len = data.len().min(c_int::MAX as usize) as c_int;
    Own::new(
        unsafe { sys::BIO_new_mem_buf(data.as_ptr().cast(), len) },
        free_bio,
    )
}

/// `BIO_new(BIO_s_mem())`.
pub(super) fn mem_bio() -> Own<sys::BIO> {
    Own::new(unsafe { sys::BIO_new(sys::BIO_s_mem()) }, free_bio)
}

/// What a memory BIO holds (`BIO_get_mem_ptr`).
pub(super) fn bio_bytes(b: *mut sys::BIO) -> Vec<u8> {
    let mut p: *mut c_char = ptr::null_mut();
    let n = unsafe { sys::BIO_get_mem_data(b, &mut p) };
    if p.is_null() || n <= 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(p.cast::<u8>(), n as usize) }.to_vec()
}

/// `BIO_new_file(path, mode)`; NULL on failure.
pub(super) fn bio_file(path: &CStr, mode: &CStr) -> Own<sys::BIO> {
    Own::new(
        unsafe { sys::BIO_new_file(path.as_ptr(), mode.as_ptr()) },
        free_bio,
    )
}

pub(super) fn cstr_bytes<'a>(p: *const c_char) -> &'a [u8] {
    if p.is_null() {
        return b"";
    }
    unsafe { CStr::from_ptr(p) }.to_bytes()
}

pub(super) fn cstr_lossy(p: *const c_char) -> String {
    String::from_utf8_lossy(cstr_bytes(p)).into_owned()
}

/// A C string of `b` up to its first NUL, as C code sees a zend_string.
pub(super) fn cstring(b: &[u8]) -> CString {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    CString::new(&b[..end]).unwrap_or_default()
}

/// `OPENSSL_free`.
pub(super) unsafe fn openssl_free<T>(p: *mut T) {
    if !p.is_null() {
        unsafe { sys::CRYPTO_free(p.cast(), c"".as_ptr(), 0) };
    }
}

// ---------------------------------------------------------------------------
// arguments
// ---------------------------------------------------------------------------

/// Parameter `i`'s name and type in php's stub for `fname`.
pub(super) fn param(fname: &str, i: usize) -> (&'static str, &'static str) {
    use crate::signatures::Signature;
    crate::signatures::function(fname.as_bytes())
        .and_then(|f| f.param(i))
        .map_or(("", ""), |p| (p.name, p.ty))
}

pub(super) fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(deref).unwrap_or(Value::Null)
}

/// `zval_try_get_string`.
pub(super) fn try_string(host: &mut dyn Host, v: &Value) -> Result<Vec<u8>, Thrown> {
    match deref(v) {
        Value::Array(_) => {
            host.emit_diagnostic(crate::E_WARNING, "Array to string conversion")?;
            Ok(b"Array".to_vec())
        }
        Value::Object(_) => {
            let o = deref(v);
            match crate::object::to_string(host, &o)? {
                Some(s) => Ok(s),
                None => Err(Thrown::error(
                    "Error",
                    format!(
                        "Object of class {} could not be converted to string",
                        crate::argparse::zval_value_name(host, &o)
                    ),
                )),
            }
        }
        o => Ok(conv::to_string_bytes(&o)),
    }
}

/// `Z_PARAM_OBJ_OF_CLASS_OR_STR` (and `_OR_NULL`): an object of `class` or
/// a string. `Ok(None)` for null when `nullable`.
pub(super) enum ObjOrStr {
    Obj(Value),
    Str(Vec<u8>),
}

pub(super) fn obj_or_str(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    class: &str,
) -> Result<ObjOrStr, Thrown> {
    let v = arg(args, i);
    let (pname, ty) = param(fname, i);
    if let Value::Object(_) = &v {
        if host
            .object_class_name(&v)
            .is_some_and(|c| c.eq_ignore_ascii_case(class.as_bytes()))
        {
            return Ok(ObjOrStr::Obj(v));
        }
    }
    match &v {
        Value::Str(s) => Ok(ObjOrStr::Str(s.as_bytes().to_vec())),
        Value::Null => {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                &format!(
                    "{fname}(): Passing null to parameter #{} (${pname}) of type {ty} is deprecated",
                    i + 1
                ),
            )?;
            Ok(ObjOrStr::Str(Vec::new()))
        }
        Value::Int(_) | Value::Float(_) | Value::Bool(_) => {
            Ok(ObjOrStr::Str(conv::to_string_bytes(&v)))
        }
        Value::Object(_) => match crate::object::to_string(host, &v)? {
            Some(s) => Ok(ObjOrStr::Str(s)),
            None => Err(type_error(host, fname, i, &v)),
        },
        _ => Err(type_error(host, fname, i, &v)),
    }
}

/// zpp's "must be of type T, X given" (`zend_zval_value_name`).
pub(super) fn type_error(host: &dyn Host, fname: &str, i: usize, v: &Value) -> Thrown {
    let (pname, ty) = param(fname, i);
    let given = crate::argparse::zval_value_name(host, v);
    Thrown::type_error(format!(
        "{fname}(): Argument #{} (${pname}) must be of type {ty}, {given} given",
        i + 1
    ))
}

/// `Z_PARAM_OBJECT_OF_CLASS(…, php_openssl_pkey_ce)`.
pub(super) fn key_arg(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
) -> Result<(Value, *mut sys::EVP_PKEY, bool), Thrown> {
    let v = arg(args, i);
    match key_of(&v) {
        Some((k, private)) => Ok((v, k, private)),
        None => Err(type_error(host, fname, i, &v)),
    }
}

pub(super) fn str_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
) -> Result<Vec<u8>, Thrown> {
    let (pname, _) = param(fname, i);
    crate::ext::zpp::str_arg(host, args, i, fname, pname)
}

pub(super) fn opt_str_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => str_arg(host, args, i, fname).map(Some),
    }
}

pub(super) fn int_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    default: i64,
) -> Result<i64, Thrown> {
    let (pname, _) = param(fname, i);
    crate::ext::zpp::int_arg_or(host, args, i, fname, pname, default)
}

pub(super) fn bool_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    default: bool,
) -> Result<bool, Thrown> {
    let (pname, _) = param(fname, i);
    crate::ext::zpp::bool_arg_or(host, args, i, fname, pname, default)
}

/// `zend_argument_value_error`.
pub(super) fn value_error(fname: &str, i: usize, msg: &str) -> Thrown {
    let (pname, _) = param(fname, i);
    crate::ext::zpp::value_error(fname, i + 1, pname, msg)
}

pub(super) use crate::ext::zpp::set_ref;

/// php_openssl_check_path_ex: the absolute path (`expand_filepath`) of
/// `path` (after `file://` when `file_protocol`). `Ok(None)` after a
/// warning; a NUL byte is a ValueError. `arg_num` is 1-based, 0 for an
/// option without an argument number.
pub(super) fn check_path(
    host: &mut dyn Host,
    fname: &str,
    path: &[u8],
    arg_num: usize,
    file_protocol: bool,
    from_array: bool,
    option: Option<&str>,
) -> Result<Option<CString>, Thrown> {
    if path.is_empty() {
        return Ok(Some(CString::default()));
    }
    let fs = if file_protocol {
        if path.len() <= 7 {
            return Ok(None);
        }
        &path[7..]
    } else {
        path
    };
    let (msg, error) = if fs.contains(&0) {
        ("must not contain any null bytes", true)
    } else {
        // expand_filepath fails on a path of MAXPATHLEN or more
        let real = crate::ext::dom::expand_filepath(fs);
        if fs.len() < libc::PATH_MAX as usize && real.len() < libc::PATH_MAX as usize {
            return Ok(Some(CString::new(real).unwrap_or_default()));
        }
        ("must be a valid file path", false)
    };
    if arg_num == 0 {
        let title = option.unwrap_or("unknown");
        let label = if from_array { "array item" } else { "option" };
        warn(host, fname, &format!("Path for {title} {label} {msg}"))?;
        return Ok(None);
    }
    let text = match (from_array, option) {
        (true, Some(o)) => format!("option {o} array item {msg}"),
        (true, None) => format!("array item {msg}"),
        (false, Some(o)) => format!("option {o} {msg}"),
        (false, None) => msg.to_string(),
    };
    if error {
        throw(value_error(fname, arg_num - 1, &text));
        return Ok(None);
    }
    // php_verror with the argument's name
    let (pname, _) = param(fname, arg_num - 1);
    crate::warn_arg(host, fname, pname.as_bytes(), &text)?;
    Ok(None)
}

/// php_openssl_bio_new_file.
pub(super) fn bio_new_file(
    host: &mut dyn Host,
    fname: &str,
    filename: &[u8],
    arg_num: usize,
    mode: &CStr,
) -> Result<Own<sys::BIO>, Thrown> {
    let Some(path) = check_path(host, fname, filename, arg_num, false, false, None)? else {
        return Ok(Own::null());
    };
    let b = bio_file(&path, mode);
    if b.is_null() {
        store_errors();
    }
    Ok(b)
}

// ---------------------------------------------------------------------------
// phpinfo, constants, registration
// ---------------------------------------------------------------------------

/// PHP_MINFO_FUNCTION(openssl)'s rows.
pub fn info_rows() -> Vec<(&'static str, String)> {
    vec![
        ("OpenSSL support", "enabled".into()),
        ("OpenSSL Library Version", library_version()),
        ("OpenSSL Header Version", VERSION_TEXT.into()),
        (
            "Openssl default config",
            String::from_utf8_lossy(conf_filename().to_bytes()).into_owned(),
        ),
    ]
}

fn fn_get_cert_locations(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    init();
    let mut a = rhp_values::PhpArray::new();
    let mut add = |k: &str, v: Vec<u8>| {
        a.insert(
            rhp_values::ArrayKey::from_bytes(k.as_bytes()),
            Value::str(v),
        );
    };
    unsafe {
        add(
            "default_cert_file",
            cstr_bytes(sys::X509_get_default_cert_file()).to_vec(),
        );
        add(
            "default_cert_file_env",
            cstr_bytes(sys::X509_get_default_cert_file_env()).to_vec(),
        );
        add(
            "default_cert_dir",
            cstr_bytes(sys::X509_get_default_cert_dir()).to_vec(),
        );
        add(
            "default_cert_dir_env",
            cstr_bytes(sys::X509_get_default_cert_dir_env()).to_vec(),
        );
        add(
            "default_private_dir",
            cstr_bytes(sys::X509_get_default_private_dir()).to_vec(),
        );
        add(
            "default_default_cert_area",
            cstr_bytes(sys::X509_get_default_cert_area()).to_vec(),
        );
    }
    let ini = |n: &[u8]| {
        crate::runtime_fns::ini_value(n)
            .flatten()
            .unwrap_or_default()
    };
    add("ini_cafile", ini(b"openssl.cafile"));
    add("ini_capath", ini(b"openssl.capath"));
    Ok(Value::Array(a))
}

pub fn register(m: &mut crate::Registry) {
    x509::register(m);
    pkey::register(m);
    csr::register(m);
    cipher::register(m);
    pkcs::register(m);
    smime::register(m);
    crate::register(
        m,
        &[
            ("openssl_error_string", fn_error_string),
            ("openssl_get_cert_locations", fn_get_cert_locations),
        ],
    );
}

/// The constants, in php's registration order.
pub fn constants() -> Vec<(&'static str, Value)> {
    let i = |n: &'static str, v: i64| (n, Value::Int(v));
    vec![
        ("OPENSSL_VERSION_TEXT", Value::str(VERSION_TEXT)),
        i("OPENSSL_VERSION_NUMBER", version_number()),
        i("X509_PURPOSE_SSL_CLIENT", 1),
        i("X509_PURPOSE_SSL_SERVER", 2),
        i("X509_PURPOSE_NS_SSL_SERVER", 3),
        i("X509_PURPOSE_SMIME_SIGN", 4),
        i("X509_PURPOSE_SMIME_ENCRYPT", 5),
        i("X509_PURPOSE_CRL_SIGN", 6),
        i("X509_PURPOSE_ANY", 7),
        i("X509_PURPOSE_OCSP_HELPER", 8),
        i("X509_PURPOSE_TIMESTAMP_SIGN", 9),
        i("OPENSSL_ALGO_SHA1", 1),
        i("OPENSSL_ALGO_MD5", 2),
        i("OPENSSL_ALGO_MD4", 3),
        i("OPENSSL_ALGO_SHA224", 6),
        i("OPENSSL_ALGO_SHA256", 7),
        i("OPENSSL_ALGO_SHA384", 8),
        i("OPENSSL_ALGO_SHA512", 9),
        i("OPENSSL_ALGO_RMD160", 10),
        i("PKCS7_DETACHED", 0x40),
        i("PKCS7_TEXT", 0x1),
        i("PKCS7_NOINTERN", 0x10),
        i("PKCS7_NOVERIFY", 0x20),
        i("PKCS7_NOCHAIN", 0x8),
        i("PKCS7_NOCERTS", 0x2),
        i("PKCS7_NOATTR", 0x100),
        i("PKCS7_BINARY", 0x80),
        i("PKCS7_NOSIGS", 0x4),
        i("PKCS7_NOOLDMIMETYPE", 0x400),
        i("PKCS7_NOSMIMECAP", 0x200),
        i("PKCS7_CRLFEOL", 0x800),
        i("PKCS7_NOCRL", 0x2000),
        i("PKCS7_NO_DUAL_CONTENT", 0x10000),
        i("OPENSSL_CMS_DETACHED", 0x40),
        i("OPENSSL_CMS_TEXT", 0x1),
        i("OPENSSL_CMS_NOINTERN", 0x10),
        i("OPENSSL_CMS_NOVERIFY", 0x20),
        i("OPENSSL_CMS_NOCERTS", 0x2),
        i("OPENSSL_CMS_NOATTR", 0x100),
        i("OPENSSL_CMS_BINARY", 0x80),
        i("OPENSSL_CMS_NOSIGS", 0xc),
        i("OPENSSL_CMS_OLDMIMETYPE", 0x400),
        i("OPENSSL_PKCS1_PADDING", 1),
        i("OPENSSL_NO_PADDING", 3),
        i("OPENSSL_PKCS1_OAEP_PADDING", 4),
        i("OPENSSL_PKCS1_PSS_PADDING", 6),
        (
            "OPENSSL_DEFAULT_STREAM_CIPHERS",
            Value::str(DEFAULT_STREAM_CIPHERS),
        ),
        i("OPENSSL_CIPHER_RC2_40", 0),
        i("OPENSSL_CIPHER_RC2_128", 1),
        i("OPENSSL_CIPHER_RC2_64", 2),
        i("OPENSSL_CIPHER_DES", 3),
        i("OPENSSL_CIPHER_3DES", 4),
        i("OPENSSL_CIPHER_AES_128_CBC", 5),
        i("OPENSSL_CIPHER_AES_192_CBC", 6),
        i("OPENSSL_CIPHER_AES_256_CBC", 7),
        i("OPENSSL_KEYTYPE_RSA", 0),
        i("OPENSSL_KEYTYPE_DSA", 1),
        i("OPENSSL_KEYTYPE_DH", 2),
        i("OPENSSL_KEYTYPE_EC", 3),
        i("OPENSSL_KEYTYPE_X25519", 4),
        i("OPENSSL_KEYTYPE_ED25519", 5),
        i("OPENSSL_KEYTYPE_X448", 6),
        i("OPENSSL_KEYTYPE_ED448", 7),
        i("OPENSSL_RAW_DATA", 1),
        i("OPENSSL_ZERO_PADDING", 2),
        i("OPENSSL_DONT_ZERO_PAD_KEY", 4),
        i("OPENSSL_TLSEXT_SERVER_NAME", 1),
        i("OPENSSL_ENCODING_DER", 0),
        i("OPENSSL_ENCODING_SMIME", 1),
        i("OPENSSL_ENCODING_PEM", 2),
    ]
}

/// php_openssl.h's OPENSSL_DEFAULT_STREAM_CIPHERS.
const DEFAULT_STREAM_CIPHERS: &str = "ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:\
ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-AES256-GCM-SHA384:DHE-RSA-AES128-GCM-SHA256:\
DHE-DSS-AES128-GCM-SHA256:kEDH+AESGCM:ECDHE-RSA-AES128-SHA256:ECDHE-ECDSA-AES128-SHA256:\
ECDHE-RSA-AES128-SHA:ECDHE-ECDSA-AES128-SHA:ECDHE-RSA-AES256-SHA384:ECDHE-ECDSA-AES256-SHA384:\
ECDHE-RSA-AES256-SHA:ECDHE-ECDSA-AES256-SHA:DHE-RSA-AES128-SHA256:DHE-RSA-AES128-SHA:\
DHE-DSS-AES128-SHA256:DHE-RSA-AES256-SHA256:DHE-DSS-AES256-SHA:DHE-RSA-AES256-SHA:\
AES128-GCM-SHA256:AES256-GCM-SHA384:AES128:AES256:HIGH:!SSLv2:!aNULL:!eNULL:!EXPORT:!DES:\
!MD5:!RC4:!ADH";

#[cfg(test)]
mod tests;
