//! `CurlHandle` and the curl_* functions on it: php-src's interface.c.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_double, c_int, c_long, c_void};
use std::rc::{Rc, Weak};

use rhp_values::{conv, ArrayKey, ObjectData, ObjectHandle, PhpArray, Value};

use super::sys::{self, curl_off_t, CURL};
use super::{
    absolute_path, akey, cstr_bytes, cstring, easy_arg, global_init, long_arg, new_object,
    share_of, zval_get_long, EASY,
};
use crate::ext::libxml::ffi;
use crate::ext::zpp::deref;
use crate::{Host, Thrown};

/// php's `PHP_CURL_*` handler methods.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Method {
    Stdout,
    File,
    User,
    Direct,
    Return,
    Ignore,
}

/// `php_curl_write` / `php_curl_read`.
#[derive(Clone)]
pub(super) struct Handler {
    pub method: Method,
    pub stream: Option<Value>,
    pub fcc: Option<Value>,
}

impl Handler {
    fn new(method: Method) -> Handler {
        Handler {
            method,
            stream: None,
            fcc: None,
        }
    }
}

/// A file upload's source (`struct mime_data_cb_arg`): a stream opened
/// when the option is set (any wrapper, as php_stream_open_wrapper), and
/// reopened if libcurl's free callback closed it.
struct MimeArg {
    filename: Vec<u8>,
    stream: RefCell<Option<Value>>,
}

/// php's `to_free`: C allocations a handle and its copies share, because
/// `curl_easy_duphandle` copies the pointers.
#[derive(Default)]
struct ToFree {
    slists: Vec<(i64, *mut sys::curl_slist)>,
    mimes: Vec<*mut sys::curl_mime>,
    /// Boxed: libcurl holds pointers to them.
    #[allow(clippy::vec_box)]
    args: Vec<Box<MimeArg>>,
}

impl Drop for ToFree {
    fn drop(&mut self) {
        for m in self.mimes.drain(..) {
            unsafe { sys::curl_mime_free(m) };
        }
        self.args.clear();
        for (_, s) in self.slists.drain(..) {
            unsafe { sys::curl_slist_free_all(s) };
        }
    }
}

/// The stdio stream libcurl writes verbose output to for CURLOPT_STDERR.
struct CookieFile {
    file: *mut sys::FILE,
    _stream: Box<Value>,
}

impl Drop for CookieFile {
    fn drop(&mut self) {
        if !self.file.is_null() {
            unsafe { sys::fclose(self.file) };
        }
    }
}

/// `php_curl`.
pub(crate) struct Easy {
    pub cp: *mut CURL,
    /// The CurlHandle, passed to callbacks.
    obj: Weak<RefCell<ObjectData>>,
    /// CURLOPT_ERRORBUFFER (`err.str`).
    err: Box<[u8]>,
    pub errno: i32,
    pub write: Handler,
    pub write_header: Handler,
    pub read: Handler,
    /// The CURLOPT_RETURNTRANSFER buffer (`None`: never written).
    pub buf: Option<Vec<u8>>,
    std_err: Option<Value>,
    stderr_file: Option<Rc<CookieFile>>,
    progress: Option<Value>,
    xferinfo: Option<Value>,
    fnmatch: Option<Value>,
    debug: Option<Value>,
    prereq: Option<Value>,
    sshhostkey: Option<Value>,
    /// What CURLINFO_HEADER_OUT reports (`header.str`).
    header_out: Option<Vec<u8>>,
    pub in_callback: bool,
    to_free: Rc<RefCell<ToFree>>,
    /// The CURLOPT_POSTFIELDS array, kept once it held a file: a copy of
    /// the handle rebuilds the body from it.
    postfields: Option<Value>,
    private: Option<Value>,
    share: Option<Value>,
}

impl Drop for Easy {
    /// curl_free_obj. Callbacks during cleanup find no handle.
    fn drop(&mut self) {
        if self.cp.is_null() {
            return;
        }
        for d in DATA_OPTS {
            setopt_ptr(self.cp, opt(d), std::ptr::null());
        }
        unsafe { sys::curl_easy_cleanup(self.cp) };
    }
}

/// The `*DATA` options that point libcurl's callbacks at the handle.
const DATA_OPTS: &[&str] = &[
    "CURLOPT_WRITEDATA",
    "CURLOPT_HEADERDATA",
    "CURLOPT_READDATA",
    "CURLOPT_DEBUGDATA",
    "CURLOPT_PROGRESSDATA",
    "CURLOPT_XFERINFODATA",
    "CURLOPT_FNMATCH_DATA",
    "CURLOPT_PREREQDATA",
    "CURLOPT_SSH_HOSTKEYDATA",
];

impl Easy {
    fn new(cp: *mut CURL, obj: &ObjectHandle) -> Easy {
        let size = sys::internal("CURL_ERROR_SIZE") as usize;
        Easy {
            cp,
            obj: Rc::downgrade(obj),
            err: vec![0u8; size + 1].into_boxed_slice(),
            errno: 0,
            write: Handler::new(Method::Stdout),
            write_header: Handler::new(Method::Ignore),
            read: Handler::new(Method::Direct),
            buf: None,
            std_err: None,
            stderr_file: None,
            progress: None,
            xferinfo: None,
            fnmatch: None,
            debug: None,
            prereq: None,
            sshhostkey: None,
            header_out: None,
            in_callback: false,
            to_free: Rc::new(RefCell::new(ToFree::default())),
            postfields: None,
            private: None,
            share: None,
        }
    }

    /// `_php_curl_cleanup_handle`: forget the last transfer's results.
    pub(super) fn cleanup_handle(&mut self) {
        self.buf = None;
        self.header_out = None;
        self.err.fill(0);
        self.errno = 0;
    }

    fn error_str(&self) -> Vec<u8> {
        let end = self
            .err
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.err.len());
        self.err[..end].to_vec()
    }
}

/// The value of an option or info name: a php constant, or one of
/// consts.rs's `INTERNAL` names.
fn opt(name: &str) -> i64 {
    if sys::php_defined(name) {
        sys::php(name)
    } else {
        sys::internal(name)
    }
}

fn ctx(e: &Rc<RefCell<Easy>>) -> *const c_void {
    Rc::as_ptr(e) as *const c_void
}

fn setopt_long(cp: *mut CURL, option: i64, v: i64) -> c_int {
    unsafe { sys::curl_easy_setopt(cp, option as c_int, v as c_long) }
}

fn setopt_off(cp: *mut CURL, option: i64, v: i64) -> c_int {
    unsafe { sys::curl_easy_setopt(cp, option as c_int, v as curl_off_t) }
}

fn setopt_ptr(cp: *mut CURL, option: i64, p: *const c_void) -> c_int {
    unsafe { sys::curl_easy_setopt(cp, option as c_int, p) }
}

/// The C stdio `stderr`.
fn c_stderr() -> *const c_void {
    unsafe { sys::rhp_curl_stderr() as *const c_void }
}

/// `_php_curl_set_default_options`.
fn set_default_options(host: &dyn Host, e: &Rc<RefCell<Easy>>) {
    let p = ctx(e);
    let (cp, err) = {
        let mut ch = e.borrow_mut();
        (ch.cp, ch.err.as_mut_ptr())
    };
    setopt_long(cp, opt("CURLOPT_NOPROGRESS"), 1);
    setopt_long(cp, opt("CURLOPT_VERBOSE"), 0);
    setopt_ptr(cp, opt("CURLOPT_ERRORBUFFER"), err as *const c_void);
    setopt_ptr(
        cp,
        opt("CURLOPT_WRITEFUNCTION"),
        write_cb as sys::WriteFn as *const c_void,
    );
    setopt_ptr(cp, opt("CURLOPT_WRITEDATA"), p);
    setopt_ptr(
        cp,
        opt("CURLOPT_READFUNCTION"),
        read_cb as sys::ReadFn as *const c_void,
    );
    setopt_ptr(cp, opt("CURLOPT_READDATA"), p);
    setopt_ptr(
        cp,
        opt("CURLOPT_HEADERFUNCTION"),
        header_cb as sys::WriteFn as *const c_void,
    );
    setopt_ptr(cp, opt("CURLOPT_HEADERDATA"), p);
    setopt_long(cp, opt("CURLOPT_DNS_CACHE_TIMEOUT"), 120);
    setopt_long(cp, opt("CURLOPT_MAXREDIRS"), 20);
    let cainfo = host
        .ini_get(b"openssl.cafile")
        .filter(|v| !v.is_empty())
        .or_else(|| host.ini_get(b"curl.cainfo"))
        .filter(|v| !v.is_empty());
    if let Some(ca) = cainfo {
        let c = cstring(&absolute_path(&ca));
        setopt_ptr(cp, opt("CURLOPT_CAINFO"), c.as_ptr() as *const c_void);
    }
    // php's ZTS build: rhp runs a VM per thread.
    setopt_long(cp, opt("CURLOPT_NOSIGNAL"), 1);
}

/// Point a new handle's callbacks at itself (after `curl_easy_duphandle`
/// or a server push copied the source's pointers), and give it the
/// source's handlers: `_php_setup_easy_copy_handlers`.
fn copy_handlers(new: &Rc<RefCell<Easy>>, src: &RefCell<Easy>) {
    let p = ctx(new);
    let (cp, err) = {
        let src = src.borrow();
        let mut ch = new.borrow_mut();
        ch.write = src.write.clone();
        ch.read = src.read.clone();
        ch.write_header = src.write_header.clone();
        ch.progress = src.progress.clone();
        ch.xferinfo = src.xferinfo.clone();
        ch.fnmatch = src.fnmatch.clone();
        ch.debug = src.debug.clone();
        ch.prereq = src.prereq.clone();
        ch.sshhostkey = src.sshhostkey.clone();
        ch.private = src.private.clone();
        ch.to_free = src.to_free.clone();
        // libcurl copied the CURLOPT_STDERR stream pointer.
        ch.stderr_file = src.stderr_file.clone();
        (ch.cp, ch.err.as_mut_ptr())
    };
    setopt_ptr(cp, opt("CURLOPT_ERRORBUFFER"), err as *const c_void);
    for d in DATA_OPTS {
        setopt_ptr(cp, opt(d), p);
    }
}

/// A copy of `src` for the object `dst` (`curl_clone_obj`), or `None` when
/// libcurl cannot duplicate it or the body cannot be rebuilt. Errs only
/// with what cannot be caught (`exit`).
pub(super) fn duplicate(
    host: &mut dyn Host,
    src: &Rc<RefCell<Easy>>,
    dst: &ObjectHandle,
) -> Result<Option<Rc<RefCell<Easy>>>, Thrown> {
    let (src_cp, postfields) = {
        let s = src.borrow();
        (s.cp, s.postfields.clone())
    };
    let cp = unsafe { sys::curl_easy_duphandle(src_cp) };
    if cp.is_null() {
        return Ok(None);
    }
    let new = Rc::new(RefCell::new(Easy::new(cp, dst)));
    copy_handlers(&new, src);
    let Some(Value::Array(pf)) = postfields else {
        return Ok(Some(new));
    };
    // php rebuilds the body from the array, reading its CURLFile objects
    // again.
    let parts = match mime_parts(host, "curl_copy_handle", &pf) {
        Ok((parts, _)) => parts,
        Err(t) if t.is_catchable() => return Ok(None),
        Err(t) => return Err(t),
    };
    new.borrow_mut().postfields = Some(Value::Array(pf));
    if !apply_mime(host, &new, &parts) {
        return Ok(None);
    }
    Ok(Some(new))
}

