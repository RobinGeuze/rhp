//! ext/standard/file.c: `fopen` and the per-handle functions (`fread`,
//! `fgets`, `flock`, `fseek`, ...), whole-file reads and writes (`file`,
//! `file_get_contents`, `file_put_contents`, `readfile`), `mkdir`/`unlink`/
//! `rename`/`copy`, temporary files, CSV, `popen` and `fnmatch`.

use std::ffi::CString;

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, Value};

use crate::ext::standard::filestat::arr_from_stat;
use crate::ext::standard::ftp_fopen_wrapper as ftp;
use crate::ext::standard::proc_open::{zpp_path, SPAWN_LOCK};
use crate::main::php_glob::{
    GLOB_AVAILABLE, GLOB_BRACE, GLOB_ERR, GLOB_MARK, GLOB_NOCHECK, GLOB_NOESCAPE, GLOB_NOSORT,
    GLOB_ONLYDIR,
};
use crate::main::streams::split_scheme;
use crate::zend::zpp::{int_param, str_param};
use crate::{Host, StreamKind, Thrown};

pub(crate) fn str_arg(args: &[Value], i: usize) -> Vec<u8> {
    args.get(i)
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_default()
}

pub(crate) fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => default,
        Some(v) => conv::to_int(&v),
    }
}

/// A `Z_PARAM_PATH` argument: a string without NUL bytes.
pub(crate) fn path_param(
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    let p = str_arg(args, i);
    if p.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #{} (${pname}) must not contain any null bytes",
                i + 1
            ),
        ));
    }
    Ok(p)
}

/// A `resource $stream` argument that must be a live stream.
pub(crate) fn stream_param(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<(Value, StreamKind), Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    if !matches!(v, Value::Resource(_)) {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #{} (${pname}) must be of type resource, {} given",
            i + 1,
            crate::zend::zpp::zval_value_name(host, &v)
        )));
    }
    match host.stream_kind(&v) {
        Some(k) => Ok((v, k)),
        None => Err(Thrown::type_error(format!(
            "{fname}(): Argument #{} (${pname}) must be an open stream resource",
            i + 1
        ))),
    }
}

/// php's `php_stream_context_from_zval(zcontext, 0)`: with no explicit
/// context argument the default context is fetched (allocating its resource
/// on first use): the context argument at `idx`, else the default context.
pub(crate) fn ctx_or_default(host: &mut dyn Host, args: &[Value], idx: usize) -> Value {
    match args.get(idx).map(deref) {
        Some(v @ Value::Resource(_)) => v,
        _ => host.default_stream_context(),
    }
}

pub(crate) fn touch_ctx(host: &mut dyn Host, args: &[Value], idx: usize) {
    if matches!(args.get(idx).map(deref), None | Some(Value::Null)) {
        host.default_stream_context();
    }
}

/// An http(s) or ftp(s) URL a built-in wrapper serves (no user wrapper
/// replaced it).
fn builtin_http(host: &dyn Host, path: &[u8]) -> bool {
    crate::ext::standard::http_fopen_wrapper::is_url_stream(path)
        && !(host.has_user_wrappers()
            && crate::main::streams::userspace::locate(host, path).is_some())
}

fn fn_fopen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "fopen", "filename")?;
    let mode = str_arg(args, 1);
    if path.is_empty() {
        return Err(Thrown::error("ValueError", "Path must not be empty"));
    }
    touch_ctx(host, args, 3);
    if builtin_http(host, &path) {
        return Ok(crate::ext::standard::http_fopen_wrapper::open_arg(
            host,
            "fopen",
            &path,
            &mode,
            args.get(3),
        )?
        .unwrap_or(Value::Bool(false)));
    }
    crate::main::streams::clear_errno();
    let ctx = args.get(3).map(deref).unwrap_or(Value::Null);
    let use_path = args.get(2).is_some_and(|v| conv::to_bool(&deref(v)));
    let opened = if use_path && split_scheme(&path).0 == b"file" {
        let resolved = include_path_target(host, &path);
        let Some(target) = crate::locate_for_open(host, "fopen", &resolved)? else {
            return Ok(Value::Bool(false));
        };
        host.stream_open_ctx(&target, &mode, &ctx)
    } else {
        let Some(target) = crate::locate_for_open(host, "fopen", &path)? else {
            return Ok(Value::Bool(false));
        };
        host.stream_open_ctx(&target, &mode, &ctx)
    };
    match opened {
        Some(v) => Ok(v),
        None => {
            if crate::main::streams::split_scheme(&path).0 == b"file"
                && crate::main::streams::parse_fopen_mode(&mode).is_none()
            {
                crate::warn_arg(
                    host,
                    "fopen",
                    &path,
                    &[
                        b"Failed to open stream: `".as_slice(),
                        &mode,
                        b"' is not a valid mode for fopen",
                    ]
                    .concat(),
                )?;
            } else {
                crate::open_failed(host, "fopen", &path)?;
            }
            Ok(Value::Bool(false))
        }
    }
}

/// `php_stream_open_wrapper_ex(path, mode, [USE_PATH |] REPORT_ERRORS)`
/// from builtin `fname`: with USE_PATH a plain file opens at its
/// php_resolve_path result when there is one; failures are reported.
pub(crate) fn open_stream(
    host: &mut dyn Host,
    fname: &str,
    path: &[u8],
    mode: &[u8],
    ctx: &Value,
    use_path: bool,
) -> Result<Option<Value>, Thrown> {
    let path_to_open = if use_path {
        include_path_target(host, path)
    } else {
        path.to_vec()
    };
    let Some(target) = crate::locate_for_open(host, fname, &path_to_open)? else {
        return Ok(None);
    };
    crate::main::streams::clear_errno();
    match host.stream_open_ctx(&target, mode, ctx) {
        Some(v) => Ok(Some(v)),
        None => {
            crate::open_failed(host, fname, path)?;
            Ok(None)
        }
    }
}

// --- fsync() / fdatasync() ------------------------------------------------------

/// The descriptor behind a plain-file or fd stream (the streams whose ops
/// support SYNC and LOCKING); `None` for sockets, memory and wrapper
/// streams.
pub(crate) fn plain_fd(host: &dyn Host, h: &Value) -> Option<i32> {
    if host.stream_sock_info(h).is_some() {
        return None;
    }
    host.stream_fd(h)
}

/// `fsync` / `fdatasync`: flush, then sync the descriptor (macOS has no
/// fdatasync; php uses fsync there too).
fn stream_sync(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, fname, "stream")?;
    let Some(fd) = plain_fd(host, &h) else {
        crate::warn(host, fname, "Can't fsync this stream!")?;
        return Ok(Value::Bool(false));
    };
    host.stream_flush(&h);
    Ok(Value::Bool(unsafe { libc::fsync(fd) } == 0))
}

fn fn_fsync(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    stream_sync(host, args, "fsync")
}

fn fn_fdatasync(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    stream_sync(host, args, "fdatasync")
}

// --- fscanf() ------------------------------------------------------------------

/// `fscanf`: one line read like `php_stream_get_line`, then the sscanf
/// engine.
fn fn_fscanf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    if !matches!(v, Value::Resource(_)) {
        return Err(Thrown::type_error(format!(
            "fscanf(): Argument #1 ($stream) must be of type resource, {} given",
            crate::zend::zpp::zval_value_name(host, &v)
        )));
    }
    if host.stream_kind(&v).is_none() {
        return Err(Thrown::type_error(
            "fscanf(): supplied resource is not a valid File-Handle resource",
        ));
    }
    let format = str_arg(args, 1);
    let Some(line) = host.stream_gets(&v) else {
        return Ok(Value::Bool(false));
    };
    crate::ext::standard::scanf::php_sscanf_internal(&line, &format, &args[2.min(args.len())..])
}

// --- file() --------------------------------------------------------------------

fn fn_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "file", "filename")?;
    let flags = int_arg(args, 1, 0);
    const USE_INCLUDE_PATH: i64 = 1;
    const IGNORE_NEW_LINES: i64 = 2;
    const SKIP_EMPTY_LINES: i64 = 4;
    const NO_DEFAULT_CONTEXT: i64 = 16;
    if flags & !(USE_INCLUDE_PATH | IGNORE_NEW_LINES | SKIP_EMPTY_LINES | NO_DEFAULT_CONTEXT) != 0 {
        return Err(Thrown::error(
            "ValueError",
            "file(): Argument #2 ($flags) must be a valid flag value",
        ));
    }
    if flags & NO_DEFAULT_CONTEXT == 0 {
        touch_ctx(host, args, 2);
    }
    let include_new_line = flags & IGNORE_NEW_LINES == 0;
    let skip_blank = flags & SKIP_EMPTY_LINES != 0;
    crate::main::streams::clear_errno();
    let mut opened = builtin_http(host, &path);
    let data = if opened {
        match crate::ext::standard::http_fopen_wrapper::get_contents(
            host,
            "file",
            &path,
            args.get(2),
            0,
            None,
        )? {
            Some(b) => Some(b),
            None => return Ok(Value::Bool(false)),
        }
    } else if let Some(r) = read_via_opener(host, "file", &path, args.get(2))? {
        opened = true;
        match r {
            Some(b) => Some(b),
            None => return Ok(Value::Bool(false)),
        }
    } else {
        let target = if flags & USE_INCLUDE_PATH != 0 {
            include_path_target(host, &path)
        } else {
            path.clone()
        };
        let Some(target) = crate::locate_for_open(host, "file", &target)? else {
            return Ok(Value::Bool(false));
        };
        host.stream_get_contents(&target)
    };
    let Some(buf) = data else {
        if crate::main::streams::last_errno() == Some(libc::EISDIR) {
            host.consume_resource_ids(1);
            crate::notice(
                host,
                "file",
                "Read of 8192 bytes failed with errno=21 Is a directory",
            )?;
            return Ok(Value::Array(PhpArray::new()));
        }
        crate::open_failed(host, "file", &path)?;
        return Ok(Value::Bool(false));
    };
    if !opened {
        host.consume_resource_ids(crate::main::streams::open_resource_ids(&path));
    }
    let mut a = PhpArray::new();
    if buf.is_empty() {
        return Ok(Value::Array(a));
    }
    let e = buf.len();
    let mut s = 0usize;
    let find = |from: usize| {
        buf[from..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| from + i)
    };
    match find(0) {
        None => {
            a.push(Value::str(buf.clone()));
            return Ok(Value::Array(a));
        }
        Some(first) => {
            let mut p = first;
            if include_new_line {
                loop {
                    p += 1;
                    a.push(Value::str(&buf[s..p]));
                    s = p;
                    match find(p) {
                        Some(n) => p = n,
                        None => break,
                    }
                }
            } else {
                loop {
                    let windows_eol = usize::from(p != 0 && buf[p - 1] == b'\r');
                    if skip_blank && p - s - windows_eol == 0 {
                        p += 1;
                        s = p;
                    } else {
                        a.push(Value::str(&buf[s..p - windows_eol]));
                        p += 1;
                        s = p;
                    }
                    match find(p) {
                        Some(n) => p = n,
                        None => break,
                    }
                }
            }
        }
    }
    if s != e {
        a.push(Value::str(&buf[s..e]));
    }
    Ok(Value::Array(a))
}

