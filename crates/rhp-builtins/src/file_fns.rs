//! Filesystem & path builtins (php `ext/standard` file functions): existence
//! checks, `basename`/`dirname`/`pathinfo`, `getcwd`/`chdir`, and whole-file
//! read/write through the stream wrappers.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

pub(crate) fn path_arg(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

fn as_str(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

// --- local-fs metadata via libc (matches php's own ext/standard/filestat.c) ---

fn fn_file_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_exists(&path_arg(args, 0))))
}

fn fn_getcwd(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(crate::zend::virtual_cwd::current_dir().map_or(Value::Bool(false), Value::str))
}

/// `chdir()`: the request's working directory (see [`crate::zend::virtual_cwd`]).
fn fn_chdir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let dir = crate::zend::zpp::path_param(host, args, 0, "chdir", "directory")?;
    match crate::zend::virtual_cwd::chdir(&dir) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            let msg = format!("{} (errno {e})", crate::main::streams::strerror(e));
            crate::warn(host, "chdir", &msg)?;
            Ok(Value::Bool(false))
        }
    }
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
    let url = path_arg(args, 0);
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
    crate::fs_fns::touch_ctx(host, args, 2);
    if crate::http::is_http(&url) {
        return Ok(
            match crate::http::get_contents(
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
    let scheme = crate::main::streams::split_scheme(&url).0.to_vec();
    if host
        .stream_wrapper(&scheme)
        .is_some_and(|w| w.has_stream_opener())
    {
        let ctx = args.get(2).map(deref).unwrap_or(Value::Null);
        return get_contents_via_stream(host, &url, &ctx, offset, maxlen);
    }
    let use_include_path = args.get(1).is_some_and(|v| conv::to_bool(&deref(v)));
    if !use_include_path {
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
            crate::open_failed(host, "file_get_contents", &url)?;
            Ok(Value::Bool(false))
        }
    }
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
        crate::open_failed(host, "file_get_contents", url)?;
        return Ok(Value::Bool(false));
    };
    if offset != 0 {
        let whence = if offset < 0 {
            libc::SEEK_END
        } else {
            libc::SEEK_SET
        };
        if !crate::fs_fns::stream_seek(host, "file_get_contents", &res, offset, whence)? {
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
    if !host
        .stream_wrapper(&scheme)
        .is_some_and(|w| w.has_stream_opener())
    {
        return Ok(None);
    }
    let ctx = ctx.map(deref).unwrap_or(Value::Null);
    let Some(res) = host.stream_open_ctx(url, b"rb", &ctx) else {
        crate::open_failed(host, fname, url)?;
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
    let path = path_arg(args, 0);
    let src = deref(&args[1]);
    if matches!(src, Value::Resource(_)) && host.stream_kind(&src).is_none() {
        return Err(Thrown::type_error(
            "file_put_contents(): supplied resource is not a valid stream resource",
        ));
    }
    let flags = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    const FILE_APPEND: i64 = 8;
    if crate::http::is_http(&path) {
        // php opens the target before reading `$data`; the http wrapper
        // refuses to write
        let mode: &[u8] = if flags & FILE_APPEND != 0 {
            b"ab"
        } else {
            b"wb"
        };
        if let Some(res) =
            crate::http::open_arg(host, "file_put_contents", &path, mode, args.get(3))?
        {
            host.stream_close(&res);
        }
        return Ok(Value::Bool(false));
    }
    // `$data` is a string, an array (concatenated) or a stream resource
    // (copied from its current position to the end).
    let data = match src {
        r @ Value::Resource(_) => host.stream_read(&r, -1).unwrap_or_default(),
        Value::Array(a) => {
            let mut b = Vec::new();
            for (_, v) in a.iter() {
                b.extend_from_slice(&conv::to_string_bytes(v));
            }
            b
        }
        other => conv::to_string_bytes(&other),
    };
    if flags & 16 == 0 {
        crate::fs_fns::touch_ctx(host, args, 3);
    }
    let to_write = if flags & FILE_APPEND != 0 {
        let mut existing = host.stream_get_contents(&path).unwrap_or_default();
        existing.extend_from_slice(&data);
        existing
    } else {
        data.clone()
    };
    crate::main::streams::clear_errno();
    match host.stream_put_contents(&path, &to_write) {
        // php returns the number of bytes written (the `$data` length).
        Some(_) => {
            host.consume_resource_ids(1);
            Ok(Value::Int(data.len() as i64))
        }
        None => {
            crate::open_failed(host, "file_put_contents", &path)?;
            Ok(Value::Bool(false))
        }
    }
}

fn fn_unlink(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    crate::fs_fns::touch_ctx(host, args, 1);
    let path = path_arg(args, 0);
    let c = crate::zend::virtual_cwd::c_path(&path).unwrap_or_default();
    // php_plain_files_unlink: unlink(2) (a directory fails with EPERM/EISDIR).
    if unsafe { libc::unlink(c.as_ptr()) } != 0 {
        let e = std::io::Error::last_os_error();
        crate::warn_arg(host, "unlink", &path, &os_text(&e))?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

/// `strerror` of an `io::Error`'s errno (php's warning text).
fn os_text(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(c) => crate::main::streams::strerror(c),
        None => e.to_string(),
    }
}

fn fn_rmdir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    crate::fs_fns::touch_ctx(host, args, 1);
    let path = path_arg(args, 0);
    if let Err(e) = std::fs::remove_dir(crate::zend::virtual_cwd::os_path(&path)) {
        crate::warn_arg(host, "rmdir", &path, &os_text(&e))?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_rename(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    crate::fs_fns::touch_ctx(host, args, 2);
    let from_b = path_arg(args, 0);
    let to_b = path_arg(args, 1);
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

fn fn_touch(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_arg(args, 0);
    let path_str = as_str(&path).into_owned();
    // Create the file if absent (php's touch creates an empty file).
    if !crate::zend::virtual_cwd::os_path(&path).exists() {
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND | libc::O_CLOEXEC;
        if let Err(e) = crate::zend::virtual_cwd::open(&path, flags, 0o666) {
            crate::warn(
                host,
                "touch",
                &format!("Unable to create file {path_str} because {}", os_text(&e)),
            )?;
            return Ok(Value::Bool(false));
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // mtime: given or now; atime: given or the (effective) mtime — php's rule.
    let given = |i: usize| {
        args.get(i)
            .map(deref)
            .filter(|v| !matches!(v, Value::Null))
            .map(|v| conv::to_int(&v))
    };
    let mtime = given(1).unwrap_or(now);
    let atime = given(2).unwrap_or(mtime);
    let ok = match crate::zend::virtual_cwd::c_path(&path) {
        Some(c) => {
            let times = [
                libc::timeval {
                    tv_sec: atime as libc::time_t,
                    tv_usec: 0,
                },
                libc::timeval {
                    tv_sec: mtime as libc::time_t,
                    tv_usec: 0,
                },
            ];
            unsafe { libc::utimes(c.as_ptr(), times.as_ptr()) == 0 }
        }
        None => false,
    };
    if !ok {
        let e = std::io::Error::last_os_error();
        crate::warn(host, "touch", &format!("Utime failed: {}", os_text(&e)))?;
    }
    Ok(Value::Bool(ok))
}

/// `clearstatcache(): void`. rhp keeps no stat or realpath cache, so both
/// parameters have nothing to clear.
fn fn_clearstatcache(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Null)
}

fn fn_chmod(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mode = conv::to_int(&deref(&args[1])) as libc::mode_t;
    let ok = match crate::zend::virtual_cwd::c_path(&path_arg(args, 0)) {
        Some(c) => unsafe { libc::chmod(c.as_ptr(), mode) == 0 },
        None => false,
    };
    if !ok {
        let e = std::io::Error::last_os_error();
        crate::warn(host, "chmod", &os_text(&e))?;
    }
    Ok(Value::Bool(ok))
}

fn fn_readlink(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match std::fs::read_link(crate::zend::virtual_cwd::os_path(&path_arg(args, 0))) {
        Ok(p) => Ok(Value::str(p.to_string_lossy().into_owned().into_bytes())),
        Err(e) => {
            crate::warn(host, "readlink", &os_text(&e))?;
            Ok(Value::Bool(false))
        }
    }
}

/// `symlink()`: the target is stored as given (it is relative to the
/// link), the link is made from the working directory.
fn fn_symlink(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use std::os::unix::ffi::OsStrExt;
    let target = path_arg(args, 0);
    let link = path_arg(args, 1);
    let target = std::ffi::OsStr::from_bytes(&target);
    match std::os::unix::fs::symlink(target, crate::zend::virtual_cwd::os_path(&link)) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            crate::warn(host, "symlink", &os_text(&e))?;
            Ok(Value::Bool(false))
        }
    }
}

fn statvfs_of(path: &[u8]) -> Option<libc::statvfs> {
    let c = crate::zend::virtual_cwd::c_path(path)?;
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) == 0 {
            Some(st)
        } else {
            crate::main::streams::note_errno();
            None
        }
    }
}

/// `disk_free_space` — `f_bavail * f_frsize` as a float (php's formula).
fn fn_disk_free_space(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match statvfs_of(&path_arg(args, 0)) {
        Some(s) => Ok(Value::Float(s.f_bavail as f64 * s.f_frsize as f64)),
        None => {
            crate::warn(
                host,
                "disk_free_space",
                &crate::main::streams::last_strerror(),
            )?;
            Ok(Value::Bool(false))
        }
    }
}

/// `disk_total_space` — `f_blocks * f_frsize` as a float (php's formula).
fn fn_disk_total_space(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match statvfs_of(&path_arg(args, 0)) {
        Some(s) => Ok(Value::Float(s.f_blocks as f64 * s.f_frsize as f64)),
        None => {
            crate::warn(
                host,
                "disk_total_space",
                &crate::main::streams::last_strerror(),
            )?;
            Ok(Value::Bool(false))
        }
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("file_exists", fn_file_exists),
            ("getcwd", fn_getcwd),
            ("file_get_contents", fn_file_get_contents),
            ("file_put_contents", fn_file_put_contents),
            ("unlink", fn_unlink),
            ("rmdir", fn_rmdir),
            ("rename", fn_rename),
            ("touch", fn_touch),
            ("chmod", fn_chmod),
            ("chdir", fn_chdir),
            ("umask", fn_umask),
            ("clearstatcache", fn_clearstatcache),
            ("readlink", fn_readlink),
            ("symlink", fn_symlink),
            ("disk_free_space", fn_disk_free_space),
            ("diskfreespace", fn_disk_free_space),
            ("disk_total_space", fn_disk_total_space),
        ],
    );
}