fn register(id: u32, e: Rc<RefCell<Easy>>) {
    EASY.with(|m| m.borrow_mut().insert(id, e));
}

// ---------------------------------------------------------------------------
// callbacks
// ---------------------------------------------------------------------------

fn easy_from<'a>(p: *mut c_void) -> Option<&'a RefCell<Easy>> {
    if p.is_null() {
        None
    } else {
        // SAFETY: the pointer is a live Rc<RefCell<Easy>>: whoever drives the
        // transfer holds a strong reference, and Drop clears the pointers.
        Some(unsafe { &*(p as *const RefCell<Easy>) })
    }
}

fn bytes<'a>(p: *const c_char, len: usize) -> &'a [u8] {
    if p.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(p as *const u8, len) }
    }
}

/// The handle's fields, read from a callback. No libcurl call is made with
/// the handle borrowed; should one be, the callback does nothing (`None`).
fn peek<T>(cell: &RefCell<Easy>, f: impl FnOnce(&Easy) -> T) -> Option<T> {
    cell.try_borrow().ok().map(|e| f(&e))
}

/// [`peek`] for writing.
fn poke(cell: &RefCell<Easy>, f: impl FnOnce(&mut Easy)) {
    if let Ok(mut e) = cell.try_borrow_mut() {
        f(&mut e);
    }
}

fn handle_value(cell: &RefCell<Easy>) -> Option<Value> {
    peek(cell, |e| e.obj.upgrade()).flatten().map(Value::Object)
}

/// `php_curl_call_callback`: `None` when it did not run or threw (php's
/// undefined retval); a pending exception skips every later callback.
/// The callable stays alive until the call returns (it may clear its own
/// option); objects released then are destructed with the guard still set.
fn call(cell: &RefCell<Easy>, fcc: Value, args: Vec<Value>) -> Option<Value> {
    if ffi::aborted() || !ffi::has_host() {
        return None;
    }
    let was = {
        let Ok(mut e) = cell.try_borrow_mut() else {
            return None;
        };
        std::mem::replace(&mut e.in_callback, true)
    };
    let r = ffi::call_host(|h| h.call_callable(&fcc, &args));
    drop(fcc);
    drop(args);
    ffi::call_host(|h| h.run_released_destructors());
    poke(cell, |e| e.in_callback = was);
    r
}

/// After a callback returned: `_php_curl_verify_handlers(ch, 1)` and
/// `php_curl_get_long(&retval)`.
fn after_call(cell: &RefCell<Easy>, retval: &Value) -> i64 {
    let fname = ffi::fname();
    ffi::call_host(|h| {
        verify_handlers(h, &fname, cell, true)?;
        zval_get_long(h, retval)
    })
    .unwrap_or(0)
}

fn verify_only(cell: &RefCell<Easy>) {
    let fname = ffi::fname();
    ffi::call_host(|h| verify_handlers(h, &fname, cell, true));
}

/// `_php_curl_verify_handlers`: forget streams closed behind curl's back.
pub(super) fn verify_handlers(
    host: &mut dyn Host,
    fname: &str,
    cell: &RefCell<Easy>,
    report: bool,
) -> Result<(), Thrown> {
    let Some((std_err, read, header, write, cp)) = peek(cell, |e| {
        (
            e.std_err.clone(),
            e.read.stream.clone(),
            e.write_header.stream.clone(),
            e.write.stream.clone(),
            e.cp,
        )
    }) else {
        return Ok(());
    };
    let gone =
        |host: &dyn Host, s: &Option<Value>| s.as_ref().is_some_and(|s| !host.stream_is_open(s));
    if gone(host, &std_err) {
        if report {
            crate::warn(
                host,
                fname,
                "CURLOPT_STDERR resource has gone away, resetting to stderr",
            )?;
        }
        setopt_ptr(cp, opt("CURLOPT_STDERR"), c_stderr());
        let mut old = None;
        poke(cell, |e| {
            e.std_err = None;
            old = e.stderr_file.take();
        });
        drop(old);
    }
    if gone(host, &read) {
        if report {
            crate::warn(
                host,
                fname,
                "CURLOPT_INFILE resource has gone away, resetting to default",
            )?;
        }
        poke(cell, |e| e.read.stream = None);
    }
    if gone(host, &header) {
        if report {
            crate::warn(
                host,
                fname,
                "CURLOPT_WRITEHEADER resource has gone away, resetting to default",
            )?;
        }
        poke(cell, |e| {
            e.write_header.stream = None;
            e.write_header.method = Method::Ignore;
        });
    }
    if gone(host, &write) {
        if report {
            crate::warn(
                host,
                fname,
                "CURLOPT_FILE resource has gone away, resetting to default",
            )?;
        }
        poke(cell, |e| {
            e.write.stream = None;
            e.write.method = Method::Stdout;
        });
    }
    Ok(())
}

fn write_out(data: &[u8]) {
    ffi::call_host(|h| {
        h.write_out(data);
        Ok(())
    });
}

fn stream_write(s: &Value, data: &[u8]) -> usize {
    ffi::call_host(|h| Ok(h.stream_write(s, data).unwrap_or(0))).unwrap_or(0)
}

/// `curl_write`.
unsafe extern "C" fn write_cb(
    data: *mut c_char,
    size: usize,
    nmemb: usize,
    p: *mut c_void,
) -> usize {
    let length = size.wrapping_mul(nmemb);
    ffi::guard(0, || {
        let Some(cell) = easy_from(p) else {
            return length;
        };
        let data = bytes(data, length);
        let Some(h) = peek(cell, |e| e.write.clone()) else {
            return length;
        };
        match h.method {
            Method::Stdout => write_out(data),
            Method::File => return h.stream.as_ref().map_or(0, |s| stream_write(s, data)),
            Method::Return => {
                if length > 0 {
                    poke(cell, |e| {
                        e.buf.get_or_insert_with(Vec::new).extend_from_slice(data)
                    });
                }
            }
            Method::User => {
                let (Some(fcc), Some(obj)) = (h.fcc, handle_value(cell)) else {
                    return length;
                };
                if let Some(r) = call(cell, fcc, vec![obj, Value::str(data.to_vec())]) {
                    return after_call(cell, &r) as usize;
                }
            }
            Method::Direct | Method::Ignore => {}
        }
        length
    })
}

/// `curl_write_header`.
unsafe extern "C" fn header_cb(
    data: *mut c_char,
    size: usize,
    nmemb: usize,
    p: *mut c_void,
) -> usize {
    let length = size.wrapping_mul(nmemb);
    ffi::guard(0, || {
        let Some(cell) = easy_from(p) else {
            return length;
        };
        let data = bytes(data, length);
        let Some((h, write_method)) = peek(cell, |e| (e.write_header.clone(), e.write.method))
        else {
            return length;
        };
        match h.method {
            Method::Stdout => {
                // The whole transfer is being returned: headers go in too.
                if write_method == Method::Return && length > 0 {
                    poke(cell, |e| {
                        e.buf.get_or_insert_with(Vec::new).extend_from_slice(data)
                    });
                } else {
                    write_out(data);
                }
                length
            }
            Method::File => h.stream.as_ref().map_or(0, |s| stream_write(s, data)),
            Method::User => {
                let (Some(fcc), Some(obj)) = (h.fcc, handle_value(cell)) else {
                    return length;
                };
                match call(cell, fcc, vec![obj, Value::str(data.to_vec())]) {
                    Some(r) => after_call(cell, &r) as usize,
                    None => length,
                }
            }
            Method::Ignore => length,
            Method::Direct | Method::Return => usize::MAX,
        }
    })
}

/// `curl_read`.
unsafe extern "C" fn read_cb(
    data: *mut c_char,
    size: usize,
    nmemb: usize,
    p: *mut c_void,
) -> usize {
    let max = size.wrapping_mul(nmemb);
    let abort = sys::internal("CURL_READFUNC_ABORT") as usize;
    ffi::guard(abort, || {
        let Some(cell) = easy_from(p) else {
            return 0;
        };
        let out = |src: &[u8]| -> usize {
            let n = src.len().min(max);
            unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), data as *mut u8, n) };
            n
        };
        let Some(h) = peek(cell, |e| e.read.clone()) else {
            return abort;
        };
        match h.method {
            Method::Direct => {
                let Some(s) = h.stream else { return 0 };
                let got = ffi::call_host(|host| Ok(host.stream_read(&s, max as i64))).flatten();
                got.map_or(0, |b| out(&b))
            }
            Method::User => {
                let (Some(fcc), Some(obj)) = (h.fcc, handle_value(cell)) else {
                    return 0;
                };
                let arg = h.stream.unwrap_or(Value::Null);
                // php: (int)size * nmemb
                let len = (size as c_int as i64).wrapping_mul(nmemb as i64);
                let Some(r) = call(cell, fcc, vec![obj, arg, Value::Int(len)]) else {
                    return 0;
                };
                verify_only(cell);
                match deref(&r) {
                    Value::Str(s) => out(s.as_bytes()),
                    Value::Int(n) => n as usize,
                    _ => 0,
                }
            }
            _ => 0,
        }
    })
}

/// A user callback whose int result means "abort" when non-zero.
fn progress_like(p: *mut c_void, pick: fn(&Easy) -> Option<Value>, nums: [i64; 4]) -> c_int {
    let Some(cell) = easy_from(p) else {
        return 0;
    };
    let fcc = peek(cell, pick).flatten();
    let (Some(fcc), Some(obj)) = (fcc, handle_value(cell)) else {
        return 0;
    };
    let mut args = vec![obj];
    args.extend(nums.iter().map(|n| Value::Int(*n)));
    match call(cell, fcc, args) {
        Some(r) if after_call(cell, &r) != 0 => 1,
        _ => 0,
    }
}

/// `curl_progress`.
unsafe extern "C" fn progress_cb(
    p: *mut c_void,
    dltotal: c_double,
    dlnow: c_double,
    ultotal: c_double,
    ulnow: c_double,
) -> c_int {
    ffi::guard(1, || {
        progress_like(
            p,
            |e| e.progress.clone(),
            [dltotal as i64, dlnow as i64, ultotal as i64, ulnow as i64],
        )
    })
}

/// `curl_xferinfo`.
unsafe extern "C" fn xferinfo_cb(
    p: *mut c_void,
    dltotal: curl_off_t,
    dlnow: curl_off_t,
    ultotal: curl_off_t,
    ulnow: curl_off_t,
) -> c_int {
    ffi::guard(1, || {
        progress_like(p, |e| e.xferinfo.clone(), [dltotal, dlnow, ultotal, ulnow])
    })
}

