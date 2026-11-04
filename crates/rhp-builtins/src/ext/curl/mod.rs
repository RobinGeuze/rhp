//! The `curl` extension on libcurl: a port of php-src's ext/curl
//! (interface.c, multi.c, share.c, curl_file.c).
//!
//! A `CurlHandle`, `CurlMultiHandle` or `CurlShareHandle` object owns its
//! libcurl handle through a per-thread map keyed by object id; the object
//! free hook ([`object_freed`]) cleans it up, as php's `free_obj` handlers
//! do. libcurl calls back on the VM thread from `curl_easy_perform` and
//! `curl_multi_perform`; callbacks reach PHP through the FFI callback layer
//! (`ext::libxml::ffi`). `curl_multi_*` runs on the VM thread.
//!
//! File paths handed to libcurl (cookie files, CA bundles) are made
//! absolute against rhp's current directory first, so curl_getinfo()'s
//! `cainfo` shows a relative CAINFO made absolute where php shows it as
//! given. SSLCERT and SSLKEY may name a key instead (`pkcs11:…`) and are
//! passed as given.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;
use std::rc::Rc;
use std::sync::Once;

use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, Value};

use crate::ext::zpp::deref;
use crate::object::{self, CastTarget, ClassHandlers, CloneObj, ObjectHandlers};
use crate::{Builtin, Host, Thrown};

// Generated (`rhp-difftest gen curl-consts`).
#[rustfmt::skip]
mod consts;
mod easy;
mod multi;
mod share;
mod sys;

use easy::Easy;
use multi::Multi;
use share::Share;

thread_local! {
    static EASY: RefCell<HashMap<u32, Rc<RefCell<Easy>>>> = RefCell::new(HashMap::new());
    static MULTI: RefCell<HashMap<u32, Rc<RefCell<Multi>>>> = RefCell::new(HashMap::new());
    static SHARE: RefCell<HashMap<u32, Rc<Share>>> = RefCell::new(HashMap::new());
}

/// `curl_global_init`, once per process (php's MINIT).
fn global_init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        sys::curl_global_init(sys::internal("CURL_GLOBAL_DEFAULT") as _);
    });
}

/// A new request: free every handle (object ids restart). Persistent shares
/// stay, as in php's module globals.
pub(crate) fn reset_state() {
    // Easy handles first: a share cannot be freed while one uses it.
    let multis = MULTI.with(|m| m.try_borrow_mut().map(|mut m| std::mem::take(&mut *m)).ok());
    drop(multis);
    let easies = EASY.with(|m| m.try_borrow_mut().map(|mut m| std::mem::take(&mut *m)).ok());
    drop(easies);
    let shares = SHARE.with(|m| m.try_borrow_mut().map(|mut m| std::mem::take(&mut *m)).ok());
    drop(shares);
}

/// The object free hook: php's `curl_free_obj`, `curl_multi_free_obj` and
/// `curl_share_free_obj`.
pub(crate) fn object_freed(id: u32) {
    let take = |id: u32| {
        let e = EASY
            .try_with(|m| m.try_borrow_mut().ok().and_then(|mut m| m.remove(&id)))
            .ok()
            .flatten();
        let mh = MULTI
            .try_with(|m| m.try_borrow_mut().ok().and_then(|mut m| m.remove(&id)))
            .ok()
            .flatten();
        let s = SHARE
            .try_with(|m| m.try_borrow_mut().ok().and_then(|mut m| m.remove(&id)))
            .ok()
            .flatten();
        (e, mh, s)
    };
    // Dropped outside the maps' borrows: freeing a multi releases the easy
    // objects it holds, which re-enters this hook.
    drop(take(id));
}

fn obj_id(v: &Value) -> Option<u32> {
    match v {
        Value::Object(h) => Some(h.borrow().id),
        Value::Ref(r) => obj_id(&r.borrow()),
        _ => None,
    }
}

fn obj_handle(v: &Value) -> Option<ObjectHandle> {
    match v {
        Value::Object(h) => Some(h.clone()),
        Value::Ref(r) => obj_handle(&r.borrow()),
        _ => None,
    }
}

fn easy_of(v: &Value) -> Option<Rc<RefCell<Easy>>> {
    let id = obj_id(v)?;
    EASY.with(|m| m.borrow().get(&id).cloned())
}

fn multi_of(v: &Value) -> Option<Rc<RefCell<Multi>>> {
    let id = obj_id(v)?;
    MULTI.with(|m| m.borrow().get(&id).cloned())
}

