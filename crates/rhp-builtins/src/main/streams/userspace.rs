//! main/streams/userspace.c: stream wrappers written in PHP
//! (`stream_wrapper_register`) and the streams they open, under a port of
//! php's generic stream layer (main/streams/streams.c: the read buffer,
//! position, eof flag, buffered seeks, chunked writes).
//!
//! The VM keeps a [`UserStream`] per stream resource and hands it out for
//! each operation, so the wrapper's methods run with no stream-table
//! borrow held (they may open, read or close other streams).

use std::cell::RefCell;
use std::rc::Rc;

use rhp_values::{conv, ArrayKey, PhpArray, Thrown, Value};

use super::{StatInfo, StreamWrapper};
use crate::{diag, zend::zpp, Host, WrapperRestore};

// main/php_streams.h
pub const OPTION_BLOCKING: i64 = 1;
pub const OPTION_READ_BUFFER: i64 = 2;
pub const OPTION_WRITE_BUFFER: i64 = 3;
pub const OPTION_READ_TIMEOUT: i64 = 4;
pub const OPTION_SET_CHUNK_SIZE: i64 = 5;
pub const OPTION_LOCKING: i64 = 6;
pub const OPTION_TRUNCATE_API: i64 = 10;
pub const OPTION_CHECK_LIVENESS: i64 = 12;
pub const OPTION_RETURN_OK: i32 = 0;
pub const OPTION_RETURN_ERR: i32 = -1;
pub const OPTION_RETURN_NOTIMPL: i32 = -2;
pub const TRUNCATE_SUPPORTED: i64 = 0;
pub const TRUNCATE_SET_SIZE: i64 = 1;
pub const BUFFER_NONE: i64 = 0;
/// `PHP_STREAM_AS_STDIO` / `PHP_STREAM_AS_FD_FOR_SELECT`.
pub const CAST_AS_STDIO: i64 = 0;
pub const CAST_FOR_SELECT: i64 = 3;
/// `PHP_STREAM_IS_URL` (stream_wrapper_register's `$flags`).
pub const IS_URL: i64 = 1;
// stream open options (main/php_streams.h)
pub const USE_PATH: i64 = 1;
pub const REPORT_ERRORS: i64 = 8;
pub const STREAM_OPEN_FOR_INCLUDE: i64 = 0x80;
// url_stat flags
pub const URL_STAT_LINK: i64 = 1;
pub const URL_STAT_QUIET: i64 = 2;
/// PHP_STREAM_URL_STAT_IGNORE_OPEN_BASEDIR (php_stat passes it).
pub const URL_STAT_IGNORE_OPEN_BASEDIR: i64 = 4;
// stream_metadata options
pub const META_TOUCH: i64 = 1;
pub const META_OWNER_NAME: i64 = 2;
pub const META_OWNER: i64 = 3;
pub const META_GROUP_NAME: i64 = 4;
pub const META_GROUP: i64 = 5;
pub const META_ACCESS: i64 = 6;

const CHUNK: usize = 8192;
const E_WARNING: i64 = crate::E_WARNING;

/// A registered user wrapper (`struct php_user_stream_wrapper`).
pub struct UserWrapper {
    /// The class name as declared (`ce->name`).
    pub class: Vec<u8>,
    /// `wrapper.is_url` (`STREAM_IS_URL`).
    pub is_url: bool,
}

