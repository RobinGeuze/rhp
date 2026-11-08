//! The socket calls of sockets.c (php-src ext/sockets): creating,
//! binding, connecting, reading and writing `Socket` objects, socket_select
//! and the `socket_addrinfo_*` family (`AddressInfo`).

use std::ffi::{c_char, c_int, CStr, CString};
use std::os::fd::FromRawFd;
use std::rc::Rc;

use rhp_values::ops::deref;
use rhp_values::{ArrayKey, PhpArray, Value};

use super::{
    closed_check, new_socket, socket_arg, socket_error, sockets_strerror, Sock, LAST_ERROR,
};
use crate::main::streams::errno;
use crate::zend::zpp::{try_assign_ref, value_error, zval_type_name};
use crate::{Host, Thrown};

/// `PHP_NORMAL_READ`.
const NORMAL_READ: i64 = 1;

extern "C" {
    fn inet_pton(af: c_int, src: *const c_char, dst: *mut libc::c_void) -> c_int;
    fn inet_ntop(
        af: c_int,
        src: *const libc::c_void,
        dst: *mut c_char,
        size: libc::socklen_t,
    ) -> *const c_char;
    fn rhp_sockets_gethostbyname(name: *const c_char, out: *mut libc::in_addr) -> c_int;
    fn rhp_sockets_h_errno() -> c_int;
}

fn int(args: &[Value], i: usize, default: i64) -> i64 {
    match args.get(i).map(deref) {
        Some(Value::Int(n)) => n,
        Some(v) => rhp_values::conv::to_int(&v),
        None => default,
    }
}

fn opt_int(args: &[Value], i: usize) -> Option<i64> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(rhp_values::conv::to_int(&v)),
    }
}

fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    match args.get(i).map(deref) {
        Some(Value::Str(s)) => s.as_bytes().to_vec(),
        Some(v) => rhp_values::conv::to_string_bytes(&v),
        None => Vec::new(),
    }
}

/// `zend_argument_value_error` (with the stub's parameter name).
fn arg_value_error(f: &str, n: usize, msg: &str) -> Thrown {
    value_error(f, n, pname(f, n), msg)
}

fn pname(f: &str, n: usize) -> &'static str {
    rhp_bytecode::signatures::function(f.as_bytes())
        .and_then(|s| s.params.get(n - 1))
        .map_or("", |p| p.name)
}

/// The valid socket of argument 1.
fn valid(host: &mut dyn Host, args: &[Value], f: &str) -> Result<Rc<Sock>, Thrown> {
    let s = socket_arg(host, args, 0, f, false)?;
    closed_check(&s, f)?;
    Ok(s)
}

fn warn_errno(host: &mut dyn Host, f: &str, msg: &str, e: i32) -> Result<(), Thrown> {
    LAST_ERROR.with(|g| g.set(e));
    crate::warn(host, f, &format!("{msg} [{e}]: {}", sockets_strerror(e)))
}

/// The `SOCK_*` check of socket_create and socket_create_pair.
fn type_check(f: &str, ty: i64) -> Result<(), Thrown> {
    #[cfg(target_os = "linux")]
    let check = ty & !((libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK) as i64);
    #[cfg(not(target_os = "linux"))]
    let check = ty;
    if check > 10 {
        let tail = if cfg!(target_os = "linux") {
            " optionally OR'ed with SOCK_CLOEXEC, SOCK_NONBLOCK"
        } else {
            ""
        };
        return Err(arg_value_error(
            f,
            2,
            &format!("must be one of SOCK_STREAM, SOCK_DGRAM, SOCK_SEQPACKET, SOCK_RAW, or SOCK_RDM{tail}"),
        ));
    }
    Ok(())
}

/// `PHP_ETH_PROTO_CHECK`.
fn eth_proto(protocol: i64, family: i64) -> i64 {
    #[cfg(target_os = "linux")]
    if protocol <= u16::MAX as i64 && family == libc::AF_PACKET as i64 {
        return (protocol as u16).to_be() as i64;
    }
    let _ = family;
    protocol
}

pub(super) fn fn_socket_create(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_create";
    let domain = int(args, 0, 0);
    let ty = int(args, 1, 0);
    let protocol = int(args, 2, 0);
    let packet = cfg!(target_os = "linux") && domain == packet_family();
    if domain != libc::AF_UNIX as i64
        && domain != libc::AF_INET6 as i64
        && !packet
        && domain != libc::AF_INET as i64
    {
        return Err(arg_value_error(
            f,
            1,
            "must be one of AF_UNIX, AF_PACKET, AF_INET6, or AF_INET",
        ));
    }
    type_check(f, ty)?;
    let protocol = eth_proto(protocol, domain);
    let (v, s) = new_socket(host, f)?;
    let fd = unsafe { libc::socket(domain as c_int, ty as c_int, protocol as c_int) };
    s.family.set(domain as i32);
    if fd < 0 {
        warn_errno(host, f, "Unable to create socket", errno())?;
        return Ok(Value::Bool(false));
    }
    s.fd.set(fd);
    Ok(v)
}

/// AF_PACKET where the platform has it (-1 elsewhere).
fn packet_family() -> i64 {
    #[cfg(target_os = "linux")]
    return libc::AF_PACKET as i64;
    #[cfg(not(target_os = "linux"))]
    -1
}

pub(super) fn fn_socket_create_listen(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let f = "socket_create_listen";
    let port = int(args, 0, 0);
    let backlog = int(args, 1, libc::SOMAXCONN as i64);
    if !(0..=u16::MAX as i64).contains(&port) {
        return Err(arg_value_error(
            f,
            1,
            &format!("must be between 0 and {}", u16::MAX),
        ));
    }
    let (v, s) = new_socket(host, f)?;
    // php_open_listen_sock: 0.0.0.0 (php_network_gethostbyname), the port.
    let mut la: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    la.sin_family = libc::AF_INET as _;
    la.sin_port = (port as u16).to_be();
    let fd = unsafe { libc::socket(libc::PF_INET, libc::SOCK_STREAM, 0) };
    s.blocking.set(true);
    if fd < 0 {
        socket_error(host, &s, f, "unable to create listening socket", errno())?;
        return Ok(Value::Bool(false));
    }
    s.fd.set(fd);
    s.family.set(libc::PF_INET);
    let len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    if unsafe { libc::bind(fd, &la as *const _ as *const libc::sockaddr, len) } != 0 {
        socket_error(host, &s, f, "unable to bind to given address", errno())?;
        unsafe { libc::close(fd) };
        s.fd.set(-1);
        return Ok(Value::Bool(false));
    }
    if unsafe { libc::listen(fd, backlog as c_int) } != 0 {
        socket_error(host, &s, f, "unable to listen on socket", errno())?;
        unsafe { libc::close(fd) };
        s.fd.set(-1);
        return Ok(Value::Bool(false));
    }
    s.error.set(0);
    Ok(v)
}

