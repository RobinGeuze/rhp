//! php's `sockets` extension (php-src ext/sockets): the `Socket` and
//! `AddressInfo` classes, the socket calls (core.rs), the options, errors
//! and constants. The constants come from sockets.stub.php
//! (`gen sockets-consts`), evaluated against the system headers by shim.c.
//!
//! Not ported: socket_sendmsg/socket_recvmsg/socket_cmsg_space
//! (sendrecvmsg.c, conversions.c) and the multicast and other structured
//! options of socket_get_option/socket_set_option (multicast.c), which
//! fall back to the int case.

mod core;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rhp_values::ops::deref;
use rhp_values::{ArrayKey, PhpArray, Value};

use crate::main::streams::errno;
use crate::zend::object_handlers::{ClassHandlers, ObjectHandlers};
use crate::zend::zpp::{zpp_long, zval_value_name};
use crate::{Host, Thrown};

/// `Socket`'s and `AddressInfo`'s handlers: no clone, not comparable.
pub(crate) const CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "Socket",
        handlers: &ObjectHandlers::OPAQUE,
        create_object: true,
    },
    ClassHandlers {
        class: "AddressInfo",
        handlers: &ObjectHandlers::OPAQUE,
        create_object: true,
    },
];

/// A `Socket` object's payload (php's `php_socket`).
pub(crate) struct Sock {
    /// php's `bsd_socket` (-1 once closed).
    fd: Cell<i32>,
    /// php's `type`: the address family.
    family: Cell<i32>,
    error: Cell<i32>,
    blocking: Cell<bool>,
    /// php's `zstream`: the stream the socket was imported from or
    /// exported to, which owns the descriptor.
    stream: RefCell<Option<Value>>,
}

impl Drop for Sock {
    /// socket_free_obj: close the descriptor unless a stream owns it.
    fn drop(&mut self) {
        if self.stream.get_mut().is_none() && self.fd.get() >= 0 {
            unsafe { libc::close(self.fd.get()) };
        }
    }
}

thread_local! {
    /// php's `SOCKETS_G(last_error)`.
    static LAST_ERROR: Cell<i32> = const { Cell::new(0) };
}

/// A new request: php keeps the last error per worker (GINIT); a request
/// here starts as a fresh process does.
pub(crate) fn reset_state() {
    LAST_ERROR.with(|e| e.set(0));
}

/// The `Socket` behind a `Socket` argument (TypeError for anything else;
/// `nullable` is php's `?Socket`).
fn socket_arg(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    nullable: bool,
) -> Result<Rc<Sock>, Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    if let Some(s) = crate::zend::object_handlers::payload::<Sock>(&v) {
        return Ok(s);
    }
    Err(Thrown::type_error(format!(
        "{fname}(): Argument #{} ($socket) must be of type {}Socket, {} given",
        i + 1,
        if nullable { "?" } else { "" },
        zval_value_name(host, &v)
    )))
}

/// `ENSURE_SOCKET_VALID`.
fn closed_check(s: &Sock, fname: &str) -> Result<(), Thrown> {
    if s.fd.get() < 0 {
        return Err(Thrown::error(
            "Error",
            format!("{fname}(): Argument #1 ($socket) has already been closed"),
        ));
    }
    Ok(())
}

/// `object_init_ex(return_value, socket_ce)`: a new `Socket` with no
/// descriptor yet.
fn new_socket(host: &mut dyn Host, fname: &str) -> Result<(Value, Rc<Sock>), Thrown> {
    let v = host
        .instantiate(b"Socket", Vec::new())
        .ok_or_else(|| Thrown::unsupported(fname, "cannot instantiate Socket"))?;
    let h = crate::zend::object_handlers::object_of(&v)
        .ok_or_else(|| Thrown::error("Error", "sockets: bad object"))?;
    let s = Rc::new(Sock {
        fd: Cell::new(-1),
        family: Cell::new(libc::AF_UNSPEC),
        error: Cell::new(0),
        blocking: Cell::new(true),
        stream: RefCell::new(None),
    });
    crate::zend::object_handlers::set_payload(&h, s.clone());
    Ok((v, s))
}

/// php's `PHP_SOCKET_ERROR`: record the error on the socket and globally,
/// and warn unless it is a would-block / in-progress condition.
fn socket_error(
    host: &mut dyn Host,
    sock: &Sock,
    fname: &str,
    msg: &str,
    err: i32,
) -> Result<(), Thrown> {
    LAST_ERROR.with(|e| e.set(err));
    sock.error.set(err);
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
        let h = if e == i32::MIN {
            2147473648u32 as i32
        } else {
            -e - 10000
        };
        let p = unsafe { hstrerror(h) };
        if !p.is_null() {
            return unsafe { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() };
        }
        return String::new();
    }
    crate::main::streams::strerror(e)
}

extern "C" {
    fn hstrerror(err: libc::c_int) -> *const libc::c_char;
    fn rhp_sockets_constants(
        cb: extern "C" fn(*mut libc::c_void, *const libc::c_char, i64),
        ctx: *mut libc::c_void,
    );
}