fn share_of(v: &Value) -> Option<Rc<Share>> {
    let id = obj_id(v)?;
    SHARE.with(|m| m.borrow().get(&id).cloned())
}

/// The handle classes' handlers (interface.c, multi.c, share.c):
/// `zend_objects_not_comparable`, `curl_cast_object` on easy and multi
/// handles, and a clone only for easy handles.
pub(crate) const CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "CurlHandle",
        handlers: &ObjectHandlers {
            clone_obj: CloneObj::Custom(curl_clone_obj),
            compare: Some(object::not_comparable),
            cast_object: Some(curl_cast_object),
            ..ObjectHandlers::STD
        },
        create_object: true,
    },
    ClassHandlers {
        class: "CurlMultiHandle",
        handlers: &ObjectHandlers {
            clone_obj: CloneObj::None,
            compare: Some(object::not_comparable),
            cast_object: Some(curl_cast_object),
            ..ObjectHandlers::STD
        },
        create_object: true,
    },
    ClassHandlers {
        class: "CurlShareHandle",
        handlers: &ObjectHandlers::OPAQUE,
        create_object: true,
    },
    ClassHandlers {
        class: "CurlSharePersistentHandle",
        handlers: &ObjectHandlers::OPAQUE,
        create_object: true,
    },
];

/// curl_cast_object: `(int)` of a handle is its object id, as it was the
/// resource id before handles were objects.
fn curl_cast_object(
    host: &mut dyn Host,
    h: &ObjectHandle,
    target: CastTarget,
) -> Result<Option<Value>, Thrown> {
    match target {
        CastTarget::Int => Ok(Some(Value::Int(h.borrow().id as i64))),
        t => host.std_cast_object(h, t),
    }
}

/// curl_clone_obj: the copy gets a duplicate of the libcurl handle.
fn curl_clone_obj(
    host: &mut dyn Host,
    src: &ObjectHandle,
    dst: &ObjectHandle,
) -> Result<(), Thrown> {
    let id = src.borrow().id;
    let Some(easy) = EASY.with(|m| m.borrow().get(&id).cloned()) else {
        return Ok(());
    };
    let Some(copy) = easy::duplicate(host, &easy, dst)? else {
        return Err(Thrown::error("Exception", "Failed to clone CurlHandle"));
    };
    let dst_id = dst.borrow().id;
    EASY.with(|m| m.borrow_mut().insert(dst_id, copy));
    Ok(())
}

/// A new object of a curl handle class (no constructor runs, as with
/// `object_init_ex`).
fn new_object(host: &mut dyn Host, class: &str) -> Result<(Value, ObjectHandle), Thrown> {
    let v = host
        .instantiate(class.as_bytes(), Vec::new())
        .ok_or_else(|| Thrown::unsupported("curl", format!("cannot instantiate {class}")))?;
    let h = obj_handle(&v).ok_or_else(|| Thrown::error("Error", "curl: bad object"))?;
    Ok((v, h))
}

// ---------------------------------------------------------------------------
// argument helpers
// ---------------------------------------------------------------------------

fn type_err(host: &dyn Host, f: &str, n: usize, pname: &str, class: &str, v: &Value) -> Thrown {
    Thrown::type_error(format!(
        "{f}(): Argument #{n} (${pname}) must be of type {class}, {} given",
        crate::argparse::value_name(host, &deref(v))
    ))
}

/// `Z_PARAM_OBJECT_OF_CLASS(zid, curl_ce)`.
fn easy_arg(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    pname: &str,
) -> Result<(Value, Rc<RefCell<Easy>>), Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    match easy_of(&v) {
        Some(e) => Ok((v, e)),
        None => Err(type_err(host, f, i + 1, pname, "CurlHandle", &v)),
    }
}

fn multi_arg(
    host: &dyn Host,
    args: &[Value],
    f: &str,
) -> Result<(Value, Rc<RefCell<Multi>>), Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    match multi_of(&v) {
        Some(m) => Ok((v, m)),
        None => Err(type_err(host, f, 1, "multi_handle", "CurlMultiHandle", &v)),
    }
}

/// A `CurlShareHandle` argument (not a persistent one).
fn share_arg(host: &dyn Host, args: &[Value], f: &str) -> Result<Rc<Share>, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    match share_of(&v) {
        Some(s) if !s.persistent => Ok(s),
        _ => Err(type_err(host, f, 1, "share_handle", "CurlShareHandle", &v)),
    }
}