// --- descriptor ops on open streams ---------------------------------------------

fn fn_fstat(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fstat", "stream")?;
    Ok(host
        .stream_fstat(&h)
        .map(|s| arr_from_stat(&s))
        .unwrap_or(Value::Bool(false)))
}

fn fn_flock(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "flock", "stream")?;
    let op = int_arg(args, 1, 0);
    let act = op & 3;
    if !(1..=3).contains(&act) {
        return Err(Thrown::error(
            "ValueError",
            "flock(): Argument #2 ($operation) must be one of LOCK_SH, LOCK_EX, or LOCK_UN",
        ));
    }
    let wb = args.get(2).and_then(|v| match v {
        Value::Ref(r) => Some(r.clone()),
        _ => None,
    });
    if let Some(r) = &wb {
        *r.borrow_mut() = Value::Int(0);
    }
    if host.has_user_wrappers() {
        let mut lop = [libc::LOCK_SH, libc::LOCK_EX, libc::LOCK_UN][act as usize - 1] as i64;
        if op & 4 != 0 {
            lop |= libc::LOCK_NB as i64;
        }
        let us = crate::main::streams::userspace::OPTION_LOCKING;
        if let Some(r) = crate::main::streams::userspace::set_option(host, &h, us, lop, None) {
            return Ok(Value::Bool(r == 0));
        }
    }
    // Socket streams do not implement php's LOCKING option.
    if host.stream_sock_info(&h).is_some() {
        return Ok(Value::Bool(false));
    }
    let Some(fd) = host.stream_fd(&h) else {
        return Ok(Value::Bool(false));
    };
    let mut lop = match act {
        1 => libc::LOCK_SH,
        2 => libc::LOCK_EX,
        _ => libc::LOCK_UN,
    };
    if op & 4 != 0 {
        lop |= libc::LOCK_NB;
    }
    let r = unsafe { libc::flock(fd, lop) };
    if r != 0 {
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if errno == libc::EWOULDBLOCK {
            if let Some(r) = &wb {
                *r.borrow_mut() = Value::Int(1);
            }
        }
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_ftruncate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, kind) = stream_param(host, args, 0, "ftruncate", "stream")?;
    let size = int_arg(args, 1, 0);
    if size < 0 {
        return Err(Thrown::error(
            "ValueError",
            "ftruncate(): Argument #2 ($size) must be greater than or equal to 0",
        ));
    }
    // Socket streams have no truncate support (php's TRUNCATE_SUPPORTED probe).
    if host.stream_sock_info(&h).is_some() {
        crate::warn(host, "ftruncate", "Can't truncate this stream!")?;
        return Ok(Value::Bool(false));
    }
    match host.stream_truncate(&h, size) {
        Some(r) => Ok(Value::Bool(r)),
        // Nor do the wrapper and extension streams without a set_option
        // (zip entries).
        None if kind == StreamKind::Memory => {
            crate::warn(host, "ftruncate", "Can't truncate this stream!")?;
            Ok(Value::Bool(false))
        }
        None => Ok(Value::Bool(false)),
    }
}

fn fn_fseek(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fseek", "stream")?;
    let off = int_arg(args, 1, 0);
    let whence = int_arg(args, 2, 0) as i32;
    let ok = stream_seek(host, "fseek", &h, off, whence)?;
    Ok(Value::Int(if ok { 0 } else { -1 }))
}

/// `_php_stream_seek` for builtin `fname`: php emulates a forward relative
/// seek on an unseekable stream by reading (failing quietly at eof);
/// anything else it cannot do warns.
pub(crate) fn stream_seek(
    host: &mut dyn Host,
    fname: &str,
    h: &Value,
    off: i64,
    whence: i32,
) -> Result<bool, Thrown> {
    if host.stream_seek(h, off, whence) {
        return Ok(true);
    }
    let forward = whence == libc::SEEK_CUR && off >= 0;
    if !forward && host.stream_meta(h).is_some_and(|m| !m.seekable) {
        crate::warn(host, fname, "Stream does not support seeking")?;
    }
    Ok(false)
}

fn fn_ftell(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "ftell", "stream")?;
    Ok(host
        .stream_tell(&h)
        .map(Value::Int)
        .unwrap_or(Value::Bool(false)))
}

fn fn_rewind(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "rewind", "stream")?;
    if host.stream_rewind(&h) {
        return Ok(Value::Bool(true));
    }
    // php's seek of an unseekable stream (pipe, socket) warns.
    if host.stream_meta(&h).is_some_and(|m| !m.seekable) {
        crate::warn(host, "rewind", "Stream does not support seeking")?;
    }
    Ok(Value::Bool(false))
}

fn fn_fgets(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fgets", "stream")?;
    let line = match args.get(1).map(deref) {
        None | Some(Value::Null) => host.stream_gets(&h),
        Some(v) => {
            let len = conv::to_int(&v);
            if len <= 0 {
                return Err(Thrown::error(
                    "ValueError",
                    "fgets(): Argument #2 ($length) must be greater than 0",
                ));
            }
            if len == 1 {
                // php: a zero-byte read returns false? no — it returns "" only
                // when not at eof; match php_stream_get_line's maxlen=0 path.
                return Ok(Value::Bool(false));
            }
            host.stream_gets_max(&h, (len - 1) as usize)
        }
    };
    Ok(line.map(Value::str).unwrap_or(Value::Bool(false)))
}

fn fn_fread(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fread", "stream")?;
    let len = int_arg(args, 1, 0);
    if len <= 0 {
        return Err(Thrown::error(
            "ValueError",
            "fread(): Argument #2 ($length) must be greater than 0",
        ));
    }
    // php_stream_read_to_str allocates `len` bytes up front, fallibly.
    if !rhp_values::mem::reserve_str(len as usize)
        || !crate::ext::standard::streamsfuncs::reserve_read_chunk(host, &h)
    {
        return Ok(Value::Null);
    }
    Ok(host
        .stream_read(&h, len)
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn fn_fwrite(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fwrite", "stream")?;
    // Borrow a string argument rather than copying it: a large write to a
    // non-blocking socket should reach send() as promptly as php's does.
    let arg = args.get(1).map(deref).unwrap_or(Value::Null);
    let owned;
    let mut data: &[u8] = match &arg {
        Value::Str(s) => s.as_bytes(),
        v => {
            owned = conv::to_string_bytes(v);
            &owned
        }
    };
    match args.get(2).map(deref) {
        None | Some(Value::Null) => {}
        Some(v) => {
            let n = conv::to_int(&v);
            if n <= 0 {
                return Ok(Value::Int(0));
            }
            if (n as usize) < data.len() {
                data = &data[..n as usize];
            }
        }
    }
    if data.is_empty() {
        return Ok(Value::Int(0));
    }
    Ok(host
        .stream_write(&h, data)
        .map(|n| Value::Int(n as i64))
        .unwrap_or(Value::Bool(false)))
}

fn fn_fclose(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, kind) = stream_param(host, args, 0, "fclose", "stream")?;
    if kind == StreamKind::Dir {
        // php: "cannot close the provided stream, as it must not be manually
        // closed" (directory handles are closed with closedir()).
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(host.stream_close(&h)))
}

fn fn_feof(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "feof", "stream")?;
    Ok(Value::Bool(host.stream_eof(&h)))
}

fn fn_fflush(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fflush", "stream")?;
    Ok(Value::Bool(host.stream_flush(&h)))
}

fn fn_fgetc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fgetc", "stream")?;
    match host.stream_read(&h, 1) {
        Some(b) if !b.is_empty() => Ok(Value::str(b)),
        _ => Ok(Value::Bool(false)),
    }
}

// --- mkdir / copy ------------------------------------------------------------------

#[allow(clippy::unnecessary_cast)] // mode_t is u16 on macOS, u32 on Linux
fn sys_mkdir(path: &[u8], mode: i64) -> Result<(), i32> {
    crate::zend::virtual_cwd::mkdir(path, mode as libc::mode_t as u32)
        .map_err(|e| e.raw_os_error().unwrap_or(libc::EINVAL))
}