fn fn_socket_import_stream(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "socket_import_stream";
    let (stream, _) = crate::ext::standard::file::stream_param(host, args, 0, fname, "stream")?;
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
    let (v, s) = new_socket(host, fname)?;
    // socket_import_file_descriptor: the family and blocking state.
    s.fd.set(fd);
    let mut family: libc::c_int = -1;
    #[cfg(target_os = "linux")]
    {
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let r = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_DOMAIN,
                &mut family as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if r != 0 {
            family = -1;
        }
    }
    if family < 0 {
        let mut st: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        if unsafe { libc::getsockname(fd, &mut st as *mut _ as *mut libc::sockaddr, &mut len) } == 0
        {
            family = st.ss_family as libc::c_int;
        } else {
            socket_error(host, &s, fname, "Unable to obtain socket family", errno())?;
            s.fd.set(-1);
            return Ok(Value::Bool(false));
        }
    }
    s.family.set(family);
    let fl = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if fl == -1 {
        socket_error(host, &s, fname, "Unable to obtain blocking state", errno())?;
        s.fd.set(-1);
        return Ok(Value::Bool(false));
    }
    s.blocking.set(fl & libc::O_NONBLOCK == 0);
    *s.stream.borrow_mut() = Some(stream);
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
    let sock = socket_arg(host, args, 0, fname, false)?;
    closed_check(&sock, fname)?;
    let fd = sock.fd.get();
    let level = zpp_long(host, args, 1, fname, "level")? as i32;
    let opt = zpp_long(host, args, 2, fname, "option")? as i32;
    let fail = |host: &mut dyn Host, e: i32| -> Result<Value, Thrown> {
        socket_error(host, &sock, fname, "Unable to retrieve socket option", e)?;
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
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(
            fd,
            level,
            opt,
            &mut v as *mut libc::c_int as *mut libc::c_void,
            &mut len,
        )
    };
    if r != 0 {
        return fail(host, errno());
    }
    if len == 1 {
        // A one-byte option: just that (native-order first) byte.
        v = v.to_ne_bytes()[0] as libc::c_int;
    }
    Ok(Value::Int(v as i64))
}

/// The option table of an array-valued option: the array, or an object's
/// property table (php's `Z_OBJPROP_P`); `None` for anything else.
fn option_table(v: &Value) -> Option<PhpArray> {
    match v {
        Value::Array(a) => Some(a.clone()),
        Value::Object(h) => {
            let mut a = PhpArray::new();
            for (k, val) in h.borrow().props().iter() {
                a.insert(ArrayKey::from_php_str(k), val.clone());
            }
            Some(a)
        }
        _ => None,
    }
}

/// The value of key `k` of an option table (`l_onoff`, `sec`, …), or php's
/// "must have key" ValueError.
fn table_entry(a: &PhpArray, k: &str, fname: &str) -> Result<Value, Thrown> {
    a.get(&ArrayKey::from_bytes(k.as_bytes()))
        .map(deref)
        .ok_or_else(|| {
            Thrown::error(
                "ValueError",
                format!("{fname}(): Argument #4 ($value) must have key \"{k}\""),
            )
        })
}

fn fn_socket_set_option(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use crate::ext::curl::zval_get_long;
    let fname = "socket_set_option";
    let sock = socket_arg(host, args, 0, fname, false)?;
    closed_check(&sock, fname)?;
    let fd = sock.fd.get();
    let level = zpp_long(host, args, 1, fname, "level")?;
    let opt = zpp_long(host, args, 2, fname, "option")?;
    let value = args.get(3).map(deref).unwrap_or(Value::Null);
    let set = |p: *const libc::c_void, len: usize| unsafe {
        libc::setsockopt(fd, level as i32, opt as i32, p, len as libc::socklen_t)
    };
    // php switches on the option name alone (whatever the level), except
    // that IPPROTO_IP / IPPROTO_IPV6 options go to the multicast handlers,
    // which hand everything they do not know to the plain int case.
    let by_name = level != libc::IPPROTO_IP as i64 && level != libc::IPPROTO_IPV6 as i64;
    let r = if by_name && opt == libc::SO_LINGER as i64 {
        let Some(a) = option_table(&value) else {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #4 ($value) must be of type array when argument #3 ($option) is SO_LINGER, {} given",
                zval_value_name(host, &value)
            )));
        };
        let onoff = table_entry(&a, "l_onoff", fname)?;
        let linger = table_entry(&a, "l_linger", fname)?;
        let onoff = zval_get_long(host, &onoff)?;
        let linger = zval_get_long(host, &linger)?;
        for (k, v) in [("l_onoff", onoff), ("l_linger", linger)] {
            if !(0..=u16::MAX as i64).contains(&v) {
                return Err(Thrown::error(
                    "ValueError",
                    format!(
                        "{fname}(): Argument #4 ($value) \"{k}\" must be between 0 and {}",
                        u16::MAX
                    ),
                ));
            }
        }
        let l = libc::linger {
            l_onoff: onoff as _,
            l_linger: linger as _,
        };
        set(
            &l as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::linger>(),
        )
    } else if by_name && (opt == libc::SO_RCVTIMEO as i64 || opt == libc::SO_SNDTIMEO as i64) {
        let Some(a) = option_table(&value) else {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #4 ($value) must be of type array when argument #3 ($option) is {}, {} given",
                if opt == libc::SO_RCVTIMEO as i64 { "SO_RCVTIMEO" } else { "SO_SNDTIMEO" },
                zval_value_name(host, &value)
            )));
        };
        let sec = table_entry(&a, "sec", fname)?;
        let usec = table_entry(&a, "usec", fname)?;
        let tv = libc::timeval {
            tv_sec: zval_get_long(host, &sec)? as _,
            tv_usec: zval_get_long(host, &usec)? as _,
        };
        set(
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>(),
        )
    } else {
        let v = zval_get_long(host, &value)? as libc::c_int;
        set(
            &v as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>(),
        )
    };
    if r != 0 {
        socket_error(host, &sock, fname, "Unable to set socket option", errno())?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_socket_strerror(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let e = zpp_long(host, args, 0, "socket_strerror", "error_code")?;
    if i32::try_from(e).is_err() {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "socket_strerror(): Argument #1 ($error_code) must be between {} and {}",
                i32::MIN,
                i32::MAX
            ),
        ));
    }
    Ok(Value::str(sockets_strerror(e as i32)))
}