pub(super) fn fn_socket_accept(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_accept";
    let s = valid(host, args, f)?;
    let (v, out) = new_socket(host, f)?;
    let mut sa: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let sap = &mut sa as *mut _ as *mut libc::sockaddr;
    #[cfg(target_os = "linux")]
    let fd = {
        let mut flags = libc::SOCK_CLOEXEC;
        if !s.blocking.get() {
            flags |= libc::SOCK_NONBLOCK;
        }
        unsafe { libc::accept4(s.fd.get(), sap, &mut len, flags) }
    };
    #[cfg(not(target_os = "linux"))]
    let fd = unsafe { libc::accept(s.fd.get(), sap, &mut len) };
    if fd < 0 {
        socket_error(
            host,
            &out,
            f,
            "unable to accept incoming connection",
            errno(),
        )?;
        return Ok(Value::Bool(false));
    }
    out.fd.set(fd);
    #[cfg(not(target_os = "linux"))]
    {
        let mode = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if mode < 0 {
            socket_error(
                host,
                &out,
                f,
                "unable to get fcntl mode on the socket",
                errno(),
            )?;
            return Ok(Value::Bool(false));
        }
        if mode | libc::FD_CLOEXEC != mode
            && unsafe { libc::fcntl(fd, libc::F_SETFD, mode | libc::FD_CLOEXEC) } < 0
        {
            socket_error(
                host,
                &out,
                f,
                "unable to set cloexec mode on the socket",
                errno(),
            )?;
            return Ok(Value::Bool(false));
        }
    }
    out.error.set(0);
    out.blocking.set(true);
    out.family.set(sa.ss_family as i32);
    Ok(v)
}

fn set_blocking(host: &mut dyn Host, args: &[Value], f: &str, on: bool) -> Result<Value, Thrown> {
    let s = valid(host, args, f)?;
    let stream = s.stream.borrow().clone();
    if let Some(stream) = stream {
        if host.stream_set_blocking(&stream, on) {
            s.blocking.set(on);
            return Ok(Value::Bool(true));
        }
    }
    let fd = s.fd.get();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    let ok = flags >= 0 && {
        let nf = if on {
            flags & !libc::O_NONBLOCK
        } else {
            flags | libc::O_NONBLOCK
        };
        (unsafe { libc::fcntl(fd, libc::F_SETFL, nf) }) >= 0
    };
    if ok {
        s.blocking.set(on);
        return Ok(Value::Bool(true));
    }
    let msg = if on {
        "unable to set blocking mode"
    } else {
        "unable to set nonblocking mode"
    };
    socket_error(host, &s, f, msg, errno())?;
    Ok(Value::Bool(false))
}

pub(super) fn fn_socket_set_nonblock(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    set_blocking(h, a, "socket_set_nonblock", false)
}

pub(super) fn fn_socket_set_block(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    set_blocking(h, a, "socket_set_block", true)
}

pub(super) fn fn_socket_listen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_listen";
    let s = valid(host, args, f)?;
    let backlog = int(args, 1, 0);
    if unsafe { libc::listen(s.fd.get(), backlog as c_int) } != 0 {
        socket_error(host, &s, f, "unable to listen on socket", errno())?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

pub(super) fn fn_socket_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = valid(host, args, "socket_close")?;
    let stream = s.stream.borrow_mut().take();
    match stream {
        Some(stream) => {
            host.stream_close(&stream);
        }
        None => {
            unsafe { libc::close(s.fd.get()) };
        }
    }
    s.fd.set(-1);
    Ok(Value::Null)
}

pub(super) fn fn_socket_write(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_write";
    let s = valid(host, args, f)?;
    let data = bytes(args, 1);
    let len = opt_int(args, 2);
    if len.is_some_and(|l| l < 0) {
        return Err(arg_value_error(f, 3, "must be greater than or equal to 0"));
    }
    let n = len.map_or(data.len(), |l| (l as u64).min(data.len() as u64) as usize);
    let r = unsafe { libc::write(s.fd.get(), data.as_ptr().cast(), n) };
    if r < 0 {
        socket_error(host, &s, f, "unable to write to socket", errno())?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(r as i64))
}

/// php_read: up to `buf.len()` bytes, one at a time, through the first
/// `\n` or `\r` (kept).
fn php_read(fd: i32, buf: &mut [u8]) -> isize {
    let fl = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if fl < 0 {
        return fl as isize;
    }
    let nonblock = fl & libc::O_NONBLOCK != 0;
    let maxlen = buf.len();
    let (mut m, mut n, mut no_read, mut t): (isize, usize, i32, usize) = (0, 0, 0, 0);
    set_errno(0);
    buf[0] = 0;
    while buf[t] != b'\n' && buf[t] != b'\r' && n < maxlen {
        if m > 0 {
            t += 1;
            n += 1;
        } else if m == 0 {
            no_read += 1;
            if nonblock && no_read >= 2 {
                return n as isize;
            }
            if no_read > 200 {
                set_errno(libc::ECONNRESET);
                return -1;
            }
        }
        if n < maxlen {
            m = unsafe { libc::recv(fd, buf[t..].as_mut_ptr().cast(), 1, 0) };
        }
        let e = errno();
        if e != 0 && e != libc::ESPIPE && e != libc::EAGAIN {
            return -1;
        }
        set_errno(0);
        if t >= maxlen {
            break;
        }
    }
    if n < maxlen {
        n += 1;
    }
    n as isize
}

fn set_errno(e: i32) {
    #[cfg(target_os = "linux")]
    unsafe {
        *libc::__errno_location() = e
    };
    #[cfg(target_vendor = "apple")]
    unsafe {
        *libc::__error() = e
    };
}