/// `mkdir($directory, $permissions = 0777, $recursive = false)` — php's
/// plain-files mkdir: the mode is honoured (minus umask); a recursive mkdir
/// creates each missing component and fails if the leaf already exists.
fn fn_mkdir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "mkdir", "directory")?;
    let mode = int_arg(args, 1, 0o777);
    let recursive = args
        .get(2)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    touch_ctx(host, args, 3);
    if let Some((w, _)) = crate::main::streams::userspace::locate(host, &path) {
        let ctx = ctx_or_default(host, args, 3);
        let us = crate::main::streams::userspace::REPORT_ERRORS | recursive as i64;
        let ok = crate::main::streams::userspace::mkdir(host, &w, &path, mode, us, Some(&ctx));
        return Ok(Value::Bool(ok));
    } else if ftp::is_ftp(&path) {
        let ctx = ctx_or_default(host, args, 3);
        return Ok(Value::Bool(ftp::mkdir(
            host, "mkdir", &path, recursive, &ctx,
        )?));
    }
    let (scheme, _) = split_scheme(&path);
    if scheme != b"file" {
        return Ok(Value::Bool(false));
    }
    let local = crate::main::streams::file_path(&path).to_vec();
    if crate::main::fopen_wrappers::active() {
        // a recursive mkdir checks the lexical expansion
        let checked = if recursive {
            crate::zend::virtual_cwd::expand_path(strip_file_scheme(&path))
        } else {
            strip_file_scheme(&path).to_vec()
        };
        if !crate::main::fopen_wrappers::check(host, "mkdir", &checked, true)? {
            return Ok(Value::Bool(false));
        }
    }
    let fail = |host: &mut dyn Host, e: i32| -> Result<Value, Thrown> {
        crate::warn(host, "mkdir", &crate::main::streams::strerror(e))?;
        Ok(Value::Bool(false))
    };
    if !recursive {
        return match sys_mkdir(&local, mode) {
            Ok(()) => Ok(Value::Bool(true)),
            Err(e) => fail(host, e),
        };
    }
    // Every cumulative prefix ending before a '/', then the whole path.
    let mut prefixes: Vec<Vec<u8>> = Vec::new();
    for (i, &b) in local.iter().enumerate() {
        if b == b'/' && i > 0 && local[i - 1] != b'/' {
            prefixes.push(local[..i].to_vec());
        }
    }
    let trimmed_end = local
        .iter()
        .rposition(|&b| b != b'/')
        .map(|i| i + 1)
        .unwrap_or(local.len());
    let leaf = local[..trimmed_end].to_vec();
    prefixes.retain(|p| p.len() < leaf.len());
    for p in &prefixes {
        if crate::main::streams::sys_stat(p, false).is_some() {
            continue;
        }
        match sys_mkdir(p, mode) {
            Ok(()) => {}
            Err(e) if e == libc::EEXIST => {}
            Err(e) => return fail(host, e),
        }
    }
    match sys_mkdir(&leaf, mode) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => fail(host, e),
    }
}

/// `copy($from, $to)` — php_copy_file_ctx: refuses directories and copying a
/// file onto itself; reads through the stream layer (so `phar://` sources
/// work) and writes like `fopen($to, "wb")` (an existing target keeps its
/// mode, a new one gets 0666 & ~umask).
fn fn_copy(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let from = path_param(args, 0, "copy", "from")?;
    let to = path_param(args, 1, "copy", "to")?;
    if host.has_user_wrappers()
        && (crate::main::streams::userspace::locate(host, &from).is_some()
            || crate::main::streams::userspace::locate(host, &to).is_some())
    {
        touch_ctx(host, args, 2);
        let ctx = args.get(2).map(deref).unwrap_or(Value::Null);
        return copy_streams(host, &from, &to, &ctx);
    }
    let ob = crate::main::fopen_wrappers::active();
    if ob
        && matches!(
            crate::main::streams::locate_url_wrapper(&from, |s| crate::diag::wrapper_registered(
                host, s
            )),
            crate::main::streams::Located::Plain(_) | crate::main::streams::Located::Unknown(_)
        )
        && !crate::main::fopen_wrappers::check(host, "copy", &from, true)?
    {
        return Ok(Value::Bool(false));
    }
    touch_ctx(host, args, 2);
    // the quiet url_stat of a refused target fails: copy goes on to open it
    let to_plain = split_scheme(&to).0 == b"file";
    let stat_to = !ob
        || !to_plain
        || crate::main::fopen_wrappers::check(
            host,
            "copy",
            crate::main::streams::file_path(&to),
            false,
        )?;
    if let Some(s) = host.stream_stat(&from) {
        if s.is_dir {
            crate::warn(
                host,
                "copy",
                "The first argument to copy() function cannot be a directory",
            )?;
            return Ok(Value::Bool(false));
        }
        if let Some(d) = stat_to.then(|| host.stream_stat(&to)).flatten() {
            if d.is_dir {
                crate::warn(
                    host,
                    "copy",
                    "The second argument to copy() function cannot be a directory",
                )?;
                return Ok(Value::Bool(false));
            }
            if s.ino != 0 && d.ino != 0 {
                if s.ino == d.ino && s.dev == d.dev {
                    return Ok(Value::Bool(false));
                }
            } else if realpath_bytes(&from).is_some()
                && realpath_bytes(&from) == realpath_bytes(&to)
            {
                return Ok(Value::Bool(false));
            }
        }
    }
    crate::main::streams::clear_errno();
    let data = if builtin_http(host, &from) {
        match crate::ext::standard::http_fopen_wrapper::get_contents(
            host,
            "copy",
            &from,
            args.get(2),
            0,
            None,
        )? {
            Some(b) => b,
            None => return Ok(Value::Bool(false)),
        }
    } else if let Some(r) = read_via_opener(host, "copy", &from, args.get(2))? {
        match r {
            Some(b) => b,
            None => return Ok(Value::Bool(false)),
        }
    } else if let Some(copied) = copy_plain_files(host, &from, &to)? {
        return Ok(Value::Bool(copied));
    } else {
        let Some(data) = host.stream_get_contents(&from) else {
            crate::open_failed(host, "copy", &from)?;
            return Ok(Value::Bool(false));
        };
        host.consume_resource_ids(1);
        data
    };
    let (scheme, _) = split_scheme(&to);
    crate::main::streams::clear_errno();
    if scheme == b"file" {
        if !crate::main::fopen_wrappers::check_open(
            host,
            "copy",
            crate::main::streams::file_path(&to),
            &to,
        )? {
            return Ok(Value::Bool(false));
        }
        let Some(mut f) = crate::main::streams::LocalFile::open(&to, b"wb") else {
            crate::open_failed(host, "copy", &to)?;
            return Ok(Value::Bool(false));
        };
        host.consume_resource_ids(1);
        return Ok(Value::Bool(f.write(&data).is_some()));
    }
    if host.stream_put_contents(&to, &data).is_none() {
        crate::open_failed(host, "copy", &to)?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

/// php_copy_file_ctx over streams (a user wrapper on either side): the
/// stat checks, then a chunked copy from the source to the target.
fn copy_streams(host: &mut dyn Host, from: &[u8], to: &[u8], ctx: &Value) -> Result<Value, Thrown> {
    use crate::main::streams::userspace as us;
    let stat = |host: &mut dyn Host, p: &[u8], quiet: bool| match us::cached_stat(
        host, p, false, quiet, 0,
    ) {
        Some(s) => s,
        None => host.stream_stat(p),
    };
    if let Some(s) = stat(host, from, false) {
        if s.is_dir {
            crate::warn(
                host,
                "copy",
                "The first argument to copy() function cannot be a directory",
            )?;
            return Ok(Value::Bool(false));
        }
        if let Some(d) = stat(host, to, true) {
            if d.is_dir {
                crate::warn(
                    host,
                    "copy",
                    "The second argument to copy() function cannot be a directory",
                )?;
                return Ok(Value::Bool(false));
            }
        }
    }
    if host.has_pending_throw() {
        return Ok(Value::Bool(false));
    }
    let Some(src) = open_stream(host, "copy", from, b"rb", ctx, false)? else {
        return Ok(Value::Bool(false));
    };
    let Some(dst) = open_stream(host, "copy", to, b"wb", ctx, false)? else {
        host.stream_close(&src);
        return Ok(Value::Bool(false));
    };
    let mut ok = true;
    loop {
        match host.stream_read(&src, 8192) {
            Some(b) if !b.is_empty() => {
                if host.stream_write(&dst, &b) != Some(b.len()) {
                    ok = false;
                    break;
                }
            }
            Some(_) => break,
            None => {
                ok = false;
                break;
            }
        }
    }
    host.stream_close(&src);
    host.stream_close(&dst);
    Ok(Value::Bool(ok))
}

/// copy() from one local file to another through the default `file://`
/// wrapper: php opens the source "rb" and the target "wb", then copies
/// between the descriptors (copy_file_range(2) on Linux), so a file of any
/// size is copied without holding it in memory. `None` when either side is
/// not a plain file.
fn copy_plain_files(host: &mut dyn Host, from: &[u8], to: &[u8]) -> Result<Option<bool>, Thrown> {
    use crate::main::streams::{FileWrapper, LocalFile};
    if split_scheme(from).0 != b"file"
        || split_scheme(to).0 != b"file"
        || !host
            .stream_wrapper(b"file")
            .is_some_and(|w| w.as_any().is::<FileWrapper>())
    {
        return Ok(None);
    }
    let Some(mut src) = LocalFile::open(from, b"rb") else {
        crate::open_failed(host, "copy", from)?;
        return Ok(Some(false));
    };
    host.consume_resource_ids(1);
    crate::main::streams::clear_errno();
    if !crate::main::fopen_wrappers::check_open(
        host,
        "copy",
        crate::main::streams::file_path(to),
        to,
    )? {
        return Ok(Some(false));
    }
    let Some(mut dst) = LocalFile::open(to, b"wb") else {
        crate::open_failed(host, "copy", to)?;
        return Ok(Some(false));
    };
    host.consume_resource_ids(1);
    Ok(Some(src.copy_to(&mut dst)))
}

// --- temp files / paths ------------------------------------------------------------

/// php_get_temporary_directory(): $TMPDIR (trailing slash dropped), else
/// P_tmpdir.
pub fn temp_dir() -> Vec<u8> {
    // php_get_temporary_directory: sys_temp_dir first (one trailing slash
    // dropped).
    if let Some(d) = crate::ini_value(b"sys_temp_dir").flatten() {
        match d.last() {
            Some(b'/') if d.len() >= 2 => return d[..d.len() - 1].to_vec(),
            Some(c) if *c != b'/' => return d,
            _ => {}
        }
    }
    if let Some(s) = std::env::var_os("TMPDIR") {
        use std::os::unix::ffi::OsStrExt;
        let b = s.as_bytes();
        if !b.is_empty() {
            let mut v = b.to_vec();
            if v.last() == Some(&b'/') {
                v.pop();
            }
            return v;
        }
    }
    if cfg!(target_os = "macos") {
        b"/var/tmp/".to_vec()
    } else {
        b"/tmp".to_vec()
    }
}

fn fn_sys_get_temp_dir(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(temp_dir()))
}

use crate::zend::virtual_cwd::realpath as realpath_bytes;

fn fn_realpath(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let p = path_param(args, 0, "realpath", "path")?;
    let Some(real) = realpath_bytes(&p) else {
        return Ok(Value::Bool(false));
    };
    if !crate::main::fopen_wrappers::check(host, "realpath", &real, true)? {
        return Ok(Value::Bool(false));
    }
    Ok(Value::str(real))
}

pub(crate) fn open_temp(dir: &[u8], pfx: &[u8]) -> Option<Vec<u8>> {
    if dir.is_empty() {
        return None;
    }
    let real = realpath_bytes(dir)?;
    let mut rnd = [0u8; 8];
    getrandom_fill(&mut rnd);
    let mut random = u64::from_le_bytes(rnd);
    const ALPHA: &[u8] = b"0123456789abcdefghijklmnopqrstuv";
    let mut name = real.clone();
    if name.last() != Some(&b'/') {
        name.push(b'/');
    }
    name.extend_from_slice(pfx);
    for _ in 0..13 {
        name.push(ALPHA[(random % 32) as usize]);
        random /= 32;
    }
    name.extend_from_slice(b"XXXXXX");
    let c = CString::new(name).ok()?;
    let raw = c.into_raw();
    let fd = unsafe { libc::mkstemp(raw) };
    let c = unsafe { CString::from_raw(raw) };
    if fd < 0 {
        return None;
    }
    crate::zend::virtual_cwd::created_fd(fd, 0o600);
    unsafe { libc::close(fd) };
    Some(c.into_bytes())
}

fn getrandom_fill(buf: &mut [u8]) {
    use std::io::Read;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(buf).is_ok() {
            return;
        }
    }
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ (std::process::id() as u64) << 32;
    buf.copy_from_slice(&t.to_le_bytes()[..buf.len().min(8)]);
}