fn fn_socket_last_error(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(deref) {
        None | Some(Value::Null) => Ok(Value::Int(LAST_ERROR.with(Cell::get) as i64)),
        Some(_) => {
            let sock = socket_arg(host, args, 0, "socket_last_error", true)?;
            closed_check(&sock, "socket_last_error")?;
            Ok(Value::Int(sock.error.get() as i64))
        }
    }
}

fn fn_socket_clear_error(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(deref) {
        None | Some(Value::Null) => LAST_ERROR.with(|e| e.set(0)),
        Some(_) => {
            let sock = socket_arg(host, args, 0, "socket_clear_error", true)?;
            closed_check(&sock, "socket_clear_error")?;
            sock.error.set(0);
        }
    }
    Ok(Value::Null)
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("socket_select", core::fn_socket_select),
            ("socket_create_listen", core::fn_socket_create_listen),
            ("socket_accept", core::fn_socket_accept),
            ("socket_set_nonblock", core::fn_socket_set_nonblock),
            ("socket_set_block", core::fn_socket_set_block),
            ("socket_listen", core::fn_socket_listen),
            ("socket_close", core::fn_socket_close),
            ("socket_write", core::fn_socket_write),
            ("socket_read", core::fn_socket_read),
            ("socket_getsockname", core::fn_socket_getsockname),
            ("socket_getpeername", core::fn_socket_getpeername),
            ("socket_create", core::fn_socket_create),
            ("socket_connect", core::fn_socket_connect),
            ("socket_bind", core::fn_socket_bind),
            ("socket_recv", core::fn_socket_recv),
            ("socket_send", core::fn_socket_send),
            ("socket_recvfrom", core::fn_socket_recvfrom),
            ("socket_sendto", core::fn_socket_sendto),
            ("socket_create_pair", core::fn_socket_create_pair),
            ("socket_shutdown", core::fn_socket_shutdown),
            ("socket_atmark", core::fn_socket_atmark),
            ("socket_export_stream", core::fn_socket_export_stream),
            ("socket_addrinfo_lookup", core::fn_socket_addrinfo_lookup),
            ("socket_addrinfo_bind", core::fn_socket_addrinfo_bind),
            ("socket_addrinfo_connect", core::fn_socket_addrinfo_connect),
            ("socket_addrinfo_explain", core::fn_socket_addrinfo_explain),
            ("socket_import_stream", fn_socket_import_stream),
            ("socket_get_option", fn_socket_get_option),
            ("socket_getopt", fn_socket_get_option),
            ("socket_set_option", fn_socket_set_option),
            ("socket_setopt", fn_socket_set_option),
            ("socket_strerror", fn_socket_strerror),
            ("socket_last_error", fn_socket_last_error),
            ("socket_clear_error", fn_socket_clear_error),
        ],
    );
}

pub const PRELUDE_PHP: &str = include_str!("prelude.php");

/// The extension's constants: sockets.stub.php's, with the platform's
/// values and guards (shim.c, consts.h).
pub fn constants() -> Vec<(&'static str, Value)> {
    extern "C" fn add(ctx: *mut libc::c_void, name: *const libc::c_char, v: i64) {
        let out = unsafe { &mut *(ctx as *mut Vec<(&'static str, Value)>) };
        let name = unsafe { std::ffi::CStr::from_ptr(name) };
        // The names are string literals of the static C table.
        let name: &'static std::ffi::CStr = unsafe { &*(name as *const std::ffi::CStr) };
        out.push((name.to_str().unwrap_or(""), Value::Int(v)));
    }
    let mut out: Vec<(&'static str, Value)> = Vec::new();
    unsafe { rhp_sockets_constants(add, &mut out as *mut _ as *mut libc::c_void) };
    out
}