/// An `int` parameter. The VM's stub-signature check has already reported
/// a null as deprecated.
fn long_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    p: &str,
) -> Result<i64, Thrown> {
    if matches!(args.get(i).map(deref), Some(Value::Null)) && !host.caller_strict_types() {
        return Ok(0);
    }
    crate::ext::zpp::long_arg(host, args, i, f, p)
}

/// php's `zval_get_long()` for an option value (objects warn and give 1).
pub(crate) fn zval_get_long(host: &mut dyn Host, v: &Value) -> Result<i64, Thrown> {
    let v = deref(v);
    if matches!(v, Value::Object(_) | Value::Closure(_)) {
        host.emit_diagnostic(
            crate::E_WARNING,
            &format!(
                "Object of class {} could not be converted to int",
                crate::argparse::value_name(host, &v)
            ),
        )?;
    }
    Ok(conv::to_int(&v))
}

fn akey(s: &str) -> ArrayKey {
    ArrayKey::from_bytes(s.as_bytes())
}

/// A NUL-terminated copy of `s`.
fn cstring(s: &[u8]) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap_or_default()
}

/// The bytes of a C string, or empty for NULL.
fn cstr_bytes(p: *const std::ffi::c_char) -> Vec<u8> {
    if p.is_null() {
        Vec::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
    }
}

/// `path` made absolute against rhp's current directory, as libcurl must
/// see it: it resolves relative paths against the process's.
fn absolute_path(path: &[u8]) -> Vec<u8> {
    use std::os::unix::ffi::OsStringExt;
    if path.is_empty() || path.starts_with(b"/") || path == b"-" {
        return path.to_vec();
    }
    match std::env::current_dir() {
        Ok(cwd) => {
            let mut out = cwd.into_os_string().into_vec();
            if !out.ends_with(b"/") {
                out.push(b'/');
            }
            out.extend_from_slice(path);
            out
        }
        Err(_) => path.to_vec(),
    }
}

// ---------------------------------------------------------------------------
// functions without a handle
// ---------------------------------------------------------------------------

fn strerror_fn(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    lookup: unsafe extern "C" fn(std::ffi::c_int) -> *const std::ffi::c_char,
) -> Result<Value, Thrown> {
    let code = long_arg(host, args, 0, f, "error_code")?;
    let p = unsafe { lookup(code as std::ffi::c_int) };
    Ok(if p.is_null() {
        Value::Null
    } else {
        Value::str(cstr_bytes(p))
    })
}

fn fn_curl_strerror(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strerror_fn(host, args, "curl_strerror", sys::curl_easy_strerror)
}

fn fn_curl_multi_strerror(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strerror_fn(host, args, "curl_multi_strerror", sys::curl_multi_strerror)
}

fn fn_curl_share_strerror(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strerror_fn(host, args, "curl_share_strerror", sys::curl_share_strerror)
}

/// The feature names php lists in curl_version() and phpinfo(), with their
/// `CURL_VERSION_*` bits.
const FEATS: &[(&str, &str)] = &[
    ("AsynchDNS", "CURL_VERSION_ASYNCHDNS"),
    ("CharConv", "CURL_VERSION_CONV"),
    ("Debug", "CURL_VERSION_DEBUG"),
    ("GSS-Negotiate", "CURL_VERSION_GSSNEGOTIATE"),
    ("IDN", "CURL_VERSION_IDN"),
    ("IPv6", "CURL_VERSION_IPV6"),
    ("krb4", "CURL_VERSION_KERBEROS4"),
    ("Largefile", "CURL_VERSION_LARGEFILE"),
    ("libz", "CURL_VERSION_LIBZ"),
    ("NTLM", "CURL_VERSION_NTLM"),
    ("NTLMWB", "CURL_VERSION_NTLM_WB"),
    ("SPNEGO", "CURL_VERSION_SPNEGO"),
    ("SSL", "CURL_VERSION_SSL"),
    ("SSPI", "CURL_VERSION_SSPI"),
    ("TLS-SRP", "CURL_VERSION_TLSAUTH_SRP"),
    ("HTTP2", "CURL_VERSION_HTTP2"),
    ("GSSAPI", "CURL_VERSION_GSSAPI"),
    ("KERBEROS5", "CURL_VERSION_KERBEROS5"),
    ("UNIX_SOCKETS", "CURL_VERSION_UNIX_SOCKETS"),
    ("PSL", "CURL_VERSION_PSL"),
    ("HTTPS_PROXY", "CURL_VERSION_HTTPS_PROXY"),
    ("MULTI_SSL", "CURL_VERSION_MULTI_SSL"),
    ("BROTLI", "CURL_VERSION_BROTLI"),
    ("ALTSVC", "CURL_VERSION_ALTSVC"),
    ("HTTP3", "CURL_VERSION_HTTP3"),
    ("UNICODE", "CURL_VERSION_UNICODE"),
    ("ZSTD", "CURL_VERSION_ZSTD"),
    ("HSTS", "CURL_VERSION_HSTS"),
    ("GSASL", "CURL_VERSION_GSASL"),
];