fn fn_tempnam(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let dir = path_param(args, 0, "tempnam", "directory")?;
    let prefix = path_param(args, 1, "tempnam", "prefix")?;
    // php_basename(prefix), truncated to 63 bytes.
    let end = prefix
        .iter()
        .rposition(|&b| b != b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let start = prefix[..end]
        .iter()
        .rposition(|&b| b == b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut pfx = prefix[start..end].to_vec();
    pfx.truncate(63);
    // php_open_temporary_fd_ex with PHP_TMP_FILE_OPEN_BASEDIR_CHECK_ALWAYS
    if !dir.is_empty() {
        if !crate::main::fopen_wrappers::check(host, "tempnam", &dir, true)? {
            return Ok(Value::Bool(false));
        }
        if let Some(p) = open_temp(&dir, &pfx) {
            return Ok(Value::str(p));
        }
        crate::notice(
            host,
            "tempnam",
            "file created in the system's temporary directory",
        )?;
    }
    let tmp = temp_dir();
    if tmp.is_empty() || !crate::main::fopen_wrappers::check(host, "tempnam", &tmp, true)? {
        return Ok(Value::Bool(false));
    }
    Ok(open_temp(&tmp, &pfx).map_or(Value::Bool(false), Value::str))
}

/// `readfile($filename)`: copy a file to the output; bytes written or false.
fn fn_readfile(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "readfile", "filename")?;
    if path.is_empty() {
        return Err(Thrown::error("ValueError", "Path must not be empty"));
    }
    touch_ctx(host, args, 2);
    if builtin_http(host, &path) {
        let Some(data) = crate::ext::standard::http_fopen_wrapper::get_contents(
            host,
            "readfile",
            &path,
            args.get(2),
            0,
            None,
        )?
        else {
            return Ok(Value::Bool(false));
        };
        host.write_out(&data);
        return Ok(Value::Int(data.len() as i64));
    }
    if host.has_user_wrappers() && crate::main::streams::userspace::locate(host, &path).is_some() {
        // _php_stream_passthru: 8 KiB reads, each written out
        let ctx = args.get(2).map(deref).unwrap_or(Value::Null);
        let Some(res) = open_stream(host, "readfile", &path, b"rb", &ctx, false)? else {
            return Ok(Value::Bool(false));
        };
        let mut n: i64 = 0;
        let mut failed = false;
        loop {
            match host.stream_read(&res, 8192) {
                Some(b) if !b.is_empty() => {
                    host.write_out(&b);
                    n += b.len() as i64;
                }
                Some(_) => break,
                None => {
                    failed = n == 0;
                    break;
                }
            }
        }
        host.stream_close(&res);
        return Ok(Value::Int(if failed { -1 } else { n }));
    }
    if let Some(r) = read_via_opener(host, "readfile", &path, args.get(2))? {
        let Some(data) = r else {
            return Ok(Value::Bool(false));
        };
        host.write_out(&data);
        return Ok(Value::Int(data.len() as i64));
    }
    let target = if args.get(1).is_some_and(|v| conv::to_bool(&deref(v))) {
        include_path_target(host, &path)
    } else {
        path.clone()
    };
    let Some(target) = crate::locate_for_open(host, "readfile", &target)? else {
        return Ok(Value::Bool(false));
    };
    crate::main::streams::clear_errno();
    let data = host.stream_get_contents(&target);
    let Some(data) = data else {
        if crate::main::streams::last_errno() == Some(libc::EISDIR) {
            // The open succeeds; the first read fails (php returns -1).
            host.consume_resource_ids(1);
            crate::notice(
                host,
                "readfile",
                "Read of 8192 bytes failed with errno=21 Is a directory",
            )?;
            return Ok(Value::Int(-1));
        }
        crate::open_failed(host, "readfile", &path)?;
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    host.write_out(&data);
    Ok(Value::Int(data.len() as i64))
}

/// The separator / enclosure / escape arguments of the CSV functions
/// (`sep_idx` is the separator's index): single characters, then
/// `php_csv_handle_escape_argument` (`None`: no escape character).
pub(crate) fn csv_args(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    sep_idx: usize,
) -> Result<(u8, u8, Option<u8>), Thrown> {
    let mut chars = *b",\"";
    for (k, pname) in ["separator", "enclosure"].iter().enumerate() {
        let i = sep_idx + k;
        if let Some(v) = args.get(i) {
            let v = conv::to_string_bytes(&deref(v));
            if v.len() != 1 {
                return Err(crate::zend::zpp::value_error(
                    fname,
                    i + 1,
                    pname,
                    "must be a single character",
                ));
            }
            chars[k] = v[0];
        }
    }
    let esc_idx = sep_idx + 2;
    let esc = match args.get(esc_idx) {
        Some(v) => {
            let v = conv::to_string_bytes(&deref(v));
            if v.len() > 1 {
                return Err(crate::zend::zpp::value_error(
                    fname,
                    esc_idx + 1,
                    "escape",
                    "must be empty or a single character",
                ));
            }
            v.first().copied()
        }
        None => {
            crate::deprecated(
                host,
                fname,
                "the $escape parameter must be provided as its default value will change",
            )?;
            Some(b'\\')
        }
    };
    Ok((chars[0], chars[1], esc))
}

/// `php_fgetcsv_lookup_trailing_spaces` (with a single-byte locale): the
/// length without a final "\n", "\r" or "\r\n".
fn csv_trailing(b: &[u8]) -> usize {
    match b {
        [.., b'\r', b'\n'] => b.len() - 2,
        [.., b'\n' | b'\r'] => b.len() - 1,
        _ => b.len(),
    }
}

/// `php_fgetcsv`: parse one CSV record from `buf`; an enclosure still open
/// at the end of the line continues on the lines `next_line` returns
/// (`None` for str_getcsv, which has no stream). `None` for a blank line.
///
/// php steps through the line with mblen(); in the "C" locale every byte
/// is one character, which is what this port assumes.
pub(crate) fn php_fgetcsv(
    mut buf: Vec<u8>,
    delimiter: u8,
    enclosure: u8,
    escape: Option<u8>,
    mut next_line: Option<&mut dyn FnMut() -> Option<Vec<u8>>>,
) -> Option<Vec<Vec<u8>>> {
    let at = |b: &[u8], i: usize| b.get(i).copied().unwrap_or(0);
    let isspace = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    // The C code walks a NUL-terminated buffer and can step onto (and
    // copy) the terminator; keep one here too.
    let mut bptr = 0usize;
    let mut limit = csv_trailing(&buf);
    let mut line_end = limit;
    let mut buf_len = buf.len();
    buf.push(0);
    let mut values = Vec::new();
    let mut temp: Vec<u8> = Vec::new();
    let mut first_field = true;
    loop {
        temp.clear();
        let mut inc_len = usize::from(bptr < limit);
        if inc_len == 1 {
            let mut tmp = bptr;
            while at(&buf, tmp) != delimiter && isspace(at(&buf, tmp)) {
                tmp += 1;
            }
            if at(&buf, tmp) == enclosure && tmp < limit {
                bptr = tmp;
            }
        }
        if first_field && bptr == line_end {
            return None;
        }
        first_field = false;
        let copy = |temp: &mut Vec<u8>, b: &[u8], from: usize, to: usize| {
            let to = to.min(b.len());
            if from < to {
                temp.extend_from_slice(&b[from..to]);
            }
        };
        let comp_end;
        if inc_len != 0 && at(&buf, bptr) == enclosure {
            let mut state = 0;
            bptr += 1;
            let mut hunk = bptr;
            loop {
                if inc_len == 0 {
                    if state == 2 {
                        copy(&mut temp, &buf, hunk, bptr - 1);
                        hunk = bptr;
                        break;
                    }
                    if state == 1 {
                        copy(&mut temp, &buf, hunk, bptr);
                        hunk = bptr;
                    }
                    if hunk != line_end {
                        copy(&mut temp, &buf, hunk, bptr);
                        hunk = bptr;
                    }
                    // the embedded line end belongs to the field
                    copy(&mut temp, &buf, line_end, buf_len);
                    let more = match next_line.as_mut() {
                        Some(f) => f(),
                        None => None,
                    };
                    let Some(nb) = more else {
                        // unterminated enclosure: the rest is the field
                        if bptr > limit {
                            if hunk == bptr {
                                hunk -= 1;
                            }
                            bptr -= 1;
                        }
                        break;
                    };
                    buf = nb;
                    buf_len = buf.len();
                    buf.push(0);
                    bptr = 0;
                    hunk = 0;
                    limit = csv_trailing(&buf[..buf_len]);
                    line_end = limit;
                    state = 0;
                } else {
                    match state {
                        1 => {
                            bptr += 1;
                            state = 0;
                        }
                        2 => {
                            if at(&buf, bptr) != enclosure {
                                copy(&mut temp, &buf, hunk, bptr - 1);
                                hunk = bptr;
                                break;
                            }
                            copy(&mut temp, &buf, hunk, bptr);
                            bptr += 1;
                            hunk = bptr;
                            state = 0;
                        }
                        _ => {
                            let c = at(&buf, bptr);
                            if c == enclosure {
                                state = 2;
                            } else if escape == Some(c) {
                                state = 1;
                            }
                            bptr += 1;
                        }
                    }
                }
                inc_len = usize::from(bptr < limit);
            }
            // up to the delimiter
            while inc_len != 0 && at(&buf, bptr) != delimiter {
                bptr += 1;
                inc_len = usize::from(bptr < limit);
            }
            copy(&mut temp, &buf, hunk, bptr);
            bptr += inc_len;
            comp_end = temp.len();
        } else {
            let hunk = bptr;
            while inc_len != 0 && at(&buf, bptr) != delimiter {
                bptr += 1;
                inc_len = usize::from(bptr < limit);
            }
            copy(&mut temp, &buf, hunk, bptr);
            comp_end = csv_trailing(&temp);
            if at(&buf, bptr) == delimiter {
                bptr += 1;
            }
        }
        values.push(temp[..comp_end].to_vec());
        if inc_len == 0 {
            break;
        }
    }
    Some(values)
}

/// A parsed record as php returns it: `[null]` for a blank line.
pub(crate) fn csv_record(rec: Option<Vec<Vec<u8>>>) -> Value {
    let mut out = PhpArray::new();
    match rec {
        Some(fields) => fields.into_iter().for_each(|f| out.push(Value::str(f))),
        None => out.push(Value::Null),
    }
    Value::Array(out)
}

/// `fgetcsv($stream, $length = null, $separator = ",", $enclosure = "\"",
/// $escape = "\\")`: one record; false at EOF.
fn fn_fgetcsv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fgetcsv", "stream")?;
    let (sep, enc, esc) = csv_args(host, args, "fgetcsv", 2)?;
    let len = match args.get(1).map(deref) {
        None | Some(Value::Null) => 0,
        Some(v) => conv::to_int(&v),
    };
    if !(0..i64::MAX).contains(&len) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "fgetcsv(): Argument #2 ($length) must be between 0 and {}",
                i64::MAX - 1
            ),
        ));
    }
    let first = if len == 0 {
        host.stream_gets(&h)
    } else {
        // emalloc(len + 1): past php's hard limit the process stops here.
        if !rhp_values::mem::alloc_check(len as usize + 1) {
            return Ok(Value::Null);
        }
        host.stream_gets_max(&h, len as usize)
    };
    let Some(buf) = first else {
        return Ok(Value::Bool(false));
    };
    let mut next = || host.stream_gets(&h);
    Ok(csv_record(php_fgetcsv(buf, sep, enc, esc, Some(&mut next))))
}