impl StreamWrapper for UserWrapper {
    fn exists(&self, _url: &[u8]) -> bool {
        false
    }
    fn get_contents(&self, _url: &[u8]) -> Option<Vec<u8>> {
        None
    }
    fn stat(&self, _url: &[u8]) -> Option<StatInfo> {
        None
    }
    fn has_stream_opener(&self) -> bool {
        true
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

thread_local! {
    /// FG(user_stream_current_filename): the URL a user opener is opening.
    static CURRENT_FILENAME: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    /// PG(in_user_include): a local user wrapper opens for include.
    static IN_USER_INCLUDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// php_stat's cache (BG(CurrentStatFile), BG(CurrentLStatFile)) as far
/// as it shows: for paths a user wrapper stats.
type StatCache = (Option<(Vec<u8>, StatInfo)>, Option<(Vec<u8>, StatInfo)>);

thread_local! {
    static STAT_CACHE: RefCell<StatCache> = const { RefCell::new((None, None)) };
}

/// `php_clear_stat_cache`.
pub fn clear_stat_cache() {
    STAT_CACHE.with(|c| *c.borrow_mut() = (None, None));
}

/// php_stat's `url_stat` through the stat cache: `None` when no user
/// wrapper serves `filename`.
pub fn cached_stat(
    host: &mut dyn Host,
    filename: &[u8],
    link: bool,
    quiet: bool,
    extra: i64,
) -> Option<Option<StatInfo>> {
    if !host.has_user_wrappers() {
        return None;
    }
    let (w, p) = locate(host, filename)?;
    let hit = STAT_CACHE.with(|c| {
        let c = c.borrow();
        let e = if link { &c.1 } else { &c.0 };
        e.as_ref()
            .filter(|(f, _)| f == filename)
            .map(|(_, s)| s.clone())
    });
    if hit.is_some() {
        return Some(hit);
    }
    let flags =
        if link { URL_STAT_LINK } else { 0 } | if quiet { URL_STAT_QUIET } else { 0 } | extra;
    // a failed stat leaves the cache as it was
    let Some(st) = url_stat(host, &w, &p, flags, None) else {
        return Some(None);
    };
    STAT_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if link {
            c.1 = Some((filename.to_vec(), st.clone()));
        }
        if !link || !st.is_link() {
            c.0 = Some((filename.to_vec(), st.clone()));
        }
    });
    Some(Some(st))
}

pub(crate) fn reset_state() {
    clear_stat_cache();
    CURRENT_FILENAME.with(|c| *c.borrow_mut() = None);
    IN_USER_INCLUDE.with(|c| c.set(false));
}

/// An ini flag's boolean value.
fn ini_on(host: &dyn Host, name: &[u8]) -> bool {
    host.ini_get(name).is_some_and(|v| {
        let v = v.to_ascii_lowercase();
        matches!(&v[..], b"1" | b"on" | b"yes" | b"true")
            || (!v.is_empty() && v[0].is_ascii_digit() && v != b"0")
    })
}

/// PG(in_user_include).
pub fn in_user_include() -> bool {
    IN_USER_INCLUDE.with(|c| c.get())
}

/// The length of the `scheme` of a `scheme://` path (php's protocol
/// scan: alphanumerics, `+`, `-`, `.`; at least two of them).
pub fn protocol_len(path: &[u8]) -> Option<usize> {
    let n = path
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        .count();
    (n > 1 && path[n..].starts_with(b"://")).then_some(n)
}

/// `php_resolve_path` over include_path (wrapper entries get a quiet
/// `url_stat`).
pub fn resolve_path(host: &mut dyn Host, name: &[u8]) -> Option<Vec<u8>> {
    crate::ext::standard::file::resolve_include_path(host, name)
}

/// `php_stream_locate_url_wrapper` for user wrappers: the wrapper the
/// path's protocol names (or that replaced `file://`) and the path its
/// opener gets. `None` when a built-in wrapper (or none) serves the path.
pub fn locate(host: &dyn Host, path: &[u8]) -> Option<(Rc<UserWrapper>, Vec<u8>)> {
    if !host.has_user_wrappers() {
        return None;
    }
    let n = path
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        .count();
    let rest = &path[n..];
    let protocol = (rest.first() == Some(&b':')
        && n > 1
        && (rest[1..].starts_with(b"//") || (n == 4 && path.starts_with(b"data:"))))
    .then(|| &path[..n]);
    if let Some(p) = protocol {
        if !p.eq_ignore_ascii_case(b"file") {
            let w = host
                .user_wrapper(p)
                .or_else(|| host.user_wrapper(&p.to_ascii_lowercase()))?;
            return Some((w, path.to_vec()));
        }
        // file://: the path after the protocol (a remote host is refused
        // by the caller's own lookup).
        let w = host
            .user_wrapper(p)
            .or_else(|| host.user_wrapper(b"file"))?;
        let mut q = &path[n + 1..];
        if path.len() >= 17 && path[..17].eq_ignore_ascii_case(b"file://localhost/") {
            q = &path[n + 1 + 11..];
        }
        while q.len() > 1 && q[1] == b'/' {
            q = &q[1..];
        }
        return Some((w, q.to_vec()));
    }
    let w = host.user_wrapper(b"file")?;
    Some((w, path.to_vec()))
}

// --- calling the wrapper's methods ---------------------------------------------

/// What `zend_call_method_if_exists` gave.
enum Call {
    /// FAILURE: no such callable method.
    Missing,
    /// The call threw (or an exception was pending): `retval` is UNDEF.
    Undef,
    Ret(Value),
}

fn call(host: &mut dyn Host, obj: &Value, name: &str, args: &[Value]) -> Call {
    let mut pair = PhpArray::new();
    pair.push(obj.clone());
    pair.push(Value::str(name));
    let cb = Value::Array(pair);
    if !host.is_callable_value(&cb) {
        return Call::Missing;
    }
    // zend_call_function does nothing while an exception is pending.
    if host.has_pending_throw() {
        return Call::Undef;
    }
    match host.call_callable(&cb, args) {
        Ok(v) => Call::Ret(rhp_values::ops::deref(&v)),
        Err(t) => {
            host.set_pending_throw(t);
            Call::Undef
        }
    }
}

/// `php_error_docref(NULL, level, msg)`; an error handler's exception
/// becomes the pending one.
fn docref(host: &mut dyn Host, level: i64, msg: &[u8]) {
    if let Err(t) = host.docref_diagnostic(level, msg) {
        host.set_pending_throw(t);
    }
}

fn not_implemented(host: &mut dyn Host, class: &[u8], method: &str, tail: &str) {
    let mut m = class.to_vec();
    m.extend_from_slice(b"::");
    m.extend_from_slice(method.as_bytes());
    m.extend_from_slice(tail.as_bytes());
    docref(host, E_WARNING, &m);
}

/// zval_is_true, with an object's bool cast.
fn is_true(host: &mut dyn Host, v: &Value) -> bool {
    match crate::zend::object_handlers::is_true(host, v) {
        Ok(b) => b,
        Err(t) => {
            host.set_pending_throw(t);
            false
        }
    }
}

/// `zval_get_long`, with an object's cast.
fn get_long(host: &mut dyn Host, v: &Value) -> i64 {
    if matches!(v, Value::Object(_)) {
        return match crate::zend::object_handlers::object_to_number(
            host,
            v,
            crate::zend::object_handlers::CastTarget::Int,
        ) {
            Ok(Some(n)) => conv::to_int(&n),
            Ok(None) => 1,
            Err(t) => {
                host.set_pending_throw(t);
                0
            }
        };
    }
    match crate::zend::operators::zval_get_long(host, v) {
        Ok(n) => n,
        Err(t) => {
            host.set_pending_throw(t);
            0
        }
    }
}

/// `try_convert_to_string`: `None` after the Error it throws.
fn try_to_string(host: &mut dyn Host, v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::Array(_) => {
            docref_plain(host, E_WARNING, b"Array to string conversion");
            Some(b"Array".to_vec())
        }
        Value::Object(_) | Value::Closure(_) => {
            match crate::zend::object_handlers::to_string(host, v) {
                Ok(Some(s)) => Some(s),
                Ok(None) => {
                    let name = host.object_class_name(v).unwrap_or_default();
                    let mut m = b"Object of class ".to_vec();
                    m.extend_from_slice(&name);
                    m.extend_from_slice(b" could not be converted to string");
                    host.set_pending_throw(Thrown::error(
                        "Error",
                        String::from_utf8_lossy(&m).into_owned(),
                    ));
                    None
                }
                Err(t) => {
                    host.set_pending_throw(t);
                    None
                }
            }
        }
        _ => Some(conv::to_string_bytes(v)),
    }
}

/// zend_error without the docref prefix.
fn docref_plain(host: &mut dyn Host, level: i64, msg: &[u8]) {
    if let Err(t) = host.emit_diagnostic(level, msg) {
        host.set_pending_throw(t);
    }
}

/// `statbuf_from_array`.
fn statbuf_from_array(host: &mut dyn Host, a: &PhpArray) -> StatInfo {
    let mut s = StatInfo::default();
    let mut get = |k: &str| -> Option<i64> {
        let v = a.get(&ArrayKey::from_bytes(k.as_bytes()))?.clone();
        Some(get_long(host, &rhp_values::ops::deref(&v)))
    };
    if let Some(v) = get("dev") {
        s.dev = v;
    }
    if let Some(v) = get("ino") {
        s.ino = v;
    }
    if let Some(v) = get("mode") {
        s.mode = v as u32;
    }
    if let Some(v) = get("nlink") {
        s.nlink = v;
    }
    if let Some(v) = get("uid") {
        s.uid = v;
    }
    if let Some(v) = get("gid") {
        s.gid = v;
    }
    if let Some(v) = get("rdev") {
        s.rdev = v;
    }
    if let Some(v) = get("size") {
        s.size = v as u64;
    }
    if let Some(v) = get("atime") {
        s.atime = v;
    }
    if let Some(v) = get("mtime") {
        s.mtime = v;
    }
    if let Some(v) = get("ctime") {
        s.ctime = v;
    }
    if let Some(v) = get("blksize") {
        s.blksize = v;
    }
    if let Some(v) = get("blocks") {
        s.blocks = v;
    }
    let fmt = s.mode & libc::S_IFMT as u32;
    s.is_dir = fmt == libc::S_IFDIR as u32;
    s.is_regular = fmt == libc::S_IFREG as u32;
    s
}

