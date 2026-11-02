//! The part of php's `sockets` extension (php-src `ext/sockets`) that
//! stream-socket libraries use next to stream sockets: `socket_import_stream`
//! (a `Socket` over a socket stream's descriptor), `socket_get_option` /
//! `socket_set_option`, `socket_strerror`, `socket_last_error` /
//! `socket_clear_error`, and the extension's constants. react/socket uses
//! these (when present) to read `SO_ERROR` after a failed async connect.
//!
//! `socket_create` & co. are not implemented, so the extension is not
//! reported as loaded (`extension_loaded('sockets')` stays false).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use rhp_values::{conv, ArrayKey, ObjectData, ObjectHandle, PhpArray, Value};

use crate::proc_fns::{zpp_long, zpp_type_name};
use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// A `Socket` object of ours: the stream it was imported from (kept alive,
/// as php holds a reference) and its last error.
struct Sock {
    obj: Weak<RefCell<ObjectData>>,
    /// Held so the resource stays alive, as php's `zstream` reference does.
    #[allow(dead_code)]
    stream: Value,
    /// php's `bsd_socket`: the stream's descriptor number at import time
    /// (still used after an `fclose` of the stream, as php does).
    fd: i32,
    error: i32,
}

#[derive(Default)]
struct Registry {
    socks: HashMap<usize, Sock>,
    last_error: i32,
}

thread_local! {
    static REG: RefCell<Registry> = RefCell::new(Registry::default());
}

fn key_of(h: &ObjectHandle) -> usize {
    Rc::as_ptr(h) as *const () as usize
}

fn obj_handle(v: &Value) -> Option<ObjectHandle> {
    match v {
        Value::Object(h) => Some(h.clone()),
        Value::Ref(r) => obj_handle(&r.borrow()),
        _ => None,
    }
}

/// The object free hook's part for sockets: drop the entries of `Socket`s
/// that just died, releasing their stream reference then (php frees the
/// socket, and with it its `zstream` reference, at refcount 0) instead of
/// whenever the next import tidies up — by which time, with several runs
/// on one thread, the stream id may name another run's resource.
pub(crate) fn object_freed() {
    let dead: Vec<Sock> = REG
        .try_with(|r| {
            let Ok(mut r) = r.try_borrow_mut() else {
                return Vec::new();
            };
            if r.socks.is_empty() {
                return Vec::new();
            }
            let keys: Vec<usize> = r
                .socks
                .iter()
                .filter(|(_, s)| s.obj.strong_count() == 0)
                .map(|(k, _)| *k)
                .collect();
            keys.iter().filter_map(|k| r.socks.remove(k)).collect()
        })
        .unwrap_or_default();
    // Released outside the registry borrow.
    drop(dead);
}

/// Is `h` a `Socket` of ours?
pub fn is_socket_object(h: &ObjectHandle) -> bool {
    lookup(h).is_some()
}

/// The registry key and descriptor of the `Socket` object `h`.
fn lookup(h: &ObjectHandle) -> Option<(usize, i32)> {
    let k = key_of(h);
    REG.with(|r| {
        r.borrow().socks.get(&k).and_then(|s| {
            let live = s.obj.upgrade()?;
            Rc::ptr_eq(&live, h).then_some((k, s.fd))
        })
    })
}

/// The descriptor behind a `Socket` argument (TypeError for anything else;
/// `nullable` is php's `?Socket`).
fn socket_arg(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    nullable: bool,
) -> Result<(usize, i32), Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    if let Some(found) = obj_handle(&v).and_then(|h| lookup(&h)) {
        return Ok(found);
    }
    Err(Thrown::type_error(format!(
        "{fname}(): Argument #{} ($socket) must be of type {}Socket, {} given",
        i + 1,
        if nullable { "?" } else { "" },
        zpp_type_name(host, &v)
    )))
}

/// php's `PHP_SOCKET_ERROR`: record the error on the socket and globally,
/// and warn unless it is a would-block / in-progress condition.
fn socket_error(
    host: &mut dyn Host,
    key: usize,
    fname: &str,
    msg: &str,
    err: i32,
) -> Result<(), Thrown> {
    REG.with(|r| {
        let mut r = r.borrow_mut();
        r.last_error = err;
        if let Some(s) = r.socks.get_mut(&key) {
            s.error = err;
        }
    });
    if err != libc::EAGAIN && err != libc::EWOULDBLOCK && err != libc::EINPROGRESS {
        crate::warn(
            host,
            fname,
            &format!("{msg} [{err}]: {}", sockets_strerror(err)),
        )?;
    }
    Ok(())
}