fn version_info() -> Option<&'static sys::curl_version_info_data> {
    global_init();
    let d = unsafe { sys::curl_version_info(sys::php("CURLVERSION_NOW") as _) };
    (!d.is_null()).then(|| unsafe { &*d })
}

/// PHP_MINFO_FUNCTION(curl)'s table rows (one column for "Features"); the
/// ini entries follow.
pub(crate) fn info_rows() -> Vec<Vec<String>> {
    let Some(d) = version_info() else {
        return Vec::new();
    };
    let s = |p: *const std::ffi::c_char| String::from_utf8_lossy(&cstr_bytes(p)).into_owned();
    let mut rows = vec![
        vec!["cURL support".to_string(), "enabled".to_string()],
        vec!["cURL Information".to_string(), s(d.version)],
        vec!["Age".to_string(), d.age.to_string()],
    ];
    if d.features != 0 {
        rows.push(vec!["Features".to_string()]);
        for (name, c) in FEATS {
            let on = d.features as i64 & sys::php(c) != 0;
            rows.push(vec![
                name.to_string(),
                (if on { "Yes" } else { "No" }).to_string(),
            ]);
        }
    }
    let mut protos = Vec::new();
    let mut p = d.protocols;
    while !p.is_null() && unsafe { !(*p).is_null() } {
        protos.push(s(unsafe { *p }));
        p = unsafe { p.add(1) };
    }
    rows.push(vec!["Protocols".to_string(), protos.join(", ")]);
    rows.push(vec!["Host".to_string(), s(d.host)]);
    let mut opt = |name: &str, p: *const std::ffi::c_char| {
        if !p.is_null() {
            rows.push(vec![name.to_string(), s(p)]);
        }
    };
    opt("SSL Version", d.ssl_version);
    opt("ZLib Version", d.libz_version);
    // php's c-ares and libIDN rows sit under `#if defined(CURLVERSION_SECOND)`
    // and `defined(CURLVERSION_THIRD)`, enumerators rather than macros, so
    // php never prints them.
    if d.age >= 3 {
        opt("libSSH Version", d.libssh_version);
    }
    rows
}

/// `curl_version()`: from `curl_version_info` at run time.
fn fn_curl_version(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let Some(d) = version_info() else {
        return Ok(Value::Bool(false));
    };
    let s = |p: *const std::ffi::c_char| Value::str(cstr_bytes(p));
    let mut a = PhpArray::new();
    a.insert(akey("version_number"), Value::Int(d.version_num as i64));
    a.insert(akey("age"), Value::Int(d.age as i64));
    a.insert(akey("features"), Value::Int(d.features as i64));
    let mut fl = PhpArray::new();
    for (name, c) in FEATS {
        fl.insert(
            akey(name),
            Value::Bool(d.features as i64 & sys::php(c) != 0),
        );
    }
    a.insert(akey("feature_list"), Value::Array(fl));
    a.insert(akey("ssl_version_number"), Value::Int(d.ssl_version_num));
    a.insert(akey("version"), s(d.version));
    a.insert(akey("host"), s(d.host));
    a.insert(akey("ssl_version"), s(d.ssl_version));
    a.insert(akey("libz_version"), s(d.libz_version));
    let mut protos = PhpArray::new();
    let mut p = d.protocols;
    while !p.is_null() && unsafe { !(*p).is_null() } {
        protos.push(s(unsafe { *p }));
        p = unsafe { p.add(1) };
    }
    a.insert(akey("protocols"), Value::Array(protos));
    if d.age >= 1 {
        a.insert(akey("ares"), s(d.ares));
        a.insert(akey("ares_num"), Value::Int(d.ares_num as i64));
    }
    if d.age >= 2 {
        a.insert(akey("libidn"), s(d.libidn));
    }
    if d.age >= 3 {
        a.insert(akey("iconv_ver_num"), Value::Int(d.iconv_ver_num as i64));
        a.insert(akey("libssh_version"), s(d.libssh_version));
    }
    if d.age >= 4 {
        a.insert(akey("brotli_ver_num"), Value::Int(d.brotli_ver_num as i64));
        a.insert(akey("brotli_version"), s(d.brotli_version));
    }
    Ok(Value::Array(a))
}