/// `user_stream_create_object`: an instance of the wrapper class with
/// `context` set before its constructor runs. `None` for a class that
/// cannot be instantiated, or after an exception.
fn create_object(host: &mut dyn Host, w: &UserWrapper, ctx: Option<&Value>) -> Option<Value> {
    if host.has_pending_throw() {
        return None;
    }
    let obj = match host.reflect_new_instance(&w.class, Vec::new(), false) {
        Ok(o) => o,
        Err(Thrown::Error { message, .. })
            if message.starts_with("Cannot instantiate abstract class")
                || message.starts_with("Cannot instantiate interface")
                || message.starts_with("Cannot instantiate trait") =>
        {
            return None
        }
        Err(t) => {
            host.set_pending_throw(t);
            return None;
        }
    };
    let ctxv = ctx.cloned().unwrap_or(Value::Null);
    if let Err(t) = host.reflect_set_prop(&obj, &w.class, b"context", ctxv) {
        host.set_pending_throw(t);
    }
    if host.has_pending_throw() {
        return None;
    }
    if let Some(cid) = host.resolve_class(&w.class) {
        if host.class_has_method(cid, b"__construct") {
            if let Err(t) = host.reflect_invoke(
                &obj,
                &w.class,
                b"__construct",
                Value::Array(PhpArray::new()),
            ) {
                host.set_pending_throw(t);
            }
        }
    }
    Some(obj)
}

/// A wrapper's error for the "Failed to open stream" warning (options
/// without REPORT_ERRORS: queued).
fn log_error(host: &mut dyn Host, options: i64, msg: String) {
    if options & REPORT_ERRORS != 0 {
        docref(host, E_WARNING, msg.as_bytes());
    } else {
        super::note_wrapper_error(msg);
    }
}

fn quoted(class: &[u8], method: &str) -> String {
    format!("\"{}::{method}\"", String::from_utf8_lossy(class))
}

/// What a user opener produced.
pub struct Opened {
    pub stream: Box<UserStream>,
    /// `*opened_path` (stream_open's by-reference fourth argument).
    pub opened_path: Option<Vec<u8>>,
}

/// Enter the recursion guard for `filename`; false (after the wrapper
/// error) when the same URL is being opened already.
fn enter_open(host: &mut dyn Host, options: i64, filename: &[u8]) -> bool {
    let same = CURRENT_FILENAME.with(|c| c.borrow().as_deref() == Some(filename));
    if same {
        log_error(host, options, "infinite recursion prevented".into());
        return false;
    }
    CURRENT_FILENAME.with(|c| *c.borrow_mut() = Some(filename.to_vec()));
    true
}

fn leave_open() {
    CURRENT_FILENAME.with(|c| *c.borrow_mut() = None);
}

/// `user_wrapper_opener`.
pub fn open(
    host: &mut dyn Host,
    w: &Rc<UserWrapper>,
    filename: &[u8],
    mode: &[u8],
    options: i64,
    ctx: Option<&Value>,
) -> Option<Opened> {
    if !enter_open(host, options, filename) {
        return None;
    }
    let old_include = in_user_include();
    if !w.is_url && options & STREAM_OPEN_FOR_INCLUDE != 0 && !ini_on(host, b"allow_url_include") {
        IN_USER_INCLUDE.with(|c| c.set(true));
    }
    let r = open_inner(host, w, filename, mode, options, ctx);
    leave_open();
    IN_USER_INCLUDE.with(|c| c.set(old_include));
    r
}

fn open_inner(
    host: &mut dyn Host,
    w: &Rc<UserWrapper>,
    filename: &[u8],
    mode: &[u8],
    options: i64,
    ctx: Option<&Value>,
) -> Option<Opened> {
    let obj = create_object(host, w, ctx)?;
    let opened = Rc::new(RefCell::new(Value::Null));
    let args = [
        Value::str(filename.to_vec()),
        Value::str(mode.to_vec()),
        Value::Int(options),
        Value::Ref(opened.clone()),
    ];
    match call(host, &obj, "stream_open", &args) {
        Call::Missing => {
            log_error(
                host,
                options,
                format!("{} is not implemented", quoted(&w.class, "stream_open")),
            );
            None
        }
        Call::Undef => None,
        Call::Ret(v) => {
            if is_true(host, &v) {
                let opened_path = match &*opened.borrow() {
                    Value::Str(s) => Some(s.as_bytes().to_vec()),
                    _ => None,
                };
                Some(Opened {
                    stream: Box::new(UserStream::new(obj, w.clone(), mode, false)),
                    opened_path,
                })
            } else {
                log_error(
                    host,
                    options,
                    format!("{} call failed", quoted(&w.class, "stream_open")),
                );
                None
            }
        }
    }
}

/// `user_wrapper_opendir`.
pub fn opendir(
    host: &mut dyn Host,
    w: &Rc<UserWrapper>,
    filename: &[u8],
    options: i64,
    ctx: Option<&Value>,
) -> Option<Box<UserStream>> {
    if !enter_open(host, options, filename) {
        return None;
    }
    let r = (|| {
        let obj = create_object(host, w, ctx)?;
        let args = [Value::str(filename.to_vec()), Value::Int(options)];
        match call(host, &obj, "dir_opendir", &args) {
            Call::Missing => {
                log_error(
                    host,
                    options,
                    format!("{} is not implemented", quoted(&w.class, "dir_opendir")),
                );
                None
            }
            Call::Undef => None,
            Call::Ret(v) => {
                if is_true(host, &v) {
                    let mut s = UserStream::new(obj, w.clone(), b"r", true);
                    s.no_buffer = true;
                    Some(Box::new(s))
                } else {
                    log_error(
                        host,
                        options,
                        format!("{} call failed", quoted(&w.class, "dir_opendir")),
                    );
                    None
                }
            }
        }
    })();
    leave_open();
    r
}