/// `curl_fnmatch`.
unsafe extern "C" fn fnmatch_cb(
    p: *mut c_void,
    pattern: *const c_char,
    string: *const c_char,
) -> c_int {
    let fail = sys::php("CURL_FNMATCHFUNC_FAIL") as c_int;
    ffi::guard(fail, || {
        let Some(cell) = easy_from(p) else {
            return fail;
        };
        let fcc = peek(cell, |e| e.fnmatch.clone()).flatten();
        let (Some(fcc), Some(obj)) = (fcc, handle_value(cell)) else {
            return fail;
        };
        let args = vec![
            obj,
            Value::str(cstr_bytes(pattern)),
            Value::str(cstr_bytes(string)),
        ];
        match call(cell, fcc, args) {
            Some(r) => after_call(cell, &r) as c_int,
            None => fail,
        }
    })
}

/// `curl_prereqfunction`.
unsafe extern "C" fn prereq_cb(
    p: *mut c_void,
    primary_ip: *mut c_char,
    local_ip: *mut c_char,
    primary_port: c_int,
    local_port: c_int,
) -> c_int {
    let ok = sys::php("CURL_PREREQFUNC_OK");
    let abort = sys::php("CURL_PREREQFUNC_ABORT");
    ffi::guard(abort as c_int, || {
        let Some(cell) = easy_from(p) else {
            return ok as c_int;
        };
        let fcc = peek(cell, |e| e.prereq.clone()).flatten();
        let (Some(fcc), Some(obj)) = (fcc, handle_value(cell)) else {
            return ok as c_int;
        };
        let args = vec![
            obj,
            Value::str(cstr_bytes(primary_ip)),
            Value::str(cstr_bytes(local_ip)),
            Value::Int(primary_port as i64),
            Value::Int(local_port as i64),
        ];
        let Some(r) = call(cell, fcc, args) else {
            return ok as c_int;
        };
        verify_only(cell);
        const MSG: &str = "The CURLOPT_PREREQFUNCTION callback must return either \
                           CURL_PREREQFUNC_OK or CURL_PREREQFUNC_ABORT";
        match deref(&r) {
            Value::Int(n) if n == ok || n == abort => n as c_int,
            Value::Int(_) => {
                ffi::stash(Thrown::error("ValueError", MSG));
                ok as c_int
            }
            _ => {
                ffi::stash(Thrown::type_error(MSG));
                ok as c_int
            }
        }
    })
}

/// `curl_ssh_hostkeyfunction`.
unsafe extern "C" fn sshhostkey_cb(
    p: *mut c_void,
    keytype: c_int,
    key: *const c_char,
    keylen: usize,
) -> c_int {
    let ok = sys::php("CURLKHMATCH_OK");
    let mismatch = sys::php("CURLKHMATCH_MISMATCH");
    ffi::guard(mismatch as c_int, || {
        let Some(cell) = easy_from(p) else {
            return mismatch as c_int;
        };
        let fcc = peek(cell, |e| e.sshhostkey.clone()).flatten();
        let (Some(fcc), Some(obj)) = (fcc, handle_value(cell)) else {
            return mismatch as c_int;
        };
        let args = vec![
            obj,
            Value::Int(keytype as i64),
            Value::str(bytes(key, keylen).to_vec()),
            Value::Int(keylen as i64),
        ];
        let Some(r) = call(cell, fcc, args) else {
            return mismatch as c_int;
        };
        verify_only(cell);
        match deref(&r) {
            Value::Int(n) if n == ok || n == mismatch => n as c_int,
            _ => {
                ffi::stash(Thrown::error(
                    "Error",
                    "The CURLOPT_SSH_HOSTKEYFUNCTION callback must return either \
                     CURLKHMATCH_OK or CURLKHMATCH_MISMATCH",
                ));
                mismatch as c_int
            }
        }
    })
}

/// `curl_debug`.
unsafe extern "C" fn debug_cb(
    _h: *mut CURL,
    ty: c_int,
    data: *mut c_char,
    size: usize,
    p: *mut c_void,
) -> c_int {
    ffi::guard(0, || {
        let Some(cell) = easy_from(p) else {
            return 0;
        };
        let data = bytes(data, size);
        // Kept for CURLINFO_HEADER_OUT, with or without a user callback.
        if ty as i64 == sys::php("CURLINFO_HEADER_OUT") {
            poke(cell, |e| e.header_out = Some(data.to_vec()));
        }
        let fcc = peek(cell, |e| e.debug.clone()).flatten();
        if let (Some(fcc), Some(obj)) = (fcc, handle_value(cell)) {
            call(
                cell,
                fcc,
                vec![obj, Value::Int(ty as i64), Value::str(data.to_vec())],
            );
        }
        0
    })
}

/// shim.c's CURLOPT_STDERR stream writes here.
#[no_mangle]
pub extern "C" fn rhp_curl_stderr_write(
    cookie: *mut c_void,
    buf: *const c_char,
    len: usize,
) -> c_long {
    ffi::guard(-1, || {
        // SAFETY: the cookie is the CookieFile's boxed stream value.
        let stream = unsafe { &*(cookie as *const Value) };
        let data = bytes(buf, len);
        match ffi::call_host(|h| Ok(h.stream_write(stream, data))) {
            Some(Some(n)) => n as c_long,
            Some(None) => -1,
            // No builtin running (a handle freed by the engine): dropped.
            None => len as c_long,
        }
    })
}

// ---------------------------------------------------------------------------
// mime (CURLOPT_POSTFIELDS with an array)
// ---------------------------------------------------------------------------

unsafe extern "C" fn mime_read(
    buf: *mut c_char,
    size: usize,
    nitems: usize,
    arg: *mut c_void,
) -> usize {
    let abort = sys::internal("CURL_READFUNC_ABORT") as usize;
    ffi::guard(abort, || {
        let a = unsafe { &*(arg as *const MimeArg) };
        let open = a.stream.borrow().clone();
        let stream = match open {
            Some(s) => s,
            None => {
                let opened = ffi::call_host(|h| Ok(h.stream_open(&a.filename, b"rb"))).flatten();
                let Some(s) = opened else {
                    return abort;
                };
                *a.stream.borrow_mut() = Some(s.clone());
                s
            }
        };
        let max = size.wrapping_mul(nitems);
        match ffi::call_host(|h| Ok(h.stream_read(&stream, max as i64))).flatten() {
            Some(data) => {
                let n = data.len().min(max);
                unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, n) };
                n
            }
            None => {
                a.stream.borrow_mut().take();
                abort
            }
        }
    })
}

unsafe extern "C" fn mime_seek(arg: *mut c_void, offset: curl_off_t, origin: c_int) -> c_int {
    let ok = sys::internal("CURL_SEEKFUNC_OK") as c_int;
    let cant = sys::internal("CURL_SEEKFUNC_CANTSEEK") as c_int;
    ffi::guard(cant, || {
        let a = unsafe { &*(arg as *const MimeArg) };
        let Some(stream) = a.stream.borrow().clone() else {
            return cant;
        };
        match ffi::call_host(|h| Ok(h.stream_seek(&stream, offset, origin))) {
            Some(true) => ok,
            _ => cant,
        }
    })
}

unsafe extern "C" fn mime_free(arg: *mut c_void) {
    ffi::guard((), || {
        let a = unsafe { &*(arg as *const MimeArg) };
        let s = a.stream.borrow_mut().take();
        drop(s);
    })
}

/// `instanceof_function(Z_OBJCE_P(v), <class>)`.
fn instance_of(host: &dyn Host, v: &Value, class: &[u8]) -> bool {
    let Value::Object(h) = v else { return false };
    let mut id = Some(h.borrow().class);
    while let Some(c) = id {
        if host
            .class_name(c)
            .is_some_and(|n| n.eq_ignore_ascii_case(class))
        {
            return true;
        }
        id = host
            .class_parent_name(c)
            .and_then(|p| host.resolve_class(&p));
    }
    false
}

/// A C string from bytes that may hold a NUL (C sees up to it).
fn c_trunc(b: &[u8]) -> std::ffi::CString {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    cstring(&b[..end])
}

fn key_bytes(k: &ArrayKey) -> Vec<u8> {
    match k {
        ArrayKey::Int(i) => i.to_string().into_bytes(),
        ArrayKey::Str(s) => s.as_bytes().to_vec(),
    }
}

/// One part of a multipart POST, as read from the CURLOPT_POSTFIELDS array.
enum Part {
    /// A CURLFile: the file (opened through rhp's streams, as php opens it
    /// with php_stream_open_wrapper), its upload name and type.
    File {
        key: Vec<u8>,
        path: Vec<u8>,
        filename: Vec<u8>,
        mime: Vec<u8>,
    },
    /// A CURLStringFile.
    Data {
        key: Vec<u8>,
        data: Vec<u8>,
        filename: Vec<u8>,
        mime: Vec<u8>,
    },
    Field {
        key: Vec<u8>,
        value: Vec<u8>,
    },
}

/// The first half of `build_mime_structure_from_hash`: the parts, and
/// whether a file was among them (php then keeps the array on the handle).
fn mime_parts(
    host: &mut dyn Host,
    f: &str,
    postfields: &PhpArray,
) -> Result<(Vec<Part>, bool), Thrown> {
    let mut parts = Vec::new();
    let mut files = false;
    for (k, current) in postfields.iter() {
        let key = key_bytes(k);
        let current = deref(current);
        let prop = |host: &dyn Host, name: &str| {
            host.object_prop(&current, name.as_bytes())
                .map(|v| deref(&v))
        };
        let nonempty = |v: Option<Value>| match v {
            Some(Value::Str(s)) if !s.is_empty() => Some(s.as_bytes().to_vec()),
            _ => None,
        };
        if instance_of(host, &current, b"CURLFile") {
            match prop(host, "name") {
                Some(Value::Str(name)) => {
                    let name = name.as_bytes().to_vec();
                    let mime = nonempty(prop(host, "mime"));
                    let post = nonempty(prop(host, "postname"));
                    files = true;
                    parts.push(Part::File {
                        key,
                        path: name.clone(),
                        filename: post.unwrap_or(name),
                        mime: mime.unwrap_or_else(|| b"application/octet-stream".to_vec()),
                    });
                }
                _ => crate::warn(
                    host,
                    f,
                    &format!("Invalid filename for key {}", String::from_utf8_lossy(&key)),
                )?,
            }
            continue;
        }
        if instance_of(host, &current, b"CURLStringFile") {
            let get = |host: &dyn Host, n: &str| {
                match prop(host, n) {
                Some(Value::Str(s)) => Ok(s.as_bytes().to_vec()),
                _ => Err(Thrown::error(
                    "Error",
                    format!("Typed property CURLStringFile::${n} must not be accessed before initialization"),
                )),
            }
            };
            let filename = get(host, "postname")?;
            let mime = get(host, "mime")?;
            let data = get(host, "data")?;
            files = true;
            parts.push(Part::Data {
                key,
                data,
                filename,
                mime,
            });
            continue;
        }
        let values: Vec<Value> = match &current {
            Value::Array(a) => a.iter().map(|(_, v)| v.clone()).collect(),
            other => vec![other.clone()],
        };
        for v in values {
            let value = crate::argparse::zval_get_string(host, &deref(&v))?;
            parts.push(Part::Field {
                key: key.clone(),
                value,
            });
        }
    }
    Ok((parts, files))
}