/// `curl_file_create()` (curlfile_ctor on a new CURLFile).
fn fn_curl_file_create(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_file_create";
    let name = crate::ext::zpp::str_arg(host, args, 0, f, "filename")?;
    if name.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!("{f}(): Argument #1 ($filename) must not contain any null bytes"),
        ));
    }
    let mime = crate::ext::zpp::opt_str_arg(host, args, 1, f, "mime_type")?;
    let post = crate::ext::zpp::opt_str_arg(host, args, 2, f, "posted_filename")?;
    let mut props = vec![(b"name".to_vec(), Value::str(name))];
    if let Some(m) = mime {
        props.push((b"mime".to_vec(), Value::str(m)));
    }
    if let Some(p) = post {
        props.push((b"postname".to_vec(), Value::str(p)));
    }
    host.instantiate(b"CURLFile", props)
        .ok_or_else(|| Thrown::unsupported("curl", "cannot instantiate CURLFile"))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn constants() -> Vec<(&'static str, Value)> {
    sys::php_constants()
        .into_iter()
        .map(|(n, v)| (n, Value::Int(v)))
        .collect()
}

/// `#[\Deprecated]` on a curl constant: php's E_DEPRECATED message.
pub fn constant_deprecation(name: &[u8]) -> Option<String> {
    consts::DEPRECATED
        .iter()
        .find(|(n, _, _)| n.as_bytes() == name)
        .map(|(n, since, msg)| format!("Constant {n} is deprecated since {since}, {msg}"))
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let mut put = |name: &'static str, func: crate::NativeFn, min: usize, max: Option<usize>| {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    };
    put("curl_init", easy::fn_curl_init, 0, Some(1));
    put("curl_setopt", easy::fn_curl_setopt, 3, Some(3));
    put("curl_setopt_array", easy::fn_curl_setopt_array, 2, Some(2));
    put("curl_exec", easy::fn_curl_exec, 1, Some(1));
    put("curl_getinfo", easy::fn_curl_getinfo, 1, Some(2));
    put("curl_error", easy::fn_curl_error, 1, Some(1));
    put("curl_errno", easy::fn_curl_errno, 1, Some(1));
    put("curl_strerror", fn_curl_strerror, 1, Some(1));
    put("curl_close", easy::fn_curl_close, 1, Some(1));
    put("curl_reset", easy::fn_curl_reset, 1, Some(1));
    put("curl_copy_handle", easy::fn_curl_copy_handle, 1, Some(1));
    put("curl_escape", easy::fn_curl_escape, 2, Some(2));
    put("curl_unescape", easy::fn_curl_unescape, 2, Some(2));
    put("curl_pause", easy::fn_curl_pause, 2, Some(2));
    put("curl_upkeep", easy::fn_curl_upkeep, 1, Some(1));
    put("curl_version", fn_curl_version, 0, Some(0));
    put("curl_file_create", fn_curl_file_create, 1, Some(3));
    put("curl_multi_init", multi::fn_curl_multi_init, 0, Some(0));
    put(
        "curl_multi_add_handle",
        multi::fn_curl_multi_add_handle,
        2,
        Some(2),
    );
    put(
        "curl_multi_remove_handle",
        multi::fn_curl_multi_remove_handle,
        2,
        Some(2),
    );
    put("curl_multi_exec", multi::fn_curl_multi_exec, 2, Some(2));
    put("curl_multi_select", multi::fn_curl_multi_select, 1, Some(2));
    put(
        "curl_multi_info_read",
        multi::fn_curl_multi_info_read,
        1,
        Some(2),
    );
    put(
        "curl_multi_getcontent",
        multi::fn_curl_multi_getcontent,
        1,
        Some(1),
    );
    put(
        "curl_multi_get_handles",
        multi::fn_curl_multi_get_handles,
        1,
        Some(1),
    );
    put("curl_multi_close", multi::fn_curl_multi_close, 1, Some(1));
    put("curl_multi_errno", multi::fn_curl_multi_errno, 1, Some(1));
    put("curl_multi_strerror", fn_curl_multi_strerror, 1, Some(1));
    put("curl_multi_setopt", multi::fn_curl_multi_setopt, 3, Some(3));
    put("curl_share_init", share::fn_curl_share_init, 0, Some(0));
    put(
        "curl_share_init_persistent",
        share::fn_curl_share_init_persistent,
        1,
        Some(1),
    );
    put("curl_share_setopt", share::fn_curl_share_setopt, 3, Some(3));
    put("curl_share_close", share::fn_curl_share_close, 1, Some(1));
    put("curl_share_errno", share::fn_curl_share_errno, 1, Some(1));
    put("curl_share_strerror", fn_curl_share_strerror, 1, Some(1));
}

