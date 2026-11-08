//! What libmagic calls back into (rhp-csrc's glue/fileinfo.c): php's
//! streams, PCRE and php_error_docref. Each hook reaches the VM through
//! the C-callback bridge ([`ffi`]); the builtin that runs libmagic does so
//! inside [`ffi::with_host`].

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::ptr;
use std::rc::Rc;

use rhp_csrc::fileinfo::{Hooks, Stat};
use rhp_values::Value;

use crate::ext::libxml::ffi::{self, call_host, guard};
use crate::ext::pcre::php_pcre;
use crate::ext::pcre::Regex;
use crate::Host;

/// A `php_stream *` libmagic holds: a stream resource, or a directory's
/// entries.
pub(super) enum Handle {
    /// `owned`: libmagic opened it and closes it.
    Stream {
        res: Value,
        owned: bool,
    },
    Dir {
        entries: Vec<Vec<u8>>,
        pos: usize,
    },
}

impl Handle {
    /// A stream php's C passes to libmagic (`magic_stream`): the caller frees
    /// the box with [`Handle::free`].
    pub(super) fn borrowed(res: Value) -> *mut c_void {
        Box::into_raw(Box::new(Handle::Stream { res, owned: false })).cast()
    }

    /// # Safety
    ///
    /// `p` came from [`Handle::borrowed`] and is not used again.
    pub(super) unsafe fn free(p: *mut c_void) {
        drop(Box::from_raw(p as *mut Handle));
    }
}

thread_local! {
    /// The patterns libmagic holds (`pcre_cache_entry *`), alive until the
    /// call that compiled them returns, whatever the cache drops.
    static HELD: RefCell<Vec<Rc<Regex>>> = const { RefCell::new(Vec::new()) };
}

/// Run a libmagic call for builtin `fname`, its callbacks reaching `host`.
pub(super) fn run<R>(
    host: &mut dyn Host,
    fname: &str,
    f: impl FnOnce() -> R,
) -> Result<R, crate::Thrown> {
    rhp_csrc::fileinfo::set_hooks(&HOOKS);
    let held = HELD.with(|h| h.borrow().len());
    let r = ffi::with_host(host, fname, f);
    HELD.with(|h| h.borrow_mut().truncate(held));
    r
}

static HOOKS: Hooks = Hooks {
    open,
    opendir,
    close,
    read,
    write,
    get_line,
    readdir,
    stat,
    stat_path,
    fd,
    error,
    pcre_compile,
    pcre_match,
    pcre_test,
    pcre_replace,
};

/// # Safety
///
/// `p` is NULL or NUL-terminated.
unsafe fn bytes<'a>(p: *const c_char) -> &'a [u8] {
    if p.is_null() {
        b""
    } else {
        CStr::from_ptr(p).to_bytes()
    }
}

/// # Safety
///
/// `h` is a live [`Handle`].
unsafe fn handle<'a>(h: *mut c_void) -> &'a mut Handle {
    &mut *(h as *mut Handle)
}

fn res_of(h: &Handle) -> Option<&Value> {
    match h {
        Handle::Stream { res, .. } => Some(res),
        Handle::Dir { .. } => None,
    }
}

/// php_stream_open_wrapper(path, mode, options, NULL): with REPORT_ERRORS
/// a failure warns from the running builtin.
unsafe extern "C" fn open(path: *const c_char, mode: *const c_char, report: c_int) -> *mut c_void {
    guard(ptr::null_mut(), || {
        let path = unsafe { bytes(path) }.to_vec();
        let mode = unsafe { bytes(mode) }.to_vec();
        let fname = ffi::fname();
        let res = call_host(|h| {
            if report != 0 {
                crate::ext::standard::file::open_stream(
                    h,
                    &fname,
                    &path,
                    &mode,
                    &Value::Null,
                    false,
                )
            } else {
                Ok(h.stream_open_ctx(&path, &mode, &Value::Null))
            }
        })
        .flatten();
        match res {
            Some(res) => Box::into_raw(Box::new(Handle::Stream { res, owned: true })).cast(),
            None => ptr::null_mut(),
        }
    })
}

/// php_stream_opendir(path, options, NULL).
unsafe extern "C" fn opendir(path: *const c_char, _report: c_int) -> *mut c_void {
    guard(ptr::null_mut(), || {
        let path = unsafe { bytes(path) }.to_vec();
        match call_host(|h| Ok(h.stream_opendir(&path))).flatten() {
            Some(entries) => Box::into_raw(Box::new(Handle::Dir { entries, pos: 0 })).cast(),
            None => ptr::null_mut(),
        }
    })
}