pub(super) fn fn_socket_read(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_read";
    let s = valid(host, args, f)?;
    let length = int(args, 1, 0);
    let mode = int(args, 2, 2);
    if length <= 0 || length == i64::MAX {
        return Ok(Value::Bool(false));
    }
    let mut buf = vec![0u8; length as usize];
    let r = if mode == NORMAL_READ {
        php_read(s.fd.get(), &mut buf)
    } else {
        unsafe { libc::recv(s.fd.get(), buf.as_mut_ptr().cast(), buf.len(), 0) }
    };
    if r == -1 {
        let e = errno();
        if e == libc::EAGAIN || e == libc::EWOULDBLOCK {
            s.error.set(e);
            LAST_ERROR.with(|g| g.set(e));
        } else {
            socket_error(host, &s, f, "unable to read from socket", e)?;
        }
        return Ok(Value::Bool(false));
    }
    buf.truncate(r.max(0) as usize);
    Ok(Value::str(buf))
}

/// The address and port of a socket address, as getsockname and
/// getpeername report them.
enum Named {
    Inet(String, u16),
    Unix(Vec<u8>),
    #[cfg(target_os = "linux")]
    Packet(Vec<u8>, i32),
    Other,
}

fn ntop(family: c_int, addr: *const libc::c_void) -> Option<String> {
    let mut buf = [0 as c_char; 64];
    let p = unsafe { inet_ntop(family, addr, buf.as_mut_ptr(), buf.len() as _) };
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

fn sun_path(su: &libc::sockaddr_un) -> Vec<u8> {
    let raw: Vec<u8> = su.sun_path.iter().map(|&c| c as u8).collect();
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    raw[..end].to_vec()
}

fn named(sa: &libc::sockaddr_storage) -> Named {
    match sa.ss_family as c_int {
        libc::AF_INET6 => {
            let s6 = unsafe { &*(sa as *const _ as *const libc::sockaddr_in6) };
            let a = ntop(libc::AF_INET6, &s6.sin6_addr as *const _ as *const _).unwrap_or_default();
            Named::Inet(a, u16::from_be(s6.sin6_port))
        }
        libc::AF_INET => {
            let s4 = unsafe { &*(sa as *const _ as *const libc::sockaddr_in) };
            let a = ntop(libc::AF_INET, &s4.sin_addr as *const _ as *const _).unwrap_or_default();
            Named::Inet(a, u16::from_be(s4.sin_port))
        }
        libc::AF_UNIX => Named::Unix(sun_path(unsafe {
            &*(sa as *const _ as *const libc::sockaddr_un)
        })),
        #[cfg(target_os = "linux")]
        libc::AF_PACKET => {
            let sll = unsafe { &*(sa as *const _ as *const libc::sockaddr_ll) };
            let mut name = [0 as c_char; libc::IF_NAMESIZE];
            let p = unsafe { libc::if_indextoname(sll.sll_ifindex as u32, name.as_mut_ptr()) };
            if p.is_null() {
                return Named::Packet(Vec::new(), -1);
            }
            Named::Packet(
                unsafe { CStr::from_ptr(p) }.to_bytes().to_vec(),
                sll.sll_ifindex,
            )
        }
        _ => Named::Other,
    }
}

fn sock_name(host: &mut dyn Host, args: &[Value], peer: bool) -> Result<Value, Thrown> {
    let f = if peer {
        "socket_getpeername"
    } else {
        "socket_getsockname"
    };
    let s = valid(host, args, f)?;
    let mut sa: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let sap = &mut sa as *mut _ as *mut libc::sockaddr;
    let r = unsafe {
        if peer {
            libc::getpeername(s.fd.get(), sap, &mut len)
        } else {
            libc::getsockname(s.fd.get(), sap, &mut len)
        }
    };
    if r != 0 {
        let msg = if peer {
            "unable to retrieve peer name"
        } else {
            "unable to retrieve socket name"
        };
        socket_error(host, &s, f, msg, errno())?;
        return Ok(Value::Bool(false));
    }
    match named(&sa) {
        Named::Inet(a, port) => {
            try_assign_ref(host, args, 1, Value::str(a))?;
            if args.len() > 2 {
                try_assign_ref(host, args, 2, Value::Int(port as i64))?;
            }
        }
        Named::Unix(p) => try_assign_ref(host, args, 1, Value::str(p))?,
        #[cfg(target_os = "linux")]
        Named::Packet(name, idx) if !peer => {
            if idx < 0 {
                return Err(Thrown::error("Error", "invalid interface index"));
            }
            try_assign_ref(host, args, 1, Value::str(name))?;
            if args.len() > 2 {
                try_assign_ref(host, args, 2, Value::Int(idx as i64))?;
            }
        }
        _ => {
            let fams = if peer {
                "must be one of AF_UNIX, AF_INET, or AF_INET6"
            } else {
                "must be one of AF_UNIX, AF_PACKET, AF_INET, or AF_INET6"
            };
            return Err(arg_value_error(f, 1, fams));
        }
    }
    Ok(Value::Bool(true))
}

pub(super) fn fn_socket_getsockname(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    sock_name(h, a, false)
}

pub(super) fn fn_socket_getpeername(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    sock_name(h, a, true)
}

/// php_set_inet_addr: an IPv4 literal, else a gethostbyname lookup.
fn set_inet_addr(
    host: &mut dyn Host,
    s: &Sock,
    f: &str,
    addr: &[u8],
) -> Result<Option<libc::in_addr>, Thrown> {
    let c = CString::new(&addr[..addr.iter().position(|&b| b == 0).unwrap_or(addr.len())])
        .unwrap_or_default();
    let mut out: libc::in_addr = unsafe { std::mem::zeroed() };
    if unsafe { inet_pton(libc::AF_INET, c.as_ptr(), &mut out as *mut _ as *mut _) } == 1 {
        return Ok(Some(out));
    }
    // MAXFQDNLEN
    let r = if addr.len() > 255 {
        0
    } else {
        unsafe { rhp_sockets_gethostbyname(c.as_ptr(), &mut out) }
    };
    match r {
        1 => Ok(Some(out)),
        2 => {
            crate::warn(
                host,
                f,
                "Host lookup failed: Non AF_INET domain returned on AF_INET socket",
            )?;
            Ok(None)
        }
        _ => {
            let e = -10000 - unsafe { rhp_sockets_h_errno() };
            socket_error(host, s, f, "Host lookup failed", e)?;
            Ok(None)
        }
    }
}

/// php_set_inet6_addr: an IPv6 literal (with `%scope`), else getaddrinfo.
fn set_inet6_addr(
    host: &mut dyn Host,
    s: &Sock,
    f: &str,
    addr: &[u8],
    sin6: &mut libc::sockaddr_in6,
) -> Result<bool, Thrown> {
    let cut = &addr[..addr.iter().position(|&b| b == 0).unwrap_or(addr.len())];
    let c = CString::new(cut).unwrap_or_default();
    let mut tmp: libc::in6_addr = unsafe { std::mem::zeroed() };
    if unsafe { inet_pton(libc::AF_INET6, c.as_ptr(), &mut tmp as *mut _ as *mut _) } == 1 {
        sin6.sin6_addr = tmp;
    } else {
        let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
        hints.ai_family = libc::AF_INET6;
        hints.ai_flags = libc::AI_V4MAPPED | libc::AI_ADDRCONFIG;
        let mut res: *mut libc::addrinfo = std::ptr::null_mut();
        unsafe { libc::getaddrinfo(c.as_ptr(), std::ptr::null(), &hints, &mut res) };
        if res.is_null() {
            let e = -10000 - unsafe { rhp_sockets_h_errno() };
            socket_error(host, s, f, "Host lookup failed", e)?;
            return Ok(false);
        }
        let ai = unsafe { &*res };
        if ai.ai_family != libc::PF_INET6
            || ai.ai_addrlen as usize != std::mem::size_of::<libc::sockaddr_in6>()
        {
            unsafe { libc::freeaddrinfo(res) };
            crate::warn(
                host,
                f,
                "Host lookup failed: Non AF_INET6 domain returned on AF_INET6 socket",
            )?;
            return Ok(false);
        }
        sin6.sin6_addr = unsafe { (*(ai.ai_addr as *const libc::sockaddr_in6)).sin6_addr };
        unsafe { libc::freeaddrinfo(res) };
    }
    if let Some(i) = cut.iter().position(|&b| b == b'%') {
        let scope = &cut[i + 1..];
        let mut id: u32 = 0;
        match rhp_values::conv::numeric_string(scope) {
            Some(rhp_values::conv::Num::Int(n)) if scope.iter().all(|b| b.is_ascii_digit()) => {
                if n > 0 && n <= u32::MAX as i64 {
                    id = n as u32;
                }
            }
            _ => {
                let cs = CString::new(scope).unwrap_or_default();
                id = unsafe { libc::if_nametoindex(cs.as_ptr()) };
            }
        }
        sin6.sin6_scope_id = id;
    }
    Ok(true)
}

/// A `sockaddr_un` for `addr`, or php's "must be less than" ValueError.
fn sockaddr_un(f: &str, n: usize, addr: &[u8]) -> Result<libc::sockaddr_un, Thrown> {
    let mut su: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if addr.len() >= su.sun_path.len() {
        return Err(arg_value_error(
            f,
            n,
            &format!("must be less than {}", su.sun_path.len()),
        ));
    }
    su.sun_family = libc::AF_UNIX as _;
    for (d, &b) in su.sun_path.iter_mut().zip(addr) {
        *d = b as c_char;
    }
    Ok(su)
}

fn sun_offset() -> usize {
    std::mem::offset_of!(libc::sockaddr_un, sun_path)
}

fn sin_for(port: i64, addr: libc::in_addr) -> libc::sockaddr_in {
    let mut sin: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sin.sin_family = libc::AF_INET as _;
    sin.sin_port = (port as u16).to_be();
    sin.sin_addr = addr;
    sin
}

fn sin6_zero(port: i64) -> libc::sockaddr_in6 {
    let mut sin6: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
    sin6.sin6_family = libc::AF_INET6 as _;
    sin6.sin6_port = (port as u16).to_be();
    sin6
}

fn sa<T>(v: &T) -> (*const libc::sockaddr, libc::socklen_t) {
    (
        v as *const T as *const libc::sockaddr,
        std::mem::size_of::<T>() as libc::socklen_t,
    )
}

pub(super) fn fn_socket_connect(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_connect";
    let s = valid(host, args, f)?;
    let addr = bytes(args, 1);
    let port = opt_int(args, 2);
    let fd = s.fd.get();
    let r = match s.family.get() {
        libc::AF_INET6 => {
            let Some(port) = port else {
                return Err(arg_value_error(
                    f,
                    3,
                    "cannot be null when the socket type is AF_INET6",
                ));
            };
            let mut sin6 = sin6_zero(port);
            if !set_inet6_addr(host, &s, f, &addr, &mut sin6)? {
                return Ok(Value::Bool(false));
            }
            let (p, l) = sa(&sin6);
            unsafe { libc::connect(fd, p, l) }
        }
        libc::AF_INET => {
            let Some(port) = port else {
                return Err(arg_value_error(
                    f,
                    3,
                    "cannot be null when the socket type is AF_INET",
                ));
            };
            let Some(a) = set_inet_addr(host, &s, f, &addr)? else {
                return Ok(Value::Bool(false));
            };
            let sin = sin_for(port, a);
            let (p, l) = sa(&sin);
            unsafe { libc::connect(fd, p, l) }
        }
        libc::AF_UNIX => {
            let su = sockaddr_un(f, 2, &addr)?;
            let len = (sun_offset() + addr.len()) as libc::socklen_t;
            unsafe { libc::connect(fd, &su as *const _ as *const libc::sockaddr, len) }
        }
        _ => {
            return Err(arg_value_error(
                f,
                1,
                "must be one of AF_UNIX, AF_INET, or AF_INET6",
            ))
        }
    };
    if r != 0 {
        socket_error(host, &s, f, "unable to connect", errno())?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

pub(super) fn fn_socket_bind(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_bind";
    let s = valid(host, args, f)?;
    let addr = bytes(args, 1);
    let port = int(args, 2, 0);
    if !(0..=u16::MAX as i64).contains(&port) {
        return Err(arg_value_error(
            f,
            3,
            &format!("must be between 0 and {}", u16::MAX),
        ));
    }
    let fd = s.fd.get();
    let r = match s.family.get() {
        libc::AF_UNIX => {
            let su = sockaddr_un(f, 2, &addr)?;
            let len = (sun_offset() + addr.len()) as libc::socklen_t;
            unsafe { libc::bind(fd, &su as *const _ as *const libc::sockaddr, len) }
        }
        libc::AF_INET => {
            let Some(a) = set_inet_addr(host, &s, f, &addr)? else {
                return Ok(Value::Bool(false));
            };
            let sin = sin_for(port, a);
            let (p, l) = sa(&sin);
            unsafe { libc::bind(fd, p, l) }
        }
        libc::AF_INET6 => {
            let mut sin6 = sin6_zero(port);
            if !set_inet6_addr(host, &s, f, &addr, &mut sin6)? {
                return Ok(Value::Bool(false));
            }
            let (p, l) = sa(&sin6);
            unsafe { libc::bind(fd, p, l) }
        }
        #[cfg(target_os = "linux")]
        libc::AF_PACKET => {
            let mut st: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
            // php passes sizeof(sa), a pointer's size.
            let mut len = std::mem::size_of::<*const u8>() as libc::socklen_t;
            if unsafe { libc::getsockname(fd, &mut st as *mut _ as *mut libc::sockaddr, &mut len) }
                < 0
            {
                return Err(Thrown::error("ValueError", "invalid AF_PACKET socket"));
            }
            let sll = unsafe { &mut *(&mut st as *mut _ as *mut libc::sockaddr_ll) };
            let c = CString::new(addr.clone()).unwrap_or_default();
            sll.sll_ifindex = unsafe { libc::if_nametoindex(c.as_ptr()) } as i32;
            let l = std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;
            unsafe { libc::bind(fd, &st as *const _ as *const libc::sockaddr, l) }
        }
        _ => {
            return Err(arg_value_error(
                f,
                1,
                "must be one of AF_UNIX, AF_PACKET, AF_INET, or AF_INET6",
            ))
        }
    };
    if r != 0 {
        socket_error(host, &s, f, "Unable to bind address", errno())?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

pub(super) fn fn_socket_recv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_recv";
    let s = valid(host, args, f)?;
    let len = int(args, 2, 0);
    let flags = int(args, 3, 0);
    if len <= 0 || len == i64::MAX {
        return Ok(Value::Bool(false));
    }
    let mut buf = vec![0u8; len as usize];
    let r = unsafe {
        libc::recv(
            s.fd.get(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            flags as c_int,
        )
    };
    let e = errno();
    if r < 1 {
        try_assign_ref(host, args, 1, Value::Null)?;
    } else {
        buf.truncate(r as usize);
        try_assign_ref(host, args, 1, Value::str(buf))?;
    }
    if r == -1 {
        socket_error(host, &s, f, "Unable to read from socket", e)?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(r as i64))
}

pub(super) fn fn_socket_send(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_send";
    let s = valid(host, args, f)?;
    let data = bytes(args, 1);
    let len = int(args, 2, 0);
    let flags = int(args, 3, 0);
    if len < 0 {
        return Err(arg_value_error(f, 3, "must be greater than or equal to 0"));
    }
    let n = (len as u64).min(data.len() as u64) as usize;
    let r = unsafe { libc::send(s.fd.get(), data.as_ptr().cast(), n, flags as c_int) };
    if r == -1 {
        socket_error(host, &s, f, "Unable to write to socket", errno())?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(r as i64))
}

pub(super) fn fn_socket_recvfrom(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_recvfrom";
    let s = valid(host, args, f)?;
    let len = int(args, 2, 0);
    let flags = int(args, 3, 0) as c_int;
    if len <= 0 || len > i64::MAX - 1 {
        return Ok(Value::Bool(false));
    }
    let mut buf = vec![0u8; len as usize + 1];
    let fd = s.fd.get();
    let mut st: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let stp = &mut st as *mut _ as *mut libc::sockaddr;
    let family = s.family.get();
    let mut slen = match family {
        libc::AF_UNIX => std::mem::size_of::<libc::sockaddr_un>(),
        libc::AF_INET => std::mem::size_of::<libc::sockaddr_in>(),
        libc::AF_INET6 => std::mem::size_of::<libc::sockaddr_in6>(),
        _ => {
            return Err(arg_value_error(
                f,
                1,
                "must be one of AF_UNIX, AF_INET, or AF_INET6",
            ))
        }
    } as libc::socklen_t;
    st.ss_family = family as _;
    if family != libc::AF_UNIX && args.len() < 6 {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!("Wrong parameter count for {f}()"),
        ));
    }
    let r = unsafe {
        libc::recvfrom(
            fd,
            buf.as_mut_ptr().cast(),
            len as usize,
            flags,
            stp,
            &mut slen,
        )
    };
    if r < 0 {
        let msg = if family == libc::AF_INET6 {
            "unable to recvfrom"
        } else {
            "Unable to recvfrom"
        };
        socket_error(host, &s, f, msg, errno())?;
        return Ok(Value::Bool(false));
    }
    buf.truncate(r as usize);
    try_assign_ref(host, args, 1, Value::str(buf))?;
    match named(&st) {
        Named::Unix(p) => try_assign_ref(host, args, 4, Value::str(p))?,
        Named::Inet(a, port) => {
            let a = if a.is_empty() {
                if family == libc::AF_INET6 {
                    "::".to_string()
                } else {
                    "0.0.0.0".to_string()
                }
            } else {
                a
            };
            try_assign_ref(host, args, 4, Value::str(a))?;
            try_assign_ref(host, args, 5, Value::Int(port as i64))?;
        }
        _ => {
            // A datagram from an unnamed peer keeps the family php set.
            if family == libc::AF_UNIX {
                try_assign_ref(host, args, 4, Value::str(""))?;
            } else {
                let a = if family == libc::AF_INET6 {
                    "::"
                } else {
                    "0.0.0.0"
                };
                try_assign_ref(host, args, 4, Value::str(a))?;
                try_assign_ref(host, args, 5, Value::Int(0))?;
            }
        }
    }
    Ok(Value::Int(r as i64))
}

pub(super) fn fn_socket_sendto(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_sendto";
    let s = valid(host, args, f)?;
    let data = bytes(args, 1);
    let len = int(args, 2, 0);
    let flags = int(args, 3, 0) as c_int;
    let addr = bytes(args, 4);
    let port = opt_int(args, 5);
    let p = port.unwrap_or(0);
    if !(0..=u16::MAX as i64).contains(&p) {
        return Err(arg_value_error(
            f,
            6,
            &format!("must be between 0 and {}", u16::MAX),
        ));
    }
    if len < 0 {
        return Err(arg_value_error(f, 3, "must be greater than or equal to 0"));
    }
    let n = (len as u64).min(data.len() as u64) as usize;
    let fd = s.fd.get();
    let buf = data.as_ptr().cast();
    let r = match s.family.get() {
        libc::AF_UNIX => {
            let mut su = sockaddr_un(f, 5, &addr)?;
            // snprintf("%s"): cut at a NUL.
            if let Some(z) = addr.iter().position(|&b| b == 0) {
                for c in su.sun_path[z..].iter_mut() {
                    *c = 0;
                }
            }
            let l = (sun_offset() + sun_path(&su).len()) as libc::socklen_t;
            unsafe {
                libc::sendto(
                    fd,
                    buf,
                    n,
                    flags,
                    &su as *const _ as *const libc::sockaddr,
                    l,
                )
            }
        }
        libc::AF_INET => {
            let Some(port) = port else {
                return Err(arg_value_error(
                    f,
                    6,
                    "cannot be null when the socket type is AF_INET",
                ));
            };
            let Some(a) = set_inet_addr(host, &s, f, &addr)? else {
                return Ok(Value::Bool(false));
            };
            let sin = sin_for(port, a);
            let (sp, sl) = sa(&sin);
            unsafe { libc::sendto(fd, buf, n, flags, sp, sl) }
        }
        libc::AF_INET6 => {
            let Some(port) = port else {
                return Err(arg_value_error(
                    f,
                    6,
                    "cannot be null when the socket type is AF_INET6",
                ));
            };
            let mut sin6 = sin6_zero(port);
            if !set_inet6_addr(host, &s, f, &addr, &mut sin6)? {
                return Ok(Value::Bool(false));
            }
            let (sp, sl) = sa(&sin6);
            unsafe { libc::sendto(fd, buf, n, flags, sp, sl) }
        }
        _ => {
            return Err(arg_value_error(
                f,
                1,
                "must be one of AF_UNIX, AF_INET, or AF_INET6",
            ))
        }
    };
    if r == -1 {
        socket_error(host, &s, f, "Unable to write to socket", errno())?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(r as i64))
}

pub(super) fn fn_socket_create_pair(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_create_pair";
    let domain = int(args, 0, 0);
    let ty = int(args, 1, 0);
    let protocol = int(args, 2, 0);
    if domain != libc::AF_INET as i64
        && domain != libc::AF_INET6 as i64
        && domain != libc::AF_UNIX as i64
    {
        return Err(arg_value_error(
            f,
            1,
            "must be one of AF_UNIX, AF_INET6, or AF_INET",
        ));
    }
    type_check(f, ty)?;
    let mut fds = [0 as c_int; 2];
    if unsafe {
        libc::socketpair(
            domain as c_int,
            ty as c_int,
            protocol as c_int,
            fds.as_mut_ptr(),
        )
    } != 0
    {
        warn_errno(host, f, "Unable to create socket pair", errno())?;
        return Ok(Value::Bool(false));
    }
    try_assign_ref(host, args, 3, Value::Array(PhpArray::new()))?;
    let (v0, s0) = new_socket(host, f)?;
    let (v1, s1) = new_socket(host, f)?;
    for (s, fd) in [(&s0, fds[0]), (&s1, fds[1])] {
        s.fd.set(fd);
        s.family.set(domain as i32);
    }
    let mut a = PhpArray::with_capacity(2);
    a.insert(ArrayKey::Int(0), v0);
    a.insert(ArrayKey::Int(1), v1);
    try_assign_ref(host, args, 3, Value::Array(a))?;
    Ok(Value::Bool(true))
}

pub(super) fn fn_socket_shutdown(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_shutdown";
    let s = valid(host, args, f)?;
    let how = int(args, 1, 2);
    if !(libc::SHUT_RD as i64..=libc::SHUT_RDWR as i64).contains(&how) {
        return Err(arg_value_error(
            f,
            2,
            "must be one of SHUT_RD, SHUT_WR or SHUT_RDWR",
        ));
    }
    if unsafe { libc::shutdown(s.fd.get(), how as c_int) } != 0 {
        socket_error(host, &s, f, "Unable to shutdown socket", errno())?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

extern "C" {
    fn sockatmark(fd: c_int) -> c_int;
}

pub(super) fn fn_socket_atmark(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_atmark";
    let s = valid(host, args, f)?;
    let r = unsafe { sockatmark(s.fd.get()) };
    if r < 0 {
        socket_error(host, &s, f, "Unable to apply sockmark", errno())?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(r != 0))
}

pub(super) fn fn_socket_export_stream(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let f = "socket_export_stream";
    let s = valid(host, args, f)?;
    if let Some(stream) = s.stream.borrow().clone() {
        return Ok(stream);
    }
    let fd = s.fd.get();
    let opt = |name: c_int| -> c_int {
        let mut v: c_int = 0;
        let mut l = std::mem::size_of::<c_int>() as libc::socklen_t;
        unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                name,
                &mut v as *mut _ as *mut _,
                &mut l,
            )
        };
        v
    };
    let family = s.family.get();
    let mut ty = "generic_socket";
    if family == libc::PF_INET || family == libc::PF_INET6 {
        let st = opt(libc::SO_TYPE);
        if st == libc::SOCK_STREAM {
            #[cfg(target_os = "linux")]
            let tcp = opt(libc::SO_PROTOCOL) == libc::IPPROTO_TCP;
            #[cfg(not(target_os = "linux"))]
            let tcp = true;
            if tcp {
                ty = "tcp_socket";
            }
        } else if st == libc::SOCK_DGRAM {
            ty = "udp_socket";
        }
    } else if family == libc::PF_UNIX {
        let st = opt(libc::SO_TYPE);
        if st == libc::SOCK_STREAM {
            ty = "unix_socket";
        } else if st == libc::SOCK_DGRAM {
            ty = "udg_socket";
        }
    }
    let timeout = crate::main::streams::xp_socket::default_socket_timeout(host);
    let mut info = crate::main::streams::plain_wrapper::SockInfo::new(
        ty,
        (timeout >= 0).then_some((timeout, 0)),
    );
    info.blocked = s.blocking.get();
    let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    let Some(stream) = host.stream_from_socket(owned, info) else {
        crate::warn(host, f, "Failed to create stream")?;
        return Ok(Value::Bool(false));
    };
    *s.stream.borrow_mut() = Some(stream.clone());
    Ok(stream)
}

/// php_sock_array_to_fd_set: the descriptors of an array argument.
fn fd_set_of(
    host: &dyn Host,
    f: &str,
    n: usize,
    arr: &PhpArray,
    set: &mut libc::fd_set,
    max: &mut i32,
) -> Result<usize, Thrown> {
    let mut count = 0;
    for v in arr.values() {
        let v = deref(v);
        let Some(s) = crate::zend::object_handlers::payload::<Sock>(&v) else {
            return Err(Thrown::type_error(format!(
                "{f}(): Argument #{n} (${}) must only have elements of type Socket, {} given",
                pname(f, n),
                crate::zend::zpp::zval_value_name(host, &v)
            )));
        };
        let fd = s.fd.get();
        if fd < 0 {
            return Err(Thrown::type_error(format!(
                "{f}(): Argument #{n} (${}) contains a closed socket",
                pname(f, n)
            )));
        }
        if (fd as usize) < libc::FD_SETSIZE {
            unsafe { libc::FD_SET(fd, set) };
        }
        *max = (*max).max(fd);
        count += 1;
    }
    Ok(count)
}

/// php_sock_array_from_fd_set: the entries whose descriptor is set, keys
/// kept.
fn from_fd_set(arr: &PhpArray, set: &libc::fd_set) -> PhpArray {
    let mut out = PhpArray::new();
    for (k, v) in arr.iter() {
        let d = deref(v);
        let Some(s) = crate::zend::object_handlers::payload::<Sock>(&d) else {
            continue;
        };
        let fd = s.fd.get();
        if fd >= 0 && (fd as usize) < libc::FD_SETSIZE && unsafe { libc::FD_ISSET(fd, set) } {
            out.insert(k, d);
        }
    }
    out
}

pub(super) fn fn_socket_select(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "socket_select";
    let arrays: Vec<Option<PhpArray>> = (0..3)
        .map(|i| match args.get(i).map(deref) {
            Some(Value::Array(a)) => Some(a),
            _ => None,
        })
        .collect();
    let mut sets: [libc::fd_set; 3] = unsafe { std::mem::zeroed() };
    for s in sets.iter_mut() {
        unsafe { libc::FD_ZERO(s) };
    }
    let mut max_fd = 0;
    let mut max_count = 0;
    for (i, a) in arrays.iter().enumerate() {
        if let Some(a) = a {
            let n = fd_set_of(host, f, i + 1, a, &mut sets[i], &mut max_fd)?;
            max_count = max_count.max(n);
        }
    }
    if max_count == 0 {
        return Err(Thrown::error(
            "ValueError",
            "socket_select(): At least one array argument must be passed",
        ));
    }
    if max_fd as usize >= libc::FD_SETSIZE {
        crate::warn(
            host,
            f,
            &format!(
                "You MUST recompile PHP with a larger value of FD_SETSIZE.\n\
                 It is set to {}, but you have descriptors numbered at least as high as {}.\n \
                 --enable-fd-setsize={} is recommended, but you may want to set it\n\
                 to equal the maximum number of open files supported by your system,\n\
                 in order to avoid seeing this error again at a later date.",
                libc::FD_SETSIZE,
                max_fd,
                (max_fd + 1024) & !1023
            ),
        )?;
        return Ok(Value::Bool(false));
    }
    let sec = opt_int(args, 3);
    let usec = int(args, 4, 0);
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    let tvp = match sec {
        Some(sec) => {
            if usec > 999_999 {
                tv.tv_sec = (sec + usec / 1_000_000) as _;
                tv.tv_usec = (usec % 1_000_000) as _;
            } else {
                tv.tv_sec = sec as _;
                tv.tv_usec = usec as _;
            }
            &mut tv as *mut libc::timeval
        }
        None => std::ptr::null_mut(),
    };
    let [r, w, e] = &mut sets;
    let n = unsafe { libc::select(max_fd + 1, r, w, e, tvp) };
    if n == -1 {
        let err = errno();
        LAST_ERROR.with(|g| g.set(err));
        crate::warn(
            host,
            f,
            &format!("Unable to select [{err}]: {}", sockets_strerror(err)),
        )?;
        return Ok(Value::Bool(false));
    }
    for (i, a) in arrays.iter().enumerate() {
        if let Some(a) = a {
            let out = from_fd_set(a, &sets[i]);
            if let Some(Value::Ref(rc)) = args.get(i) {
                *rc.borrow_mut() = Value::Array(out);
            }
        }
    }
    Ok(Value::Int(n as i64))
}

// ---------------------------------------------------------------------------
// AddressInfo
// ---------------------------------------------------------------------------

/// An `AddressInfo` object's payload (php's `php_addrinfo`).
pub(super) struct AddrInfo {
    flags: i32,
    family: i32,
    socktype: i32,
    protocol: i32,
    addr: Vec<u8>,
    canonname: Option<Vec<u8>>,
}

fn addrinfo_arg(host: &dyn Host, args: &[Value], f: &str) -> Result<Rc<AddrInfo>, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    crate::zend::object_handlers::payload::<AddrInfo>(&v).ok_or_else(|| {
        Thrown::type_error(format!(
            "{f}(): Argument #1 ($address) must be of type AddressInfo, {} given",
            crate::zend::zpp::zval_value_name(host, &v)
        ))
    })
}

const HINT_KEYS: &str =
    "must only contain array keys \"ai_flags\", \"ai_socktype\", \"ai_protocol\", or \"ai_family\"";

pub(super) fn fn_socket_addrinfo_lookup(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let f = "socket_addrinfo_lookup";
    let hostname = bytes(args, 0);
    let service = match args.get(1).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(rhp_values::conv::to_string_bytes(&v)),
    };
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    if let Some(Value::Array(h)) = args.get(2).map(deref) {
        if h.is_packed() && !h.is_empty() {
            return Err(arg_value_error(f, 3, HINT_KEYS));
        }
        for (k, v) in h.iter() {
            let ArrayKey::Str(k) = k else {
                return Err(arg_value_error(f, 3, HINT_KEYS));
            };
            let key = k.as_bytes();
            let name = std::str::from_utf8(key).unwrap_or("");
            if !matches!(
                name,
                "ai_flags" | "ai_socktype" | "ai_protocol" | "ai_family"
            ) {
                return Err(arg_value_error(f, 3, HINT_KEYS));
            }
            let d = deref(v);
            let Some(val) = crate::zend::zpp::try_get_long(host, &d)? else {
                return Err(Thrown::type_error(format!(
                    "{f}(): Argument #3 ($hints) \"{name}\" key must be of type int, {} given",
                    zval_type_name(host, &d)
                )));
            };
            if name == "ai_family" {
                if val != libc::AF_INET as i64 && val != libc::AF_INET6 as i64 {
                    return Err(arg_value_error(
                        f,
                        3,
                        "\"ai_family\" key must be AF_INET or AF_INET6",
                    ));
                }
                hints.ai_family = val as c_int;
                continue;
            }
            if !(0..=i32::MAX as i64).contains(&val) {
                return Err(arg_value_error(
                    f,
                    3,
                    &format!("\"{name}\" key must be between 0 and {}", i32::MAX),
                ));
            }
            match name {
                "ai_flags" => hints.ai_flags = val as c_int,
                "ai_socktype" => hints.ai_socktype = val as c_int,
                _ => hints.ai_protocol = val as c_int,
            }
        }
    }
    let hc = CString::new(
        &hostname[..hostname
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(hostname.len())],
    )
    .unwrap_or_default();
    let sc = service.map(|s| {
        CString::new(&s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]).unwrap_or_default()
    });
    let mut res: *mut libc::addrinfo = std::ptr::null_mut();
    let r = unsafe {
        libc::getaddrinfo(
            hc.as_ptr(),
            sc.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            &hints,
            &mut res,
        )
    };
    if r != 0 {
        return Ok(Value::Bool(false));
    }
    let mut out = PhpArray::new();
    let mut rp = res;
    while !rp.is_null() {
        let ai = unsafe { &*rp };
        rp = ai.ai_next;
        if ai.ai_family == libc::AF_UNSPEC {
            continue;
        }
        let addr =
            unsafe { std::slice::from_raw_parts(ai.ai_addr as *const u8, ai.ai_addrlen as usize) }
                .to_vec();
        let canonname = (!ai.ai_canonname.is_null()).then(|| {
            unsafe { CStr::from_ptr(ai.ai_canonname) }
                .to_bytes()
                .to_vec()
        });
        let v = host
            .instantiate(b"AddressInfo", Vec::new())
            .ok_or_else(|| Thrown::unsupported(f, "cannot instantiate AddressInfo"))?;
        let h = crate::zend::object_handlers::object_of(&v)
            .ok_or_else(|| Thrown::error("Error", "sockets: bad object"))?;
        crate::zend::object_handlers::set_payload(
            &h,
            Rc::new(AddrInfo {
                flags: ai.ai_flags,
                family: ai.ai_family,
                socktype: ai.ai_socktype,
                protocol: ai.ai_protocol,
                addr,
                canonname,
            }),
        );
        out.push(v);
    }
    unsafe { libc::freeaddrinfo(res) };
    Ok(Value::Array(out))
}