pub const PRELUDE_PHP: &str = r#"
final class CurlHandle {
    public function __construct() { throw new Error("Cannot directly construct CurlHandle, use curl_init() instead"); }
}
final class CurlMultiHandle {
    public function __construct() { throw new Error("Cannot directly construct CurlMultiHandle, use curl_multi_init() instead"); }
}
final class CurlShareHandle {
    public function __construct() { throw new Error("Cannot directly construct CurlShareHandle, use curl_share_init() instead"); }
}
final class CurlSharePersistentHandle {
    public readonly array $options;
    public function __construct() { throw new Error("Cannot directly construct CurlSharePersistentHandle, use curl_share_init_persistent() instead"); }
}
class CURLFile {
    public string $name = "";
    public string $mime = "";
    public string $postname = "";
    public function __construct(string $filename, ?string $mime_type = null, ?string $posted_filename = null) {
        if (str_contains($filename, "\0")) {
            throw new ValueError('CURLFile::__construct(): Argument #1 ($filename) must not contain any null bytes');
        }
        $this->name = $filename;
        if ($mime_type !== null) { $this->mime = $mime_type; }
        if ($posted_filename !== null) { $this->postname = $posted_filename; }
    }
    public function getFilename(): string { return $this->name; }
    public function getMimeType(): string { return $this->mime; }
    public function getPostFilename(): string { return $this->postname; }
    public function setMimeType(string $mime_type): void { $this->mime = $mime_type; }
    public function setPostFilename(string $posted_filename): void { $this->postname = $posted_filename; }
}
class CURLStringFile {
    public string $data;
    public string $postname;
    public string $mime;
    public function __construct(string $data, string $postname, string $mime = "application/octet-stream") {
        $this->data = $data;
        $this->postname = $postname;
        $this->mime = $mime;
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_layouts_match() {
        assert_eq!(sys::c_layout(), &sys::rust_layout()[..]);
    }

    #[test]
    fn constants_come_from_curl_h() {
        let c = sys::php_constants();
        let get = |n: &str| c.iter().find(|(k, _)| *k == n).map(|(_, v)| *v);
        assert_eq!(get("CURLOPT_URL"), Some(10002));
        assert_eq!(get("CURLOPT_RETURNTRANSFER"), Some(19913));
        assert_eq!(get("CURLOPT_SAFE_UPLOAD"), Some(-1));
        assert_eq!(get("CURLE_OK"), Some(0));
        assert_eq!(sys::internal("CURLOPT_ERRORBUFFER"), 10010);
        assert_eq!(sys::internal("CURL_ERROR_SIZE"), 256);
        assert_eq!(constants().len(), c.len());
    }

    #[test]
    fn libcurl_is_linked_statically() {
        if env!("RHP_CURL_LINK_MODE") != "static" {
            return;
        }
        global_init();
        let d = unsafe { &*sys::curl_version_info(sys::php("CURLVERSION_NOW") as _) };
        assert_eq!(
            cstr_bytes(d.version),
            env!("RHP_LIBCURL_VERSION").as_bytes(),
            "the linked libcurl is not the one whose headers rhp compiled against"
        );
        let exe = std::env::current_exe().unwrap();
        let (tool, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
            ("otool", &["-L"])
        } else {
            ("ldd", &[])
        };
        let Ok(out) = std::process::Command::new(tool)
            .args(args)
            .arg(&exe)
            .output()
        else {
            return;
        };
        let deps = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{tool} failed: {deps}");
        assert!(!deps.contains("libcurl"), "{deps}");
    }
}
