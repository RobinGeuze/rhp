//! ext/standard/filestat.c: the `stat` family, the `is_*` file checks,
//! `touch`, `chmod`/`chown`/`chgrp`, `clearstatcache` and the disk space
//! queries.

use std::ffi::CString;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::ext::standard::file::{os_text, path_arg, path_param, str_arg};
use crate::main::streams::{Located, StatInfo};
use crate::{Host, Thrown};

pub(crate) fn arr_from_stat(s: &StatInfo) -> Value {
    let vals = [
        ("dev", s.dev),
        ("ino", s.ino),
        ("mode", s.mode as i64),
        ("nlink", s.nlink),
        ("uid", s.uid),
        ("gid", s.gid),
        ("rdev", s.rdev),
        ("size", s.size as i64),
        ("atime", s.atime),
        ("mtime", s.mtime),
        ("ctime", s.ctime),
        ("blksize", s.blksize),
        ("blocks", s.blocks),
    ];
    let mut a = PhpArray::new();
    for (_, v) in vals.iter() {
        a.push(Value::Int(*v));
    }
    for (k, v) in vals.iter() {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), Value::Int(*v));
    }
    Value::Array(a)
}

// --- stat family -----------------------------------------------------------------

/// `php_stream_stat_path` of the first argument for builtin `fname`.
fn stat_path(
    host: &mut dyn Host,
    args: &[Value],
    link: bool,
    fname: &str,
) -> Result<Option<StatInfo>, Thrown> {
    let p = str_arg(args, 0);
    if p.is_empty() || p.contains(&0) {
        return Ok(None);
    }
    let url = match crate::locate_url(host, fname, &p)? {
        Located::Plain(local) => local,
        Located::Wrapper(u) => u,
        // the plain url_stat of the whole path, or no wrapper at all
        Located::Unknown(_) => return Ok(crate::main::streams::sys_stat(&p, link)),
        Located::RemoteHost => return Ok(None),
    };
    Ok(if link {
        host.stream_lstat(&url)
    } else {
        host.stream_stat(&url)
    })
}

/// The path php_stat's access checks (`file_exists`, `is_readable`, …)
/// give `access(2)`: the local path, expanded when the wrapper was not
/// found. `Err(url)` for a URL of a registered wrapper; `Ok(None)` when no
/// wrapper fits.
fn access_path(
    host: &mut dyn Host,
    fname: &str,
    p: &[u8],
) -> Result<Result<Option<Vec<u8>>, Vec<u8>>, Thrown> {
    Ok(match crate::locate_url(host, fname, p)? {
        Located::Plain(local) => Ok(Some(local)),
        Located::Unknown(_) => Ok(Some(crate::zend::virtual_cwd::expand_local(p))),
        Located::RemoteHost => Ok(None),
        Located::Wrapper(u) => Err(u),
    })
}

/// php_stat's failure warning (not for the existence checks).
fn stat_failed(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    link: bool,
) -> Result<Value, Thrown> {
    let p = str_arg(args, 0);
    if p.contains(&0) {
        crate::warn(host, fname, "Filename contains null byte")?;
    } else if !p.is_empty() {
        crate::warn(
            host,
            fname,
            &[
                if link {
                    b"Lstat failed for ".as_slice()
                } else {
                    b"stat failed for "
                },
                &p,
            ]
            .concat(),
        )?;
    }
    Ok(Value::Bool(false))
}

fn fn_stat(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match stat_path(host, args, false, "stat")? {
        Some(s) => Ok(arr_from_stat(&s)),
        None => stat_failed(host, args, "stat", false),
    }
}

fn fn_lstat(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match stat_path(host, args, true, "lstat")? {
        Some(s) => Ok(arr_from_stat(&s)),
        None => stat_failed(host, args, "lstat", true),
    }
}

macro_rules! stat_field_fn {
    ($name:ident, $fname:literal, $link:expr, $warn:expr, |$s:ident| $e:expr) => {
        fn $name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            match stat_path(host, args, $link, $fname)? {
                Some($s) => Ok($e),
                None if $warn => stat_failed(host, args, $fname, $link),
                None => Ok(Value::Bool(false)),
            }
        }
    };
}