/// The second half: the curl_mime, set as CURLOPT_MIMEPOST.
fn apply_mime(host: &mut dyn Host, e: &Rc<RefCell<Easy>>, parts: &[Part]) -> bool {
    let cp = e.borrow().cp;
    let mut mime: *mut sys::curl_mime = std::ptr::null_mut();
    if !parts.is_empty() {
        mime = unsafe { sys::curl_mime_init(cp) };
        if mime.is_null() {
            return false;
        }
    }
    let mut error: c_int = 0;
    for p in parts {
        let part = unsafe { sys::curl_mime_addpart(mime) };
        if part.is_null() {
            if matches!(p, Part::Field { .. }) {
                error = sys::php("CURLE_OUT_OF_MEMORY") as c_int;
                continue;
            }
            unsafe { sys::curl_mime_free(mime) };
            return false;
        }
        let mut r;
        match p {
            Part::File {
                key,
                path,
                filename,
                mime: ty,
            } => {
                // A file that does not open now is tried again on the first
                // read.
                let stream = host.stream_open(path, b"rb");
                let size: curl_off_t = match &stream {
                    Some(s) => host.stream_fstat(s).map_or(-1, |st| st.size as curl_off_t),
                    None => -1,
                };
                let seek: Option<sys::SeekFn> = if stream.is_some() {
                    Some(mime_seek)
                } else {
                    None
                };
                let arg = Box::new(MimeArg {
                    filename: path.clone(),
                    stream: RefCell::new(stream),
                });
                let argp = &*arg as *const MimeArg as *mut c_void;
                let (name, fname, ty) = (c_trunc(key), c_trunc(filename), c_trunc(ty));
                unsafe {
                    r = sys::curl_mime_name(part, name.as_ptr());
                    if r == 0 {
                        r = sys::curl_mime_data_cb(
                            part,
                            size,
                            Some(mime_read),
                            seek,
                            Some(mime_free),
                            argp,
                        );
                    }
                    if r == 0 {
                        r = sys::curl_mime_filename(part, fname.as_ptr());
                    }
                    if r == 0 {
                        r = sys::curl_mime_type(part, ty.as_ptr());
                    }
                }
                e.borrow().to_free.borrow_mut().args.push(arg);
            }
            Part::Data {
                key,
                data,
                filename,
                mime: ty,
            } => {
                let (name, fname, ty) = (c_trunc(key), c_trunc(filename), c_trunc(ty));
                unsafe {
                    r = sys::curl_mime_name(part, name.as_ptr());
                    if r == 0 {
                        r = sys::curl_mime_data(part, data.as_ptr() as *const c_char, data.len());
                    }
                    if r == 0 {
                        r = sys::curl_mime_filename(part, fname.as_ptr());
                    }
                    if r == 0 {
                        r = sys::curl_mime_type(part, ty.as_ptr());
                    }
                }
            }
            Part::Field { key, value } => {
                let name = c_trunc(key);
                unsafe {
                    r = sys::curl_mime_name(part, name.as_ptr());
                    if r == 0 {
                        r = sys::curl_mime_data(part, value.as_ptr() as *const c_char, value.len());
                    }
                }
            }
        }
        if r != 0 {
            error = r;
        }
    }
    e.borrow_mut().errno = error;
    if error != 0 {
        unsafe { sys::curl_mime_free(mime) };
        return false;
    }
    let old: Vec<_> = {
        let ch = e.borrow();
        let shared = Rc::strong_count(&ch.to_free) > 1;
        let mut tf = ch.to_free.borrow_mut();
        let old = if shared {
            Vec::new()
        } else {
            std::mem::take(&mut tf.mimes)
        };
        tf.mimes.push(mime);
        old
    };
    for m in old {
        unsafe { sys::curl_mime_free(m) };
    }
    let r = setopt_ptr(cp, opt("CURLOPT_MIMEPOST"), mime as *const c_void);
    e.borrow_mut().errno = r;
    r == 0
}

// ---------------------------------------------------------------------------
// curl_setopt
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cb {
    Write,
    Header,
    Read,
    Progress,
    Xferinfo,
    Fnmatch,
    Debug,
    Prereq,
    SshHostkey,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Callable(Cb),
    SslVerifyHost,
    Long,
    SafeUpload,
    Str,
    /// A string option that names a file libcurl opens.
    Path,
    Nullable,
    NullablePath,
    Private,
    Url,
    FileHandle,
    Slist,
    Ignore,
    PostFields,
    ReturnTransfer,
    OffT,
    PostRedir,
    /// A file option php checks against open_basedir.
    FilePath,
    HeaderOut,
    Share,
    Blob,
}

// The case labels of _php_curl_setopt, by what it does with the value.
const LONG: &[&str] = &[
    "AUTOREFERER",
    "BUFFERSIZE",
    "CONNECTTIMEOUT",
    "COOKIESESSION",
    "CRLF",
    "DNS_CACHE_TIMEOUT",
    "FAILONERROR",
    "FILETIME",
    "FORBID_REUSE",
    "FRESH_CONNECT",
    "FTP_USE_EPRT",
    "FTP_USE_EPSV",
    "HEADER",
    "HTTPGET",
    "HTTPPROXYTUNNEL",
    "HTTP_VERSION",
    "INFILESIZE",
    "LOW_SPEED_LIMIT",
    "LOW_SPEED_TIME",
    "MAXCONNECTS",
    "MAXREDIRS",
    "NETRC",
    "NOBODY",
    "NOPROGRESS",
    "NOSIGNAL",
    "PORT",
    "POST",
    "PROXYPORT",
    "PROXYTYPE",
    "PUT",
    "RESUME_FROM",
    "SSLVERSION",
    "SSL_VERIFYPEER",
    "TIMECONDITION",
    "TIMEOUT",
    "TIMEVALUE",
    "TRANSFERTEXT",
    "UNRESTRICTED_AUTH",
    "UPLOAD",
    "VERBOSE",
    "HTTPAUTH",
    "FTP_CREATE_MISSING_DIRS",
    "PROXYAUTH",
    "SERVER_RESPONSE_TIMEOUT",
    "IPRESOLVE",
    "MAXFILESIZE",
    "TCP_NODELAY",
    "FTPSSLAUTH",
    "IGNORE_CONTENT_LENGTH",
    "FTP_SKIP_PASV_IP",
    "FTP_FILEMETHOD",
    "CONNECT_ONLY",
    "LOCALPORT",
    "LOCALPORTRANGE",
    "SSL_SESSIONID_CACHE",
    "FTP_SSL_CCC",
    "SSH_AUTH_TYPES",
    "CONNECTTIMEOUT_MS",
    "HTTP_CONTENT_DECODING",
    "HTTP_TRANSFER_DECODING",
    "TIMEOUT_MS",
    "NEW_DIRECTORY_PERMS",
    "NEW_FILE_PERMS",
    "USE_SSL",
    "APPEND",
    "DIRLISTONLY",
    "PROXY_TRANSFER_MODE",
    "ADDRESS_SCOPE",
    "CERTINFO",
    "PROTOCOLS",
    "REDIR_PROTOCOLS",
    "SOCKS5_GSSAPI_NEC",
    "TFTP_BLKSIZE",
    "FTP_USE_PRET",
    "RTSP_CLIENT_CSEQ",
    "RTSP_REQUEST",
    "RTSP_SERVER_CSEQ",
    "WILDCARDMATCH",
    "GSSAPI_DELEGATION",
    "ACCEPTTIMEOUT_MS",
    "SSL_OPTIONS",
    "TCP_KEEPALIVE",
    "TCP_KEEPIDLE",
    "TCP_KEEPINTVL",
    "SASL_IR",
    "EXPECT_100_TIMEOUT_MS",
    "SSL_ENABLE_ALPN",
    "SSL_ENABLE_NPN",
    "HEADEROPT",
    "SSL_VERIFYSTATUS",
    "PATH_AS_IS",
    "SSL_FALSESTART",
    "PIPEWAIT",
    "STREAM_WEIGHT",
    "TFTP_NO_OPTIONS",
    "TCP_FASTOPEN",
    "KEEP_SENDING_ON_ERROR",
    "PROXY_SSL_OPTIONS",
    "PROXY_SSL_VERIFYHOST",
    "PROXY_SSL_VERIFYPEER",
    "PROXY_SSLVERSION",
    "SUPPRESS_CONNECT_HEADERS",
    "SOCKS5_AUTH",
    "SSH_COMPRESSION",
    "HAPPY_EYEBALLS_TIMEOUT_MS",
    "DNS_SHUFFLE_ADDRESSES",
    "HAPROXYPROTOCOL",
    "DISALLOW_USERNAME_IN_URL",
    "UPKEEP_INTERVAL_MS",
    "UPLOAD_BUFFERSIZE",
    "HTTP09_ALLOWED",
    "ALTSVC_CTRL",
    "MAXAGE_CONN",
    "MAIL_RCPT_ALLLOWFAILS",
    "HSTS_CTRL",
    "DOH_SSL_VERIFYHOST",
    "DOH_SSL_VERIFYPEER",
    "DOH_SSL_VERIFYSTATUS",
    "MAXLIFETIME_CONN",
    "MIME_OPTIONS",
    "WS_OPTIONS",
    "CA_CACHE_TIMEOUT",
    "QUICK_EXIT",
    "TCP_KEEPCNT",
    "FOLLOWLOCATION",
];
const STR: &[&str] = &[
    "COOKIE",
    "INTERFACE",
    "PROXY",
    "PROXYUSERPWD",
    "REFERER",
    "SSLCERTTYPE",
    "SSLENGINE",
    "SSLENGINE_DEFAULT",
    "SSLKEYPASSWD",
    "SSLKEYTYPE",
    "SSL_CIPHER_LIST",
    "USERAGENT",
    "COOKIELIST",
    "FTP_ALTERNATIVE_TO_USER",
    "SSH_HOST_PUBLIC_KEY_MD5",
    "PROXYPASSWORD",
    "PROXYUSERNAME",
    "NOPROXY",
    "SOCKS5_GSSAPI_SERVICE",
    "MAIL_FROM",
    "RTSP_STREAM_URI",
    "RTSP_TRANSPORT",
    "TLSAUTH_TYPE",
    "TLSAUTH_PASSWORD",
    "TLSAUTH_USERNAME",
    "TRANSFER_ENCODING",
    "DNS_SERVERS",
    "MAIL_AUTH",
    "LOGIN_OPTIONS",
    "PINNEDPUBLICKEY",
    "PROXY_SERVICE_NAME",
    "SERVICE_NAME",
    "DEFAULT_PROTOCOL",
    "PRE_PROXY",
    "PROXY_KEYPASSWD",
    "PROXY_PINNEDPUBLICKEY",
    "PROXY_SSL_CIPHER_LIST",
    "PROXY_SSLCERTTYPE",
    "PROXY_SSLKEYTYPE",
    "PROXY_TLSAUTH_PASSWORD",
    "PROXY_TLSAUTH_TYPE",
    "PROXY_TLSAUTH_USERNAME",
    "ABSTRACT_UNIX_SOCKET",
    "REQUEST_TARGET",
    "PROXY_TLS13_CIPHERS",
    "TLS13_CIPHERS",
    "SASL_AUTHZID",
    "SSL_EC_CURVES",
    "AWS_SIGV4",
    "SSH_HOST_PUBLIC_KEY_SHA256",
    "PROTOCOLS_STR",
    "REDIR_PROTOCOLS_STR",
];
/// String options (same case as [`STR`]) that name files.
const STR_PATH: &[&str] = &[
    "CAINFO",
    "CAPATH",
    "EGDSOCKET",
    "SSLKEY",
    "PROXY_CAINFO",
    "PROXY_CAPATH",
    "PROXY_CRLFILE",
    "PROXY_SSLCERT",
    "PROXY_SSLKEY",
    "ALTSVC",
    "PROXY_ISSUERCERT",
];
const NULLABLE: &[&str] = &[
    "CUSTOMREQUEST",
    "FTPPORT",
    "RANGE",
    "FTP_ACCOUNT",
    "RTSP_SESSION_ID",
    "ACCEPT_ENCODING",
    "DNS_INTERFACE",
    "DNS_LOCAL_IP4",
    "DNS_LOCAL_IP6",
    "XOAUTH2_BEARER",
    "DOH_URL",
    "KRBLEVEL",
    "USERPWD",
    "USERNAME",
    "PASSWORD",
    "SSL_SIGNATURE_ALGORITHMS",
];
const NULLABLE_PATH: &[&str] = &["UNIX_SOCKET_PATH", "HSTS"];
const FILE_PATH: &[&str] = &[
    "COOKIEFILE",
    "COOKIEJAR",
    "RANDOM_FILE",
    "SSLCERT",
    "NETRC_FILE",
    "SSH_PRIVATE_KEYFILE",
    "SSH_PUBLIC_KEYFILE",
    "CRLFILE",
    "ISSUERCERT",
    "SSH_KNOWNHOSTS",
];
const SLIST: &[&str] = &[
    "HTTP200ALIASES",
    "HTTPHEADER",
    "POSTQUOTE",
    "PREQUOTE",
    "QUOTE",
    "TELNETOPTIONS",
    "MAIL_RCPT",
    "RESOLVE",
    "PROXYHEADER",
    "CONNECT_TO",
];
const OFF_T: &[&str] = &[
    "INFILESIZE_LARGE",
    "MAX_RECV_SPEED_LARGE",
    "MAX_SEND_SPEED_LARGE",
    "MAXFILESIZE_LARGE",
    "TIMEVALUE_LARGE",
];
const BLOB: &[&str] = &[
    "ISSUERCERT_BLOB",
    "PROXY_ISSUERCERT_BLOB",
    "PROXY_SSLCERT_BLOB",
    "PROXY_SSLKEY_BLOB",
    "SSLCERT_BLOB",
    "SSLKEY_BLOB",
    "CAINFO_BLOB",
    "PROXY_CAINFO_BLOB",
];
const CALLABLES: &[(&str, Cb)] = &[
    ("WRITEFUNCTION", Cb::Write),
    ("HEADERFUNCTION", Cb::Header),
    ("READFUNCTION", Cb::Read),
    ("PROGRESSFUNCTION", Cb::Progress),
    ("XFERINFOFUNCTION", Cb::Xferinfo),
    ("FNMATCH_FUNCTION", Cb::Fnmatch),
    ("DEBUGFUNCTION", Cb::Debug),
    ("PREREQFUNCTION", Cb::Prereq),
    ("SSH_HOSTKEYFUNCTION", Cb::SshHostkey),
];

