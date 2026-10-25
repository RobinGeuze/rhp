//! Filesystem & path builtins (php `ext/standard` file functions): existence
//! checks, `basename`/`dirname`, `realpath`/`getcwd`, and whole-file read/write.
//! Local paths only (no stream wrappers / URLs yet).

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

/// `libc::access(path, mode)` == 0 (R_OK/W_OK/X_OK against the effective uid/gid),
/// exactly what php's `is_readable`/`is_writable`/`is_executable` do.
fn access_ok(path: &[u8], mode: libc::c_int) -> bool {
    match CString::new(path.to_vec()) {
        Ok(c) => unsafe { libc::access(c.as_ptr(), mode) == 0 },
        Err(_) => false,
    }
}

/// The full `st_mode` (type bits + permission bits) from `libc::stat` (follows
/// symlinks), or `None` if the path cannot be stat'd.
fn stat_mode(path: &[u8]) -> Option<u32> {
    let c = CString::new(path.to_vec()).ok()?;
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::stat(c.as_ptr(), &mut st) == 0 {
            Some(st.st_mode as u32)
        } else {
            None
        }
    }
}

/// The full `st_mode` from `libc::lstat` (does NOT follow symlinks), used by
/// `filetype`/`is_link` so a symlink reports as a link.
fn lstat_mode(path: &[u8]) -> Option<u32> {
    let c = CString::new(path.to_vec()).ok()?;
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::lstat(c.as_ptr(), &mut st) == 0 {
            Some(st.st_mode as u32)
        } else {
            None
        }
    }
}

fn fn_file_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_exists(&path_arg(args, 0))))
}

fn fn_is_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_stat(&path_arg(args, 0)).is_some_and(|s| !s.is_dir)))
}

fn fn_is_dir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_stat(&path_arg(args, 0)).is_some_and(|s| s.is_dir)))
}

fn fn_is_readable(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(access_ok(&path_arg(args, 0), libc::R_OK)))
}

/// `basename(path, suffix="")` — the trailing name component, byte-wise, matching
/// php (trailing slashes ignored; an exact suffix stripped unless it's the whole).
fn fn_basename(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_arg(args, 0);
    let suffix = args.get(1).map(|v| conv::to_string_bytes(&deref(v))).unwrap_or_default();
    // drop trailing '/'
    let end = path.iter().rposition(|&b| b != b'/').map(|i| i + 1).unwrap_or(0);
    let trimmed = &path[..end];
    let start = trimmed.iter().rposition(|&b| b == b'/').map(|i| i + 1).unwrap_or(0);
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
    let end = path.iter().rposition(|&b| b != b'/').map(|i| i + 1).unwrap_or(0);
    let trimmed = &path[..end];
    let base_start = trimmed.iter().rposition(|&b| b == b'/').map(|i| i + 1).unwrap_or(0);
    let basename = &trimmed[base_start..];
    // extension / filename: split basename on the LAST '.', but a leading dot
    // (dotfile with no other dot) has no extension.
    let (filename, extension): (&[u8], Option<&[u8]>) = match basename.iter().rposition(|&b| b == b'.') {
        None => (basename, None),
        Some(dot) => (&basename[..dot], Some(&basename[dot + 1..])),
    };

    let dir = dirname_once(&path);
    // php: dirname is "." for a bare name only when the input had no slash.
    let dirname: Vec<u8> = if path.contains(&b'/') { dir } else { b".".to_vec() };

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
    let levels = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(1).max(1);
    for _ in 0..levels {
        path = dirname_once(&path);
    }
    Ok(Value::str(path))
}

fn dirname_once(path: &[u8]) -> Vec<u8> {
    // drop trailing slashes
    let end = path.iter().rposition(|&b| b != b'/').map(|i| i + 1).unwrap_or(0);
    let trimmed = &path[..end];
    match trimmed.iter().rposition(|&b| b == b'/') {
        None => b".".to_vec(),
        Some(0) => b"/".to_vec(),
        Some(i) => trimmed[..i].to_vec(),
    }
}