/// php_fputcsv: a field is enclosed when it holds the separator, the
/// enclosure, the escape character or whitespace; enclosures inside are
/// doubled (unless escaped). Returns the bytes written.
fn fn_fputcsv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fputcsv", "stream")?;
    let Value::Array(fields) = args.get(1).map(deref).unwrap_or(Value::Null) else {
        return Err(Thrown::type_error(
            "fputcsv(): Argument #2 ($fields) must be of type array",
        ));
    };
    let (sep, enc, esc) = csv_args(host, args, "fputcsv", 2)?;
    let eol = match args.get(5).map(deref) {
        None | Some(Value::Null) => b"\n".to_vec(),
        Some(v) => conv::to_string_bytes(&v),
    };
    let mut line = Vec::new();
    for (i, (_, v)) in fields.iter().enumerate() {
        if i > 0 {
            line.push(sep);
        }
        // zval_try_get_tmp_string: Array to string conversion, __toString.
        let f = crate::zend::zpp::zval_get_string(host, v)?;
        let needs = f.iter().any(|&c| {
            c == sep || c == enc || Some(c) == esc || matches!(c, b'\n' | b'\r' | b'\t' | b' ')
        });
        if needs {
            line.push(enc);
            let mut escaped = false;
            for &c in &f {
                if Some(c) == esc && esc.is_some() {
                    escaped = true;
                } else if !escaped && c == enc {
                    line.push(enc);
                } else {
                    escaped = false;
                }
                line.push(c);
            }
            line.push(enc);
        } else {
            line.extend_from_slice(&f);
        }
    }
    line.extend_from_slice(&eol);
    Ok(match host.stream_write(&h, &line) {
        Some(n) => Value::Int(n as i64),
        None => Value::Bool(false),
    })
}

/// `tmpfile()`: a new unlinked temporary file opened "w+b".
fn fn_tmpfile(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let Some(path) = open_temp(&temp_dir(), b"php") else {
        return Ok(Value::Bool(false));
    };
    let r = host.stream_open(&path, b"w+b");
    if let Ok(c) = CString::new(path) {
        unsafe { libc::unlink(c.as_ptr()) };
    }
    Ok(r.unwrap_or(Value::Bool(false)))
}

/// `fpassthru($stream)`: output the rest of the stream; bytes written.
fn fn_fpassthru(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "fpassthru", "stream")?;
    // _php_stream_passthru: 8K reads; a failed first read is -1
    let mut count = 0i64;
    loop {
        match host.stream_read(&h, 8192) {
            Some(d) if !d.is_empty() => {
                host.write_out(&d);
                count += d.len() as i64;
            }
            Some(_) => break,
            None if count == 0 => return Ok(Value::Int(-1)),
            None => break,
        }
    }
    Ok(Value::Int(count))
}

/// The length of a leading `scheme` followed by `://` (php's protocol
/// test: alphanumerics, `+`, `-`, `.`, at least two of them).
fn protocol_len(s: &[u8]) -> Option<usize> {
    let p = s
        .iter()
        .position(|&c| !(c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')))
        .unwrap_or(s.len());
    (p > 1 && s[p..].starts_with(b"://")).then_some(p)
}

/// The directory of the executing script, for php's include_path fallback
/// (`None` when the file name has no slash).
fn exec_dir(host: &dyn Host) -> Option<Vec<u8>> {
    let f = host.executing_file()?;
    let i = f.iter().rposition(|&c| c == b'/')?;
    (i > 0).then(|| f[..i].to_vec())
}

/// `php_resolve_path(filename, PG(include_path))`: the real path of the
/// first include_path entry (then the executing script's directory) that
/// holds `name`.
pub(crate) fn resolve_include_path(host: &mut dyn Host, name: &[u8]) -> Option<Vec<u8>> {
    if name.contains(&0) {
        return None;
    }
    // Paths with a protocol: only the plain-files wrapper resolves.
    if protocol_len(name).is_some() {
        let located = crate::main::streams::locate_url_wrapper(name, |s| {
            crate::diag::wrapper_registered(host, s)
        });
        return match located {
            crate::main::streams::Located::Plain(local) => realpath_bytes(&local),
            _ => None,
        };
    }
    let path = host.ini_get(b"include_path").unwrap_or_default();
    if name.starts_with(b"./")
        || name.starts_with(b"../")
        || name.starts_with(b"/")
        || path.is_empty()
    {
        return realpath_bytes(name);
    }
    // A candidate: a wrapper entry is stat'ed through its wrapper, a plain
    // one resolved with realpath.
    let try_path = |host: &mut dyn Host, trypath: &[u8], wrapper: bool| -> Option<Vec<u8>> {
        if wrapper {
            if let Some((w, _)) = crate::main::streams::userspace::locate(host, trypath) {
                use crate::main::streams::userspace as us;
                return us::url_stat(host, &w, trypath, us::URL_STAT_QUIET, None)
                    .map(|_| trypath.to_vec());
            }
            let (scheme, _) = split_scheme(trypath);
            if scheme.eq_ignore_ascii_case(b"file") {
                return realpath_bytes(crate::main::streams::file_path(trypath));
            }
            return host.stream_stat(trypath).map(|_| trypath.to_vec());
        }
        realpath_bytes(trypath)
    };
    let mut rest: &[u8] = &path;
    while !rest.is_empty() {
        // `.://` and `..://` are not wrappers
        let mut skip = 0;
        let mut wrapper = false;
        if let Some(p) = protocol_len(rest) {
            if !(p == 2 && &rest[..2] == b"..") {
                skip = p + 3;
                wrapper = true;
            }
        }
        let (entry, next) = match rest[skip..].iter().position(|&c| c == b':') {
            Some(i) => (&rest[..skip + i], &rest[skip + i + 1..]),
            None => (rest, &[][..]),
        };
        rest = next;
        let mut trypath = entry.to_vec();
        trypath.push(b'/');
        trypath.extend_from_slice(name);
        if let Some(r) = try_path(host, &trypath, wrapper) {
            return Some(r);
        }
        if host.has_pending_throw() {
            return None;
        }
    }
    let mut trypath = exec_dir(host)?;
    trypath.push(b'/');
    trypath.extend_from_slice(name);
    let wrapper = protocol_len(&trypath).is_some();
    try_path(host, &trypath, wrapper)
}

/// Where a plain-file open with `use_include_path` goes: the resolved
/// path, else the name as given.
pub(crate) fn include_path_target(host: &mut dyn Host, name: &[u8]) -> Vec<u8> {
    if split_scheme(name).0 != b"file" {
        return name.to_vec();
    }
    resolve_include_path(host, name).unwrap_or_else(|| name.to_vec())
}

/// `file_put_contents`'s flag to append.
const FILE_APPEND: i64 = 8;

pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("FILE_USE_INCLUDE_PATH", Value::Int(1)),
        ("FILE_IGNORE_NEW_LINES", Value::Int(2)),
        ("FILE_SKIP_EMPTY_LINES", Value::Int(4)),
        ("FILE_APPEND", Value::Int(FILE_APPEND)),
        ("FILE_NO_DEFAULT_CONTEXT", Value::Int(16)),
        ("FILE_TEXT", Value::Int(0)),
        ("FILE_BINARY", Value::Int(0)),
        ("LOCK_SH", Value::Int(1)),
        ("LOCK_EX", Value::Int(2)),
        ("LOCK_UN", Value::Int(3)),
        ("LOCK_NB", Value::Int(4)),
        ("SEEK_SET", Value::Int(0)),
        ("SEEK_CUR", Value::Int(1)),
        ("SEEK_END", Value::Int(2)),
        ("SCANDIR_SORT_ASCENDING", Value::Int(0)),
        ("SCANDIR_SORT_DESCENDING", Value::Int(1)),
        ("SCANDIR_SORT_NONE", Value::Int(2)),
        ("GLOB_BRACE", Value::Int(GLOB_BRACE)),
        ("GLOB_ERR", Value::Int(GLOB_ERR)),
        ("GLOB_MARK", Value::Int(GLOB_MARK)),
        ("GLOB_NOCHECK", Value::Int(GLOB_NOCHECK)),
        ("GLOB_NOESCAPE", Value::Int(GLOB_NOESCAPE)),
        ("GLOB_NOSORT", Value::Int(GLOB_NOSORT)),
        ("GLOB_ONLYDIR", Value::Int(GLOB_ONLYDIR)),
        ("GLOB_AVAILABLE_FLAGS", Value::Int(GLOB_AVAILABLE)),
    ]
}