unsafe extern "C" fn close(h: *mut c_void) -> c_int {
    guard(-1, || {
        let h = unsafe { Box::from_raw(h as *mut Handle) };
        if let Handle::Stream { res, owned: true } = *h {
            call_host(|host| Ok(host.stream_close(&res)));
        }
        0
    })
}

/// php_stream_read: what one fread() of `count` bytes returns.
unsafe extern "C" fn read(h: *mut c_void, buf: *mut c_char, count: usize) -> isize {
    guard(-1, || {
        let Some(res) = res_of(unsafe { handle(h) }).cloned() else {
            return -1;
        };
        match call_host(|host| Ok(host.stream_read(&res, count as i64))).flatten() {
            Some(data) => {
                let n = data.len().min(count);
                unsafe { ptr::copy_nonoverlapping(data.as_ptr(), buf.cast::<u8>(), n) };
                n as isize
            }
            None => -1,
        }
    })
}

unsafe extern "C" fn write(h: *mut c_void, buf: *const c_char, count: usize) -> isize {
    guard(-1, || {
        let Some(res) = res_of(unsafe { handle(h) }).cloned() else {
            return -1;
        };
        let data = unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), count) }.to_vec();
        match call_host(|host| Ok(host.stream_write(&res, &data))).flatten() {
            Some(n) => n as isize,
            None => -1,
        }
    })
}

/// A line of at most `max` bytes, its newline included, or -1 at the end.
unsafe extern "C" fn get_line(h: *mut c_void, buf: *mut c_char, max: usize) -> isize {
    guard(-1, || {
        let Some(res) = res_of(unsafe { handle(h) }).cloned() else {
            return -1;
        };
        match call_host(|host| Ok(host.stream_gets_max(&res, max))).flatten() {
            Some(line) => {
                let n = line.len().min(max);
                unsafe { ptr::copy_nonoverlapping(line.as_ptr(), buf.cast::<u8>(), n) };
                n as isize
            }
            None => -1,
        }
    })
}

unsafe extern "C" fn readdir(h: *mut c_void, buf: *mut c_char, cap: usize) -> isize {
    guard(-1, || match unsafe { handle(h) } {
        Handle::Dir { entries, pos } if *pos < entries.len() && cap > 0 => {
            let e = &entries[*pos];
            *pos += 1;
            let n = e.len().min(cap - 1);
            unsafe { ptr::copy_nonoverlapping(e.as_ptr(), buf.cast::<u8>(), n) };
            n as isize
        }
        _ => -1,
    })
}

fn to_stat(st: &crate::StatInfo) -> Stat {
    Stat {
        mode: st.mode,
        size: st.size as i64,
        dev: st.dev,
        ino: st.ino,
        nlink: st.nlink,
        uid: st.uid,
        gid: st.gid,
        rdev: st.rdev,
        atime: st.atime,
        mtime: st.mtime,
        ctime: st.ctime,
        blksize: st.blksize,
        blocks: st.blocks,
    }
}

unsafe extern "C" fn stat(h: *mut c_void, out: *mut Stat) -> c_int {
    guard(-1, || {
        let Some(res) = res_of(unsafe { handle(h) }).cloned() else {
            return -1;
        };
        match call_host(|host| Ok(host.stream_fstat(&res))).flatten() {
            Some(st) => {
                unsafe { *out = to_stat(&st) };
                0
            }
            None => -1,
        }
    })
}

unsafe extern "C" fn stat_path(path: *const c_char, out: *mut Stat) -> c_int {
    guard(-1, || {
        let path = unsafe { bytes(path) }.to_vec();
        match call_host(|host| Ok(host.stream_stat(&path))).flatten() {
            Some(st) => {
                unsafe { *out = to_stat(&st) };
                0
            }
            None => -1,
        }
    })
}

/// php_stream_cast(stream, PHP_STREAM_AS_FD): the descriptor of a stream
/// that has one (libmagic reads a regular file's tail through it). A user
/// stream casts to the stream its `stream_cast` returns.
unsafe extern "C" fn fd(h: *mut c_void) -> c_int {
    guard(-1, || {
        let Some(res) = res_of(unsafe { handle(h) }).cloned() else {
            return -1;
        };
        call_host(|host| Ok(cast_fd(host, &res)))
            .flatten()
            .unwrap_or(-1)
    })
}