fn fn_realpath(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match std::fs::canonicalize(&*as_str(&path_arg(args, 0))) {
        Ok(p) => Ok(Value::str(p.to_string_lossy().into_owned().into_bytes())),
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn fn_getcwd(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    match std::env::current_dir() {
        Ok(p) => Ok(Value::str(p.to_string_lossy().into_owned().into_bytes())),
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn fn_file_get_contents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match host.stream_get_contents(&path_arg(args, 0)) {
        Some(bytes) => Ok(Value::str(bytes)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_file_put_contents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = conv::to_string_bytes(&deref(&args[1]));
    match host.stream_put_contents(&path_arg(args, 0), &data) {
        Some(n) => Ok(Value::Int(n as i64)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_unlink(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(std::fs::remove_file(&*as_str(&path_arg(args, 0))).is_ok()))
}

fn fn_sys_get_temp_dir(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(std::env::temp_dir().to_string_lossy().into_owned().into_bytes()))
}

fn fn_is_writable(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(access_ok(&path_arg(args, 0), libc::W_OK)))
}

fn fn_is_executable(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(access_ok(&path_arg(args, 0), libc::X_OK)))
}

fn fn_mkdir(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = as_str(&path_arg(args, 0)).into_owned();
    let recursive = args.get(2).map(|v| conv::to_bool(&deref(v))).unwrap_or(false);
    let r = if recursive { std::fs::create_dir_all(&path) } else { std::fs::create_dir(&path) };
    Ok(Value::Bool(r.is_ok()))
}

fn fn_rmdir(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(std::fs::remove_dir(&*as_str(&path_arg(args, 0))).is_ok()))
}

fn fn_rename(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let from = as_str(&path_arg(args, 0)).into_owned();
    let to = as_str(&path_arg(args, 1)).into_owned();
    Ok(Value::Bool(std::fs::rename(&from, &to).is_ok()))
}

fn fn_copy(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let from = as_str(&path_arg(args, 0)).into_owned();
    let to = as_str(&path_arg(args, 1)).into_owned();
    Ok(Value::Bool(std::fs::copy(&from, &to).is_ok()))
}

fn fn_filemtime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match host.stream_stat(&path_arg(args, 0)) {
        Some(s) => Ok(Value::Int(s.mtime)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_filesize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match host.stream_stat(&path_arg(args, 0)) {
        Some(s) => Ok(Value::Int(s.size as i64)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_touch(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_arg(args, 0);
    let path_str = as_str(&path).into_owned();
    // Create the file if absent (php's touch creates an empty file).
    if !std::path::Path::new(&path_str).exists()
        && std::fs::OpenOptions::new().create(true).append(true).open(&path_str).is_err()
    {
        return Ok(Value::Bool(false));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // mtime: given or now; atime: given or the (effective) mtime — php's rule.
    let given = |i: usize| {
        args.get(i).map(|v| deref(v)).filter(|v| !matches!(v, Value::Null)).map(|v| conv::to_int(&v))
    };
    let mtime = given(1).unwrap_or(now);
    let atime = given(2).unwrap_or(mtime);
    let ok = match CString::new(path) {
        Ok(c) => {
            let times = [
                libc::timeval { tv_sec: atime as libc::time_t, tv_usec: 0 },
                libc::timeval { tv_sec: mtime as libc::time_t, tv_usec: 0 },
            ];
            unsafe { libc::utimes(c.as_ptr(), times.as_ptr()) == 0 }
        }
        Err(_) => false,
    };
    Ok(Value::Bool(ok))
}

fn fn_true1(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

fn fn_fileperms(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // php returns the full st_mode (type bits + permissions), e.g. 0o100644 for a
    // regular 0644 file; false if the path cannot be stat'd.
    match stat_mode(&path_arg(args, 0)) {
        Some(mode) => Ok(Value::Int(mode as i64)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_chmod(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mode = conv::to_int(&deref(&args[1])) as libc::mode_t;
    let ok = match CString::new(path_arg(args, 0)) {
        Ok(c) => unsafe { libc::chmod(c.as_ptr(), mode) == 0 },
        Err(_) => false,
    };
    Ok(Value::Bool(ok))
}

fn fn_is_link(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let is_link = lstat_mode(&path_arg(args, 0))
        .is_some_and(|m| m & libc::S_IFMT as u32 == libc::S_IFLNK as u32);
    Ok(Value::Bool(is_link))
}

fn fn_readlink(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match std::fs::read_link(&*as_str(&path_arg(args, 0))) {
        Ok(p) => Ok(Value::str(p.to_string_lossy().into_owned().into_bytes())),
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn fn_symlink(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let target = as_str(&path_arg(args, 0)).into_owned();
    let link = as_str(&path_arg(args, 1)).into_owned();
    Ok(Value::Bool(std::os::unix::fs::symlink(&target, &link).is_ok()))
}

/// `filetype` — php reports via `lstat`, so a symlink is "link".
fn fn_filetype(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match lstat_mode(&path_arg(args, 0)) {
        Some(m) => {
            let t = m & libc::S_IFMT as u32;
            let s: &[u8] = if t == libc::S_IFLNK as u32 {
                b"link"
            } else if t == libc::S_IFDIR as u32 {
                b"dir"
            } else if t == libc::S_IFREG as u32 {
                b"file"
            } else if t == libc::S_IFCHR as u32 {
                b"char"
            } else if t == libc::S_IFBLK as u32 {
                b"block"
            } else if t == libc::S_IFIFO as u32 {
                b"fifo"
            } else if t == libc::S_IFSOCK as u32 {
                b"socket"
            } else {
                b"unknown"
            };
            Ok(Value::str(s.to_vec()))
        }
        None => Ok(Value::Bool(false)),
    }
}

fn statvfs_of(path: &[u8]) -> Option<libc::statvfs> {
    let c = CString::new(path.to_vec()).ok()?;
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) == 0 {
            Some(st)
        } else {
            None
        }
    }
}

/// `disk_free_space` — `f_bavail * f_frsize` as a float (php's formula).
fn fn_disk_free_space(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match statvfs_of(&path_arg(args, 0)) {
        Some(s) => Ok(Value::Float(s.f_bavail as f64 * s.f_frsize as f64)),
        None => Ok(Value::Bool(false)),
    }
}

/// `disk_total_space` — `f_blocks * f_frsize` as a float (php's formula).
fn fn_disk_total_space(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match statvfs_of(&path_arg(args, 0)) {
        Some(s) => Ok(Value::Float(s.f_blocks as f64 * s.f_frsize as f64)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_scandir(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = as_str(&path_arg(args, 0)).into_owned();
    match std::fs::read_dir(&path) {
        Ok(rd) => {
            let mut names: Vec<Vec<u8>> = vec![b".".to_vec(), b"..".to_vec()];
            for e in rd.flatten() {
                names.push(e.file_name().to_string_lossy().into_owned().into_bytes());
            }
            names.sort();
            let mut a = rhp_values::PhpArray::new();
            for n in names {
                a.push(Value::str(n));
            }
            Ok(Value::Array(a))
        }
        Err(_) => Ok(Value::Bool(false)),
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
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    }
    put(m, "file_exists", fn_file_exists, 1, Some(1));
    put(m, "is_file", fn_is_file, 1, Some(1));
    put(m, "is_dir", fn_is_dir, 1, Some(1));
    put(m, "is_readable", fn_is_readable, 1, Some(1));
    put(m, "basename", fn_basename, 1, Some(2));
    put(m, "dirname", fn_dirname, 1, Some(2));
    put(m, "pathinfo", fn_pathinfo, 1, Some(2));
    put(m, "realpath", fn_realpath, 1, Some(1));
    put(m, "getcwd", fn_getcwd, 0, Some(0));
    put(m, "file_get_contents", fn_file_get_contents, 1, Some(1));
    put(m, "file_put_contents", fn_file_put_contents, 2, Some(3));
    put(m, "unlink", fn_unlink, 1, Some(1));
    put(m, "sys_get_temp_dir", fn_sys_get_temp_dir, 0, Some(0));
    put(m, "is_writable", fn_is_writable, 1, Some(1));
    put(m, "is_writeable", fn_is_writable, 1, Some(1));
    put(m, "is_executable", fn_is_executable, 1, Some(1));
    put(m, "mkdir", fn_mkdir, 1, Some(4));
    put(m, "rmdir", fn_rmdir, 1, Some(2));
    put(m, "rename", fn_rename, 2, Some(3));
    put(m, "copy", fn_copy, 2, Some(3));
    put(m, "filemtime", fn_filemtime, 1, Some(1));
    put(m, "filesize", fn_filesize, 1, Some(1));
    put(m, "fileperms", fn_fileperms, 1, Some(1));
    put(m, "touch", fn_touch, 1, Some(3));
    put(m, "chmod", fn_chmod, 2, Some(2));
    put(m, "chdir", fn_true1, 1, Some(1));
    put(m, "clearstatcache", fn_true1, 0, Some(2));
    put(m, "scandir", fn_scandir, 1, Some(3));
    put(m, "is_link", fn_is_link, 1, Some(1));
    put(m, "readlink", fn_readlink, 1, Some(1));
    put(m, "symlink", fn_symlink, 2, Some(2));
    put(m, "filetype", fn_filetype, 1, Some(1));
    put(m, "disk_free_space", fn_disk_free_space, 1, Some(1));
    put(m, "diskfreespace", fn_disk_free_space, 1, Some(1));
    put(m, "disk_total_space", fn_disk_total_space, 1, Some(1));
}