pub(crate) fn path_arg(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

/// `umask(?int $mask = null): int` — get, or set-and-return-old, the
/// request's file mode creation mask.
fn fn_umask(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let old = match args.first().map(deref) {
        Some(v) if !matches!(v, Value::Null) => {
            crate::zend::virtual_cwd::set_umask(conv::to_int(&v) as u32)
        }
        _ => crate::zend::virtual_cwd::umask(),
    };
    Ok(Value::Int(old as i64))
}

fn fn_file_get_contents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let url = path_param(args, 0, "file_get_contents", "filename")?;
    let offset = args.get(3).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let maxlen = match args.get(4).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = conv::to_int(&v);
            if n < 0 {
                return Err(Thrown::error(
                    "ValueError",
                    "file_get_contents(): Argument #5 ($length) must be greater than or equal to 0",
                ));
            }
            Some(n as usize)
        }
    };
    // php_stream_open_wrapper_ex
    if url.is_empty() {
        return Err(Thrown::error("ValueError", "Path must not be empty"));
    }
    touch_ctx(host, args, 2);
    if builtin_http(host, &url) {
        return Ok(
            match crate::ext::standard::http_fopen_wrapper::get_contents(
                host,
                "file_get_contents",
                &url,
                args.get(2),
                offset,
                maxlen,
            )? {
                Some(b) => Value::str(b),
                None => Value::Bool(false),
            },
        );
    }
    if url.eq_ignore_ascii_case(b"php://stdin") {
        return stdin_contents(host, offset, maxlen);
    }
    let scheme = crate::main::streams::split_scheme(&url).0.to_vec();
    if crate::main::streams::is_filter_url(&url)
        || host
            .stream_wrapper(&scheme)
            .is_some_and(|w| w.has_stream_opener())
    {
        let ctx = args.get(2).map(deref).unwrap_or(Value::Null);
        return get_contents_via_stream(host, &url, &ctx, offset, maxlen);
    }
    let use_include_path = args.get(1).is_some_and(|v| conv::to_bool(&deref(v)));
    let shown = url.clone();
    let url = if use_include_path {
        include_path_target(host, &url)
    } else {
        url
    };
    if host.has_user_wrappers() && crate::main::streams::userspace::locate(host, &url).is_some() {
        let ctx = args.get(2).map(deref).unwrap_or(Value::Null);
        return get_contents_via_stream(host, &url, &ctx, offset, maxlen);
    }
    let Some(url) = crate::locate_for_open(host, "file_get_contents", &url)? else {
        return Ok(Value::Bool(false));
    };
    {
        if !reserve_file_read(&url, offset, maxlen) {
            host.consume_resource_ids(1);
            return Ok(Value::str(""));
        }
        if let Some(part) = read_file_part(&url, offset, maxlen) {
            host.consume_resource_ids(1);
            let Some(part) = part else {
                crate::warn(
                    host,
                    "file_get_contents",
                    &format!("Failed to seek to position {offset} in the stream"),
                )?;
                return Ok(Value::Bool(false));
            };
            return Ok(Value::str(part));
        }
    }
    crate::main::streams::clear_errno();
    match host.stream_get_contents(&url) {
        Some(bytes) => {
            host.consume_resource_ids(crate::main::streams::open_resource_ids(&url));
            // php seeks to `$offset` (SEEK_SET, or SEEK_END when negative)
            // before copying at most `$length` bytes. A plain file may be
            // positioned past its end (reads nothing); other wrappers, and
            // any position before the start, fail the seek -> false.
            let len = bytes.len() as i64;
            let start = if offset > 0 {
                if offset > len && crate::main::streams::split_scheme(&url).0 != b"file" {
                    return Ok(Value::Bool(false));
                }
                offset.min(len)
            } else if offset < 0 {
                if -offset > len {
                    return Ok(Value::Bool(false));
                }
                len + offset
            } else {
                0
            } as usize;
            let mut out = bytes;
            out.drain(..start);
            if let Some(m) = maxlen {
                out.truncate(m);
            }
            Ok(Value::str(out))
        }
        None => {
            if crate::main::streams::last_errno() == Some(libc::EISDIR) {
                host.consume_resource_ids(1);
                crate::notice(
                    host,
                    "file_get_contents",
                    "Read of 8320 bytes failed with errno=21 Is a directory",
                )?;
                return Ok(Value::str(""));
            }
            crate::open_failed(host, "file_get_contents", &shown)?;
            Ok(Value::Bool(false))
        }
    }
}