fn addrinfo_socket(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    connect: bool,
) -> Result<Value, Thrown> {
    let ai = addrinfo_arg(host, args, f)?;
    let protocol = eth_proto(ai.protocol as i64, ai.family as i64);
    let (v, s) = new_socket(host, f)?;
    let fd = unsafe { libc::socket(ai.family, ai.socktype, protocol as c_int) };
    s.family.set(ai.family);
    if fd < 0 {
        warn_errno(host, f, "Unable to create socket", errno())?;
        return Ok(Value::Bool(false));
    }
    s.fd.set(fd);
    let (p, l) = (
        ai.addr.as_ptr() as *const libc::sockaddr,
        ai.addr.len() as libc::socklen_t,
    );
    let r = unsafe {
        if connect {
            libc::connect(fd, p, l)
        } else {
            libc::bind(fd, p, l)
        }
    };
    if r != 0 {
        let msg = if connect {
            "Unable to connect address"
        } else {
            "Unable to bind address"
        };
        socket_error(host, &s, f, msg, errno())?;
        unsafe { libc::close(fd) };
        s.fd.set(-1);
        return Ok(Value::Bool(false));
    }
    Ok(v)
}

pub(super) fn fn_socket_addrinfo_bind(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    addrinfo_socket(h, a, "socket_addrinfo_bind", false)
}

