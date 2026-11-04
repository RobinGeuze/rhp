//! The `posix` extension (php-src `ext/posix/posix.c`): process ids, user and
//! group database lookups, `posix_kill`, `posix_isatty`, `posix_uname`, and the
//! `posix_get_last_error`/`posix_strerror` error channel — thin wrappers over
//! the real libc calls.

use std::collections::HashMap;
use std::ffi::{CStr, CString};

use rhp_semantics::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::proc_fns::{getpwnam, getpwuid, zpp_long, zpp_string, Passwd};
use crate::{Builtin, Host, Thrown};

/// php's `PHP_POSIX_CHECK_PID`: ValueError unless `lower <= pid <= upper`.
fn check_pid(pid: i64, arg: usize, fname: &str, pname: &str, lower: i64) -> Result<(), Thrown> {
    let upper = i32::MAX as i64;
    if pid < lower || pid > upper {
        // php passes the C `int` bounds to a ZEND_LONG_FMT ("%lld") format, so
        // INT_MIN prints as its zero-extended 32-bit pattern (2147483648).
        let shown = lower as i32 as u32;
        return Err(Thrown::error(
            "ValueError",
            format!("{fname}(): Argument #{arg} (${pname}) must be between {shown} and {upper}"),
        ));
    }
    Ok(())
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// The module's `last_error` (posix_get_last_error).
struct PosixState {
    last_error: i32,
}

fn set_last_error(host: &mut dyn Host, e: i32) {
    if let Some(st) = host.module_state("posix", || Box::new(PosixState { last_error: 0 })) {
        if let Some(s) = st.downcast_mut::<PosixState>() {
            s.last_error = e;
        }
    }
}

fn last_error(host: &mut dyn Host) -> i32 {
    host.module_state("posix", || Box::new(PosixState { last_error: 0 }))
        .and_then(|st| st.downcast_mut::<PosixState>().map(|s| s.last_error))
        .unwrap_or(0)
}

fn assoc(pairs: Vec<(&str, Value)>) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in pairs {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    }
    Value::Array(a)
}

fn passwd_array(p: Passwd) -> Value {
    assoc(vec![
        ("name", Value::str(p.name)),
        ("passwd", Value::str(p.passwd)),
        ("uid", Value::Int(p.uid as i64)),
        ("gid", Value::Int(p.gid as i64)),
        ("gecos", Value::str(p.gecos)),
        ("dir", Value::str(p.dir)),
        ("shell", Value::str(p.shell)),
    ])
}

unsafe fn cbytes(p: *const libc::c_char) -> Vec<u8> {
    if p.is_null() {
        Vec::new()
    } else {
        CStr::from_ptr(p).to_bytes().to_vec()
    }
}

/// php's `php_posix_group_to_array`.
unsafe fn group_array(g: &libc::group) -> Value {
    let mut members = PhpArray::new();
    let mut m = g.gr_mem;
    if !m.is_null() {
        while !(*m).is_null() {
            members.push(Value::str(cbytes(*m)));
            m = m.add(1);
        }
    }
    assoc(vec![
        ("name", Value::str(cbytes(g.gr_name))),
        ("passwd", Value::str(cbytes(g.gr_passwd))),
        ("members", Value::Array(members)),
        ("gid", Value::Int(g.gr_gid as i64)),
    ])
}

macro_rules! id_fn {
    ($name:ident, $call:expr) => {
        fn $name(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
            Ok(Value::Int(unsafe { $call } as i64))
        }
    };
}

id_fn!(fn_getpid, libc::getpid());
id_fn!(fn_getppid, libc::getppid());
id_fn!(fn_getuid, libc::getuid());
id_fn!(fn_geteuid, libc::geteuid());
id_fn!(fn_getgid, libc::getgid());
id_fn!(fn_getegid, libc::getegid());
id_fn!(fn_getpgrp, libc::getpgrp());

fn fn_getsid(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pid = zpp_long(host, args, 0, "posix_getsid", "process_id")?;
    check_pid(pid, 1, "posix_getsid", "process_id", 0)?;
    let r = unsafe { libc::getsid(pid as libc::pid_t) };
    if r < 0 {
        set_last_error(host, errno());
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(r as i64))
}

fn fn_getpgid(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pid = zpp_long(host, args, 0, "posix_getpgid", "process_id")?;
    let r = unsafe { libc::getpgid(pid as libc::pid_t) };
    if r < 0 {
        set_last_error(host, errno());
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(r as i64))
}