/// file_get_contents("php://stdin"): a fresh stream on a dup of fd 0, seeked
/// to `offset` as any stream. It seeks only where fd 0 does (a regular file;
/// on Linux also /dev/null and the other memory devices); elsewhere the seek
/// fails with php's two warnings.
fn stdin_contents(
    host: &mut dyn Host,
    offset: i64,
    maxlen: Option<usize>,
) -> Result<Value, Thrown> {
    use crate::main::streams::plain_wrapper::{dup_raw, FdStream};
    let Some(fd) = dup_raw(0) else {
        crate::open_failed(host, "file_get_contents", b"php://stdin")?;
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    let mut stream = FdStream::new(fd);
    if offset != 0 {
        let whence = if offset > 0 {
            libc::SEEK_SET
        } else {
            libc::SEEK_END
        };
        if !stream.seek(offset, whence) {
            if !stream.is_seekable() {
                crate::warn(host, "file_get_contents", "Stream does not support seeking")?;
            }
            crate::warn(
                host,
                "file_get_contents",
                &format!("Failed to seek to position {offset} in the stream"),
            )?;
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::str(stream.read_all(maxlen.map_or(-1, |m| m as i64))))
}

/// file_get_contents over a wrapper's own stream (`zip://`): open with the
/// context, seek to `offset`, copy at most `maxlen` bytes, close.
fn get_contents_via_stream(
    host: &mut dyn Host,
    url: &[u8],
    ctx: &Value,
    offset: i64,
    maxlen: Option<usize>,
) -> Result<Value, Thrown> {
    let Some(res) = host.stream_open_ctx(url, b"rb", ctx) else {
        if !host.has_pending_throw() {
            crate::open_failed(host, "file_get_contents", url)?;
        }
        return Ok(Value::Bool(false));
    };
    if offset != 0 {
        let whence = if offset < 0 {
            libc::SEEK_END
        } else {
            libc::SEEK_SET
        };
        if !stream_seek(host, "file_get_contents", &res, offset, whence)? {
            crate::warn(
                host,
                "file_get_contents",
                &format!("Failed to seek to position {offset} in the stream"),
            )?;
            host.stream_close(&res);
            return Ok(Value::Bool(false));
        }
    }
    let data = match maxlen {
        None => host.stream_read(&res, -1).unwrap_or_default(),
        Some(max) => {
            let mut out = Vec::new();
            while out.len() < max && !host.stream_eof(&res) {
                match host.stream_read(&res, (max - out.len()) as i64) {
                    Some(b) if !b.is_empty() => out.extend_from_slice(&b),
                    _ => break,
                }
            }
            out
        }
    };
    host.stream_close(&res);
    Ok(Value::str(data))
}

/// A whole-stream read through a wrapper's own opener (`zip://`), with the
/// call's context, for builtin `fname`: `None` when the wrapper has no
/// opener (read as before), `Some(None)` when the open failed (warned).
pub(crate) fn read_via_opener(
    host: &mut dyn Host,
    fname: &str,
    url: &[u8],
    ctx: Option<&Value>,
) -> Result<Option<Option<Vec<u8>>>, Thrown> {
    let scheme = crate::main::streams::split_scheme(url).0.to_vec();
    if !crate::main::streams::is_filter_url(url)
        && !host
            .stream_wrapper(&scheme)
            .is_some_and(|w| w.has_stream_opener())
    {
        return Ok(None);
    }
    let ctx = ctx.map(deref).unwrap_or(Value::Null);
    let Some(res) = host.stream_open_ctx(url, b"rb", &ctx) else {
        if !host.has_pending_throw() {
            crate::open_failed(host, fname, url)?;
        }
        return Ok(Some(None));
    };
    let data = host.stream_read(&res, -1).unwrap_or_default();
    host.stream_close(&res);
    Ok(Some(Some(data)))
}

/// A local path given without a scheme or as `file://`.
fn plain_path(url: &[u8]) -> Option<std::borrow::Cow<'_, std::path::Path>> {
    match crate::main::streams::split_scheme(url) {
        (b"file", p) => Some(crate::zend::virtual_cwd::os_path(
            crate::main::streams::file_path(p),
        )),
        _ => None,
    }
}

/// php_stream_copy_to_mem's first block for a plain file: the size left
/// after the read position plus a chunk (at most `$length`), allocated
/// fallibly. `false` when that crosses memory_limit (then recorded) and
/// file_get_contents() returns "" without reading.
fn reserve_file_read(url: &[u8], offset: i64, maxlen: Option<usize>) -> bool {
    const CHUNK: usize = 8192;
    if maxlen == Some(0) {
        return true;
    }
    let Some(path) = plain_path(url) else {
        return true;
    };
    // (A regular file php can open: stat before opening, a FIFO would block.)
    let size = match std::fs::metadata(&path) {
        Ok(m) if m.is_file() && m.len() > 0 => m.len() as i64,
        _ => return true,
    };
    if std::fs::File::open(&path).is_err() {
        return true;
    }
    // (A seek before the start fails before php allocates.)
    let pos = match offset {
        o if o < 0 && size + o < 0 => return true,
        o if o < 0 => size + o,
        o => o,
    };
    let mut len = (size - pos).max(0) as usize + CHUNK;
    if let Some(m) = maxlen {
        if m < 4 * CHUNK || len > m {
            len = m;
        }
    }
    rhp_values::mem::reserve_str(len)
}

/// file_get_contents() of part of a regular file (`$offset` or `$length`
/// given): read only that part, as php does, not the whole file (which may
/// be past memory_limit). `Some(None)`: a negative offset before the start
/// (the seek fails). `None`: not a regular file, left to the wrapper.
fn read_file_part(url: &[u8], offset: i64, maxlen: Option<usize>) -> Option<Option<Vec<u8>>> {
    use std::io::{Read, Seek, SeekFrom};
    if offset == 0 && maxlen.is_none() {
        return None;
    }
    let path = plain_path(url)?;
    if !std::fs::metadata(&path).ok()?.is_file() {
        return None;
    }
    let mut f = std::fs::File::open(&path).ok()?;
    let len = f.metadata().ok()?.len() as i64;
    // A plain file may be positioned past its end (reads nothing).
    let start = match offset {
        o if o < 0 && len + o < 0 => return Some(None),
        o if o < 0 => len + o,
        o => o.min(len),
    };
    f.seek(SeekFrom::Start(start as u64)).ok()?;
    let want = ((len - start) as usize).min(maxlen.unwrap_or(usize::MAX));
    let mut out = Vec::with_capacity(want);
    f.take(want as u64).read_to_end(&mut out).ok()?;
    Some(Some(out))
}

fn fn_file_put_contents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "file_put_contents", "filename")?;
    let src = deref(&args[1]);
    if matches!(src, Value::Resource(_)) && host.stream_kind(&src).is_none() {
        return Err(Thrown::type_error(
            "file_put_contents(): supplied resource is not a valid stream resource",
        ));
    }
    let flags = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let ctx = args.get(3).map(deref).unwrap_or(Value::Null);
    match &ctx {
        Value::Resource(r) if r.type_name != "stream-context" => {
            return Err(Thrown::type_error(
                "file_put_contents(): supplied resource is not a valid Stream-Context resource",
            ));
        }
        Value::Null if flags & 16 == 0 => {
            host.default_stream_context();
        }
        _ => {}
    }
    const LOCK_EX: i64 = 2;
    let mode: &[u8] = if flags & FILE_APPEND != 0 {
        b"ab"
    } else if flags & LOCK_EX != 0 {
        if path.windows(3).any(|w| w == b"://")
            && !path
                .get(..7)
                .is_some_and(|p| p.eq_ignore_ascii_case(b"file://"))
        {
            crate::warn(
                host,
                "file_put_contents",
                "Exclusive locks may only be set for regular files",
            )?;
            return Ok(Value::Bool(false));
        }
        b"cb"
    } else {
        b"wb"
    };
    if path.is_empty() {
        return Err(Thrown::error("ValueError", "Path must not be empty"));
    }
    if builtin_http(host, &path) {
        // the http wrapper refuses to write
        if let Some(res) = crate::ext::standard::http_fopen_wrapper::open_arg(
            host,
            "file_put_contents",
            &path,
            mode,
            args.get(3),
        )? {
            host.stream_close(&res);
        }
        return Ok(Value::Bool(false));
    }
    let stream = if ftp::is_ftp(&path) {
        crate::ext::standard::http_fopen_wrapper::open_arg(
            host,
            "file_put_contents",
            &path,
            mode,
            args.get(3),
        )?
    } else {
        open_stream(host, "file_put_contents", &path, mode, &ctx, flags & 1 != 0)?
    };
    let Some(stream) = stream else {
        return Ok(Value::Bool(false));
    };
    if flags & LOCK_EX != 0 {
        let locked = host
            .stream_fd(&stream)
            .is_some_and(|fd| unsafe { libc::flock(fd, libc::LOCK_EX) } == 0);
        if !locked {
            host.stream_close(&stream);
            crate::warn(
                host,
                "file_put_contents",
                "Exclusive locks are not supported for this stream",
            )?;
            return Ok(Value::Bool(false));
        }
    }
    if mode[0] == b'c' {
        host.stream_truncate(&stream, 0);
    }
    let write_all = |host: &mut dyn Host, data: &[u8]| -> i64 {
        match host.stream_write(&stream, data) {
            Some(n) if n == data.len() => n as i64,
            Some(n) => {
                let _ = crate::warn(
                    host,
                    "file_put_contents",
                    &format!(
                        "Only {n} of {} bytes written, possibly out of free disk space",
                        data.len()
                    ),
                );
                -1
            }
            None => -1,
        }
    };
    let mut numbytes: i64 = 0;
    let mut thrown = None;
    match &src {
        r @ Value::Resource(_) => {
            let data = host.stream_read(r, -1).unwrap_or_default();
            if !data.is_empty() && host.stream_write(&stream, &data).is_none() {
                numbytes = -1;
            } else {
                numbytes = data.len() as i64;
            }
        }
        Value::Array(a) => {
            for (_, v) in a.iter() {
                let piece = match crate::zend::zpp::zval_get_string(host, v) {
                    Ok(p) => p,
                    Err(t) => {
                        thrown = Some(t);
                        break;
                    }
                };
                if piece.is_empty() {
                    continue;
                }
                numbytes += piece.len() as i64;
                if host.stream_write(&stream, &piece) != Some(piece.len()) {
                    let msg = [
                        format!("Failed to write {} bytes to ", piece.len()).as_bytes(),
                        &path[..],
                    ]
                    .concat();
                    let _ = crate::warn(host, "file_put_contents", &msg);
                    numbytes = -1;
                    break;
                }
            }
        }
        o @ (Value::Object(_) | Value::Closure(_)) => {
            match crate::zend::object_handlers::to_string(host, o) {
                Ok(Some(d)) => numbytes = write_all(host, &d),
                Ok(None) => numbytes = -1,
                Err(t) => thrown = Some(t),
            }
        }
        other => {
            let d = conv::to_string_bytes(other);
            if !d.is_empty() {
                numbytes = write_all(host, &d);
            }
        }
    }
    host.stream_close(&stream);
    if let Some(t) = thrown {
        return Err(t);
    }
    Ok(if numbytes < 0 {
        Value::Bool(false)
    } else {
        Value::Int(numbytes)
    })
}

fn fn_unlink(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    touch_ctx(host, args, 1);
    let path = path_arg(args, 0);
    if let Some((w, _)) = crate::main::streams::userspace::locate(host, &path) {
        let ctx = ctx_or_default(host, args, 1);
        let ok = crate::main::streams::userspace::unlink(host, &w, &path, Some(&ctx));
        return Ok(Value::Bool(ok));
    } else if ftp::is_ftp(&path) {
        let ctx = ctx_or_default(host, args, 1);
        return Ok(Value::Bool(ftp::unlink(host, "unlink", &path, &ctx)?));
    }
    if !crate::main::fopen_wrappers::check(host, "unlink", strip_file_scheme(&path), true)? {
        return Ok(Value::Bool(false));
    }
    let c = crate::zend::virtual_cwd::c_path(strip_file_scheme(&path)).unwrap_or_default();
    // php_plain_files_unlink: unlink(2) (a directory fails with EPERM/EISDIR).
    if unsafe { libc::unlink(c.as_ptr()) } != 0 {
        let e = std::io::Error::last_os_error();
        crate::warn_arg(host, "unlink", &path, &os_text(&e))?;
        return Ok(Value::Bool(false));
    }
    crate::zend::virtual_cwd::realpath_cache::clean();
    Ok(Value::Bool(true))
}

/// The plain wrapper's rmdir/unlink/mkdir take a leading `file://` off
/// (any case).
pub(crate) fn strip_file_scheme(url: &[u8]) -> &[u8] {
    if url.len() >= 7 && url[..7].eq_ignore_ascii_case(b"file://") {
        &url[7..]
    } else {
        url
    }
}

/// `strerror` of an `io::Error`'s errno (php's warning text).
pub(crate) fn os_text(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(c) => crate::main::streams::strerror(c),
        None => e.to_string(),
    }
}

fn fn_rmdir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "rmdir", "directory")?;
    touch_ctx(host, args, 1);
    if let Some((w, _)) = crate::main::streams::userspace::locate(host, &path) {
        let ctx = ctx_or_default(host, args, 1);
        let us = crate::main::streams::userspace::REPORT_ERRORS;
        let ok = crate::main::streams::userspace::rmdir(host, &w, &path, us, Some(&ctx));
        return Ok(Value::Bool(ok));
    } else if ftp::is_ftp(&path) {
        let ctx = ctx_or_default(host, args, 1);
        return Ok(Value::Bool(ftp::rmdir(host, "rmdir", &path, &ctx)?));
    }
    if !crate::main::fopen_wrappers::check(host, "rmdir", strip_file_scheme(&path), true)? {
        return Ok(Value::Bool(false));
    }
    if let Err(e) = std::fs::remove_dir(crate::zend::virtual_cwd::os_path(strip_file_scheme(&path)))
    {
        crate::warn_arg(host, "rmdir", &path, &os_text(&e))?;
        return Ok(Value::Bool(false));
    }
    crate::zend::virtual_cwd::realpath_cache::clean();
    Ok(Value::Bool(true))
}

fn fn_rename(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let r = rename(host, args)?;
    if matches!(r, Value::Bool(true)) {
        // php_plain_files_rename clears the realpath cache
        crate::zend::virtual_cwd::realpath_cache::clean();
    }
    Ok(r)
}