pub(super) fn fn_socket_addrinfo_connect(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    addrinfo_socket(h, a, "socket_addrinfo_connect", true)
}

pub(super) fn fn_socket_addrinfo_explain(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let ai = addrinfo_arg(host, args, "socket_addrinfo_explain")?;
    let k = |s: &str| ArrayKey::from_bytes(s.as_bytes());
    let mut r = PhpArray::new();
    r.insert(k("ai_flags"), Value::Int(ai.flags as i64));
    r.insert(k("ai_family"), Value::Int(ai.family as i64));
    r.insert(k("ai_socktype"), Value::Int(ai.socktype as i64));
    r.insert(k("ai_protocol"), Value::Int(ai.protocol as i64));
    if let Some(c) = &ai.canonname {
        r.insert(k("ai_canonname"), Value::str(c.clone()));
    }
    let mut sa = PhpArray::new();
    let mut st: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let n = ai
        .addr
        .len()
        .min(std::mem::size_of::<libc::sockaddr_storage>());
    unsafe { std::ptr::copy_nonoverlapping(ai.addr.as_ptr(), &mut st as *mut _ as *mut u8, n) };
    if let Named::Inet(a, port) = named(&st) {
        if ai.family == libc::AF_INET {
            sa.insert(k("sin_port"), Value::Int(port as i64));
            sa.insert(k("sin_addr"), Value::str(a));
        } else if ai.family == libc::AF_INET6 {
            sa.insert(k("sin6_port"), Value::Int(port as i64));
            sa.insert(k("sin6_addr"), Value::str(a));
        }
    }
    r.insert(k("ai_addr"), Value::Array(sa));
    Ok(Value::Array(r))
}