fn build_kinds() -> HashMap<i64, Kind> {
    let mut m = HashMap::new();
    let mut add = |name: String, k: Kind| {
        if sys::php_defined(&name) {
            m.insert(sys::php(&name), k);
        }
    };
    let groups: &[(&[&str], Kind)] = &[
        (LONG, Kind::Long),
        (STR, Kind::Str),
        (STR_PATH, Kind::Path),
        (NULLABLE, Kind::Nullable),
        (NULLABLE_PATH, Kind::NullablePath),
        (FILE_PATH, Kind::FilePath),
        (SLIST, Kind::Slist),
        (OFF_T, Kind::OffT),
        (BLOB, Kind::Blob),
        (
            &["FILE", "INFILE", "STDERR", "WRITEHEADER"],
            Kind::FileHandle,
        ),
        (&["BINARYTRANSFER", "DNS_USE_GLOBAL_CACHE"], Kind::Ignore),
        (&["SSL_VERIFYHOST"], Kind::SslVerifyHost),
        (&["SAFE_UPLOAD"], Kind::SafeUpload),
        (&["PRIVATE"], Kind::Private),
        (&["URL"], Kind::Url),
        (&["POSTFIELDS"], Kind::PostFields),
        (&["RETURNTRANSFER"], Kind::ReturnTransfer),
        (&["POSTREDIR"], Kind::PostRedir),
        (&["SHARE"], Kind::Share),
    ];
    for (names, k) in groups {
        for n in *names {
            add(format!("CURLOPT_{n}"), *k);
        }
    }
    for (n, cb) in CALLABLES {
        add(format!("CURLOPT_{n}"), Kind::Callable(*cb));
    }
    add("CURLINFO_HEADER_OUT".into(), Kind::HeaderOut);
    m
}

fn kinds() -> &'static HashMap<i64, Kind> {
    static KINDS: std::sync::OnceLock<HashMap<i64, Kind>> = std::sync::OnceLock::new();
    KINDS.get_or_init(|| rhp_values::mem::uncounted(build_kinds))
}

/// The name php gives a callable option in errors.
fn cb_name(cb: Cb) -> &'static str {
    match cb {
        Cb::Write => "CURLOPT_WRITEFUNCTION",
        Cb::Header => "CURLOPT_HEADERFUNCTION",
        Cb::Read => "CURLOPT_READFUNCTION",
        Cb::Progress => "CURLOPT_PROGRESSFUNCTION",
        Cb::Xferinfo => "CURLOPT_XFERINFOFUNCTION",
        Cb::Fnmatch => "CURLOPT_FNMATCH_FUNCTION",
        Cb::Debug => "CURLOPT_DEBUGFUNCTION",
        Cb::Prereq => "CURLOPT_PREREQFUNCTION",
        Cb::SshHostkey => "CURLOPT_SSH_HOSTKEYFUNCTION",
    }
}

/// Run `op`, a curl_easy_setopt call, with the host reachable: libcurl may
/// log through CURLOPT_DEBUGFUNCTION or into CURLOPT_STDERR. An exception
/// a callback throws is kept in `pending` and skips later callbacks, as
/// php's EG(exception) does; `exit` returns at once.
fn logged<R: Copy + Default>(
    host: &mut dyn Host,
    f: &str,
    pending: &mut Option<Thrown>,
    op: impl FnOnce() -> R,
) -> Result<R, Thrown> {
    let carried = pending.take();
    let (r, thrown) = drive(host, f, || {
        if let Some(t) = carried {
            ffi::stash(t);
        }
        op()
    });
    match thrown {
        Ok(()) => Ok(r),
        Err(t) if t.is_catchable() => {
            *pending = Some(t);
            Ok(r)
        }
        Err(t) => Err(t),
    }
}

/// A callable as php's FCC keeps it: resolved in the scope that set it, so
/// `[$this, 'privateMethod']` set inside the class can be called later.
pub(super) fn bind_callable(host: &dyn Host, v: &Value) -> Value {
    match v {
        Value::Closure(_) | Value::Object(_) => v.clone(),
        other => host.wrap_callable(other.clone()),
    }
}

/// `php_curl_option_str`.
fn option_str(
    host: &mut dyn Host,
    f: &str,
    e: &RefCell<Easy>,
    pending: &mut Option<Thrown>,
    option: i64,
    s: &[u8],
) -> Result<bool, Thrown> {
    if s.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!("{f}(): cURL option must not contain any null bytes"),
        ));
    }
    let c = cstring(s);
    let cp = e.borrow().cp;
    let r = logged(host, f, pending, || {
        setopt_ptr(cp, option, c.as_ptr() as *const c_void)
    })?;
    e.borrow_mut().errno = r;
    Ok(r == 0)
}