fn fn_kill(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pid = zpp_long(host, args, 0, "posix_kill", "process_id")?;
    let sig = zpp_long(host, args, 1, "posix_kill", "signal")?;
    check_pid(pid, 1, "posix_kill", "process_id", i32::MIN as i64)?;
    if unsafe { libc::kill(pid as libc::pid_t, sig as i32) } < 0 {
        set_last_error(host, errno());
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

/// The fd named by an int or a stream resource argument.
fn fd_of(host: &mut dyn Host, v: &Value) -> Option<i64> {
    match v {
        Value::Resource(_) => host.stream_select_fd(v).map(|(fd, _)| fd as i64),
        Value::Int(n) => Some(*n),
        Value::Bool(b) => Some(*b as i64),
        Value::Float(f) => Some(*f as i64),
        Value::Str(s) => conv::numeric_string(s.as_bytes()).map(|n| conv::to_int(&n.to_value())),
        _ => None,
    }
}

fn fn_isatty(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    let Some(fd) = fd_of(host, &v) else {
        return Ok(Value::Bool(false));
    };
    if fd < 0 || fd > i32::MAX as i64 {
        set_last_error(host, libc::EBADF);
        return Ok(Value::Bool(false));
    }
    if unsafe { libc::isatty(fd as i32) } == 1 {
        Ok(Value::Bool(true))
    } else {
        set_last_error(host, errno());
        Ok(Value::Bool(false))
    }
}

fn fn_ttyname(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    let fd = match (&v, fd_of(host, &v)) {
        (_, Some(fd)) => fd,
        (Value::Resource(_), None) => return Ok(Value::Bool(false)),
        // php warns, then falls back to zval_get_long().
        (_, None) => conv::to_int(&v),
    };
    if !matches!(v, Value::Resource(_)) && (fd < 0 || fd > i32::MAX as i64) {
        set_last_error(host, libc::EBADF);
        return Ok(Value::Bool(false));
    }
    let mut buf = vec![0 as libc::c_char; 256];
    let r = unsafe { libc::ttyname_r(fd as i32, buf.as_mut_ptr(), buf.len()) };
    if r != 0 {
        set_last_error(host, r);
        return Ok(Value::Bool(false));
    }
    Ok(Value::str(unsafe { cbytes(buf.as_ptr()) }))
}

fn fn_getpwuid(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let uid = zpp_long(host, args, 0, "posix_getpwuid", "user_id")?;
    match getpwuid(uid as libc::uid_t) {
        Some(p) => Ok(passwd_array(p)),
        None => {
            set_last_error(host, errno());
            Ok(Value::Bool(false))
        }
    }
}

fn fn_getpwnam(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = zpp_string(host, args, 0, "posix_getpwnam", "username")?;
    match getpwnam(&name) {
        Some(p) => Ok(passwd_array(p)),
        None => {
            set_last_error(host, errno());
            Ok(Value::Bool(false))
        }
    }
}

fn fn_getgrgid(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let gid = zpp_long(host, args, 0, "posix_getgrgid", "group_id")? as libc::gid_t;
    unsafe {
        let mut g: libc::group = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 65536];
        let mut res: *mut libc::group = std::ptr::null_mut();
        let r = libc::getgrgid_r(gid, &mut g, buf.as_mut_ptr(), buf.len(), &mut res);
        if r != 0 || res.is_null() {
            set_last_error(host, r);
            return Ok(Value::Bool(false));
        }
        Ok(group_array(&g))
    }
}

fn fn_getgrnam(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = zpp_string(host, args, 0, "posix_getgrnam", "name")?;
    let Ok(c) = CString::new(name) else {
        return Ok(Value::Bool(false));
    };
    unsafe {
        let mut g: libc::group = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 65536];
        let mut res: *mut libc::group = std::ptr::null_mut();
        let r = libc::getgrnam_r(c.as_ptr(), &mut g, buf.as_mut_ptr(), buf.len(), &mut res);
        if r != 0 || res.is_null() {
            set_last_error(host, r);
            return Ok(Value::Bool(false));
        }
        Ok(group_array(&g))
    }
}