/// The bool a url op (`unlink`, `rename`, `mkdir`, `rmdir`, `metadata`)
/// returned: only `true` counts.
fn url_op(
    host: &mut dyn Host,
    w: &UserWrapper,
    method: &str,
    args: &[Value],
    ctx: Option<&Value>,
) -> bool {
    let Some(obj) = create_object(host, w, ctx) else {
        return false;
    };
    match call(host, &obj, method, args) {
        Call::Missing => {
            not_implemented(host, &w.class, method, " is not implemented!");
            false
        }
        Call::Ret(Value::Bool(b)) => b,
        _ => false,
    }
}

/// `user_wrapper_unlink`.
pub fn unlink(host: &mut dyn Host, w: &UserWrapper, url: &[u8], ctx: Option<&Value>) -> bool {
    url_op(host, w, "unlink", &[Value::str(url.to_vec())], ctx)
}

/// `user_wrapper_rename`.
pub fn rename(
    host: &mut dyn Host,
    w: &UserWrapper,
    from: &[u8],
    to: &[u8],
    ctx: Option<&Value>,
) -> bool {
    let args = [Value::str(from.to_vec()), Value::str(to.to_vec())];
    url_op(host, w, "rename", &args, ctx)
}

/// `user_wrapper_mkdir`.
pub fn mkdir(
    host: &mut dyn Host,
    w: &UserWrapper,
    url: &[u8],
    mode: i64,
    options: i64,
    ctx: Option<&Value>,
) -> bool {
    let args = [
        Value::str(url.to_vec()),
        Value::Int(mode),
        Value::Int(options),
    ];
    url_op(host, w, "mkdir", &args, ctx)
}

/// `user_wrapper_rmdir`.
pub fn rmdir(
    host: &mut dyn Host,
    w: &UserWrapper,
    url: &[u8],
    options: i64,
    ctx: Option<&Value>,
) -> bool {
    let args = [Value::str(url.to_vec()), Value::Int(options)];
    url_op(host, w, "rmdir", &args, ctx)
}

/// `user_wrapper_metadata`: `value` is the option's argument as php
/// passes it (an array of times for touch).
pub fn metadata(
    host: &mut dyn Host,
    w: &UserWrapper,
    url: &[u8],
    option: i64,
    value: Value,
) -> bool {
    let args = [Value::str(url.to_vec()), Value::Int(option), value];
    url_op(host, w, "stream_metadata", &args, None)
}

/// `user_wrapper_stat_url`.
pub fn url_stat(
    host: &mut dyn Host,
    w: &UserWrapper,
    url: &[u8],
    flags: i64,
    ctx: Option<&Value>,
) -> Option<StatInfo> {
    let obj = create_object(host, w, ctx)?;
    let args = [Value::str(url.to_vec()), Value::Int(flags)];
    match call(host, &obj, "url_stat", &args) {
        Call::Missing => {
            not_implemented(host, &w.class, "url_stat", " is not implemented!");
            None
        }
        Call::Ret(Value::Array(a)) => Some(statbuf_from_array(host, &a)),
        _ => None,
    }
}

// --- the stream ----------------------------------------------------------------

/// A stream a user wrapper opened: php's `php_stream` fields that matter
/// plus `php_userstream_data` (the object).
pub struct UserStream {
    /// The wrapper object (`us->object`, also `stream->wrapperdata`).
    pub obj: Value,
    pub wrapper: Rc<UserWrapper>,
    /// Opened by `dir_opendir` (php_stream_userspace_dir_ops).
    pub dir: bool,
    readbuf: Vec<u8>,
    readpos: usize,
    writepos: usize,
    /// `stream->position`.
    pub position: i64,
    /// `stream->eof`.
    pub eof: bool,
    /// PHP_STREAM_FLAG_NO_SEEK (stream_seek is missing).
    pub no_seek: bool,
    /// PHP_STREAM_FLAG_NO_BUFFER.
    pub no_buffer: bool,
    pub chunk_size: usize,
    /// PHP_STREAM_FLAG_WAS_WRITTEN.
    was_written: bool,
    pub mode: Vec<u8>,
    /// `stream->orig_path`.
    pub orig_path: Vec<u8>,
}

impl UserStream {
    fn new(obj: Value, wrapper: Rc<UserWrapper>, mode: &[u8], dir: bool) -> UserStream {
        UserStream {
            obj,
            wrapper,
            dir,
            readbuf: Vec::new(),
            readpos: 0,
            writepos: 0,
            position: 0,
            eof: false,
            no_seek: false,
            no_buffer: false,
            chunk_size: CHUNK,
            was_written: false,
            mode: mode.to_vec(),
            orig_path: Vec::new(),
        }
    }

    fn class(&self) -> Vec<u8> {
        self.wrapper.class.clone()
    }