/// `_php_curl_setopt`: `Ok(false)` is php's FAILURE without an exception.
/// No borrow of the handle is held across a libcurl call.
fn set_option(
    host: &mut dyn Host,
    f: &str,
    e: &Rc<RefCell<Easy>>,
    option: i64,
    zvalue: &Value,
    is_array: bool,
    pending: &mut Option<Thrown>,
) -> Result<bool, Thrown> {
    let zvalue = deref(zvalue);
    let cp = e.borrow().cp;
    let kind = kinds().get(&option).copied();
    let mut error: c_int = 0;
    match kind {
        Some(Kind::Callable(cb)) => {
            let user = matches!(cb, Cb::Write | Cb::Header | Cb::Read);
            {
                let mut ch = e.borrow_mut();
                match cb {
                    Cb::Write => ch.write.fcc = None,
                    Cb::Header => ch.write_header.fcc = None,
                    Cb::Read => ch.read.fcc = None,
                    Cb::Progress => ch.progress = None,
                    Cb::Xferinfo => ch.xferinfo = None,
                    Cb::Fnmatch => ch.fnmatch = None,
                    Cb::Debug => ch.debug = None,
                    Cb::Prereq => ch.prereq = None,
                    Cb::SshHostkey => ch.sshhostkey = None,
                }
            }
            let default = match cb {
                Cb::Write => Method::Stdout,
                Cb::Header => Method::Ignore,
                _ => Method::Direct,
            };
            let set_method = |m: Method| {
                let mut ch = e.borrow_mut();
                match cb {
                    Cb::Write => ch.write.method = m,
                    Cb::Header => ch.write_header.method = m,
                    Cb::Read => ch.read.method = m,
                    _ => {}
                }
            };
            let fcc = if matches!(zvalue, Value::Null) {
                None
            } else {
                match host.check_callable(&zvalue)? {
                    None => Some(bind_callable(host, &zvalue)),
                    Some(why) => {
                        if user {
                            set_method(default);
                        }
                        let (n, p) = if is_array {
                            (2, "options")
                        } else {
                            (3, "value")
                        };
                        return Err(Thrown::type_error(format!(
                            "{f}(): Argument #{n} (${p}) must be a valid callback for option {}, {why}",
                            cb_name(cb)
                        )));
                    }
                }
            };
            if user {
                let has = fcc.is_some();
                {
                    let mut ch = e.borrow_mut();
                    match cb {
                        Cb::Write => ch.write.fcc = fcc,
                        Cb::Header => ch.write_header.fcc = fcc,
                        _ => ch.read.fcc = fcc,
                    }
                }
                if !has {
                    set_method(default);
                    return Ok(true);
                }
                set_method(Method::User);
            } else {
                let (func, data): (*const c_void, &str) = match cb {
                    Cb::Progress => (
                        progress_cb as sys::ProgressFn as *const c_void,
                        "CURLOPT_PROGRESSDATA",
                    ),
                    Cb::Xferinfo => (
                        xferinfo_cb as sys::XferinfoFn as *const c_void,
                        "CURLOPT_XFERINFODATA",
                    ),
                    Cb::Fnmatch => (
                        fnmatch_cb as sys::FnmatchFn as *const c_void,
                        "CURLOPT_FNMATCH_DATA",
                    ),
                    Cb::Debug => (
                        debug_cb as sys::DebugFn as *const c_void,
                        "CURLOPT_DEBUGDATA",
                    ),
                    Cb::Prereq => (
                        prereq_cb as sys::PrereqFn as *const c_void,
                        "CURLOPT_PREREQDATA",
                    ),
                    _ => (
                        sshhostkey_cb as sys::SshHostkeyFn as *const c_void,
                        "CURLOPT_SSH_HOSTKEYDATA",
                    ),
                };
                {
                    let mut ch = e.borrow_mut();
                    match cb {
                        Cb::Progress => ch.progress = fcc,
                        Cb::Xferinfo => ch.xferinfo = fcc,
                        Cb::Fnmatch => ch.fnmatch = fcc,
                        Cb::Debug => ch.debug = fcc,
                        Cb::Prereq => ch.prereq = fcc,
                        _ => ch.sshhostkey = fcc,
                    }
                }
                let p = ctx(e);
                logged(host, f, pending, || {
                    setopt_ptr(cp, option, func);
                    setopt_ptr(cp, opt(data), p);
                })?;
            }
        }
        Some(Kind::SslVerifyHost) => {
            let lval = zval_get_long(host, &zvalue)?;
            if lval == 1 {
                crate::notice(
                    host,
                    f,
                    "CURLOPT_SSL_VERIFYHOST no longer accepts the value 1, value 2 will be used instead",
                )?;
                error = logged(host, f, pending, || setopt_long(cp, option, 2))?;
            } else {
                error = logged(host, f, pending, || setopt_long(cp, option, lval))?;
            }
        }
        Some(Kind::Long) => {
            let lval = zval_get_long(host, &zvalue)?;
            error = logged(host, f, pending, || setopt_long(cp, option, lval))?;
        }
        Some(Kind::SafeUpload) => {
            if !conv::to_bool(&zvalue) {
                return Err(Thrown::error(
                    "ValueError",
                    format!("{f}(): Disabling safe uploads is no longer supported"),
                ));
            }
        }
        Some(Kind::Str) | Some(Kind::Url) => {
            let s = crate::argparse::zval_get_string(host, &zvalue)?;
            return option_str(host, f, e, pending, option, &s);
        }
        Some(Kind::Path) | Some(Kind::FilePath) => {
            let s = crate::argparse::zval_get_string(host, &zvalue)?;
            return option_str(host, f, e, pending, option, &absolute_path(&s));
        }
        Some(Kind::Nullable) | Some(Kind::NullablePath) => {
            if matches!(zvalue, Value::Null) {
                error = logged(host, f, pending, || {
                    setopt_ptr(cp, option, std::ptr::null())
                })?;
            } else {
                let mut s = crate::argparse::zval_get_string(host, &zvalue)?;
                if kind == Some(Kind::NullablePath) {
                    s = absolute_path(&s);
                }
                return option_str(host, f, e, pending, option, &s);
            }
        }
        Some(Kind::Private) => {
            e.borrow_mut().private = Some(zvalue.clone());
            return Ok(true);
        }
        Some(Kind::FileHandle) => {
            let stream = match &zvalue {
                Value::Null => None,
                Value::Resource(_) if host.stream_is_open(&zvalue) => Some(zvalue.clone()),
                Value::Resource(_) => {
                    return Err(Thrown::type_error(format!(
                        "{f}(): supplied resource is not a valid File-Handle resource"
                    )))
                }
                _ => {
                    return Err(Thrown::type_error(format!(
                        "{f}(): supplied argument is not a valid File-Handle resource"
                    )))
                }
            };
            let writable = |host: &dyn Host, s: &Value| {
                host.stream_mode(s)
                    .is_none_or(|m| m.first() != Some(&b'r') || m.get(1) == Some(&b'+'))
            };
            let not_writable = || {
                Thrown::error(
                    "ValueError",
                    format!("{f}(): The provided file handle must be writable"),
                )
            };
            if option == sys::php("CURLOPT_FILE") {
                match stream {
                    None => {
                        let mut ch = e.borrow_mut();
                        ch.write.stream = None;
                        ch.write.method = Method::Stdout;
                    }
                    Some(s) if writable(host, &s) => {
                        let mut ch = e.borrow_mut();
                        ch.write.stream = Some(s);
                        ch.write.method = Method::File;
                    }
                    Some(_) => return Err(not_writable()),
                }
            } else if option == sys::php("CURLOPT_WRITEHEADER") {
                match stream {
                    None => {
                        let mut ch = e.borrow_mut();
                        ch.write_header.stream = None;
                        ch.write_header.method = Method::Ignore;
                    }
                    Some(s) if writable(host, &s) => {
                        let mut ch = e.borrow_mut();
                        ch.write_header.stream = Some(s);
                        ch.write_header.method = Method::File;
                    }
                    Some(_) => return Err(not_writable()),
                }
            } else if option == sys::php("CURLOPT_INFILE") {
                e.borrow_mut().read.stream = stream;
            } else {
                // CURLOPT_STDERR: libcurl writes to a stdio stream.
                match stream {
                    None => {
                        error = logged(host, f, pending, || {
                            setopt_ptr(cp, option, std::ptr::null())
                        })?;
                        let old = {
                            let mut ch = e.borrow_mut();
                            ch.std_err = None;
                            ch.stderr_file.take()
                        };
                        drop(old);
                    }
                    Some(s) if writable(host, &s) => {
                        let boxed = Box::new(s.clone());
                        let file = unsafe {
                            sys::rhp_curl_cookie_file(&*boxed as *const Value as *mut c_void)
                        };
                        if file.is_null() {
                            return Ok(false);
                        }
                        let cf = Rc::new(CookieFile {
                            file,
                            _stream: boxed,
                        });
                        error = logged(host, f, pending, || {
                            setopt_ptr(cp, option, file as *const c_void)
                        })?;
                        let old = {
                            let mut ch = e.borrow_mut();
                            ch.std_err = Some(s);
                            ch.stderr_file.replace(cf)
                        };
                        drop(old);
                    }
                    Some(_) => return Err(not_writable()),
                }
            }
        }
        Some(Kind::Slist) => {
            let Value::Array(a) = &zvalue else {
                let name = SLIST
                    .iter()
                    .map(|n| format!("CURLOPT_{n}"))
                    .find(|n| sys::php_defined(n) && sys::php(n) == option)
                    .unwrap_or_default();
                return Err(Thrown::type_error(format!(
                    "{f}(): The {name} option must have an array value"
                )));
            };
            let mut lines = Vec::with_capacity(a.len());
            for (_, v) in a.iter() {
                lines.push(crate::argparse::zval_get_string(host, &deref(v))?);
            }
            let mut slist: *mut sys::curl_slist = std::ptr::null_mut();
            for s in &lines {
                let c = c_trunc(s);
                let new = unsafe { sys::curl_slist_append(slist, c.as_ptr()) };
                if new.is_null() {
                    unsafe { sys::curl_slist_free_all(slist) };
                    crate::warn(host, f, "Could not build curl_slist")?;
                    return Ok(false);
                }
                slist = new;
            }
            if !slist.is_null() {
                let ch = e.borrow();
                let shared = Rc::strong_count(&ch.to_free) > 1;
                let mut tf = ch.to_free.borrow_mut();
                if shared {
                    tf.slists.push((-1, slist));
                } else if let Some(slot) = tf.slists.iter_mut().find(|(o, _)| *o == option) {
                    let old = std::mem::replace(&mut slot.1, slist);
                    unsafe { sys::curl_slist_free_all(old) };
                } else {
                    tf.slists.push((option, slist));
                }
            }
            error = logged(host, f, pending, || {
                setopt_ptr(cp, option, slist as *const c_void)
            })?;
        }
        Some(Kind::Ignore) => {}
        Some(Kind::PostFields) => {
            if let Value::Array(a) = &zvalue {
                if a.is_empty() {
                    error = logged(host, f, pending, || {
                        setopt_ptr(cp, option, c"".as_ptr() as *const c_void);
                        setopt_long(cp, opt("CURLOPT_POSTFIELDSIZE"), 0)
                    })?;
                } else {
                    let (parts, files) = mime_parts(host, f, a)?;
                    if files {
                        e.borrow_mut().postfields = Some(zvalue.clone());
                    }
                    return Ok(apply_mime(host, e, &parts));
                }
            } else {
                let s = crate::argparse::zval_get_string(host, &zvalue)?;
                let c = {
                    let mut v = s.clone();
                    v.push(0);
                    v
                };
                error = logged(host, f, pending, || {
                    setopt_long(cp, opt("CURLOPT_POSTFIELDSIZE"), s.len() as i64);
                    setopt_ptr(
                        cp,
                        opt("CURLOPT_COPYPOSTFIELDS"),
                        c.as_ptr() as *const c_void,
                    )
                })?;
            }
        }
        Some(Kind::ReturnTransfer) => {
            e.borrow_mut().write.method = if conv::to_bool(&zvalue) {
                Method::Return
            } else {
                Method::Stdout
            };
        }
        Some(Kind::OffT) => {
            let lval = zval_get_long(host, &zvalue)?;
            error = logged(host, f, pending, || setopt_off(cp, option, lval))?;
        }
        Some(Kind::PostRedir) => {
            let lval = zval_get_long(host, &zvalue)?;
            let v = lval & sys::php("CURL_REDIR_POST_ALL");
            error = logged(host, f, pending, || setopt_long(cp, option, v))?;
        }
        Some(Kind::HeaderOut) => {
            if e.borrow().debug.is_some() {
                return Err(Thrown::error(
                    "ValueError",
                    "CURLINFO_HEADER_OUT option must not be set when the CURLOPT_DEBUGFUNCTION option is set",
                ));
            }
            let p = ctx(e);
            let on = conv::to_bool(&zvalue);
            logged(host, f, pending, || {
                if on {
                    setopt_ptr(
                        cp,
                        opt("CURLOPT_DEBUGFUNCTION"),
                        debug_cb as sys::DebugFn as *const c_void,
                    );
                    setopt_ptr(cp, opt("CURLOPT_DEBUGDATA"), p);
                    setopt_long(cp, opt("CURLOPT_VERBOSE"), 1);
                } else {
                    setopt_ptr(cp, opt("CURLOPT_DEBUGFUNCTION"), std::ptr::null());
                    setopt_ptr(cp, opt("CURLOPT_DEBUGDATA"), std::ptr::null());
                    setopt_long(cp, opt("CURLOPT_VERBOSE"), 0);
                }
            })?;
        }
        Some(Kind::Share) => {
            if let Some(sh) = share_of(&zvalue) {
                let share = sh.share;
                logged(host, f, pending, || {
                    setopt_ptr(cp, option, share as *const c_void)
                })?;
                let old = e.borrow_mut().share.replace(zvalue.clone());
                drop(old);
            }
        }
        Some(Kind::Blob) => {
            let mut s = crate::argparse::zval_get_string(host, &zvalue)?;
            let blob = sys::curl_blob {
                data: s.as_mut_ptr() as *mut c_void,
                len: s.len(),
                flags: sys::internal("CURL_BLOB_COPY") as _,
            };
            error = logged(host, f, pending, || {
                setopt_ptr(cp, option, &blob as *const sys::curl_blob as *const c_void)
            })?;
        }
        None => {
            let msg = if is_array {
                format!("{f}(): Argument #2 ($options) must contain only valid cURL options")
            } else {
                format!("{f}(): Argument #2 ($option) is not a valid cURL option")
            };
            e.borrow_mut().errno = sys::internal("CURLE_UNKNOWN_OPTION") as i32;
            return Err(Thrown::error("ValueError", msg));
        }
    }
    e.borrow_mut().errno = error;
    Ok(error == 0)
}

