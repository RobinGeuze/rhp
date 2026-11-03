//! Filesystem & path builtins (php `ext/standard` file functions): existence
//! checks, `basename`/`dirname`/`pathinfo`, `getcwd`/`chdir`, and whole-file
//! read/write through the stream wrappers.

use std::collections::HashMap;
use std::ffi::CString;

use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn path_arg(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

fn as_str(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

// --- local-fs metadata via libc (matches php's own ext/standard/filestat.c) ---

fn fn_file_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_exists(&path_arg(args, 0))))
}

/// `basename(path, suffix="")` — the trailing name component, byte-wise, matching
/// php (trailing slashes ignored; an exact suffix stripped unless it's the whole).
fn fn_basename(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_arg(args, 0);
    let suffix = args
        .get(1)
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_default();
    // drop trailing '/'
    let end = path
        .iter()
        .rposition(|&b| b != b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let trimmed = &path[..end];
    let start = trimmed
        .iter()
        .rposition(|&b| b == b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut base = &trimmed[start..];
    if !suffix.is_empty() && base.len() > suffix.len() && base.ends_with(&suffix[..]) {
        base = &base[..base.len() - suffix.len()];
    }
    Ok(Value::str(base.to_vec()))
}

/// `pathinfo(path, flags=PATHINFO_ALL)` — splits a path into dirname/basename/
/// extension/filename. Returns an assoc array, or a single string when exactly
/// one flag bit is requested.
fn fn_pathinfo(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const DIRNAME: i64 = 1;
    const BASENAME: i64 = 2;
    const EXTENSION: i64 = 4;
    const FILENAME: i64 = 8;
    const ALL: i64 = DIRNAME | BASENAME | EXTENSION | FILENAME;

    let path = path_arg(args, 0);
    let flags = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(ALL);

    // basename (php keeps trailing-slash handling consistent with basename()).
    let end = path
        .iter()
        .rposition(|&b| b != b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let trimmed = &path[..end];
    let base_start = trimmed
        .iter()
        .rposition(|&b| b == b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let basename = &trimmed[base_start..];
    // extension / filename: split basename on the LAST '.', but a leading dot
    // (dotfile with no other dot) has no extension.
    let (filename, extension): (&[u8], Option<&[u8]>) =
        match basename.iter().rposition(|&b| b == b'.') {
            None => (basename, None),
            Some(dot) => (&basename[..dot], Some(&basename[dot + 1..])),
        };

    let dir = dirname_once(&path);
    // php: dirname is "." for a bare name only when the input had no slash.
    let dirname: Vec<u8> = if path.contains(&b'/') {
        dir
    } else {
        b".".to_vec()
    };

    // Single-component request -> return a string.
    if flags != ALL && (flags & (flags - 1)) == 0 {
        let s: Vec<u8> = match flags {
            DIRNAME => dirname,
            BASENAME => basename.to_vec(),
            EXTENSION => extension.unwrap_or(b"").to_vec(),
            FILENAME => filename.to_vec(),
            _ => Vec::new(),
        };
        return Ok(Value::str(s));
    }

    use rhp_values::{ArrayKey as K, PhpArray};
    let mut arr = PhpArray::new();
    if flags & DIRNAME != 0 {
        arr.insert(K::from_bytes(b"dirname"), Value::str(dirname));
    }
    if flags & BASENAME != 0 {
        arr.insert(K::from_bytes(b"basename"), Value::str(basename.to_vec()));
    }
    if flags & EXTENSION != 0 {
        if let Some(ext) = extension {
            arr.insert(K::from_bytes(b"extension"), Value::str(ext.to_vec()));
        }
    }
    if flags & FILENAME != 0 {
        arr.insert(K::from_bytes(b"filename"), Value::str(filename.to_vec()));
    }
    Ok(Value::Array(arr))
}

/// `dirname(path, levels=1)` — matching php: "." when no slash, "/" at root.
fn fn_dirname(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut path = path_arg(args, 0);
    let levels = args
        .get(1)
        .map(|v| conv::to_int(&deref(v)))
        .unwrap_or(1)
        .max(1);
    for _ in 0..levels {
        path = dirname_once(&path);
    }
    Ok(Value::str(path))
}

fn dirname_once(path: &[u8]) -> Vec<u8> {
    // drop trailing slashes
    let end = path
        .iter()
        .rposition(|&b| b != b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let trimmed = &path[..end];
    match trimmed.iter().rposition(|&b| b == b'/') {
        None => b".".to_vec(),
        Some(0) => b"/".to_vec(),
        Some(i) => trimmed[..i].to_vec(),
    }
}

fn fn_getcwd(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    match std::env::current_dir() {
        Ok(p) => Ok(Value::str(p.to_string_lossy().into_owned().into_bytes())),
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn fn_chdir(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let dir = path_arg(args, 0);
    Ok(Value::Bool(
        std::env::set_current_dir(&*as_str(&dir)).is_ok(),
    ))
}

/// `umask(?int $mask = null): int` — get, or set-and-return-old, the file mode
/// creation mask.
fn fn_umask(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(|v| conv::to_int(&deref(v))) {
        Some(mask) => Ok(Value::Int(
            unsafe { libc::umask(mask as libc::mode_t) } as i64
        )),
        None => {
            // Read without changing: set to 0, capture, restore.
            let cur = unsafe { libc::umask(0) };
            unsafe { libc::umask(cur) };
            Ok(Value::Int(cur as i64))
        }
    }
}

/// `php_strip_whitespace($filename)` — php returns the source with comments and
/// whitespace removed (via the tokenizer). Composer only uses it to feed its own
/// comment-stripping classmap scanner, so returning the raw source is correct
/// for that purpose (and `''` only for a missing/empty file).
fn fn_php_strip_whitespace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match host.stream_get_contents(&path_arg(args, 0)) {
        Some(bytes) => Ok(Value::str(bytes)),
        None => Ok(Value::str("")),
    }
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
    // http(s): perform a real request (Composer's non-curl transport). The
    // context (arg 2) carries the http method/headers/body.
    let lower = url.to_ascii_lowercase();
    if lower.starts_with(b"http://") || lower.starts_with(b"https://") {
        // args: (url, use_include_path, context, offset, maxlen)
        let ctx = args.get(2).map(deref).unwrap_or(Value::Null);
        return Ok(crate::http::fetch(host, &url, &ctx));
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
    crate::streams::clear_errno();
    match host.stream_get_contents(&url) {
        Some(bytes) => {
            host.consume_resource_ids(crate::streams::open_resource_ids(&url));
            // php seeks to `$offset` (SEEK_SET, or SEEK_END when negative)
            // before copying at most `$length` bytes. A plain file may be
            // positioned past its end (reads nothing); other wrappers, and
            // any position before the start, fail the seek -> false.
            let len = bytes.len() as i64;
            let start = if offset > 0 {
                if offset > len && crate::streams::split_scheme(&url).0 != b"file" {
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
            if crate::streams::last_errno() == Some(libc::EISDIR) {
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

/// A local path given without a scheme or as `file://`.
fn plain_path(url: &[u8]) -> Option<&std::ffi::OsStr> {
    use std::os::unix::ffi::OsStrExt;
    match crate::streams::split_scheme(url) {
        (b"file", p) => Some(std::ffi::OsStr::from_bytes(
            p.strip_prefix(b"file://").unwrap_or(p),
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
    let size = match std::fs::metadata(path) {
        Ok(m) if m.is_file() && m.len() > 0 => m.len() as i64,
        _ => return true,
    };
    if std::fs::File::open(path).is_err() {
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
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut f = std::fs::File::open(path).ok()?;
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
    // `$data` is a string, an array (concatenated) or a stream resource
    // (copied from its current position to the end).
    let data = match deref(&args[1]) {
        r @ Value::Resource(_) => {
            if host.stream_kind(&r).is_none() {
                return Err(Thrown::type_error(
                    "file_put_contents(): supplied resource is not a valid stream resource",
                ));
            }
            host.stream_read(&r, -1).unwrap_or_default()
        }
        Value::Array(a) => {
            let mut b = Vec::new();
            for (_, v) in a.iter() {
                b.extend_from_slice(&conv::to_string_bytes(v));
            }
            b
        }
        other => conv::to_string_bytes(&other),
    };
    let flags = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if flags & 16 == 0 {
        crate::fs_fns::touch_ctx(host, args, 3);
    }
    const FILE_APPEND: i64 = 8;
    let to_write = if flags & FILE_APPEND != 0 {
        let mut existing = host.stream_get_contents(&path).unwrap_or_default();
        existing.extend_from_slice(&data);
        existing
    } else {
        data.clone()
    };
    crate::streams::clear_errno();
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
    let c = CString::new(path.clone()).unwrap_or_default();
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
        Some(c) => crate::streams::strerror(c),
        None => e.to_string(),
    }
}

fn fn_rmdir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    crate::fs_fns::touch_ctx(host, args, 1);
    let path = path_arg(args, 0);
    if let Err(e) = std::fs::remove_dir(&*as_str(&path)) {
        crate::warn_arg(host, "rmdir", &path, &os_text(&e))?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_rename(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    crate::fs_fns::touch_ctx(host, args, 2);
    let from_b = path_arg(args, 0);
    let to_b = path_arg(args, 1);
    let from = as_str(&from_b).into_owned();
    let to = as_str(&to_b).into_owned();
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
    if !std::path::Path::new(&path_str).exists() {
        if let Err(e) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path_str)
        {
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
    let ok = match CString::new(path) {
        Ok(c) => {
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
        Err(_) => false,
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
    let ok = match CString::new(path_arg(args, 0)) {
        Ok(c) => unsafe { libc::chmod(c.as_ptr(), mode) == 0 },
        Err(_) => false,
    };
    if !ok {
        let e = std::io::Error::last_os_error();
        crate::warn(host, "chmod", &os_text(&e))?;
    }
    Ok(Value::Bool(ok))
}

fn fn_readlink(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match std::fs::read_link(&*as_str(&path_arg(args, 0))) {
        Ok(p) => Ok(Value::str(p.to_string_lossy().into_owned().into_bytes())),
        Err(e) => {
            crate::warn(host, "readlink", &os_text(&e))?;
            Ok(Value::Bool(false))
        }
    }
}

fn fn_symlink(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let target = as_str(&path_arg(args, 0)).into_owned();
    let link = as_str(&path_arg(args, 1)).into_owned();
    match std::os::unix::fs::symlink(&target, &link) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            crate::warn(host, "symlink", &os_text(&e))?;
            Ok(Value::Bool(false))
        }
    }
}

fn statvfs_of(path: &[u8]) -> Option<libc::statvfs> {
    let c = CString::new(path.to_vec()).ok()?;
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) == 0 {
            Some(st)
        } else {
            crate::streams::note_errno();
            None
        }
    }
}

/// `disk_free_space` — `f_bavail * f_frsize` as a float (php's formula).
fn fn_disk_free_space(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match statvfs_of(&path_arg(args, 0)) {
        Some(s) => Ok(Value::Float(s.f_bavail as f64 * s.f_frsize as f64)),
        None => {
            crate::warn(host, "disk_free_space", &crate::streams::last_strerror())?;
            Ok(Value::Bool(false))
        }
    }
}

/// `disk_total_space` — `f_blocks * f_frsize` as a float (php's formula).
fn fn_disk_total_space(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match statvfs_of(&path_arg(args, 0)) {
        Some(s) => Ok(Value::Float(s.f_blocks as f64 * s.f_frsize as f64)),
        None => {
            crate::warn(host, "disk_total_space", &crate::streams::last_strerror())?;
            Ok(Value::Bool(false))
        }
    }
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
    put(m, "file_exists", fn_file_exists, 1, Some(1));
    put(m, "basename", fn_basename, 1, Some(2));
    put(m, "dirname", fn_dirname, 1, Some(2));
    put(m, "pathinfo", fn_pathinfo, 1, Some(2));
    put(m, "getcwd", fn_getcwd, 0, Some(0));
    put(m, "file_get_contents", fn_file_get_contents, 1, Some(5));
    put(m, "file_put_contents", fn_file_put_contents, 2, Some(4));
    put(m, "unlink", fn_unlink, 1, Some(2));
    put(m, "rmdir", fn_rmdir, 1, Some(2));
    put(m, "rename", fn_rename, 2, Some(3));
    put(m, "touch", fn_touch, 1, Some(3));
    put(m, "chmod", fn_chmod, 2, Some(2));
    put(m, "chdir", fn_chdir, 1, Some(1));
    put(m, "umask", fn_umask, 0, Some(1));
    put(
        m,
        "php_strip_whitespace",
        fn_php_strip_whitespace,
        1,
        Some(1),
    );
    put(m, "clearstatcache", fn_clearstatcache, 0, Some(2));
    put(m, "readlink", fn_readlink, 1, Some(1));
    put(m, "symlink", fn_symlink, 2, Some(2));
    put(m, "disk_free_space", fn_disk_free_space, 1, Some(1));
    put(m, "diskfreespace", fn_disk_free_space, 1, Some(1));
    put(m, "disk_total_space", fn_disk_total_space, 1, Some(1));
}