fn fn_getgroups(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut gids = vec![0 as libc::gid_t; 1024];
    let n = unsafe { libc::getgroups(gids.len() as i32, gids.as_mut_ptr()) };
    if n < 0 {
        set_last_error(host, errno());
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    for g in &gids[..n as usize] {
        a.push(Value::Int(*g as i64));
    }
    Ok(Value::Array(a))
}

fn fn_getlogin(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let p = unsafe { libc::getlogin() };
    if p.is_null() {
        set_last_error(host, errno());
        return Ok(Value::Bool(false));
    }
    Ok(Value::str(unsafe { cbytes(p) }))
}

fn fn_uname(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut u) < 0 {
            set_last_error(host, errno());
            return Ok(Value::Bool(false));
        }
        #[allow(unused_mut)]
        let mut pairs = vec![
            ("sysname", Value::str(cbytes(u.sysname.as_ptr()))),
            ("nodename", Value::str(cbytes(u.nodename.as_ptr()))),
            ("release", Value::str(cbytes(u.release.as_ptr()))),
            ("version", Value::str(cbytes(u.version.as_ptr()))),
            ("machine", Value::str(cbytes(u.machine.as_ptr()))),
        ];
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        pairs.push(("domainname", Value::str(cbytes(u.domainname.as_ptr()))));
        Ok(assoc(pairs))
    }
}

fn fn_getcwd(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    use std::os::unix::ffi::OsStrExt;
    match std::env::current_dir() {
        Ok(p) => Ok(Value::str(p.as_os_str().as_bytes().to_vec())),
        Err(e) => {
            set_last_error(host, e.raw_os_error().unwrap_or(0));
            Ok(Value::Bool(false))
        }
    }
}

fn fn_access(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = zpp_string(host, args, 0, "posix_access", "filename")?;
    if path.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "posix_access(): Argument #1 ($filename) must not contain any null bytes",
        ));
    }
    let mode = match args.get(1) {
        Some(_) => zpp_long(host, args, 1, "posix_access", "flags")?,
        None => 0,
    };
    let Ok(c) = CString::new(path) else {
        return Ok(Value::Bool(false));
    };
    if unsafe { libc::access(c.as_ptr(), mode as i32) } != 0 {
        set_last_error(host, errno());
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_get_last_error(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(last_error(host) as i64))
}

fn fn_strerror(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let e = zpp_long(host, args, 0, "posix_strerror", "error_code")? as i32;
    let s = unsafe { cbytes(libc::strerror(e)) };
    Ok(Value::str(s))
}

pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("POSIX_F_OK", Value::Int(libc::F_OK as i64)),
        ("POSIX_X_OK", Value::Int(libc::X_OK as i64)),
        ("POSIX_W_OK", Value::Int(libc::W_OK as i64)),
        ("POSIX_R_OK", Value::Int(libc::R_OK as i64)),
        ("POSIX_S_IFREG", Value::Int(libc::S_IFREG as i64)),
        ("POSIX_S_IFCHR", Value::Int(libc::S_IFCHR as i64)),
        ("POSIX_S_IFBLK", Value::Int(libc::S_IFBLK as i64)),
        ("POSIX_S_IFIFO", Value::Int(libc::S_IFIFO as i64)),
        ("POSIX_S_IFSOCK", Value::Int(libc::S_IFSOCK as i64)),
    ]
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
    put(m, "posix_getpid", fn_getpid, 0, Some(0));
    put(m, "posix_getppid", fn_getppid, 0, Some(0));
    put(m, "posix_getuid", fn_getuid, 0, Some(0));
    put(m, "posix_geteuid", fn_geteuid, 0, Some(0));
    put(m, "posix_getgid", fn_getgid, 0, Some(0));
    put(m, "posix_getegid", fn_getegid, 0, Some(0));
    put(m, "posix_getpgrp", fn_getpgrp, 0, Some(0));
    put(m, "posix_getsid", fn_getsid, 1, Some(1));
    put(m, "posix_getpgid", fn_getpgid, 1, Some(1));
    put(m, "posix_kill", fn_kill, 2, Some(2));
    put(m, "posix_isatty", fn_isatty, 1, Some(1));
    put(m, "posix_ttyname", fn_ttyname, 1, Some(1));
    put(m, "posix_getpwuid", fn_getpwuid, 1, Some(1));
    put(m, "posix_getpwnam", fn_getpwnam, 1, Some(1));
    put(m, "posix_getgrgid", fn_getgrgid, 1, Some(1));
    put(m, "posix_getgrnam", fn_getgrnam, 1, Some(1));
    put(m, "posix_getgroups", fn_getgroups, 0, Some(0));
    put(m, "posix_getlogin", fn_getlogin, 0, Some(0));
    put(m, "posix_uname", fn_uname, 0, Some(0));
    put(m, "posix_getcwd", fn_getcwd, 0, Some(0));
    put(m, "posix_access", fn_access, 1, Some(2));
    put(m, "posix_get_last_error", fn_get_last_error, 0, Some(0));
    put(m, "posix_errno", fn_get_last_error, 0, Some(0));
    put(m, "posix_strerror", fn_strerror, 1, Some(1));
}