    /// `ops->label`.
    pub fn label(&self) -> &'static str {
        if self.dir {
            "user-space-dir"
        } else {
            "user-space"
        }
    }

    /// `stream->writepos - stream->readpos`.
    pub fn unread(&self) -> usize {
        self.writepos - self.readpos
    }

    // --- php_stream_userspace_ops ---

    /// `php_userstreamop_read` into `readbuf[at..at + count]`.
    fn op_read(&mut self, host: &mut dyn Host, count: usize) -> (isize, Vec<u8>) {
        if self.dir {
            return (-1, Vec::new());
        }
        let obj = self.obj.clone();
        let ret = match call(host, &obj, "stream_read", &[Value::Int(count as i64)]) {
            Call::Undef => return (-1, Vec::new()),
            Call::Missing => {
                not_implemented(host, &self.class(), "stream_read", " is not implemented!");
                return (-1, Vec::new());
            }
            Call::Ret(v) => v,
        };
        if matches!(ret, Value::Bool(false)) {
            return (-1, Vec::new());
        }
        let Some(mut data) = try_to_string(host, &ret) else {
            return (-1, Vec::new());
        };
        if data.len() > count {
            let m = format!(
                "{}::stream_read - read {} bytes more data than requested ({} read, {} max) - excess data will be lost",
                String::from_utf8_lossy(&self.class()),
                data.len() - count,
                data.len(),
                count
            );
            docref(host, E_WARNING, m.as_bytes());
            data.truncate(count);
        }
        // the user stream cannot set eof itself: ask it
        match call(host, &obj, "stream_eof", &[]) {
            Call::Missing => {
                not_implemented(
                    host,
                    &self.class(),
                    "stream_eof",
                    " is not implemented! Assuming EOF",
                );
                self.eof = true;
                return (-1, data);
            }
            Call::Undef => {
                self.eof = true;
                return (-1, data);
            }
            Call::Ret(v) => {
                if is_true(host, &v) {
                    self.eof = true;
                }
            }
        }
        (data.len() as isize, data)
    }

    /// `php_userstreamop_write`.
    fn op_write(&mut self, host: &mut dyn Host, data: &[u8]) -> isize {
        let obj = self.obj.clone();
        let ret = match call(host, &obj, "stream_write", &[Value::str(data.to_vec())]) {
            Call::Missing => {
                not_implemented(host, &self.class(), "stream_write", " is not implemented!");
                return -1;
            }
            Call::Undef => return -1,
            Call::Ret(v) => v,
        };
        let mut didwrite = if matches!(ret, Value::Bool(false)) {
            -1
        } else {
            get_long(host, &ret) as isize
        };
        let count = data.len() as isize;
        if didwrite > 0 && didwrite > count {
            let m = format!(
                "{}::stream_write wrote {} bytes more data than requested ({} written, {} max)",
                String::from_utf8_lossy(&self.class()),
                didwrite - count,
                didwrite,
                count
            );
            docref(host, E_WARNING, m.as_bytes());
            didwrite = count;
        }
        didwrite
    }

    /// `php_userstreamop_flush`: 0 or -1.
    fn op_flush(&mut self, host: &mut dyn Host) -> i32 {
        if self.dir {
            return 0;
        }
        let obj = self.obj.clone();
        match call(host, &obj, "stream_flush", &[]) {
            Call::Ret(v) if is_true(host, &v) => 0,
            _ => -1,
        }
    }

    /// `php_userstreamop_seek` (or `rewinddir`): 0 or -1, and the new
    /// offset.
    fn op_seek(&mut self, host: &mut dyn Host, offset: i64, whence: i32) -> (i32, Option<i64>) {
        let obj = self.obj.clone();
        if self.dir {
            let _ = call(host, &obj, "dir_rewinddir", &[]);
            return (0, None);
        }
        let args = [Value::Int(offset), Value::Int(whence as i64)];
        match call(host, &obj, "stream_seek", &args) {
            Call::Missing => {
                self.no_seek = true;
                return (-1, None);
            }
            Call::Ret(v) if is_true(host, &v) => {}
            _ => return (-1, None),
        }
        match call(host, &obj, "stream_tell", &[]) {
            Call::Ret(Value::Int(n)) => (0, Some(n)),
            Call::Missing => {
                not_implemented(host, &self.class(), "stream_tell", " is not implemented!");
                (-1, None)
            }
            _ => (-1, None),
        }
    }

    /// `php_userstreamop_stat`.
    fn op_stat(&mut self, host: &mut dyn Host) -> Option<StatInfo> {
        if self.dir {
            return None;
        }
        let obj = self.obj.clone();
        match call(host, &obj, "stream_stat", &[]) {
            Call::Missing => {
                not_implemented(host, &self.class(), "stream_stat", " is not implemented!");
                None
            }
            Call::Ret(Value::Array(a)) => Some(statbuf_from_array(host, &a)),
            _ => None,
        }
    }

    /// `php_userstreamop_set_option`.
    fn op_set_option(
        &mut self,
        host: &mut dyn Host,
        option: i64,
        value: i64,
        ptr: Option<i64>,
    ) -> i32 {
        if self.dir {
            return OPTION_RETURN_NOTIMPL;
        }
        let obj = self.obj.clone();
        let class = self.class();
        match option {
            OPTION_CHECK_LIVENESS => match call(host, &obj, "stream_eof", &[]) {
                Call::Missing => {
                    not_implemented(
                        host,
                        &class,
                        "stream_eof",
                        " is not implemented! Assuming EOF",
                    );
                    OPTION_RETURN_ERR
                }
                Call::Undef => OPTION_RETURN_ERR,
                Call::Ret(Value::Bool(true)) => OPTION_RETURN_ERR,
                Call::Ret(Value::Bool(false)) => OPTION_RETURN_OK,
                Call::Ret(v) => {
                    let m = format!(
                        "{}::stream_eof value must be of type bool, {} given",
                        String::from_utf8_lossy(&class),
                        zpp::zval_value_name(host, &v)
                    );
                    docref(host, E_WARNING, m.as_bytes());
                    OPTION_RETURN_ERR
                }
            },
            OPTION_LOCKING => {
                // flock's LOCK_* back to PHP_LOCK_*
                let mut lock = 0;
                if value & libc::LOCK_NB as i64 != 0 {
                    lock |= 4;
                }
                match value & !(libc::LOCK_NB as i64) {
                    v if v == libc::LOCK_SH as i64 => lock |= 1,
                    v if v == libc::LOCK_EX as i64 => lock |= 2,
                    v if v == libc::LOCK_UN as i64 => lock |= 3,
                    _ => {}
                }
                match call(host, &obj, "stream_lock", &[Value::Int(lock)]) {
                    Call::Missing => {
                        if value == 0 {
                            return OPTION_RETURN_OK;
                        }
                        not_implemented(host, &class, "stream_lock", " is not implemented!");
                        OPTION_RETURN_ERR
                    }
                    Call::Undef => OPTION_RETURN_ERR,
                    Call::Ret(Value::Bool(b)) => (!b) as i32,
                    Call::Ret(_) => OPTION_RETURN_NOTIMPL,
                }
            }
            OPTION_TRUNCATE_API => {
                if value == TRUNCATE_SUPPORTED {
                    let mut pair = PhpArray::new();
                    pair.push(obj.clone());
                    pair.push(Value::str("stream_truncate"));
                    return if host.is_callable_value(&Value::Array(pair)) {
                        OPTION_RETURN_OK
                    } else {
                        OPTION_RETURN_ERR
                    };
                }
                let size = ptr.unwrap_or(0);
                if size < 0 {
                    return OPTION_RETURN_ERR;
                }
                match call(host, &obj, "stream_truncate", &[Value::Int(size)]) {
                    Call::Missing => {
                        not_implemented(host, &class, "stream_truncate", " is not implemented!");
                        OPTION_RETURN_ERR
                    }
                    Call::Undef => OPTION_RETURN_ERR,
                    Call::Ret(Value::Bool(true)) => OPTION_RETURN_OK,
                    Call::Ret(Value::Bool(false)) => OPTION_RETURN_ERR,
                    Call::Ret(v) => {
                        let m = format!(
                            "{}::stream_truncate value must be of type bool, {} given",
                            String::from_utf8_lossy(&class),
                            zpp::zval_value_name(host, &v)
                        );
                        docref(host, E_WARNING, m.as_bytes());
                        OPTION_RETURN_ERR
                    }
                }
            }
            OPTION_READ_BUFFER | OPTION_WRITE_BUFFER | OPTION_READ_TIMEOUT | OPTION_BLOCKING => {
                let arg2 = match option {
                    OPTION_READ_TIMEOUT => Value::Int(ptr.unwrap_or(0)),
                    OPTION_READ_BUFFER | OPTION_WRITE_BUFFER => {
                        Value::Int(ptr.unwrap_or(libc::BUFSIZ as i64))
                    }
                    _ => Value::Null,
                };
                let args = [Value::Int(option), Value::Int(value), arg2];
                match call(host, &obj, "stream_set_option", &args) {
                    Call::Missing => {
                        not_implemented(host, &class, "stream_set_option", " is not implemented!");
                        OPTION_RETURN_ERR
                    }
                    Call::Undef => OPTION_RETURN_ERR,
                    Call::Ret(v) => {
                        if is_true(host, &v) {
                            OPTION_RETURN_OK
                        } else {
                            OPTION_RETURN_ERR
                        }
                    }
                }
            }
            _ => OPTION_RETURN_NOTIMPL,
        }
    }

    /// `php_userstreamop_cast` up to the returned stream: `Some(res)` is
    /// the stream to cast in its place. `report` is false when php only
    /// asks whether the stream can be cast.
    pub fn cast(
        &mut self,
        host: &mut dyn Host,
        castas: i64,
        report: bool,
        me: u32,
    ) -> Option<Value> {
        if self.dir {
            return None;
        }
        let obj = self.obj.clone();
        let class = self.class();
        let arg = if castas == CAST_FOR_SELECT {
            CAST_FOR_SELECT
        } else {
            CAST_AS_STDIO
        };
        let ret = match call(host, &obj, "stream_cast", &[Value::Int(arg)]) {
            Call::Missing => {
                if report {
                    not_implemented(host, &class, "stream_cast", " is not implemented!");
                }
                return None;
            }
            Call::Undef => return None,
            Call::Ret(v) => v,
        };
        if !is_true(host, &ret) {
            return None;
        }
        let is_stream = matches!(&ret, Value::Resource(_)) && host.stream_is_open(&ret);
        if !is_stream {
            if report {
                not_implemented(
                    host,
                    &class,
                    "stream_cast",
                    " must return a stream resource",
                );
            }
            return None;
        }
        if matches!(&ret, Value::Resource(r) if r.id == me) {
            if report {
                not_implemented(host, &class, "stream_cast", " must not return itself");
            }
            return None;
        }
        Some(ret)
    }

    /// `php_userstreamop_close` / `closedir`.
    fn op_close(&mut self, host: &mut dyn Host) {
        let obj = std::mem::replace(&mut self.obj, Value::Null);
        let name = if self.dir {
            "dir_closedir"
        } else {
            "stream_close"
        };
        let _ = call(host, &obj, name, &[]);
    }

    // --- the generic layer (main/streams/streams.c) ---

    /// `_php_stream_fill_read_buffer` (no filters).
    fn fill_read_buffer(&mut self, host: &mut dyn Host, size: usize) -> bool {
        if self.writepos - self.readpos < size {
            if !self.readbuf.is_empty() && self.readbuf.len() - self.writepos < self.chunk_size {
                if self.writepos > self.readpos {
                    self.readbuf.copy_within(self.readpos..self.writepos, 0);
                }
                self.writepos -= self.readpos;
                self.readpos = 0;
            }
            if self.readbuf.len() - self.writepos < self.chunk_size {
                let n = self.readbuf.len() + self.chunk_size;
                self.readbuf.resize(n, 0);
            }
            let room = self.readbuf.len() - self.writepos;
            let (got, data) = self.op_read(host, room);
            if got < 0 {
                return false;
            }
            let wp = self.writepos;
            self.readbuf[wp..wp + data.len()].copy_from_slice(&data);
            self.writepos += data.len();
        }
        true
    }

    /// `_php_stream_read`: `None` is a failed read with nothing read.
    pub fn read(&mut self, host: &mut dyn Host, size: usize) -> Option<Vec<u8>> {
        let mut out: Vec<u8> = Vec::new();
        let mut size = size;
        if self.writepos > self.readpos {
            let n = (self.writepos - self.readpos).min(size);
            out.extend_from_slice(&self.readbuf[self.readpos..self.readpos + n]);
            self.readpos += n;
            size -= n;
        }
        // not greedy: a user stream reads at most once
        if size > 0 {
            if self.no_buffer || self.chunk_size == 1 {
                let (got, data) = self.op_read(host, size);
                if got >= 0 {
                    out.extend_from_slice(&data);
                } else if out.is_empty() {
                    return None;
                }
            } else if self.fill_read_buffer(host, size) {
                let toread = (self.writepos - self.readpos).min(size);
                out.extend_from_slice(&self.readbuf[self.readpos..self.readpos + toread]);
                self.readpos += toread;
            } else if out.is_empty() {
                return None;
            }
        }
        self.position += out.len() as i64;
        Some(out)
    }

    /// `_php_stream_eof`.
    pub fn stream_eof(&mut self, host: &mut dyn Host) -> bool {
        if self.writepos > self.readpos {
            return false;
        }
        if !self.eof && self.set_option(host, OPTION_CHECK_LIVENESS, 0, None) == OPTION_RETURN_ERR {
            self.eof = true;
        }
        self.eof
    }

    /// `_php_stream_get_line`: `maxlen` is php's buffer size (a line
    /// gets at most `maxlen - 1` bytes); `None` grows to the line's end.
    pub fn get_line(&mut self, host: &mut dyn Host, maxlen: Option<usize>) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut maxlen = maxlen;
        if maxlen == Some(0) {
            return None;
        }
        loop {
            let avail = self.writepos - self.readpos;
            if avail > 0 {
                let chunk = &self.readbuf[self.readpos..self.writepos];
                let (mut cpysz, mut done) = match chunk.iter().position(|&b| b == b'\n') {
                    Some(i) => (i + 1, true),
                    None => (avail, false),
                };
                if let Some(m) = maxlen {
                    if cpysz >= m - 1 {
                        cpysz = m - 1;
                        done = true;
                    }
                }
                out.extend_from_slice(&chunk[..cpysz]);
                self.position += cpysz as i64;
                self.readpos += cpysz;
                if let Some(m) = maxlen.as_mut() {
                    *m -= cpysz;
                }
                if done {
                    break;
                }
            } else if self.eof {
                break;
            } else {
                let toread = match maxlen {
                    None => self.chunk_size,
                    Some(m) => (m - 1).min(self.chunk_size),
                };
                self.fill_read_buffer(host, toread);
                if self.writepos == self.readpos {
                    break;
                }
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// `php_stream_get_record` (`stream_get_line`).
    pub fn get_record(
        &mut self,
        host: &mut dyn Host,
        maxlen: usize,
        delim: &[u8],
    ) -> Option<Vec<u8>> {
        if maxlen == 0 {
            return None;
        }
        let search = |s: &UserStream, skip: usize| -> Option<usize> {
            let seek_len = (s.writepos - s.readpos).min(maxlen);
            if seek_len <= skip {
                return None;
            }
            let hay = &s.readbuf[s.readpos + skip..s.readpos + seek_len];
            super::find(hay, delim).map(|i| i + skip)
        };
        let has_delim = !delim.is_empty();
        let mut found = if has_delim { search(self, 0) } else { None };
        let mut buffered = self.writepos - self.readpos;
        while found.is_none() && buffered < maxlen {
            let to_read_now = (maxlen - buffered).min(self.chunk_size);
            self.fill_read_buffer(host, buffered + to_read_now);
            let just_read = (self.writepos - self.readpos) - buffered;
            if just_read == 0 {
                break;
            }
            if has_delim {
                let skip = buffered.saturating_sub(delim.len() - 1);
                found = search(self, skip);
                if found.is_some() {
                    break;
                }
            }
            buffered += just_read;
        }
        let amount = self.writepos - self.readpos;
        let len = if let Some(f) = found {
            f
        } else if !has_delim && amount >= maxlen {
            maxlen
        } else if (amount < maxlen && !self.eof) || (amount == 0 && self.eof) {
            return None;
        } else {
            amount.min(maxlen)
        };
        let out = self.read(host, len).unwrap_or_default();
        if found.is_some() {
            self.readpos += delim.len();
            self.position += delim.len() as i64;
        }
        Some(out)
    }

    /// `_php_stream_write` (no filters): the bytes written or -1.
    pub fn write(&mut self, host: &mut dyn Host, data: &[u8]) -> isize {
        if data.is_empty() {
            return 0;
        }
        if self.dir {
            docref(host, crate::E_NOTICE, b"Stream is not writable");
            return -1;
        }
        // a seekable stream writes at stream->position
        if !self.no_seek && self.readpos != self.writepos {
            self.readpos = 0;
            self.writepos = 0;
            let pos = self.position;
            if let (_, Some(p)) = self.op_seek(host, pos, libc::SEEK_SET) {
                self.position = p;
            }
        }
        // GH-13071: a userspace stream writes chunk by chunk
        let mut didwrite: isize = 0;
        let mut rest = data;
        while !rest.is_empty() {
            let n = rest.len().min(self.chunk_size.max(1));
            let just = self.op_write(host, &rest[..n]);
            if just <= 0 {
                if didwrite == 0 {
                    didwrite = just;
                }
                break;
            }
            let just = just as usize;
            rest = &rest[just.min(rest.len())..];
            didwrite += just as isize;
            self.position += just as i64;
        }
        if didwrite != 0 {
            self.was_written = true;
        }
        didwrite
    }

    /// `_php_stream_flush`: 0 or -1.
    pub fn flush(&mut self, host: &mut dyn Host) -> i32 {
        self.was_written = false;
        self.op_flush(host)
    }

    /// `_php_stream_seek`: 0 or -1.
    pub fn seek(&mut self, host: &mut dyn Host, offset: i64, whence: i32) -> i32 {
        if !self.no_buffer {
            let avail = (self.writepos - self.readpos) as i64;
            match whence {
                libc::SEEK_CUR if offset > 0 && offset <= avail => {
                    self.readpos += offset as usize;
                    self.position += offset;
                    self.eof = false;
                    return 0;
                }
                libc::SEEK_SET if offset > self.position && offset <= self.position + avail => {
                    self.readpos += (offset - self.position) as usize;
                    self.position = offset;
                    self.eof = false;
                    return 0;
                }
                _ => {}
            }
        }
        if !self.no_seek {
            let (offset, whence) = match whence {
                libc::SEEK_CUR => (self.position.saturating_add(offset), libc::SEEK_SET),
                libc::SEEK_SET if offset < 0 => return -1,
                _ => (offset, whence),
            };
            let (ret, newpos) = self.op_seek(host, offset, whence);
            if let Some(p) = newpos {
                self.position = p;
            }
            if !self.no_seek || ret == 0 {
                if ret == 0 {
                    self.eof = false;
                }
                self.readpos = 0;
                self.writepos = 0;
                return ret;
            }
        }
        if whence == libc::SEEK_CUR && offset >= 0 {
            let mut left = offset;
            while left > 0 {
                match self.read(host, left.min(1024) as usize) {
                    Some(b) if !b.is_empty() => left -= b.len() as i64,
                    _ => return -1,
                }
            }
            self.eof = false;
            return 0;
        }
        docref(host, E_WARNING, b"Stream does not support seeking");
        -1
    }

    /// `_php_stream_tell`.
    pub fn tell(&self) -> i64 {
        self.position
    }

    /// `_php_stream_stat`.
    pub fn stat(&mut self, host: &mut dyn Host) -> Option<StatInfo> {
        self.op_stat(host)
    }

    /// `_php_stream_set_option`.
    pub fn set_option(
        &mut self,
        host: &mut dyn Host,
        option: i64,
        value: i64,
        ptr: Option<i64>,
    ) -> i32 {
        let ret = self.op_set_option(host, option, value, ptr);
        if ret != OPTION_RETURN_NOTIMPL {
            return ret;
        }
        match option {
            OPTION_SET_CHUNK_SIZE => {
                let old = self.chunk_size.min(i32::MAX as usize) as i32;
                self.chunk_size = value.max(0) as usize;
                old
            }
            OPTION_READ_BUFFER => {
                self.no_buffer = value == BUFFER_NONE;
                OPTION_RETURN_OK
            }
            _ => ret,
        }
    }

    /// `_php_stream_copy_to_mem`: `None` when nothing was read.
    pub fn copy_to_mem(&mut self, host: &mut dyn Host, maxlen: Option<usize>) -> Option<Vec<u8>> {
        if maxlen == Some(0) {
            return Some(Vec::new());
        }
        let mut out = Vec::new();
        if let Some(max) = maxlen.filter(|&m| m < 4 * CHUNK) {
            while out.len() < max && !self.stream_eof(host) {
                match self.read(host, max - out.len()) {
                    Some(b) if !b.is_empty() => out.extend_from_slice(&b),
                    _ => break,
                }
            }
            return (!out.is_empty()).then_some(out);
        }
        let mut buflen = match self.stat(host) {
            Some(s) if s.size > 0 => ((s.size as i64 - self.position).max(0) as usize) + CHUNK,
            _ => CHUNK,
        };
        if let Some(m) = maxlen {
            buflen = buflen.min(m);
        }
        let min_room = CHUNK / 4;
        loop {
            let b = match self.read(host, buflen - out.len()) {
                Some(b) if !b.is_empty() => b,
                _ => break,
            };
            out.extend_from_slice(&b);
            if out.len() + min_room >= buflen {
                if maxlen == Some(out.len()) {
                    break;
                }
                buflen = match maxlen {
                    Some(m) if buflen + CHUNK > m => m,
                    _ => buflen + CHUNK,
                };
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// `php_stream_free`: flush what was written, then the close op.
    pub fn close(&mut self, host: &mut dyn Host) {
        if self.was_written {
            self.flush(host);
        }
        self.op_close(host);
    }

    /// `php_userstreamop_readdir` through `_php_stream_readdir`.
    pub fn readdir(&mut self, host: &mut dyn Host) -> Option<Vec<u8>> {
        let obj = self.obj.clone();
        let ret = match call(host, &obj, "dir_readdir", &[]) {
            Call::Missing => {
                not_implemented(host, &self.class(), "dir_readdir", " is not implemented!");
                return None;
            }
            Call::Undef => return None,
            Call::Ret(v) => v,
        };
        if matches!(ret, Value::Bool(_)) {
            return None;
        }
        let mut name = try_to_string(host, &ret)?;
        // PHP_STRLCPY into d_name[MAXPATHLEN]
        name.truncate(libc::PATH_MAX as usize - 1);
        if let Some(z) = name.iter().position(|&b| b == 0) {
            name.truncate(z);
        }
        self.position += 1;
        Some(name)
    }

    /// `rewinddir`: the dir ops' seek.
    pub fn rewinddir(&mut self, host: &mut dyn Host) {
        let _ = self.op_seek(host, 0, libc::SEEK_SET);
        self.position = 0;
        self.eof = false;
    }

    /// After `_php_stream_open_wrapper_ex` opened a stream for append:
    /// revise the position from the stream.
    pub fn opened_for_append(&mut self, host: &mut dyn Host) {
        if !self.no_seek && self.mode.contains(&b'a') && self.position == 0 {
            if let (0, Some(p)) = self.op_seek(host, 0, libc::SEEK_CUR) {
                self.position = p;
            }
        }
    }
}

/// Run `f` on the user stream behind `res` (none when it is not one).
pub fn with<R>(
    host: &mut dyn Host,
    res: &Value,
    f: impl FnOnce(&mut UserStream, &mut dyn Host) -> R,
) -> Option<R> {
    let mut s = host.user_stream_take(res)?;
    let r = f(&mut s, host);
    host.user_stream_put(res, s);
    Some(r)
}

/// `php_stream_set_option` on a user stream; `None` for other streams.
pub fn set_option(
    host: &mut dyn Host,
    res: &Value,
    option: i64,
    value: i64,
    ptr: Option<i64>,
) -> Option<i32> {
    with(host, res, |s, host| s.set_option(host, option, value, ptr))
}

// --- the wrapper-table functions -------------------------------------------------

/// `stream_wrapper_register` / `stream_register_wrapper`.
fn fn_stream_wrapper_register(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = host.active_function_name();
    let fname = fname.as_str();
    let protocol = zpp::str_arg(host, args, 0, fname, "protocol")?;
    let class = zpp::str_arg(host, args, 1, fname, "class")?;
    let flags = match args.get(2).map(rhp_values::ops::deref) {
        None => 0,
        Some(v) => conv::to_int(&v),
    };
    let cid = match host.resolve_class_autoload(&class)? {
        Some(c) => c,
        None => {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #2 ($class) must be a valid class name, {} given",
                String::from_utf8_lossy(&class)
            )))
        }
    };
    let name = host.class_name(cid).unwrap_or(class);
    let w = Rc::new(UserWrapper {
        class: name.clone(),
        is_url: flags & IS_URL != 0,
    });
    // the wrapper is a resource (le_protocols), freed again on failure
    host.consume_resource_ids(1);
    let valid = protocol
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
    if valid && host.register_stream_wrapper(&protocol, w) {
        return Ok(Value::Bool(true));
    }
    let mut m = Vec::new();
    if valid {
        m.extend_from_slice(b"Protocol ");
        m.extend_from_slice(&protocol);
        m.extend_from_slice(b":// is already defined.");
    } else {
        m.extend_from_slice(
            b"Invalid protocol scheme specified. Unable to register wrapper class ",
        );
        m.extend_from_slice(&name);
        m.extend_from_slice(b" to ");
        m.extend_from_slice(&protocol);
        m.extend_from_slice(b"://");
    }
    diag::warn(host, fname, &m)?;
    Ok(Value::Bool(false))
}

fn fn_stream_wrapper_unregister(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_wrapper_unregister";
    let protocol = zpp::str_arg(host, args, 0, fname, "protocol")?;
    if host.stream_wrapper_unregister(&protocol) {
        return Ok(Value::Bool(true));
    }
    let mut m = b"Unable to unregister protocol ".to_vec();
    m.extend_from_slice(&protocol);
    m.extend_from_slice(b"://");
    diag::warn(host, fname, &m)?;
    Ok(Value::Bool(false))
}

fn fn_stream_wrapper_restore(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_wrapper_restore";
    let protocol = zpp::str_arg(host, args, 0, fname, "protocol")?;
    let mut m = protocol.clone();
    Ok(Value::Bool(match host.stream_wrapper_restore(&protocol) {
        WrapperRestore::Restored => true,
        WrapperRestore::Unchanged => {
            m.extend_from_slice(b":// was never changed, nothing to restore");
            diag::notice(host, fname, &m)?;
            true
        }
        WrapperRestore::NeverExisted => {
            m.extend_from_slice(b":// never existed, nothing to restore");
            diag::warn(host, fname, &m)?;
            false
        }
    }))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("stream_wrapper_register", fn_stream_wrapper_register),
            ("stream_register_wrapper", fn_stream_wrapper_register),
            ("stream_wrapper_unregister", fn_stream_wrapper_unregister),
            ("stream_wrapper_restore", fn_stream_wrapper_restore),
        ],
    );
}