stat_field_fn!(fn_fileinode, "fileinode", false, true, |s| Value::Int(
    s.ino
));
stat_field_fn!(fn_fileowner, "fileowner", false, true, |s| Value::Int(
    s.uid
));
stat_field_fn!(fn_filegroup, "filegroup", false, true, |s| Value::Int(
    s.gid
));
stat_field_fn!(fn_fileatime, "fileatime", false, true, |s| Value::Int(
    s.atime
));
stat_field_fn!(fn_filemtime, "filemtime", false, true, |s| Value::Int(
    s.mtime
));
stat_field_fn!(fn_filectime, "filectime", false, true, |s| Value::Int(
    s.ctime
));
stat_field_fn!(fn_filesize, "filesize", false, true, |s| Value::Int(
    s.size as i64
));
stat_field_fn!(fn_fileperms, "fileperms", false, true, |s| Value::Int(
    s.mode as i64
));
stat_field_fn!(fn_is_link, "is_link", true, false, |s| Value::Bool(
    s.is_link()
));

fn exists_check(
    host: &mut dyn Host,
    args: &[Value],
    link: bool,
    fname: &str,
) -> Result<Option<StatInfo>, Thrown> {
    stat_path(host, args, link, fname)
}

fn fn_is_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        exists_check(host, args, false, "is_file")?.is_some_and(|s| s.is_regular),
    ))
}

fn fn_is_dir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        exists_check(host, args, false, "is_dir")?.is_some_and(|s| s.is_dir),
    ))
}

#[allow(clippy::unnecessary_cast)]
fn fn_filetype(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(s) = stat_path(host, args, true, "filetype")? else {
        return stat_failed(host, args, "filetype", true);
    };
    let t = s.mode & libc::S_IFMT as u32;
    let name: &str = if t == libc::S_IFLNK as u32 {
        "link"
    } else if t == libc::S_IFIFO as u32 {
        "fifo"
    } else if t == libc::S_IFCHR as u32 {
        "char"
    } else if t == libc::S_IFDIR as u32 {
        "dir"
    } else if t == libc::S_IFBLK as u32 {
        "block"
    } else if t == libc::S_IFREG as u32 {
        "file"
    } else if t == libc::S_IFSOCK as u32 {
        "socket"
    } else {
        "unknown"
    };
    Ok(Value::str(name))
}

/// is_readable / is_writable / is_executable: `access(2)` for plain files;
/// for other wrappers php checks the stat mode bits against the uid/gid.
fn access_check(
    host: &mut dyn Host,
    args: &[Value],
    amode: libc::c_int,
    fname: &str,
) -> Result<bool, Thrown> {
    let p = str_arg(args, 0);
    if p.is_empty() || p.contains(&0) {
        return Ok(false);
    }
    let url = match access_path(host, fname, &p)? {
        Ok(local) => {
            return Ok(local.is_some_and(|l| crate::zend::virtual_cwd::access(&l, amode)));
        }
        Err(url) => url,
    };
    let Some(s) = host.stream_stat(&url) else {
        return Ok(false);
    };
    // php_stat (filestat.c): owner bits if we own it, group bits for our
    // primary or any supplementary group, else "other" bits. The root
    // shortcut only applies to the plain-files wrapper (handled above).
    let (uid, gid) = unsafe { (libc::getuid() as i64, libc::getgid() as i64) };
    let in_group = |g: i64| {
        if g == gid {
            return true;
        }
        let n = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
        if n <= 0 {
            return false;
        }
        let mut gids: Vec<libc::gid_t> = vec![0; n as usize];
        let n = unsafe { libc::getgroups(n, gids.as_mut_ptr()) };
        gids.iter().take(n.max(0) as usize).any(|&x| x as i64 == g)
    };
    let shift = if s.uid == uid {
        6
    } else if in_group(s.gid) {
        3
    } else {
        0
    };
    let bit = match amode {
        libc::R_OK => 4,
        libc::W_OK => 2,
        _ => 1,
    };
    Ok(s.mode & (bit << shift) != 0)
}

fn fn_is_readable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(access_check(
        host,
        args,
        libc::R_OK,
        "is_readable",
    )?))
}
fn fn_is_writable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(access_check(
        host,
        args,
        libc::W_OK,
        "is_writable",
    )?))
}
fn fn_is_executable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(access_check(
        host,
        args,
        libc::X_OK,
        "is_executable",
    )?))
}