fn cast_fd(host: &mut dyn Host, res: &Value) -> Option<i32> {
    use crate::main::streams::userspace as us;
    let Value::Resource(r) = res else {
        return None;
    };
    let me = r.id;
    let was = host.stream_no_fclose(res, true);
    let inner = us::with(host, res, |s, h| s.cast(h, us::CAST_AS_STDIO, true, me));
    host.stream_no_fclose(res, was);
    match inner {
        None => host.stream_fd(res),
        Some(None) => None,
        Some(Some(inner)) => cast_fd(host, &inner),
    }
}

/// php_error_docref(NULL, type, msg) from the running builtin.
unsafe extern "C" fn error(level: c_int, msg: *const c_char, len: usize) {
    guard((), || {
        let msg = unsafe { std::slice::from_raw_parts(msg.cast::<u8>(), len) }.to_vec();
        let fname = ffi::fname();
        call_host(|h| {
            if level as i64 == crate::E_WARNING {
                crate::warn(h, &fname, &msg)
            } else {
                let mut m = format!("{fname}(): ").into_bytes();
                m.extend_from_slice(&msg);
                h.emit_diagnostic(level as i64, &m)
            }
        });
    })
}

/// pcre_get_compiled_regex_cache: a compile error warns from the running
/// builtin.
unsafe extern "C" fn pcre_compile(pattern: *const c_char, len: usize) -> *mut c_void {
    guard(ptr::null_mut(), || {
        let pattern = unsafe { std::slice::from_raw_parts(pattern.cast::<u8>(), len) }.to_vec();
        let fname = ffi::fname();
        match call_host(|h| php_pcre::regex_cache(h, &fname, &pattern)).flatten() {
            Some(re) => {
                let p = Rc::as_ptr(&re) as *mut c_void;
                HELD.with(|h| h.borrow_mut().push(re));
                p
            }
            None => ptr::null_mut(),
        }
    })
}

/// # Safety
///
/// `p` is a pattern [`pcre_compile`] returned in the running call.
unsafe fn regex<'a>(p: *mut c_void) -> &'a Regex {
    &*(p as *const Regex)
}

unsafe extern "C" fn pcre_match(
    re: *mut c_void,
    subject: *const c_char,
    len: usize,
    start: *mut usize,
    end: *mut usize,
) -> c_int {
    guard(-1, || {
        let subject = unsafe { std::slice::from_raw_parts(subject.cast::<u8>(), len) };
        match php_pcre::match_impl(unsafe { regex(re) }, subject) {
            Ok(Some((s, e))) => {
                unsafe {
                    *start = s;
                    *end = e;
                }
                1
            }
            Ok(None) => 0,
            Err(()) => -1,
        }
    })
}

unsafe extern "C" fn pcre_test(
    re: *mut c_void,
    subject: *const c_char,
    len: usize,
    from: usize,
) -> c_int {
    guard(-1, || {
        let subject = unsafe { std::slice::from_raw_parts(subject.cast::<u8>(), len) };
        match unsafe { regex(re) }.find_at(subject, from) {
            Ok(Some(_)) => 1,
            Ok(None) => -1,
            Err(rc) => rc,
        }
    })
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn pcre_replace(
    re: *mut c_void,
    subject: *const c_char,
    len: usize,
    rep: *const c_char,
    rep_len: usize,
    limit: usize,
    count: *mut usize,
    out_len: *mut usize,
) -> *mut c_char {
    guard(ptr::null_mut(), || {
        let subject = unsafe { std::slice::from_raw_parts(subject.cast::<u8>(), len) };
        let rep = unsafe { std::slice::from_raw_parts(rep.cast::<u8>(), rep_len) };
        let Some((out, n)) =
            php_pcre::replace_impl(unsafe { regex(re) }, subject, rep, limit as i64)
        else {
            return ptr::null_mut();
        };
        unsafe {
            *count = n;
            *out_len = out.len();
            let p = rhp_csrc::rhp_csrc_emalloc(out.len() + 1) as *mut u8;
            ptr::copy_nonoverlapping(out.as_ptr(), p, out.len());
            *p.add(out.len()) = 0;
            p.cast()
        }
    })
}