/// php's `sockets_strerror`: `strerror`, or the resolver's text for the
/// negative (h_errno) codes.
fn sockets_strerror(e: i32) -> String {
    if e < -10000 {
        let h = -e - 10000;
        let p = unsafe { hstrerror(h) };
        if !p.is_null() {
            return unsafe { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() };
        }
    }
    crate::streams::strerror(e)
}

extern "C" {
    fn hstrerror(err: libc::c_int) -> *const libc::c_char;
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn fn_socket_import_stream(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "socket_import_stream";
    let (stream, _) = crate::fs_fns::stream_param(host, args, 0, fname, "stream")?;
    if host.stream_sock_info(&stream).is_none() {
        let ty = host
            .stream_meta(&stream)
            .map_or("unknown", |m| m.stream_type);
        crate::warn(
            host,
            fname,
            &format!("Cannot represent a stream of type {ty} as a Socket Descriptor"),
        )?;
        return Ok(Value::Bool(false));
    }
    let Some(fd) = host.stream_fd(&stream) else {
        return Ok(Value::Bool(false));
    };
    let v = host
        .instantiate(b"Socket", Vec::new())
        .ok_or_else(|| Thrown::unsupported(fname, "cannot instantiate Socket"))?;
    let h = obj_handle(&v).ok_or_else(|| Thrown::error("Error", "sockets: bad object"))?;
    // Socket uses zend_objects_not_comparable.
    rhp_semantics::ops::set_not_comparable_hook(super::not_comparable_object);
    REG.with(|r| {
        let mut r = r.borrow_mut();
        r.socks.retain(|_, s| s.obj.strong_count() > 0);
        r.socks.insert(
            key_of(&h),
            Sock {
                obj: Rc::downgrade(&h),
                stream,
                fd,
                error: 0,
            },
        );
    });
    Ok(v)
}

fn getsockopt_raw<T>(fd: i32, level: i32, name: i32, val: &mut T) -> Result<(), i32> {
    let mut len = std::mem::size_of::<T>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(
            fd,
            level,
            name,
            val as *mut T as *mut libc::c_void,
            &mut len,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(errno())
    }
}

fn fn_socket_get_option(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "socket_get_option";
    let (key, fd) = socket_arg(host, args, 0, fname, false)?;
    let level = zpp_long(host, args, 1, fname, "level")? as i32;
    let opt = zpp_long(host, args, 2, fname, "option")? as i32;
    let fail = |host: &mut dyn Host, e: i32| -> Result<Value, Thrown> {
        socket_error(host, key, fname, "Unable to retrieve socket option", e)?;
        Ok(Value::Bool(false))
    };
    if level == libc::SOL_SOCKET {
        if opt == libc::SO_LINGER {
            let mut l: libc::linger = unsafe { std::mem::zeroed() };
            if let Err(e) = getsockopt_raw(fd, level, opt, &mut l) {
                return fail(host, e);
            }
            let mut a = PhpArray::new();
            a.insert(
                ArrayKey::from_bytes(b"l_onoff"),
                Value::Int(l.l_onoff as i64),
            );
            a.insert(
                ArrayKey::from_bytes(b"l_linger"),
                Value::Int(l.l_linger as i64),
            );
            return Ok(Value::Array(a));
        }
        if opt == libc::SO_RCVTIMEO || opt == libc::SO_SNDTIMEO {
            let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
            if let Err(e) = getsockopt_raw(fd, level, opt, &mut tv) {
                return fail(host, e);
            }
            let mut a = PhpArray::new();
            a.insert(ArrayKey::from_bytes(b"sec"), Value::Int(tv.tv_sec as i64));
            a.insert(ArrayKey::from_bytes(b"usec"), Value::Int(tv.tv_usec as i64));
            return Ok(Value::Array(a));
        }
    }
    let mut v: libc::c_int = 0;
    match getsockopt_raw(fd, level, opt, &mut v) {
        Ok(()) => Ok(Value::Int(v as i64)),
        Err(e) => fail(host, e),
    }
}

/// An int-valued key of an option array (`l_onoff`, `sec`, …).
fn array_long(host: &mut dyn Host, a: &PhpArray, k: &str, fname: &str) -> Result<i64, Thrown> {
    match a.get(&ArrayKey::from_bytes(k.as_bytes())) {
        Some(v) => Ok(conv::to_int(&deref(v))),
        None => {
            let _ = host;
            Err(Thrown::error(
                "ValueError",
                format!("{fname}(): Argument #4 ($value) must have key \"{k}\""),
            ))
        }
    }
}

fn fn_socket_set_option(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "socket_set_option";
    let (key, fd) = socket_arg(host, args, 0, fname, false)?;
    let level = zpp_long(host, args, 1, fname, "level")? as i32;
    let opt = zpp_long(host, args, 2, fname, "option")? as i32;
    let value = args.get(3).map(deref).unwrap_or(Value::Null);
    let set = |p: *const libc::c_void, len: usize| unsafe {
        libc::setsockopt(fd, level, opt, p, len as libc::socklen_t)
    };
    let r = if level == libc::SOL_SOCKET && opt == libc::SO_LINGER {
        let Value::Array(a) = &value else {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #4 ($value) must be of type array when argument #3 ($option) is SO_LINGER, {} given",
                zpp_type_name(host, &value)
            )));
        };
        let l = libc::linger {
            l_onoff: array_long(host, a, "l_onoff", fname)? as _,
            l_linger: array_long(host, a, "l_linger", fname)? as _,
        };
        set(
            &l as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::linger>(),
        )
    } else if level == libc::SOL_SOCKET && (opt == libc::SO_RCVTIMEO || opt == libc::SO_SNDTIMEO) {
        let Value::Array(a) = &value else {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #4 ($value) must be of type array when argument #3 ($option) is {}, {} given",
                if opt == libc::SO_RCVTIMEO { "SO_RCVTIMEO" } else { "SO_SNDTIMEO" },
                zpp_type_name(host, &value)
            )));
        };
        let tv = libc::timeval {
            tv_sec: array_long(host, a, "sec", fname)? as _,
            tv_usec: array_long(host, a, "usec", fname)? as _,
        };
        set(
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>(),
        )
    } else {
        let v = conv::to_int(&value) as libc::c_int;
        set(
            &v as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>(),
        )
    };
    if r != 0 {
        socket_error(host, key, fname, "Unable to set socket option", errno())?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_socket_strerror(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let e = zpp_long(host, args, 0, "socket_strerror", "error_code")?;
    Ok(Value::str(sockets_strerror(e as i32)))
}

fn fn_socket_last_error(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(deref) {
        None | Some(Value::Null) => Ok(Value::Int(REG.with(|r| r.borrow().last_error) as i64)),
        Some(_) => {
            let (key, _) = socket_arg(host, args, 0, "socket_last_error", true)?;
            Ok(Value::Int(
                REG.with(|r| r.borrow().socks.get(&key).map_or(0, |s| s.error)) as i64,
            ))
        }
    }
}

fn fn_socket_clear_error(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(deref) {
        None | Some(Value::Null) => REG.with(|r| r.borrow_mut().last_error = 0),
        Some(_) => {
            let (key, _) = socket_arg(host, args, 0, "socket_clear_error", true)?;
            REG.with(|r| {
                if let Some(s) = r.borrow_mut().socks.get_mut(&key) {
                    s.error = 0;
                }
            });
        }
    }
    Ok(Value::Null)
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
    put(
        m,
        "socket_import_stream",
        fn_socket_import_stream,
        1,
        Some(1),
    );
    put(m, "socket_get_option", fn_socket_get_option, 3, Some(3));
    put(m, "socket_getopt", fn_socket_get_option, 3, Some(3));
    put(m, "socket_set_option", fn_socket_set_option, 4, Some(4));
    put(m, "socket_setopt", fn_socket_set_option, 4, Some(4));
    put(m, "socket_strerror", fn_socket_strerror, 1, Some(1));
    put(m, "socket_last_error", fn_socket_last_error, 0, Some(1));
    put(m, "socket_clear_error", fn_socket_clear_error, 0, Some(1));
}

pub const PRELUDE_PHP: &str = r#"
final class Socket {
    public function __construct() { throw new Error("Cannot directly construct Socket, use socket_create() instead"); }
}
"#;

/// The extension's constants, with the host platform's numbers.
pub fn constants() -> Vec<(&'static str, Value)> {
    let i = |n: i32| Value::Int(n as i64);
    vec![
        ("AF_UNIX", i(libc::AF_UNIX)),
        ("AF_INET", i(libc::AF_INET)),
        ("AF_INET6", i(libc::AF_INET6)),
        ("SOCK_STREAM", i(libc::SOCK_STREAM)),
        ("SOCK_DGRAM", i(libc::SOCK_DGRAM)),
        ("SOCK_RAW", i(libc::SOCK_RAW)),
        ("SOCK_SEQPACKET", i(libc::SOCK_SEQPACKET)),
        ("SOCK_RDM", i(libc::SOCK_RDM)),
        ("MSG_OOB", i(libc::MSG_OOB)),
        ("MSG_WAITALL", i(libc::MSG_WAITALL)),
        ("MSG_CTRUNC", i(libc::MSG_CTRUNC)),
        ("MSG_TRUNC", i(libc::MSG_TRUNC)),
        ("MSG_PEEK", i(libc::MSG_PEEK)),
        ("MSG_DONTROUTE", i(libc::MSG_DONTROUTE)),
        ("MSG_EOR", i(libc::MSG_EOR)),
        ("MSG_NOSIGNAL", i(libc::MSG_NOSIGNAL)),
        ("MSG_DONTWAIT", i(libc::MSG_DONTWAIT)),
        ("SO_DEBUG", i(libc::SO_DEBUG)),
        ("SO_REUSEADDR", i(libc::SO_REUSEADDR)),
        ("SO_REUSEPORT", i(libc::SO_REUSEPORT)),
        ("SO_KEEPALIVE", i(libc::SO_KEEPALIVE)),
        ("SO_DONTROUTE", i(libc::SO_DONTROUTE)),
        ("SO_LINGER", i(libc::SO_LINGER)),
        ("SO_BROADCAST", i(libc::SO_BROADCAST)),
        ("SO_OOBINLINE", i(libc::SO_OOBINLINE)),
        ("SO_SNDBUF", i(libc::SO_SNDBUF)),
        ("SO_RCVBUF", i(libc::SO_RCVBUF)),
        ("SO_SNDLOWAT", i(libc::SO_SNDLOWAT)),
        ("SO_RCVLOWAT", i(libc::SO_RCVLOWAT)),
        ("SO_SNDTIMEO", i(libc::SO_SNDTIMEO)),
        ("SO_RCVTIMEO", i(libc::SO_RCVTIMEO)),
        ("SO_TYPE", i(libc::SO_TYPE)),
        ("SO_ERROR", i(libc::SO_ERROR)),
        ("SOL_SOCKET", i(libc::SOL_SOCKET)),
        ("SOMAXCONN", i(libc::SOMAXCONN)),
        ("TCP_NODELAY", i(libc::TCP_NODELAY)),
        ("PHP_NORMAL_READ", i(1)),
        ("PHP_BINARY_READ", i(2)),
        ("IPV6_V6ONLY", i(libc::IPV6_V6ONLY)),
        ("SOCKET_EPERM", i(libc::EPERM)),
        ("SOCKET_ENOENT", i(libc::ENOENT)),
        ("SOCKET_EINTR", i(libc::EINTR)),
        ("SOCKET_EIO", i(libc::EIO)),
        ("SOCKET_ENXIO", i(libc::ENXIO)),
        ("SOCKET_E2BIG", i(libc::E2BIG)),
        ("SOCKET_EBADF", i(libc::EBADF)),
        ("SOCKET_EAGAIN", i(libc::EAGAIN)),
        ("SOCKET_ENOMEM", i(libc::ENOMEM)),
        ("SOCKET_EACCES", i(libc::EACCES)),
        ("SOCKET_EFAULT", i(libc::EFAULT)),
        ("SOCKET_ENOTBLK", i(libc::ENOTBLK)),
        ("SOCKET_EBUSY", i(libc::EBUSY)),
        ("SOCKET_EEXIST", i(libc::EEXIST)),
        ("SOCKET_EXDEV", i(libc::EXDEV)),
        ("SOCKET_ENODEV", i(libc::ENODEV)),
        ("SOCKET_ENOTDIR", i(libc::ENOTDIR)),
        ("SOCKET_EISDIR", i(libc::EISDIR)),
        ("SOCKET_EINVAL", i(libc::EINVAL)),
        ("SOCKET_ENFILE", i(libc::ENFILE)),
        ("SOCKET_EMFILE", i(libc::EMFILE)),
        ("SOCKET_ENOTTY", i(libc::ENOTTY)),
        ("SOCKET_ENOSPC", i(libc::ENOSPC)),
        ("SOCKET_ESPIPE", i(libc::ESPIPE)),
        ("SOCKET_EROFS", i(libc::EROFS)),
        ("SOCKET_EMLINK", i(libc::EMLINK)),
        ("SOCKET_EPIPE", i(libc::EPIPE)),
        ("SOCKET_ENAMETOOLONG", i(libc::ENAMETOOLONG)),
        ("SOCKET_ENOLCK", i(libc::ENOLCK)),
        ("SOCKET_ENOSYS", i(libc::ENOSYS)),
        ("SOCKET_ENOTEMPTY", i(libc::ENOTEMPTY)),
        ("SOCKET_ELOOP", i(libc::ELOOP)),
        ("SOCKET_EWOULDBLOCK", i(libc::EWOULDBLOCK)),
        ("SOCKET_ENOMSG", i(libc::ENOMSG)),
        ("SOCKET_EIDRM", i(libc::EIDRM)),
        ("SOCKET_ENOSTR", i(libc::ENOSTR)),
        ("SOCKET_ENODATA", i(libc::ENODATA)),
        ("SOCKET_ETIME", i(libc::ETIME)),
        ("SOCKET_ENOSR", i(libc::ENOSR)),
        ("SOCKET_EREMOTE", i(libc::EREMOTE)),
        ("SOCKET_ENOLINK", i(libc::ENOLINK)),
        ("SOCKET_EPROTO", i(libc::EPROTO)),
        ("SOCKET_EMULTIHOP", i(libc::EMULTIHOP)),
        ("SOCKET_EBADMSG", i(libc::EBADMSG)),
        ("SOCKET_EUSERS", i(libc::EUSERS)),
        ("SOCKET_ENOTSOCK", i(libc::ENOTSOCK)),
        ("SOCKET_EDESTADDRREQ", i(libc::EDESTADDRREQ)),
        ("SOCKET_EMSGSIZE", i(libc::EMSGSIZE)),
        ("SOCKET_EPROTOTYPE", i(libc::EPROTOTYPE)),
        ("SOCKET_ENOPROTOOPT", i(libc::ENOPROTOOPT)),
        ("SOCKET_EPROTONOSUPPORT", i(libc::EPROTONOSUPPORT)),
        ("SOCKET_ESOCKTNOSUPPORT", i(libc::ESOCKTNOSUPPORT)),
        ("SOCKET_EOPNOTSUPP", i(libc::EOPNOTSUPP)),
        ("SOCKET_EPFNOSUPPORT", i(libc::EPFNOSUPPORT)),
        ("SOCKET_EAFNOSUPPORT", i(libc::EAFNOSUPPORT)),
        ("SOCKET_EADDRINUSE", i(libc::EADDRINUSE)),
        ("SOCKET_EADDRNOTAVAIL", i(libc::EADDRNOTAVAIL)),
        ("SOCKET_ENETDOWN", i(libc::ENETDOWN)),
        ("SOCKET_ENETUNREACH", i(libc::ENETUNREACH)),
        ("SOCKET_ENETRESET", i(libc::ENETRESET)),
        ("SOCKET_ECONNABORTED", i(libc::ECONNABORTED)),
        ("SOCKET_ECONNRESET", i(libc::ECONNRESET)),
        ("SOCKET_ENOBUFS", i(libc::ENOBUFS)),
        ("SOCKET_EISCONN", i(libc::EISCONN)),
        ("SOCKET_ENOTCONN", i(libc::ENOTCONN)),
        ("SOCKET_ESHUTDOWN", i(libc::ESHUTDOWN)),
        ("SOCKET_ETOOMANYREFS", i(libc::ETOOMANYREFS)),
        ("SOCKET_ETIMEDOUT", i(libc::ETIMEDOUT)),
        ("SOCKET_ECONNREFUSED", i(libc::ECONNREFUSED)),
        ("SOCKET_EHOSTDOWN", i(libc::EHOSTDOWN)),
        ("SOCKET_EHOSTUNREACH", i(libc::EHOSTUNREACH)),
        ("SOCKET_EALREADY", i(libc::EALREADY)),
        ("SOCKET_EINPROGRESS", i(libc::EINPROGRESS)),
        ("SOCKET_EDQUOT", i(libc::EDQUOT)),
        ("IPPROTO_IP", i(libc::IPPROTO_IP)),
        ("IPPROTO_IPV6", i(libc::IPPROTO_IPV6)),
        ("SOL_TCP", i(libc::IPPROTO_TCP)),
        ("SOL_UDP", i(libc::IPPROTO_UDP)),
        ("IPPROTO_ICMP", i(libc::IPPROTO_ICMP)),
        ("IPPROTO_ICMPV6", i(libc::IPPROTO_ICMPV6)),
        ("SHUT_RD", i(libc::SHUT_RD)),
        ("SHUT_WR", i(libc::SHUT_WR)),
        ("SHUT_RDWR", i(libc::SHUT_RDWR)),
    ]
}