// ---------------------------------------------------------------------------
// functions
// ---------------------------------------------------------------------------

pub(super) fn fn_curl_init(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_init";
    let url = crate::ext::zpp::opt_str_arg(host, args, 0, f, "url")?;
    global_init();
    let cp = unsafe { sys::curl_easy_init() };
    if cp.is_null() {
        crate::warn(host, f, "Could not initialize a new cURL handle")?;
        return Ok(Value::Bool(false));
    }
    let (v, h) = match new_object(host, "CurlHandle") {
        Ok(x) => x,
        Err(t) => {
            unsafe { sys::curl_easy_cleanup(cp) };
            return Err(t);
        }
    };
    let e = Rc::new(RefCell::new(Easy::new(cp, &h)));
    set_default_options(host, &e);
    register(h.borrow().id, e.clone());
    if let Some(url) = url {
        let mut pending = None;
        let ok = option_str(host, f, &e, &mut pending, sys::php("CURLOPT_URL"), &url)?;
        if let Some(t) = pending {
            return Err(t);
        }
        if !ok {
            return Ok(Value::Bool(false));
        }
    }
    Ok(v)
}

pub(super) fn fn_curl_setopt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_setopt";
    let (_, e) = easy_arg(host, args, 0, f, "handle")?;
    let option = long_arg(host, args, 1, f, "option")?;
    let value = args.get(2).cloned().unwrap_or(Value::Null);
    let mut pending = None;
    let ok = set_option(host, f, &e, option, &value, false, &mut pending)?;
    match pending {
        Some(t) => Err(t),
        None => Ok(Value::Bool(ok)),
    }
}

pub(super) fn fn_curl_setopt_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_setopt_array";
    let (_, e) = easy_arg(host, args, 0, f, "handle")?;
    let arr = match args.get(1).map(deref) {
        Some(Value::Array(a)) => a,
        other => {
            let v = other.unwrap_or(Value::Null);
            return Err(Thrown::type_error(format!(
                "{f}(): Argument #2 ($options) must be of type array, {} given",
                crate::argparse::value_name(host, &v)
            )));
        }
    };
    // A callback's exception does not stop the loop (php checks only the
    // FAILURE return); it is thrown at the end.
    let mut pending = None;
    let mut ok = true;
    for (k, v) in arr.iter() {
        let option = match k {
            ArrayKey::Int(i) => *i,
            ArrayKey::Str(_) => {
                return Err(Thrown::error(
                    "ValueError",
                    format!("{f}(): Argument #2 ($options) contains an invalid cURL option"),
                ))
            }
        };
        if !set_option(host, f, &e, option, v, true, &mut pending)? {
            ok = false;
            break;
        }
    }
    match pending {
        Some(t) => Err(t),
        None => Ok(Value::Bool(ok)),
    }
}

/// Run `perform` with `host` reachable from libcurl's callbacks; the
/// CURLcode is kept even when a callback threw.
pub(super) fn drive<R: Copy + Default>(
    host: &mut dyn Host,
    fname: &str,
    perform: impl FnOnce() -> R,
) -> (R, Result<(), Thrown>) {
    let code = Cell::new(R::default());
    let r = ffi::with_host(host, fname, || code.set(perform()));
    (code.get(), r)
}

pub(super) fn fn_curl_exec(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_exec";
    let (_obj, e) = easy_arg(host, args, 0, f, "handle")?;
    verify_handlers(host, f, &e, true)?;
    e.borrow_mut().cleanup_handle();
    let cp = e.borrow().cp;
    let (error, thrown) = drive(host, f, || unsafe { sys::curl_easy_perform(cp) });
    e.borrow_mut().errno = error;
    if error != 0 {
        e.borrow_mut().buf = None;
        thrown?;
        return Ok(Value::Bool(false));
    }
    thrown?;
    let (std_err, write, header, buf) = {
        let ch = e.borrow();
        (
            ch.std_err.clone(),
            ch.write.clone(),
            ch.write_header.clone(),
            ch.buf.clone(),
        )
    };
    if let Some(s) = std_err {
        if host.stream_is_open(&s) {
            host.stream_flush(&s);
        }
    }
    if write.method == Method::Return {
        if let Some(b) = buf {
            return Ok(Value::str(b));
        }
    }
    if write.method == Method::File {
        if let Some(s) = &write.stream {
            host.stream_flush(s);
        }
    }
    if header.method == Method::File {
        if let Some(s) = &header.stream {
            host.stream_flush(s);
        }
    }
    Ok(if write.method == Method::Return {
        Value::str("")
    } else {
        Value::Bool(true)
    })
}

fn getinfo_str(cp: *mut CURL, info: &str) -> Option<*const c_char> {
    let mut p: *const c_char = std::ptr::null();
    let r = unsafe { sys::curl_easy_getinfo(cp, opt(info) as c_int, &mut p as *mut *const c_char) };
    (r == 0).then_some(p)
}

fn getinfo_long(cp: *mut CURL, info: &str) -> Option<i64> {
    let mut v: c_long = 0;
    let r = unsafe { sys::curl_easy_getinfo(cp, opt(info) as c_int, &mut v as *mut c_long) };
    (r == 0).then_some(v as i64)
}

fn getinfo_double(cp: *mut CURL, info: &str) -> Option<f64> {
    let mut v: c_double = 0.0;
    let r = unsafe { sys::curl_easy_getinfo(cp, opt(info) as c_int, &mut v as *mut c_double) };
    (r == 0).then_some(v)
}

fn getinfo_off(cp: *mut CURL, info: &str) -> Option<i64> {
    let mut v: curl_off_t = 0;
    let r = unsafe { sys::curl_easy_getinfo(cp, opt(info) as c_int, &mut v as *mut curl_off_t) };
    (r == 0).then_some(v)
}

/// `create_certinfo`.
fn certinfo(
    host: &mut dyn Host,
    ci: *const sys::curl_certinfo,
    out: &mut PhpArray,
) -> Result<(), Thrown> {
    if ci.is_null() {
        return Ok(());
    }
    let ci = unsafe { &*ci };
    for i in 0..ci.num_of_certs.max(0) as usize {
        let mut hash = PhpArray::new();
        let mut s = unsafe { *ci.certinfo.add(i) };
        while !s.is_null() {
            let data = cstr_bytes(unsafe { (*s).data });
            let head = &data[..data.len().min(63)];
            match head.iter().position(|&b| b == b':') {
                Some(colon) => {
                    hash.insert(
                        ArrayKey::from_bytes(&data[..colon]),
                        Value::str(data[colon + 1..].to_vec()),
                    );
                }
                None => crate::warn(
                    host,
                    "curl_getinfo",
                    "Could not extract hash key from certificate info",
                )?,
            }
            s = unsafe { (*s).next };
        }
        out.push(Value::Array(hash));
    }
    Ok(())
}