/// chown / chgrp / lchown / lchgrp: a user/group name or id; plain paths.
fn do_chown(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    group: bool,
    link: bool,
) -> Result<Value, Thrown> {
    let path = path_param(args, 0, fname, "filename")?;
    let who = args.get(1).map(deref).unwrap_or(Value::Null);
    let id: u32 = match &who {
        Value::Int(n) => *n as u32,
        Value::Str(s) => {
            let Ok(c) = CString::new(s.as_bytes().to_vec()) else {
                return Ok(Value::Bool(false));
            };
            let found = unsafe {
                if group {
                    let g = libc::getgrnam(c.as_ptr());
                    (!g.is_null()).then(|| (*g).gr_gid)
                } else {
                    let p = libc::getpwnam(c.as_ptr());
                    (!p.is_null()).then(|| (*p).pw_uid)
                }
            };
            match found {
                Some(id) => id,
                None => {
                    let what = if group { "group" } else { "user" };
                    crate::warn(
                        host,
                        fname,
                        &[
                            format!("Unable to find {what} ID for ").as_bytes(),
                            s.as_bytes(),
                        ]
                        .concat(),
                    )?;
                    return Ok(Value::Bool(false));
                }
            }
        }
        other => {
            let what = if group { "group" } else { "user" };
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #2 (${what}) must be of type string|int, {} given",
                crate::zend::zpp::zval_value_name(host, other)
            )));
        }
    };
    let Some(c) = crate::zend::virtual_cwd::c_path(crate::main::streams::file_path(&path)) else {
        return Ok(Value::Bool(false));
    };
    let (uid, gid) = if group {
        (u32::MAX, id)
    } else {
        (id, u32::MAX)
    };
    let r = unsafe {
        if link {
            libc::lchown(c.as_ptr(), uid, gid)
        } else {
            libc::chown(c.as_ptr(), uid, gid)
        }
    };
    if r != 0 {
        let e = std::io::Error::last_os_error();
        crate::warn(
            host,
            fname,
            &crate::main::streams::strerror(e.raw_os_error().unwrap_or(0)),
        )?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_chown(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    do_chown(host, args, "chown", false, false)
}
fn fn_chgrp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    do_chown(host, args, "chgrp", true, false)
}
fn fn_lchown(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    do_chown(host, args, "lchown", false, true)
}
fn fn_lchgrp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    do_chown(host, args, "lchgrp", true, true)
}

// --- local-fs metadata via libc (matches php's own ext/standard/filestat.c) ---

fn fn_file_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let p = path_arg(args, 0);
    if p.is_empty() || p.contains(&0) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(match access_path(host, "file_exists", &p)? {
        Ok(Some(local)) => host.stream_exists(&local),
        Ok(None) => false,
        Err(url) => host.stream_exists(&url),
    }))
}

fn fn_touch(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = crate::ext::standard::file::path_param(args, 0, "touch", "filename")?;
    if path.is_empty() {
        return Ok(Value::Bool(false));
    }
    let null_at = |i: usize| matches!(args.get(i).map(deref), None | Some(Value::Null));
    if null_at(1) && !null_at(2) {
        return Err(crate::zend::zpp::value_error(
            "touch",
            2,
            "mtime",
            "cannot be null when argument #3 ($atime) is an integer",
        ));
    }
    // Create the file if absent (php's touch creates an empty file).
    if !crate::zend::virtual_cwd::os_path(&path).exists() {
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND | libc::O_CLOEXEC;
        if let Err(e) = crate::zend::virtual_cwd::open(&path, flags, 0o666) {
            let msg = [
                b"Unable to create file ",
                &path[..],
                b" because ",
                os_text(&e).as_bytes(),
            ]
            .concat();
            crate::warn(host, "touch", &msg)?;
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
    let path = crate::ext::standard::file::path_param(args, 0, "disk_free_space", "directory")?;
    match statvfs_of(&path) {
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
    let path = crate::ext::standard::file::path_param(args, 0, "disk_total_space", "directory")?;
    match statvfs_of(&path) {
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
            ("stat", fn_stat),
            ("lstat", fn_lstat),
            ("fileinode", fn_fileinode),
            ("fileowner", fn_fileowner),
            ("filegroup", fn_filegroup),
            ("fileatime", fn_fileatime),
            ("filemtime", fn_filemtime),
            ("filectime", fn_filectime),
            ("filesize", fn_filesize),
            ("fileperms", fn_fileperms),
            ("is_link", fn_is_link),
            ("is_file", fn_is_file),
            ("is_dir", fn_is_dir),
            ("filetype", fn_filetype),
            ("is_readable", fn_is_readable),
            ("is_writable", fn_is_writable),
            ("is_writeable", fn_is_writable),
            ("is_executable", fn_is_executable),
            ("chown", fn_chown),
            ("chgrp", fn_chgrp),
            ("lchown", fn_lchown),
            ("lchgrp", fn_lchgrp),
            ("file_exists", fn_file_exists),
            ("touch", fn_touch),
            ("chmod", fn_chmod),
            ("clearstatcache", fn_clearstatcache),
            ("disk_free_space", fn_disk_free_space),
            ("diskfreespace", fn_disk_free_space),
            ("disk_total_space", fn_disk_total_space),
        ],
    );
}