fn rename(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    touch_ctx(host, args, 2);
    let from_b = path_arg(args, 0);
    let to_b = path_arg(args, 1);
    let (uf, ut) = (
        crate::main::streams::userspace::locate(host, &from_b),
        crate::main::streams::userspace::locate(host, &to_b),
    );
    if uf.is_some() || ut.is_some() {
        let same = matches!((&uf, &ut), (Some((a, _)), Some((b, _))) if std::rc::Rc::ptr_eq(a, b));
        if !same {
            crate::warn(host, "rename", "Cannot rename a file across wrapper types")?;
            return Ok(Value::Bool(false));
        }
        let (w, _) = uf.unwrap();
        let ctx = ctx_or_default(host, args, 2);
        let ok = crate::main::streams::userspace::rename(host, &w, &from_b, &to_b, Some(&ctx));
        return Ok(Value::Bool(ok));
    } else if ftp::is_ftp(&from_b) {
        if !ftp::is_ftp(&to_b) {
            crate::warn(host, "rename", "Cannot rename a file across wrapper types")?;
            return Ok(Value::Bool(false));
        }
        let ctx = ctx_or_default(host, args, 2);
        return Ok(Value::Bool(ftp::rename(
            host, "rename", &from_b, &to_b, &ctx,
        )?));
    }
    if !crate::main::fopen_wrappers::check(host, "rename", strip_file_scheme(&from_b), true)?
        || !crate::main::fopen_wrappers::check(host, "rename", strip_file_scheme(&to_b), true)?
    {
        return Ok(Value::Bool(false));
    }
    let from = crate::zend::virtual_cwd::os_path(&from_b);
    let to = crate::zend::virtual_cwd::os_path(&to_b);
    let both = [&from_b[..], b",", &to_b[..]].concat();
    match std::fs::rename(&from, &to) {
        Ok(()) => Ok(Value::Bool(true)),
        // php_plain_files_rename: across devices a file is copied (keeping
        // its mode) and the source unlinked.
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            let meta = match std::fs::symlink_metadata(&from) {
                Ok(m) => m,
                Err(e) => {
                    crate::warn_arg(host, "rename", &both, &os_text(&e))?;
                    return Ok(Value::Bool(false));
                }
            };
            if meta.is_dir() {
                crate::warn_arg(host, "rename", &both, &os_text(&e))?;
                return Ok(Value::Bool(false));
            }
            let r = std::fs::copy(&from, &to).and_then(|_| std::fs::remove_file(&from));
            match r {
                Ok(()) => Ok(Value::Bool(true)),
                Err(e) => {
                    crate::warn_arg(host, "rename", &both, &os_text(&e))?;
                    Ok(Value::Bool(false))
                }
            }
        }
        Err(e) => {
            crate::warn_arg(host, "rename", &both, &os_text(&e))?;
            Ok(Value::Bool(false))
        }
    }
}

/// Children of `popen()` streams, by stream resource id.
struct PopenState(std::collections::HashMap<u32, std::process::Child>);

fn popen_state(host: &mut dyn Host) -> Option<&mut PopenState> {
    host.module_state("popen", || Box::new(PopenState(Default::default())))?
        .downcast_mut::<PopenState>()
}

/// `popen($command, $mode)`: `/bin/sh -c` with its stdout (mode "r") or
/// stdin ("w") as a pipe stream.
fn fn_popen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use std::os::fd::OwnedFd;
    use std::os::unix::ffi::OsStrExt;
    use std::process::{Command, Stdio};
    let cmd = zpp_path(args, 0, "popen", "command")?;
    let mode = str_arg(args, 1);
    // php drops the first 'b', then allows "r", "w", "rb" and "wb"
    let mut posix_mode = mode.clone();
    if let Some(i) = posix_mode.iter().position(|&c| c == b'b') {
        posix_mode.remove(i);
    }
    let write = match &posix_mode[..] {
        [b'r'] | [b'r', b'b'] => false,
        [b'w'] | [b'w', b'b'] => true,
        [] => {
            // popen(3) with an empty mode: EINVAL
            let arg = [&cmd[..], b","].concat();
            crate::warn_arg(
                host,
                "popen",
                &arg,
                &crate::main::streams::strerror(libc::EINVAL),
            )?;
            return Ok(Value::Bool(false));
        }
        _ => {
            return Err(Thrown::error(
                "ValueError",
                "popen(): Argument #2 ($mode) must be one of \"r\", \"rb\", \"w\", or \"wb\"",
            ))
        }
    };
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(std::ffi::OsStr::from_bytes(&cmd));
    if let Some(dir) = crate::zend::virtual_cwd::child_cwd() {
        c.current_dir(std::ffi::OsStr::from_bytes(&dir));
    }
    if let Some(mask) = crate::zend::virtual_cwd::child_umask() {
        use std::os::unix::process::CommandExt;
        // umask(2) is async-signal-safe.
        unsafe {
            c.pre_exec(move || {
                libc::umask(mask as libc::mode_t);
                Ok(())
            })
        };
    }
    if write {
        c.stdin(Stdio::piped());
    } else {
        c.stdout(Stdio::piped());
    }
    let spawned = {
        let _spawning = SPAWN_LOCK.write().unwrap_or_else(|e| e.into_inner());
        c.spawn()
    };
    let mut child = match spawned {
        Ok(ch) => ch,
        Err(e) => {
            crate::warn(host, "popen", &format!("{e}"))?;
            return Ok(Value::Bool(false));
        }
    };
    let fd: OwnedFd = if write {
        child.stdin.take().map(OwnedFd::from)
    } else {
        child.stdout.take().map(OwnedFd::from)
    }
    .expect("piped");
    let Some(stream) = host.stream_from_fd(fd) else {
        return Ok(Value::Bool(false));
    };
    host.stream_set_mode(&stream, &mode);
    if let (Value::Resource(r), Some(st)) = (&stream, popen_state(host)) {
        st.0.insert(r.id, child);
    }
    Ok(stream)
}

/// `pclose($handle)`: close the pipe and wait; the exit status.
fn fn_pclose(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = deref(&args[0]);
    let id = match &h {
        Value::Resource(r) => r.id,
        other => {
            return Err(Thrown::type_error(format!(
                "pclose(): Argument #1 ($handle) must be of type resource, {} given",
                crate::zend::zpp::zval_value_name(host, other)
            )))
        }
    };
    host.stream_close(&h);
    let child = popen_state(host).and_then(|st| st.0.remove(&id));
    let Some(mut child) = child else {
        return Ok(Value::Int(-1));
    };
    Ok(Value::Int(match child.wait() {
        Ok(st) => {
            use std::os::unix::process::ExitStatusExt;
            st.code()
                .map(|c| c as i64)
                .unwrap_or_else(|| st.into_raw() as i64)
        }
        Err(_) => -1,
    }))
}

// --- filesystem --------------------------------------------------------------

const MAXPATHLEN: usize = libc::PATH_MAX as usize;

fn fn_fnmatch(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pattern = str_param(host, args, 0, "fnmatch", "pattern")?;
    let filename = str_param(host, args, 1, "fnmatch", "filename")?;
    let flags = int_param(host, args, 2, "fnmatch", "flags", 0)?;
    if pattern.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "fnmatch(): Argument #1 ($pattern) must not contain any null bytes",
        ));
    }
    if filename.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "fnmatch(): Argument #2 ($filename) must not contain any null bytes",
        ));
    }
    if filename.len() >= MAXPATHLEN {
        crate::warn(
            host,
            "fnmatch",
            &format!("Filename exceeds the maximum allowed length of {MAXPATHLEN} characters"),
        )?;
        return Ok(Value::Bool(false));
    }
    if pattern.len() >= MAXPATHLEN {
        crate::warn(
            host,
            "fnmatch",
            &format!("Pattern exceeds the maximum allowed length of {MAXPATHLEN} characters"),
        )?;
        return Ok(Value::Bool(false));
    }
    let p = std::ffi::CString::new(pattern).unwrap();
    let f = std::ffi::CString::new(filename).unwrap();
    let rc = unsafe { libc::fnmatch(p.as_ptr(), f.as_ptr(), flags as libc::c_int) };
    Ok(Value::Bool(rc == 0))
}

/// `fnmatch`'s `FNM_*` constants.
pub fn fnmatch_constants() -> Vec<(&'static str, Value)> {
    vec![
        ("FNM_NOESCAPE", Value::Int(libc::FNM_NOESCAPE as i64)),
        ("FNM_PATHNAME", Value::Int(libc::FNM_PATHNAME as i64)),
        ("FNM_PERIOD", Value::Int(libc::FNM_PERIOD as i64)),
        ("FNM_CASEFOLD", Value::Int(libc::FNM_CASEFOLD as i64)),
    ]
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("fopen", fn_fopen),
            ("mkdir", fn_mkdir),
            ("copy", fn_copy),
            ("file", fn_file),
            ("fstat", fn_fstat),
            ("flock", fn_flock),
            ("ftruncate", fn_ftruncate),
            ("fseek", fn_fseek),
            ("ftell", fn_ftell),
            ("rewind", fn_rewind),
            ("fgets", fn_fgets),
            ("fread", fn_fread),
            ("fwrite", fn_fwrite),
            ("fputs", fn_fwrite),
            ("fclose", fn_fclose),
            ("feof", fn_feof),
            ("fflush", fn_fflush),
            ("fgetc", fn_fgetc),
            ("sys_get_temp_dir", fn_sys_get_temp_dir),
            ("realpath", fn_realpath),
            ("tempnam", fn_tempnam),
            ("fgetcsv", fn_fgetcsv),
            ("fputcsv", fn_fputcsv),
            ("tmpfile", fn_tmpfile),
            ("readfile", fn_readfile),
            ("fpassthru", fn_fpassthru),
            ("file_get_contents", fn_file_get_contents),
            ("file_put_contents", fn_file_put_contents),
            ("unlink", fn_unlink),
            ("rmdir", fn_rmdir),
            ("rename", fn_rename),
            ("umask", fn_umask),
            ("popen", fn_popen),
            ("pclose", fn_pclose),
            ("fnmatch", fn_fnmatch),
            ("fscanf", fn_fscanf),
            ("fsync", fn_fsync),
            ("fdatasync", fn_fdatasync),
        ],
    );
}