pub(super) fn fn_curl_getinfo(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_getinfo";
    let (_, e) = easy_arg(host, args, 0, f, "handle")?;
    let option = match args.get(1).map(deref) {
        None | Some(Value::Null) => None,
        Some(_) => Some(long_arg(host, args, 1, f, "option")?),
    };
    let cp = e.borrow().cp;
    let Some(option) = option else {
        let mut a = PhpArray::new();
        let str_or_empty = |p: *const c_char| Value::str(cstr_bytes(p));
        let s = |a: &mut PhpArray, key: &str, info: &str| {
            if let Some(p) = getinfo_str(cp, info) {
                a.insert(akey(key), str_or_empty(p));
            }
        };
        s(&mut a, "url", "CURLINFO_EFFECTIVE_URL");
        if let Some(p) = getinfo_str(cp, "CURLINFO_CONTENT_TYPE") {
            a.insert(
                akey("content_type"),
                if p.is_null() {
                    Value::Null
                } else {
                    str_or_empty(p)
                },
            );
        }
        let l = |a: &mut PhpArray, key: &str, info: &str| {
            if let Some(v) = getinfo_long(cp, info) {
                a.insert(akey(key), Value::Int(v));
            }
        };
        let d = |a: &mut PhpArray, key: &str, info: &str| {
            if let Some(v) = getinfo_double(cp, info) {
                a.insert(akey(key), Value::Float(v));
            }
        };
        let o = |a: &mut PhpArray, key: &str, info: &str| {
            if let Some(v) = getinfo_off(cp, info) {
                a.insert(akey(key), Value::Int(v));
            }
        };
        l(&mut a, "http_code", "CURLINFO_HTTP_CODE");
        l(&mut a, "header_size", "CURLINFO_HEADER_SIZE");
        l(&mut a, "request_size", "CURLINFO_REQUEST_SIZE");
        l(&mut a, "filetime", "CURLINFO_FILETIME");
        l(&mut a, "ssl_verify_result", "CURLINFO_SSL_VERIFYRESULT");
        l(&mut a, "redirect_count", "CURLINFO_REDIRECT_COUNT");
        d(&mut a, "total_time", "CURLINFO_TOTAL_TIME");
        d(&mut a, "namelookup_time", "CURLINFO_NAMELOOKUP_TIME");
        d(&mut a, "connect_time", "CURLINFO_CONNECT_TIME");
        d(&mut a, "pretransfer_time", "CURLINFO_PRETRANSFER_TIME");
        d(&mut a, "size_upload", "CURLINFO_SIZE_UPLOAD");
        d(&mut a, "size_download", "CURLINFO_SIZE_DOWNLOAD");
        d(&mut a, "speed_download", "CURLINFO_SPEED_DOWNLOAD");
        d(&mut a, "speed_upload", "CURLINFO_SPEED_UPLOAD");
        d(
            &mut a,
            "download_content_length",
            "CURLINFO_CONTENT_LENGTH_DOWNLOAD",
        );
        d(
            &mut a,
            "upload_content_length",
            "CURLINFO_CONTENT_LENGTH_UPLOAD",
        );
        d(&mut a, "starttransfer_time", "CURLINFO_STARTTRANSFER_TIME");
        d(&mut a, "redirect_time", "CURLINFO_REDIRECT_TIME");
        s(&mut a, "redirect_url", "CURLINFO_REDIRECT_URL");
        s(&mut a, "primary_ip", "CURLINFO_PRIMARY_IP");
        let mut ci: *const sys::curl_certinfo = std::ptr::null();
        let r = unsafe {
            sys::curl_easy_getinfo(
                cp,
                opt("CURLINFO_CERTINFO") as c_int,
                &mut ci as *mut *const sys::curl_certinfo,
            )
        };
        if r == 0 {
            let mut list = PhpArray::new();
            certinfo(host, ci, &mut list)?;
            a.insert(akey("certinfo"), Value::Array(list));
        }
        l(&mut a, "primary_port", "CURLINFO_PRIMARY_PORT");
        s(&mut a, "local_ip", "CURLINFO_LOCAL_IP");
        l(&mut a, "local_port", "CURLINFO_LOCAL_PORT");
        l(&mut a, "http_version", "CURLINFO_HTTP_VERSION");
        l(&mut a, "protocol", "CURLINFO_PROTOCOL");
        l(
            &mut a,
            "ssl_verifyresult",
            "CURLINFO_PROXY_SSL_VERIFYRESULT",
        );
        s(&mut a, "scheme", "CURLINFO_SCHEME");
        o(&mut a, "appconnect_time_us", "CURLINFO_APPCONNECT_TIME_T");
        if sys::php_defined("CURLINFO_QUEUE_TIME_T") {
            o(&mut a, "queue_time_us", "CURLINFO_QUEUE_TIME_T");
        }
        o(&mut a, "connect_time_us", "CURLINFO_CONNECT_TIME_T");
        o(&mut a, "namelookup_time_us", "CURLINFO_NAMELOOKUP_TIME_T");
        o(&mut a, "pretransfer_time_us", "CURLINFO_PRETRANSFER_TIME_T");
        o(&mut a, "redirect_time_us", "CURLINFO_REDIRECT_TIME_T");
        o(
            &mut a,
            "starttransfer_time_us",
            "CURLINFO_STARTTRANSFER_TIME_T",
        );
        if sys::php_defined("CURLINFO_POSTTRANSFER_TIME_T") {
            o(
                &mut a,
                "posttransfer_time_us",
                "CURLINFO_POSTTRANSFER_TIME_T",
            );
        }
        o(&mut a, "total_time_us", "CURLINFO_TOTAL_TIME_T");
        if let Some(h) = e.borrow().header_out.clone() {
            a.insert(akey("request_header"), Value::str(h));
        }
        if sys::php_defined("CURLINFO_EFFECTIVE_METHOD") {
            s(&mut a, "effective_method", "CURLINFO_EFFECTIVE_METHOD");
        }
        if sys::php_defined("CURLINFO_CAPATH") {
            s(&mut a, "capath", "CURLINFO_CAPATH");
            s(&mut a, "cainfo", "CURLINFO_CAINFO");
        }
        if sys::php_defined("CURLINFO_USED_PROXY") {
            l(&mut a, "used_proxy", "CURLINFO_USED_PROXY");
        }
        if sys::php_defined("CURLINFO_HTTPAUTH_USED") {
            l(&mut a, "httpauth_used", "CURLINFO_HTTPAUTH_USED");
            l(&mut a, "proxyauth_used", "CURLINFO_PROXYAUTH_USED");
        }
        if sys::php_defined("CURLINFO_CONN_ID") {
            o(&mut a, "conn_id", "CURLINFO_CONN_ID");
        }
        return Ok(Value::Array(a));
    };
    if option == sys::php("CURLINFO_HEADER_OUT") {
        return Ok(match e.borrow().header_out.clone() {
            Some(h) => Value::str(h),
            None => Value::Bool(false),
        });
    }
    if option == sys::php("CURLINFO_CERTINFO") {
        let mut ci: *const sys::curl_certinfo = std::ptr::null();
        let r = unsafe {
            sys::curl_easy_getinfo(
                cp,
                option as c_int,
                &mut ci as *mut *const sys::curl_certinfo,
            )
        };
        if r != 0 {
            return Ok(Value::Bool(false));
        }
        let mut list = PhpArray::new();
        certinfo(host, ci, &mut list)?;
        return Ok(Value::Array(list));
    }
    if option == sys::php("CURLINFO_PRIVATE") {
        return Ok(e.borrow().private.clone().unwrap_or(Value::Bool(false)));
    }
    let ty = sys::internal("CURLINFO_TYPEMASK") & option;
    let c = option as c_int;
    let falsy = Value::Bool(false);
    if ty == sys::internal("CURLINFO_STRING") {
        let mut p: *const c_char = std::ptr::null();
        let r = unsafe { sys::curl_easy_getinfo(cp, c, &mut p as *mut *const c_char) };
        Ok(if r == 0 && !p.is_null() {
            Value::str(cstr_bytes(p))
        } else {
            falsy
        })
    } else if ty == sys::internal("CURLINFO_LONG") {
        let mut v: c_long = 0;
        let r = unsafe { sys::curl_easy_getinfo(cp, c, &mut v as *mut c_long) };
        Ok(if r == 0 { Value::Int(v as i64) } else { falsy })
    } else if ty == sys::internal("CURLINFO_DOUBLE") {
        let mut v: c_double = 0.0;
        let r = unsafe { sys::curl_easy_getinfo(cp, c, &mut v as *mut c_double) };
        Ok(if r == 0 { Value::Float(v) } else { falsy })
    } else if ty == sys::internal("CURLINFO_SLIST") {
        let mut list: *mut sys::curl_slist = std::ptr::null_mut();
        let r = unsafe { sys::curl_easy_getinfo(cp, c, &mut list as *mut *mut sys::curl_slist) };
        if r != 0 {
            return Ok(falsy);
        }
        let mut a = PhpArray::new();
        let mut cur = list;
        while !cur.is_null() {
            a.push(Value::str(cstr_bytes(unsafe { (*cur).data })));
            cur = unsafe { (*cur).next };
        }
        unsafe { sys::curl_slist_free_all(list) };
        Ok(Value::Array(a))
    } else if ty == sys::internal("CURLINFO_OFF_T") {
        let mut v: curl_off_t = 0;
        let r = unsafe { sys::curl_easy_getinfo(cp, c, &mut v as *mut curl_off_t) };
        Ok(if r == 0 { Value::Int(v) } else { falsy })
    } else {
        Ok(falsy)
    }
}

pub(super) fn fn_curl_error(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, e) = easy_arg(host, args, 0, "curl_error", "handle")?;
    let ch = e.borrow();
    if ch.errno == 0 {
        return Ok(Value::str(""));
    }
    let s = ch.error_str();
    Ok(Value::str(if s.is_empty() {
        cstr_bytes(unsafe { sys::curl_easy_strerror(ch.errno) })
    } else {
        s
    }))
}

pub(super) fn fn_curl_errno(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, e) = easy_arg(host, args, 0, "curl_errno", "handle")?;
    let n = e.borrow().errno;
    Ok(Value::Int(n as i64))
}

pub(super) fn fn_curl_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_close";
    crate::deprecated_fn(host, f, "8.5", "as it has no effect since PHP 8.0")?;
    let (_, e) = easy_arg(host, args, 0, f, "handle")?;
    if e.borrow().in_callback {
        return Err(Thrown::error(
            "Error",
            format!("{f}(): Attempt to close cURL handle from a callback"),
        ));
    }
    Ok(Value::Null)
}

pub(super) fn fn_curl_reset(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_reset";
    let (_, e) = easy_arg(host, args, 0, f, "handle")?;
    if e.borrow().in_callback {
        return Err(Thrown::error(
            "Error",
            format!("{f}(): Attempt to reset cURL handle from a callback"),
        ));
    }
    let cp = e.borrow().cp;
    unsafe { sys::curl_easy_reset(cp) };
    let old = {
        let mut ch = e.borrow_mut();
        // _php_curl_reset_handlers
        ch.write.stream = None;
        ch.write.method = Method::Stdout;
        ch.write_header.stream = None;
        ch.write_header.method = Method::Ignore;
        ch.read.stream = None;
        ch.read.method = Method::Direct;
        ch.std_err = None;
        ch.progress = None;
        ch.xferinfo = None;
        ch.fnmatch = None;
        ch.debug = None;
        ch.prereq = None;
        ch.sshhostkey = None;
        ch.stderr_file.take()
    };
    drop(old);
    set_default_options(host, &e);
    Ok(Value::Null)
}

pub(super) fn fn_curl_copy_handle(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_copy_handle";
    let (_, e) = easy_arg(host, args, 0, f, "handle")?;
    let (v, h) = new_object(host, "CurlHandle")?;
    match duplicate(host, &e, &h)? {
        Some(copy) => {
            register(h.borrow().id, copy);
            Ok(v)
        }
        None => {
            crate::warn(host, f, "Cannot duplicate cURL handle")?;
            Ok(Value::Bool(false))
        }
    }
}

fn escape_fn(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    unescape: bool,
) -> Result<Value, Thrown> {
    let (_, e) = easy_arg(host, args, 0, f, "handle")?;
    let s = crate::ext::zpp::str_arg(host, args, 1, f, "string")?;
    if s.len() > i32::MAX as usize {
        return Ok(Value::Bool(false));
    }
    let cp = e.borrow().cp;
    // NUL-terminated, as a zend_string is: libcurl calls strlen() when the
    // length is 0.
    let len = s.len() as c_int;
    let mut buf = s;
    buf.push(0);
    let mut outlen: c_int = 0;
    let p = unsafe {
        if unescape {
            sys::curl_easy_unescape(cp, buf.as_ptr() as *const c_char, len, &mut outlen)
        } else {
            sys::curl_easy_escape(cp, buf.as_ptr() as *const c_char, len)
        }
    };
    if p.is_null() {
        return Ok(Value::Bool(false));
    }
    let out = if unescape {
        bytes(p, outlen.max(0) as usize).to_vec()
    } else {
        cstr_bytes(p)
    };
    unsafe { sys::curl_free(p as *mut c_void) };
    Ok(Value::str(out))
}

pub(super) fn fn_curl_escape(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    escape_fn(host, args, "curl_escape", false)
}

pub(super) fn fn_curl_unescape(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    escape_fn(host, args, "curl_unescape", true)
}

pub(super) fn fn_curl_pause(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_pause";
    let (_obj, e) = easy_arg(host, args, 0, f, "handle")?;
    let bitmask = long_arg(host, args, 1, f, "flags")?;
    let cp = e.borrow().cp;
    // Unpausing delivers buffered data to the write callback.
    let (r, thrown) = drive(host, f, || unsafe {
        sys::curl_easy_pause(cp, bitmask as c_int)
    });
    thrown?;
    Ok(Value::Int(r as i64))
}

pub(super) fn fn_curl_upkeep(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_upkeep";
    let (_obj, e) = easy_arg(host, args, 0, f, "handle")?;
    let cp = e.borrow().cp;
    let (r, thrown) = drive(host, f, || unsafe { sys::curl_easy_upkeep(cp) });
    e.borrow_mut().errno = r;
    thrown?;
    Ok(Value::Bool(r == 0))
}

/// A CurlHandle for an easy handle libcurl made for a server push, with the
/// parent's handlers (`_php_server_push_callback`).
pub(super) fn wrap_pushed(
    host: &mut dyn Host,
    easy: *mut CURL,
    parent: &Rc<RefCell<Easy>>,
) -> Result<(Value, Rc<RefCell<Easy>>), Thrown> {
    let (v, h) = new_object(host, "CurlHandle")?;
    let e = Rc::new(RefCell::new(Easy::new(easy, &h)));
    copy_handlers(&e, parent);
    register(h.borrow().id, e.clone());
    Ok((v, e))
}

/// libcurl freed a pushed handle php declined: the object must not.
pub(super) fn forget_cp(e: &RefCell<Easy>) {
    e.borrow_mut().cp = std::ptr::null_mut();
}
