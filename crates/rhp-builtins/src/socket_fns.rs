//! Socket streams: php's `main/streams/xp_socket.c` + `main/network.c`
//! transports behind `stream_socket_server` / `_client` / `_accept` /
//! `_get_name` / `_shutdown` / `_pair` / `_sendto` / `_recvfrom`,
//! `fsockopen` / `pfsockopen`, `stream_set_timeout` and the socket knobs
//! (`stream_set_read_buffer`, `stream_set_chunk_size`,
//! `stream_socket_enable_crypto`).
//!
//! The socket descriptor is created and connected / bound here; the VM wraps
//! it as a stream resource ([`Host::stream_from_socket`]) whose reads and
//! writes follow `php_sockop_read` / `php_sockop_write`
//! ([`crate::fdstream::FdStream`] with [`SockInfo`]).

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::fdstream::{poll_fd, SockInfo};
use crate::proc_fns::{zpp_long, zpp_string, zpp_type_name};
use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn write_ref(arg: Option<&Value>, value: Value) {
    if let Some(Value::Ref(rc)) = arg {
        *rc.borrow_mut() = value;
    }
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn strerror(e: i32) -> String {
    crate::streams::strerror(e)
}

// php's xport flags.
const STREAM_CLIENT_PERSISTENT: i64 = 1;
const STREAM_CLIENT_ASYNC_CONNECT: i64 = 2;
const STREAM_CLIENT_CONNECT: i64 = 4;
const STREAM_SERVER_BIND: i64 = 4;
const STREAM_SERVER_LISTEN: i64 = 8;
const STREAM_OOB: i64 = 1;
const STREAM_PEEK: i64 = 2;

/// The largest buffer `stream_socket_recvfrom` hands to one `recv`.
const RECV_CAP: usize = 16 << 20;

/// php's `PHP_TIMEOUT_ERROR_VALUE`.
const TIMEOUT_ERR: i32 = libc::ETIMEDOUT;

type Tv = Option<(i64, i64)>;

/// `Z_PARAM_DOUBLE_OR_NULL` (coercive): `None` for null / absent.
fn zpp_double_or_null(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Option<f64>, Thrown> {
    let v = match args.get(i).map(deref) {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v,
    };
    let f = match &v {
        Value::Int(n) => Some(*n as f64),
        Value::Float(f) => Some(*f),
        Value::Bool(b) => Some(*b as i64 as f64),
        Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
            Some(conv::Num::Int(n)) => Some(n as f64),
            Some(conv::Num::Float(f)) => Some(f),
            None => None,
        },
        _ => None,
    };
    f.map(Some).ok_or_else(|| {
        Thrown::type_error(format!(
            "{fname}(): Argument #{} (${pname}) must be of type ?float, {} given",
            i + 1,
            zpp_type_name(host, &v)
        ))
    })
}

/// Socket streams live in the VM; a host without stream resources (the
/// oracle) reports them unsupported.
fn need_streams(host: &dyn Host, fname: &str) -> Result<(), Thrown> {
    if host.has_streams() {
        Ok(())
    } else {
        Err(Thrown::unsupported(fname, "socket streams need the VM"))
    }
}

/// php's timeout double → timeval (`None` when negative or too large).
fn timeout_tv(t: f64) -> Tv {
    if t < 0.0 || t >= u64::MAX as f64 / 1_000_000.0 {
        return None;
    }
    let conv = (t * 1_000_000.0) as u64;
    Some(((conv / 1_000_000) as i64, (conv % 1_000_000) as i64))
}

/// `FG(default_socket_timeout)`.
fn default_socket_timeout(host: &dyn Host) -> i64 {
    host.ini_get(b"default_socket_timeout")
        .map(|v| conv::to_int(&Value::str(v)))
        .unwrap_or(60)
}

/// The socket timeout a new socket stream starts with (`tv_sec ==
/// default_socket_timeout`; -1 means forever).
fn default_sock_tv(host: &dyn Host) -> Tv {
    let t = default_socket_timeout(host);
    (t >= 0).then_some((t, 0))
}

/// A stream argument (`PHP_Z_PARAM_STREAM`).
fn stream_arg(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Value, Thrown> {
    crate::fs_fns::stream_param(host, args, i, fname, pname).map(|(v, _)| v)
}

/// A `socket` context option of the stream context `ctx` (or of the
/// default context).
fn ctx_socket_opt(host: &dyn Host, ctx: &Value, name: &str) -> Option<Value> {
    let Value::Array(opts) = host.stream_context_options(ctx) else {
        return None;
    };
    let Some(Value::Array(sock)) = opts.get(&ArrayKey::from_bytes(b"socket")) else {
        return None;
    };
    sock.get(&ArrayKey::from_bytes(name.as_bytes())).cloned()
}

// --- addresses ---------------------------------------------------------------

/// C `atoi`: leading whitespace, optional sign, digits (wrapping like an int).
fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut n: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        n = n.wrapping_mul(10).wrapping_add((s[i] - b'0') as i64);
        i += 1;
    }
    (if neg { -n } else { n }) as i32
}

/// php's `parse_ip_address_ex`: `host:port` or `[v6]:port`.
fn parse_ip_address(s: &[u8]) -> Result<(Vec<u8>, i32), String> {
    if s.contains(&0) {
        return Err("The hostname must not contain null bytes".into());
    }
    let shown = String::from_utf8_lossy(s);
    if s.first() == Some(&b'[') && s.len() > 1 {
        let inner = &s[1..s.len() - 1];
        return match inner.iter().position(|&b| b == b']') {
            Some(p) if s.get(p + 2) == Some(&b':') => Ok((s[1..p + 1].to_vec(), atoi(&s[p + 3..]))),
            _ => Err(format!("Failed to parse IPv6 address \"{shown}\"")),
        };
    }
    let colon = if s.is_empty() {
        None
    } else {
        s[..s.len() - 1].iter().position(|&b| b == b':')
    };
    match colon {
        Some(c) => Ok((s[..c].to_vec(), atoi(&s[c + 1..]))),
        None => Err(format!("Failed to parse address \"{shown}\"")),
    }
}

/// A resolved socket address.
struct Addr {
    storage: libc::sockaddr_storage,
    len: libc::socklen_t,
}

impl Addr {
    fn family(&self) -> i32 {
        self.storage.ss_family as i32
    }
    fn ptr(&self) -> *const libc::sockaddr {
        &self.storage as *const _ as *const libc::sockaddr
    }
    fn set_port(&mut self, port: u16) {
        unsafe {
            match self.family() {
                libc::AF_INET => {
                    let sin = &mut *(&mut self.storage as *mut _ as *mut libc::sockaddr_in);
                    sin.sin_port = port.to_be();
                    self.len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
                }
                libc::AF_INET6 => {
                    let sin6 = &mut *(&mut self.storage as *mut _ as *mut libc::sockaddr_in6);
                    sin6.sin6_port = port.to_be();
                    self.len = std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t;
                }
                _ => {}
            }
        }
    }
}

/// php's `php_network_getaddresses`: getaddrinfo (AF_UNSPEC when the IPv6
/// stack works) for `socktype`. A failure warns right away (and is the
/// error text).
fn getaddresses(
    host: &mut dyn Host,
    fname: &str,
    name: &[u8],
    socktype: i32,
) -> Result<Vec<Addr>, (String, Result<(), Thrown>)> {
    resolve(name, socktype).map_err(|msg| {
        let w = crate::warn(host, fname, &msg);
        (msg, w)
    })
}

/// [`getaddresses`] without the warning: the error text on failure.
fn resolve(name: &[u8], socktype: i32) -> Result<Vec<Addr>, String> {
    let cname = CString::new(name.to_vec()).unwrap_or_default();
    let v6_ok = unsafe {
        let s = libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0);
        if s >= 0 {
            libc::close(s);
        }
        s >= 0
    };
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_family = if v6_ok {
        libc::AF_UNSPEC
    } else {
        libc::AF_INET
    };
    hints.ai_socktype = socktype;
    let mut res: *mut libc::addrinfo = std::ptr::null_mut();
    let n = unsafe { libc::getaddrinfo(cname.as_ptr(), std::ptr::null(), &hints, &mut res) };
    if n != 0 || res.is_null() {
        let msg = if n != 0 {
            let e = unsafe { CStr::from_ptr(libc::gai_strerror(n)) };
            format!(
                "php_network_getaddresses: getaddrinfo for {} failed: {}",
                String::from_utf8_lossy(name),
                e.to_string_lossy()
            )
        } else {
            format!(
                "php_network_getaddresses: getaddrinfo for {} failed (null result pointer) errno={}",
                String::from_utf8_lossy(name),
                errno()
            )
        };
        return Err(msg);
    }
    let mut out = Vec::new();
    let mut p = res;
    while !p.is_null() {
        unsafe {
            let ai = &*p;
            let mut storage: libc::sockaddr_storage = std::mem::zeroed();
            let len = (ai.ai_addrlen as usize).min(std::mem::size_of::<libc::sockaddr_storage>());
            std::ptr::copy_nonoverlapping(
                ai.ai_addr as *const u8,
                &mut storage as *mut _ as *mut u8,
                len,
            );
            out.push(Addr {
                storage,
                len: len as libc::socklen_t,
            });
            p = ai.ai_next;
        }
    }
    unsafe { libc::freeaddrinfo(res) };
    Ok(out)
}

/// php's `php_network_populate_name_from_sockaddr` text form.
fn sockaddr_text(sa: &libc::sockaddr_storage, sl: libc::socklen_t) -> Option<Vec<u8>> {
    unsafe {
        match sa.ss_family as i32 {
            libc::AF_INET => {
                let sin = &*(sa as *const _ as *const libc::sockaddr_in);
                let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                Some(format!("{}:{}", ip, u16::from_be(sin.sin_port)).into_bytes())
            }
            libc::AF_INET6 => {
                let sin6 = &*(sa as *const _ as *const libc::sockaddr_in6);
                let ip = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
                Some(format!("[{}]:{}", ip, u16::from_be(sin6.sin6_port)).into_bytes())
            }
            libc::AF_UNIX => {
                let un = &*(sa as *const _ as *const libc::sockaddr_un);
                let path: Vec<u8> = un.sun_path.iter().map(|&c| c as u8).collect();
                // php: `sl - sizeof(sa_family_t)` bytes of an abstract name
                // (on BSDs that includes one byte past the reported length).
                let hdr = std::mem::size_of::<libc::sa_family_t>();
                if path.first() == Some(&0) {
                    let len = (sl as usize).saturating_sub(hdr).min(path.len());
                    Some(path[..len].to_vec())
                } else {
                    let end = path.iter().position(|&b| b == 0).unwrap_or(path.len());
                    Some(path[..end].to_vec())
                }
            }
            _ => None,
        }
    }
}

fn sock_name(fd: RawFd, peer: bool) -> Option<Vec<u8>> {
    let mut sa: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut sl = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let r = unsafe {
        let p = &mut sa as *mut _ as *mut libc::sockaddr;
        if peer {
            libc::getpeername(fd, p, &mut sl)
        } else {
            libc::getsockname(fd, p, &mut sl)
        }
    };
    if r != 0 {
        return None;
    }
    sockaddr_text(&sa, sl)
}

/// A `sockaddr_un` for `path` (php's `parse_unix_address`, truncating an
/// over-long path with a notice).
fn unix_addr(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<Addr, Thrown> {
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let un = unsafe { &mut *(&mut storage as *mut _ as *mut libc::sockaddr_un) };
    un.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let cap = un.sun_path.len();
    let abstract_ns = path.first() == Some(&0);
    let max = if abstract_ns { cap } else { cap - 1 };
    let mut n = path.len();
    if n > max {
        n = max;
        host.emit_diagnostic(
            crate::E_NOTICE,
            &format!(
                "{fname}(): socket path exceeded the maximum allowed length of {max} bytes and was truncated"
            ),
        )?;
    }
    for (d, &s) in un.sun_path.iter_mut().zip(&path[..n]) {
        *d = s as libc::c_char;
    }
    let off = std::mem::size_of::<libc::sockaddr_un>() - cap;
    Ok(Addr {
        storage,
        len: (off + n) as libc::socklen_t,
    })
}

fn new_socket(family: i32, socktype: i32) -> Option<OwnedFd> {
    let fd = unsafe { libc::socket(family, socktype, 0) };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

fn setsockopt_int(fd: RawFd, level: i32, name: i32, val: i32) {
    unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            &val as *const i32 as *const libc::c_void,
            std::mem::size_of::<i32>() as libc::socklen_t,
        );
    }
}

fn now() -> std::time::Instant {
    std::time::Instant::now()
}

/// The instant a timeout `tv` from now ends (`None` when it is beyond what
/// the clock can represent: as good as forever).
fn deadline((s, us): (i64, i64)) -> Option<std::time::Instant> {
    let d = std::time::Duration::from_secs(s.max(0) as u64)
        + std::time::Duration::from_micros(us.max(0) as u64);
    now().checked_add(d)
}

/// php's `php_network_connect_socket`: a non-blocking `connect`, then (unless
/// asynchronous) a poll for completion within `timeout` and `SO_ERROR`; the
/// original blocking mode is restored for a synchronous connect.
/// `Ok(pending)` on success, `Err(errno)` on failure.
fn connect_socket(fd: RawFd, addr: &Addr, asynchronous: bool, timeout: Tv) -> Result<bool, i32> {
    let orig = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    unsafe { libc::fcntl(fd, libc::F_SETFL, orig | libc::O_NONBLOCK) };
    let n = unsafe { libc::connect(fd, addr.ptr(), addr.len) };
    let mut error = 0;
    if n != 0 {
        error = errno();
        if error != libc::EINPROGRESS {
            return Err(error);
        }
        if asynchronous {
            return Ok(true);
        }
        error = 0;
        let limit = timeout.and_then(deadline);
        let mut working = timeout;
        loop {
            let r = poll_fd(fd, libc::POLLIN | libc::POLLOUT, working);
            if r < 0 {
                if errno() == libc::EINTR {
                    if let Some(l) = limit {
                        let t = now();
                        if t >= l {
                            error = TIMEOUT_ERR;
                            break;
                        }
                        let left = l - t;
                        working = Some((left.as_secs() as i64, left.subsec_micros() as i64));
                    }
                    continue;
                }
            } else if r == 0 {
                error = TIMEOUT_ERR;
            } else {
                let mut len = std::mem::size_of::<i32>() as libc::socklen_t;
                unsafe {
                    libc::getsockopt(
                        fd,
                        libc::SOL_SOCKET,
                        libc::SO_ERROR,
                        &mut error as *mut i32 as *mut libc::c_void,
                        &mut len,
                    );
                }
            }
            break;
        }
    }
    if !asynchronous {
        unsafe { libc::fcntl(fd, libc::F_SETFL, orig) };
    }
    if error != 0 {
        return Err(error);
    }
    Ok(false)
}

/// A transport URL split as php's `_php_stream_xport_create` does:
/// `(protocol, rest)`, protocol defaulting to `tcp`.
fn split_transport(name: &[u8]) -> (&[u8], &[u8]) {
    let n = name
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        .count();
    if n > 1 && name[n..].starts_with(b"://") {
        (&name[..n], &name[n + 3..])
    } else {
        (b"tcp", name)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Proto {
    Tcp,
    Udp,
    Unix,
    Udg,
}

impl Proto {
    fn socktype(self) -> i32 {
        match self {
            Proto::Tcp | Proto::Unix => libc::SOCK_STREAM,
            Proto::Udp | Proto::Udg => libc::SOCK_DGRAM,
        }
    }
    fn stream_type(self) -> &'static str {
        match self {
            Proto::Tcp => "tcp_socket/ssl",
            Proto::Udp => "udp_socket",
            Proto::Unix => "unix_socket",
            Proto::Udg => "udg_socket",
        }
    }
}

/// The outcome of opening a transport: the socket, or php's
/// `(errno, errstr)` (errstr `None` = "Unknown error") and whether a stream
/// had been allocated (a resource id consumed) before the failure.
enum Opened {
    Ok(OwnedFd, Proto),
    Fail {
        code: i32,
        text: Option<String>,
        allocated: bool,
    },
}

/// `_php_stream_xport_create` for a client.
fn xport_client(
    host: &mut dyn Host,
    fname: &str,
    name: &[u8],
    connect: bool,
    asynchronous: bool,
    timeout: Tv,
    ctx: &Value,
) -> Result<Opened, Thrown> {
    let (proto, rest) = split_transport(name);
    let proto = match proto {
        b"tcp" => Proto::Tcp,
        b"udp" => Proto::Udp,
        b"unix" => Proto::Unix,
        b"udg" => Proto::Udg,
        other => {
            return Ok(Opened::Fail {
                code: 0,
                text: Some(unknown_transport(other)),
                allocated: false,
            })
        }
    };
    if !(connect || asynchronous) {
        // A client that neither connects nor binds: php keeps an unopened
        // socket stream; nothing useful can be done with it.
        return Ok(Opened::Fail {
            code: 0,
            text: None,
            allocated: true,
        });
    }
    let fail = |code: i32, text: Option<String>| Opened::Fail {
        code,
        text,
        allocated: true,
    };
    if matches!(proto, Proto::Unix | Proto::Udg) {
        let Some(fd) = new_socket(libc::AF_UNIX, proto.socktype()) else {
            return Ok(fail(0, Some("Failed to create unix socket".into())));
        };
        let addr = unix_addr(host, fname, rest)?;
        return Ok(
            match connect_socket(fd.as_raw_fd(), &addr, asynchronous, timeout) {
                Ok(_) => Opened::Ok(fd, proto),
                Err(e) => fail(e, Some(strerror(e))),
            },
        );
    }
    let (hostname, port) = match parse_ip_address(rest) {
        Ok(v) => v,
        Err(t) => return Ok(fail(0, Some(t))),
    };
    let mut bindto: Option<(Vec<u8>, i32)> = None;
    if let Some(b) = ctx_socket_opt(host, ctx, "bindto") {
        match b {
            // An unparsable address is dropped: php connects without binding.
            Value::Str(s) => bindto = parse_ip_address(s.as_bytes()).ok(),
            _ => {
                return Ok(fail(
                    0,
                    Some("local_addr context option is not a string.".into()),
                ))
            }
        }
    }
    let nodelay = proto == Proto::Tcp
        && ctx_socket_opt(host, ctx, "tcp_nodelay").is_some_and(|v| conv::to_bool(&v));
    let broadcast = proto == Proto::Udp
        && ctx_socket_opt(host, ctx, "so_broadcast").is_some_and(|v| conv::to_bool(&v));
    let addrs = match getaddresses(host, fname, &hostname, proto.socktype()) {
        Ok(a) => a,
        Err((t, w)) => {
            w?;
            return Ok(fail(0, Some(t)));
        }
    };
    let limit = timeout.and_then(deadline);
    let mut working = timeout;
    let mut last: (i32, Option<String>) = (0, None);
    for mut a in addrs {
        let fam = a.family();
        if fam != libc::AF_INET && fam != libc::AF_INET6 {
            continue;
        }
        if fam == libc::AF_INET6 && bindto.as_ref().is_some_and(|(b, _)| !b.contains(&b':')) {
            continue;
        }
        if fam == libc::AF_INET
            && bindto
                .as_ref()
                .is_some_and(|(b, _)| b.contains(&b':') || b == b"0")
        {
            // An IPv4 socket cannot bind to an IPv6 address: php drops the
            // bind address for this and every later attempt.
            bindto = None;
        }
        a.set_port(port as u16);
        let Some(fd) = new_socket(fam, proto.socktype()) else {
            continue;
        };
        if let Some((b, bport)) = &bindto {
            bind_local(host, fname, fd.as_raw_fd(), fam, b, *bport)?;
        }
        if broadcast {
            setsockopt_int(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_BROADCAST, 1);
        }
        if nodelay {
            setsockopt_int(fd.as_raw_fd(), libc::IPPROTO_TCP, libc::TCP_NODELAY, 1);
        }
        match connect_socket(fd.as_raw_fd(), &a, asynchronous, working) {
            Ok(_) => return Ok(Opened::Ok(fd, proto)),
            Err(e) => last = (e, Some(strerror(e))),
        }
        if let Some(l) = limit {
            let t = now();
            if t >= l {
                break;
            }
            let left = l - t;
            working = Some((left.as_secs() as i64, left.subsec_micros() as i64));
        }
    }
    Ok(fail(last.0, last.1))
}

/// The `bindto` context option of a client: bind before connecting.
fn bind_local(
    host: &mut dyn Host,
    fname: &str,
    fd: RawFd,
    fam: i32,
    ip: &[u8],
    port: i32,
) -> Result<(), Thrown> {
    let text = String::from_utf8_lossy(ip).into_owned();
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let len = unsafe {
        if fam == libc::AF_INET {
            match text.parse::<std::net::Ipv4Addr>() {
                Ok(v4) => {
                    let sin = &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in);
                    sin.sin_family = libc::AF_INET as libc::sa_family_t;
                    sin.sin_port = (port as u16).to_be();
                    sin.sin_addr.s_addr = u32::from(v4).to_be();
                    std::mem::size_of::<libc::sockaddr_in>()
                }
                Err(_) => 0,
            }
        } else {
            match text.parse::<std::net::Ipv6Addr>() {
                Ok(v6) => {
                    let sin6 = &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in6);
                    sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                    sin6.sin6_port = (port as u16).to_be();
                    sin6.sin6_addr.s6_addr = v6.octets();
                    std::mem::size_of::<libc::sockaddr_in6>()
                }
                Err(_) => 0,
            }
        }
    };
    if len == 0 {
        return crate::warn(host, fname, &format!("Invalid IP Address: {text}"));
    }
    let r = unsafe {
        libc::bind(
            fd,
            &storage as *const _ as *const libc::sockaddr,
            len as libc::socklen_t,
        )
    };
    if r != 0 {
        let e = errno();
        return crate::warn(
            host,
            fname,
            &format!(
                "Failed to bind to '{text}:{port}', system said: {}",
                strerror(e)
            ),
        );
    }
    Ok(())
}

fn unknown_transport(proto: &[u8]) -> String {
    let p = &proto[..proto.len().min(31)];
    format!(
        "Unable to find the socket transport \"{}\" - did you forget to enable it when you configured PHP?",
        String::from_utf8_lossy(p)
    )
}

/// `_php_stream_xport_create` for a server (bind, then listen).
fn xport_server(
    host: &mut dyn Host,
    fname: &str,
    name: &[u8],
    flags: i64,
    ctx: &Value,
) -> Result<Opened, Thrown> {
    let (proto, rest) = split_transport(name);
    let proto = match proto {
        b"tcp" => Proto::Tcp,
        b"udp" => Proto::Udp,
        b"unix" => Proto::Unix,
        b"udg" => Proto::Udg,
        other => {
            return Ok(Opened::Fail {
                code: 0,
                text: Some(unknown_transport(other)),
                allocated: false,
            })
        }
    };
    let fail = |text: Option<String>| Opened::Fail {
        code: 0,
        text,
        allocated: true,
    };
    if flags & STREAM_SERVER_BIND == 0 {
        return Ok(fail(None));
    }
    let fd = if matches!(proto, Proto::Unix | Proto::Udg) {
        let Some(fd) = new_socket(libc::AF_UNIX, proto.socktype()) else {
            let e = errno();
            return Ok(fail(Some(format!(
                "Failed to create unix{} socket {}",
                if proto == Proto::Unix { "" } else { "datagram" },
                strerror(e)
            ))));
        };
        let addr = unix_addr(host, fname, rest)?;
        if unsafe { libc::bind(fd.as_raw_fd(), addr.ptr(), addr.len) } != 0 {
            return Ok(fail(None));
        }
        fd
    } else {
        let (hostname, port) = match parse_ip_address(rest) {
            Ok(v) => v,
            Err(t) => return Ok(fail(Some(t))),
        };
        let v6only = ctx_socket_opt(host, ctx, "ipv6_v6only").filter(|v| !matches!(v, Value::Null));
        let reuseport =
            ctx_socket_opt(host, ctx, "so_reuseport").is_some_and(|v| conv::to_bool(&v));
        let broadcast = proto == Proto::Udp
            && ctx_socket_opt(host, ctx, "so_broadcast").is_some_and(|v| conv::to_bool(&v));
        let addrs = match getaddresses(host, fname, &hostname, proto.socktype()) {
            Ok(a) => a,
            Err((t, w)) => {
                w?;
                return Ok(fail(Some(t)));
            }
        };
        let mut err = 0;
        let mut bound = None;
        for mut a in addrs {
            let fam = a.family();
            if fam != libc::AF_INET && fam != libc::AF_INET6 {
                continue;
            }
            a.set_port(port as u16);
            let Some(fd) = new_socket(fam, proto.socktype()) else {
                continue;
            };
            let raw = fd.as_raw_fd();
            setsockopt_int(raw, libc::SOL_SOCKET, libc::SO_REUSEADDR, 1);
            if let Some(v) = &v6only {
                if fam == libc::AF_INET6 {
                    setsockopt_int(
                        raw,
                        libc::IPPROTO_IPV6,
                        libc::IPV6_V6ONLY,
                        conv::to_bool(v) as i32,
                    );
                }
            }
            if reuseport {
                setsockopt_int(raw, libc::SOL_SOCKET, libc::SO_REUSEPORT, 1);
            }
            if broadcast {
                setsockopt_int(raw, libc::SOL_SOCKET, libc::SO_BROADCAST, 1);
            }
            if unsafe { libc::bind(raw, a.ptr(), a.len) } == 0 {
                bound = Some(fd);
                break;
            }
            err = errno();
        }
        match bound {
            Some(fd) => fd,
            None => return Ok(fail(Some(strerror(err)))),
        }
    };
    if flags & STREAM_SERVER_LISTEN != 0 {
        let backlog = ctx_socket_opt(host, ctx, "backlog").map_or(32, |v| conv::to_int(&v) as i32);
        if unsafe { libc::listen(fd.as_raw_fd(), backlog) } != 0 {
            return Ok(fail(None));
        }
    }
    Ok(Opened::Ok(fd, proto))
}

/// The stream context argument at `i` (a resource or null), falling back to
/// the default context (allocating it, as php's
/// `php_stream_context_from_zval` does).
fn context_arg(host: &mut dyn Host, args: &[Value], i: usize) -> Value {
    match args.get(i).map(deref) {
        Some(v @ Value::Resource(_)) => v,
        _ => host.default_stream_context(),
    }
}

fn wrap_socket(host: &mut dyn Host, fd: OwnedFd, proto: Proto, no_io: bool, uri: &[u8]) -> Value {
    wrap_socket_ex(host, fd, proto, no_io, uri, None)
}

/// [`wrap_socket`], registering the stream under `persistent_key` (a
/// `persistent stream` resource) when given.
fn wrap_socket_ex(
    host: &mut dyn Host,
    fd: OwnedFd,
    proto: Proto,
    no_io: bool,
    uri: &[u8],
    persistent_key: Option<&[u8]>,
) -> Value {
    let mut info = SockInfo::new(proto.stream_type(), default_sock_tv(host));
    info.uri = Some(uri.to_vec());
    info.no_io = no_io;
    info.persistent = persistent_key.is_some();
    let v = host
        .stream_from_socket(fd, info)
        .unwrap_or(Value::Bool(false));
    if let (Some(key), Value::Resource(_)) = (persistent_key, &v) {
        if let Some(reg) = persistent_registry(host) {
            reg.0.insert(key.to_vec(), v.clone());
        }
    }
    v
}

/// php's persistent stream list (`EG(persistent_list)`) for the socket
/// transports: persistent id → the stream resource.
#[derive(Default)]
struct PersistentSockets(HashMap<Vec<u8>, Value>);

fn persistent_registry(host: &mut dyn Host) -> Option<&mut PersistentSockets> {
    host.module_state("persistent_sockets", || Box::<PersistentSockets>::default())
        .and_then(|st| st.downcast_mut::<PersistentSockets>())
}

/// php's `php_stream_from_persistent_id` + the liveness check of
/// `_php_stream_xport_create`: the live stream already open under `key`, if
/// any (a dead one is closed and forgotten).
fn persistent_lookup(host: &mut dyn Host, key: &[u8]) -> Option<Value> {
    let v = persistent_registry(host)?.0.get(key).cloned()?;
    if !host.resource_closed(&v) && !host.stream_eof(&v) {
        return Some(v);
    }
    if !host.resource_closed(&v) {
        host.stream_close(&v);
    }
    persistent_registry(host)?.0.remove(key);
    None
}

fn fn_stream_socket_server(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_server";
    need_streams(host, fname)?;
    let address = zpp_string(host, args, 0, fname, "address")?;
    let flags = match args.get(3) {
        Some(_) => zpp_long(host, args, 3, fname, "flags")?,
        None => STREAM_SERVER_BIND | STREAM_SERVER_LISTEN,
    };
    let ctx = context_arg(host, args, 4);
    write_ref(args.get(1), Value::Int(0));
    write_ref(args.get(2), Value::str(""));
    match xport_server(host, fname, &address, flags, &ctx)? {
        Opened::Ok(fd, proto) => Ok(wrap_socket(
            host,
            fd,
            proto,
            flags & STREAM_SERVER_BIND != 0,
            &address,
        )),
        Opened::Fail {
            code,
            text,
            allocated,
        } => {
            if allocated {
                host.consume_resource_ids(1);
            }
            crate::warn(
                host,
                fname,
                &format!(
                    "Unable to connect to {} ({})",
                    c_str(&address),
                    text.as_deref().unwrap_or("Unknown error")
                ),
            )?;
            write_ref(args.get(1), Value::Int(code as i64));
            if let Some(t) = text {
                write_ref(args.get(2), Value::str(t));
            }
            Ok(Value::Bool(false))
        }
    }
}

/// A string as php's `%s` of a `char *` prints it: up to the first NUL.
fn c_str(s: &[u8]) -> String {
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..end]).into_owned()
}

/// php's `php_addslashes` (for the host in the connect warning).
fn addslashes(s: &[u8]) -> String {
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        match b {
            0 => out.extend_from_slice(b"\\0"),
            b'\'' | b'"' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            _ => out.push(b),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn fn_stream_socket_client(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_client";
    need_streams(host, fname)?;
    let address = zpp_string(host, args, 0, fname, "address")?;
    let timeout = match zpp_double_or_null(host, args, 3, fname, "timeout")? {
        None => default_socket_timeout(host) as f64,
        Some(t) if !t.is_finite() => {
            return Err(Thrown::error(
                "ValueError",
                format!("{fname}(): Argument #4 ($timeout) must be a finite value"),
            ))
        }
        Some(t) => t,
    };
    let flags = match args.get(4) {
        Some(_) => zpp_long(host, args, 4, fname, "flags")?,
        None => STREAM_CLIENT_CONNECT,
    };
    let persistent_key = (flags & STREAM_CLIENT_PERSISTENT != 0).then(|| {
        let mut k = b"stream_socket_client__".to_vec();
        k.extend_from_slice(&address);
        k
    });
    let ctx = context_arg(host, args, 5);
    let tv = timeout_tv(timeout);
    write_ref(args.get(1), Value::Int(0));
    write_ref(args.get(2), Value::str(""));
    if let Some(v) = persistent_key
        .as_deref()
        .and_then(|k| persistent_lookup(host, k))
    {
        return Ok(v);
    }
    let connect = flags & STREAM_CLIENT_CONNECT != 0;
    let asynchronous = flags & STREAM_CLIENT_ASYNC_CONNECT != 0;
    match xport_client(host, fname, &address, connect, asynchronous, tv, &ctx)? {
        Opened::Ok(fd, proto) => Ok(wrap_socket_ex(
            host,
            fd,
            proto,
            false,
            &address,
            persistent_key.as_deref(),
        )),
        Opened::Fail {
            code,
            text,
            allocated,
        } => {
            if allocated {
                host.consume_resource_ids(1);
            }
            crate::warn(
                host,
                fname,
                &format!(
                    "Unable to connect to {} ({})",
                    addslashes(&address),
                    text.as_deref().unwrap_or("Unknown error")
                ),
            )?;
            write_ref(args.get(1), Value::Int(code as i64));
            if let Some(t) = text {
                write_ref(args.get(2), Value::str(t));
            }
            Ok(Value::Bool(false))
        }
    }
}

fn fsockopen_impl(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let persistent = fname == "pfsockopen";
    need_streams(host, fname)?;
    let hostname = zpp_string(host, args, 0, fname, "hostname")?;
    let port = match args.get(1) {
        Some(_) => zpp_long(host, args, 1, fname, "port")?,
        None => -1,
    };
    let timeout = zpp_double_or_null(host, args, 4, fname, "timeout")?
        .unwrap_or(default_socket_timeout(host) as f64);
    let max = u64::MAX as f64 / 1_000_000.0;
    if timeout != -1.0 && !(0.0..=max).contains(&timeout) {
        // php 8.5.8 reports this as argument #6 (no such parameter, so no
        // name) and hands the double limit to an integer format: what prints
        // is the double's bit pattern (on arm64; garbage elsewhere).
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #6 must be -1 or between 0 and {}",
                max.to_bits()
            ),
        ));
    }
    let tv = timeout_tv(timeout);
    let mut target = hostname.clone();
    if port > 0 {
        target.extend_from_slice(format!(":{port}").as_bytes());
    }
    let persistent_key = persistent.then(|| {
        let mut k = b"pfsockopen__".to_vec();
        k.extend_from_slice(&hostname);
        k.extend_from_slice(format!(":{port}").as_bytes());
        k
    });
    if let Some(v) = persistent_key
        .as_deref()
        .and_then(|k| persistent_lookup(host, k))
    {
        write_ref(args.get(2), Value::Int(0));
        write_ref(args.get(3), Value::str(""));
        return Ok(v);
    }
    match xport_client(host, fname, &target, true, false, tv, &Value::Null)? {
        Opened::Ok(fd, proto) => {
            write_ref(args.get(2), Value::Int(0));
            write_ref(args.get(3), Value::str(""));
            Ok(wrap_socket_ex(
                host,
                fd,
                proto,
                false,
                &target,
                persistent_key.as_deref(),
            ))
        }
        Opened::Fail {
            code,
            text,
            allocated,
        } => {
            if allocated {
                host.consume_resource_ids(1);
            }
            crate::warn(
                host,
                fname,
                &format!(
                    "Unable to connect to {}:{} ({})",
                    c_str(&hostname),
                    port,
                    text.as_deref().unwrap_or("Unknown error")
                ),
            )?;
            write_ref(args.get(2), Value::Int(code as i64));
            if let Some(t) = text {
                write_ref(args.get(3), Value::str(t));
            }
            Ok(Value::Bool(false))
        }
    }
}

fn fn_fsockopen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    fsockopen_impl(host, args, "fsockopen")
}

fn fn_pfsockopen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    fsockopen_impl(host, args, "pfsockopen")
}

fn fn_stream_socket_accept(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_accept";
    let srv = stream_arg(host, args, 0, fname, "socket")?;
    let timeout = match zpp_double_or_null(host, args, 1, fname, "timeout")? {
        None => default_socket_timeout(host) as f64,
        Some(t) if !t.is_finite() => {
            return Err(Thrown::error(
                "ValueError",
                format!("{fname}(): Argument #2 ($timeout) must be a finite value"),
            ))
        }
        Some(t) => t,
    };
    let tv = timeout_tv(timeout);
    let (Some(info), Some(fd)) = (host.stream_sock_info(&srv), host.stream_fd(&srv)) else {
        crate::warn(host, fname, "Accept failed: Unknown error")?;
        return Ok(Value::Bool(false));
    };
    let n = poll_fd(fd, libc::POLLIN | libc::POLLPRI, tv);
    let mut error = 0;
    let mut client = None;
    let mut peer = None;
    if n == 0 {
        error = TIMEOUT_ERR;
    } else if n < 0 {
        error = errno();
    } else {
        let mut sa: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut sl = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        let c = unsafe { libc::accept(fd, &mut sa as *mut _ as *mut libc::sockaddr, &mut sl) };
        if c >= 0 {
            client = Some(unsafe { OwnedFd::from_raw_fd(c) });
            peer = sockaddr_text(&sa, sl);
        } else {
            error = errno();
        }
    }
    let Some(c) = client else {
        crate::warn(host, fname, &format!("Accept failed: {}", strerror(error)))?;
        return Ok(Value::Bool(false));
    };
    let mut ci = info;
    ci.no_io = false;
    ci.uri = None;
    if cfg!(target_os = "linux") {
        // O_NONBLOCK is not inherited on Linux.
        ci.blocked = true;
    }
    if let Some(p) = peer {
        write_ref(args.get(2), Value::str(p));
    }
    Ok(host.stream_from_socket(c, ci).unwrap_or(Value::Bool(false)))
}

fn fn_stream_socket_get_name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_get_name";
    let s = stream_arg(host, args, 0, fname, "socket")?;
    let peer = conv::to_bool(&args.get(1).map(deref).unwrap_or(Value::Null));
    if host.stream_sock_info(&s).is_none() {
        return Ok(Value::Bool(false));
    }
    let Some(fd) = host.stream_fd(&s) else {
        return Ok(Value::Bool(false));
    };
    match sock_name(fd, peer) {
        Some(n) if !n.is_empty() && n[0] != 0 => Ok(Value::str(n)),
        _ => Ok(Value::Bool(false)),
    }
}

fn fn_stream_socket_shutdown(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_shutdown";
    let s = stream_arg(host, args, 0, fname, "stream")?;
    let how = zpp_long(host, args, 1, fname, "mode")?;
    if !(0..=2).contains(&how) {
        return Err(Thrown::error(
            "ValueError",
            format!("{fname}(): Argument #2 ($mode) must be one of STREAM_SHUT_RD, STREAM_SHUT_WR, or STREAM_SHUT_RDWR"),
        ));
    }
    if host.stream_sock_info(&s).is_none() {
        return Ok(Value::Bool(false));
    }
    let Some(fd) = host.stream_fd(&s) else {
        return Ok(Value::Bool(false));
    };
    let h = [libc::SHUT_RD, libc::SHUT_WR, libc::SHUT_RDWR][how as usize];
    Ok(Value::Bool(unsafe { libc::shutdown(fd, h) } == 0))
}

fn fn_stream_socket_pair(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_pair";
    need_streams(host, fname)?;
    let domain = zpp_long(host, args, 0, fname, "domain")?;
    let ty = zpp_long(host, args, 1, fname, "type")?;
    let proto = zpp_long(host, args, 2, fname, "protocol")?;
    let mut pair = [0 as libc::c_int; 2];
    if unsafe { libc::socketpair(domain as i32, ty as i32, proto as i32, pair.as_mut_ptr()) } != 0 {
        let e = errno();
        crate::warn(
            host,
            fname,
            &format!("Failed to create sockets: [{e}]: {}", strerror(e)),
        )?;
        return Ok(Value::Bool(false));
    }
    let tv = default_sock_tv(host);
    let mut out = PhpArray::new();
    for fd in pair {
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let v = host
            .stream_from_socket(fd, SockInfo::new("generic_socket", tv))
            .unwrap_or(Value::Bool(false));
        out.push(v);
    }
    Ok(Value::Array(out))
}

/// php's `php_network_parse_network_address_with_port`: `[v6]:port` or
/// `host:port` (split at the first colon); a numeric address first, else the
/// first address the resolver gives (a resolver failure warns).
fn parse_network_address(
    host: &mut dyn Host,
    fname: &str,
    s: &[u8],
) -> Result<Option<Addr>, Thrown> {
    let (h, p): (&[u8], &[u8]) = if s.first() == Some(&b'[') {
        let Some(end) = s[1..].iter().position(|&b| b == b']').map(|e| e + 1) else {
            return Ok(None);
        };
        if s.get(end + 1) != Some(&b':') {
            return Ok(None);
        }
        (&s[1..end], &s[end + 2..])
    } else {
        let Some(c) = s.iter().position(|&b| b == b':') else {
            return Ok(None);
        };
        (&s[..c], &s[c + 1..])
    };
    // `estrndup` + C string calls: the host ends at its first NUL.
    let h = &h[..h.iter().position(|&b| b == 0).unwrap_or(h.len())];
    let port = atoi(p) as u16;
    let text = String::from_utf8_lossy(h).into_owned();
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    if let Ok(v6) = text.parse::<std::net::Ipv6Addr>() {
        let sin6 = unsafe { &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in6) };
        sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
        sin6.sin6_port = port.to_be();
        sin6.sin6_addr.s6_addr = v6.octets();
        return Ok(Some(Addr {
            storage,
            len: std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
        }));
    }
    if let Ok(v4) = text.parse::<std::net::Ipv4Addr>() {
        let sin = unsafe { &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in) };
        sin.sin_family = libc::AF_INET as libc::sa_family_t;
        sin.sin_port = port.to_be();
        sin.sin_addr.s_addr = u32::from(v4).to_be();
        return Ok(Some(Addr {
            storage,
            len: std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        }));
    }
    match resolve(h, libc::SOCK_DGRAM) {
        Ok(addrs) => Ok(addrs
            .into_iter()
            .find(|a| matches!(a.family(), libc::AF_INET | libc::AF_INET6))
            .map(|mut a| {
                a.set_port(port);
                a
            })),
        Err(msg) => {
            // php_network_getaddresses warns itself, then the caller again.
            crate::warn(host, fname, &msg)?;
            crate::warn(host, fname, &format!("Failed to resolve `{text}': {msg}"))?;
            Ok(None)
        }
    }
}

fn fn_stream_socket_sendto(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_sendto";
    let s = stream_arg(host, args, 0, fname, "socket")?;
    let data = zpp_string(host, args, 1, fname, "data")?;
    let flags = match args.get(2) {
        Some(_) => zpp_long(host, args, 2, fname, "flags")?,
        None => 0,
    };
    let target = match args.get(3) {
        Some(_) => zpp_string(host, args, 3, fname, "address")?,
        None => Vec::new(),
    };
    let addr = if target.is_empty() {
        None
    } else {
        match parse_network_address(host, fname, &target)? {
            Some(a) => Some(a),
            None => {
                crate::warn(
                    host,
                    fname,
                    &format!(
                        "Failed to parse `{}' into a valid network address",
                        c_str(&target)
                    ),
                )?;
                return Ok(Value::Bool(false));
            }
        }
    };
    let (Some(_), Some(fd)) = (host.stream_sock_info(&s), host.stream_fd(&s)) else {
        return Ok(Value::Int(-1));
    };
    let f = if flags & STREAM_OOB != 0 {
        libc::MSG_OOB
    } else {
        0
    };
    let r = unsafe {
        match &addr {
            Some(a) => libc::sendto(
                fd,
                data.as_ptr() as *const libc::c_void,
                data.len(),
                f,
                a.ptr(),
                a.len,
            ),
            None => libc::send(fd, data.as_ptr() as *const libc::c_void, data.len(), f),
        }
    };
    if r < 0 {
        let e = errno();
        crate::warn(host, fname, &format!("{}\n", strerror(e)))?;
        return Ok(Value::Int(-1));
    }
    Ok(Value::Int(r as i64))
}

fn fn_stream_socket_recvfrom(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_recvfrom";
    let s = stream_arg(host, args, 0, fname, "socket")?;
    let len = zpp_long(host, args, 1, fname, "length")?;
    let flags = match args.get(2) {
        Some(_) => zpp_long(host, args, 2, fname, "flags")?,
        None => 0,
    };
    write_ref(args.get(3), Value::Null);
    if len <= 0 {
        return Err(Thrown::error(
            "ValueError",
            format!("{fname}(): Argument #2 ($length) must be greater than 0"),
        ));
    }
    let (Some(_), Some(fd)) = (host.stream_sock_info(&s), host.stream_fd(&s)) else {
        return Ok(Value::Bool(false));
    };
    let mut f = 0;
    if flags & STREAM_OOB != 0 {
        f |= libc::MSG_OOB;
    }
    if flags & STREAM_PEEK != 0 {
        f |= libc::MSG_PEEK;
    }
    // php allocates `$length` bytes up front (a huge length exhausts its
    // memory limit); one recv never returns more than a socket buffer, so a
    // bounded buffer reads the same.
    let mut buf = vec![0u8; (len as usize).min(RECV_CAP)];
    let mut sa: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut sl = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let r = unsafe {
        libc::recvfrom(
            fd,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
            f,
            &mut sa as *mut _ as *mut libc::sockaddr,
            &mut sl,
        )
    };
    if r < 0 {
        return Ok(Value::Bool(false));
    }
    if args.get(3).is_some() {
        let addr = if sl > 0 {
            sockaddr_text(&sa, sl)
        } else {
            Some(Vec::new())
        };
        if let Some(a) = addr {
            write_ref(args.get(3), Value::str(a));
        }
    }
    buf.truncate(r as usize);
    Ok(Value::str(buf))
}

/// `stream_socket_enable_crypto`: TLS over socket streams is not
/// implemented; a request to enable it on a socket fails. Any other stream
/// answers as php's does for a stream without the crypto API: a warning,
/// then false for a setup (enable) and true for a disable.
fn fn_stream_socket_enable_crypto(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_enable_crypto";
    let s = stream_arg(host, args, 0, fname, "stream")?;
    let enable = match args.get(1).map(deref) {
        Some(v @ (Value::Array(_) | Value::Object(_) | Value::Resource(_))) => {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #2 ($enable) must be of type bool, {} given",
                zpp_type_name(host, &v)
            )))
        }
        Some(v) => conv::to_bool(&v),
        None => false,
    };
    let method_null = match args.get(2).map(deref) {
        None | Some(Value::Null) => true,
        Some(_) => {
            zpp_long(host, args, 2, fname, "crypto_method")?;
            false
        }
    };
    match args.get(3).map(deref) {
        None | Some(Value::Null) => {}
        Some(_) => {
            crate::fs_fns::stream_param(host, args, 3, fname, "session_stream").map_err(|e| {
                let v = args.get(3).map(deref).unwrap_or(Value::Null);
                if matches!(v, Value::Resource(_)) {
                    e
                } else {
                    Thrown::type_error(format!(
                        "{fname}(): Argument #4 ($session_stream) must be of type resource or null, {} given",
                        zpp_type_name(host, &v)
                    ))
                }
            })?;
        }
    }
    if enable && method_null {
        return Err(Thrown::error(
            "ValueError",
            format!("{fname}(): Argument #3 ($crypto_method) must be specified when enabling encryption"),
        ));
    }
    // tcp sockets carry openssl's ops in php (the "tcp" transport is
    // openssl's); every other stream lacks the crypto API.
    if host
        .stream_sock_info(&s)
        .is_some_and(|i| i.stream_type == "tcp_socket/ssl")
    {
        return Ok(Value::Bool(false));
    }
    crate::warn(host, fname, "This stream does not support SSL/crypto")?;
    // A failed setup is false; the enable step's NOTIMPL (-2) reads as true.
    Ok(Value::Bool(!enable))
}

fn fn_stream_set_timeout(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_timeout_impl(host, args, "stream_set_timeout")
}

fn fn_socket_set_timeout(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // #[\Deprecated(since: '8.5')], raised before the arguments are parsed.
    crate::deprecated_fn(
        host,
        "socket_set_timeout",
        "8.5",
        "use stream_set_timeout() instead",
    )?;
    set_timeout_impl(host, args, "socket_set_timeout")
}

fn set_timeout_impl(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let s = stream_arg(host, args, 0, fname, "stream")?;
    let secs = zpp_long(host, args, 1, fname, "seconds")?;
    let usecs = match args.get(2) {
        Some(_) => zpp_long(host, args, 2, fname, "microseconds")?,
        None => 0,
    };
    let sec = secs.wrapping_add(usecs / 1_000_000);
    let usec = usecs % 1_000_000;
    let tv = if sec == -1 { None } else { Some((sec, usec)) };
    Ok(Value::Bool(host.stream_set_timeout(&s, tv)))
}

fn fn_stream_set_read_buffer(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_set_read_buffer";
    let _ = stream_arg(host, args, 0, fname, "stream")?;
    let _ = zpp_long(host, args, 1, fname, "size")?;
    Ok(Value::Int(0))
}

/// Per-stream chunk sizes (`stream_set_chunk_size` returns the previous one).
#[derive(Default)]
struct ChunkSizes(HashMap<u32, i64>);

fn fn_stream_set_chunk_size(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_set_chunk_size";
    let s = stream_arg(host, args, 0, fname, "stream")?;
    let size = zpp_long(host, args, 1, fname, "size")?;
    if size <= 0 {
        return Err(Thrown::error(
            "ValueError",
            format!("{fname}(): Argument #2 ($size) must be greater than 0"),
        ));
    }
    let Value::Resource(r) = &s else {
        return Ok(Value::Bool(false));
    };
    let id = r.id;
    let prev = host
        .module_state("stream_chunk_sizes", || Box::<ChunkSizes>::default())
        .and_then(|st| st.downcast_mut::<ChunkSizes>())
        .map(|st| st.0.insert(id, size).unwrap_or(8192))
        .unwrap_or(8192);
    Ok(Value::Int(prev))
}

/// `stream_get_transports()`: the socket transports registered here (php
/// with openssl also lists `ssl`, `tls`, `tlsv1.x`; TLS is not implemented,
/// so those are left out rather than advertised and then refused).
fn fn_stream_get_transports(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for t in ["tcp", "udp", "unix", "udg"] {
        a.push(Value::str(t));
    }
    Ok(Value::Array(a))
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
        "stream_socket_server",
        fn_stream_socket_server,
        1,
        Some(5),
    );
    put(
        m,
        "stream_socket_client",
        fn_stream_socket_client,
        1,
        Some(6),
    );
    put(
        m,
        "stream_socket_accept",
        fn_stream_socket_accept,
        1,
        Some(3),
    );
    put(
        m,
        "stream_socket_get_name",
        fn_stream_socket_get_name,
        2,
        Some(2),
    );
    put(
        m,
        "stream_socket_shutdown",
        fn_stream_socket_shutdown,
        2,
        Some(2),
    );
    put(m, "stream_socket_pair", fn_stream_socket_pair, 3, Some(3));
    put(
        m,
        "stream_socket_sendto",
        fn_stream_socket_sendto,
        2,
        Some(4),
    );
    put(
        m,
        "stream_socket_recvfrom",
        fn_stream_socket_recvfrom,
        2,
        Some(4),
    );
    put(
        m,
        "stream_socket_enable_crypto",
        fn_stream_socket_enable_crypto,
        2,
        Some(4),
    );
    put(m, "fsockopen", fn_fsockopen, 1, Some(5));
    put(
        m,
        "stream_get_transports",
        fn_stream_get_transports,
        0,
        Some(0),
    );
    put(m, "pfsockopen", fn_pfsockopen, 1, Some(5));
    put(m, "stream_set_timeout", fn_stream_set_timeout, 2, Some(3));
    put(m, "socket_set_timeout", fn_socket_set_timeout, 2, Some(3));
    // php's aliases
    put(
        m,
        "socket_get_status",
        crate::fs_fns::fn_socket_get_status,
        1,
        Some(1),
    );
    put(
        m,
        "socket_set_blocking",
        crate::stream_fns::fn_socket_set_blocking,
        2,
        Some(2),
    );
    put(
        m,
        "stream_set_read_buffer",
        fn_stream_set_read_buffer,
        2,
        Some(2),
    );
    put(
        m,
        "stream_set_chunk_size",
        fn_stream_set_chunk_size,
        2,
        Some(2),
    );
}

/// The stream / socket transport constants (php's `file.c` registrations).
pub fn constants() -> Vec<(&'static str, Value)> {
    let i = |n: i64| Value::Int(n);
    vec![
        ("STREAM_CLIENT_PERSISTENT", i(STREAM_CLIENT_PERSISTENT)),
        (
            "STREAM_CLIENT_ASYNC_CONNECT",
            i(STREAM_CLIENT_ASYNC_CONNECT),
        ),
        ("STREAM_CLIENT_CONNECT", i(STREAM_CLIENT_CONNECT)),
        ("STREAM_SERVER_BIND", i(STREAM_SERVER_BIND)),
        ("STREAM_SERVER_LISTEN", i(STREAM_SERVER_LISTEN)),
        ("STREAM_SHUT_RD", i(0)),
        ("STREAM_SHUT_WR", i(1)),
        ("STREAM_SHUT_RDWR", i(2)),
        ("STREAM_PF_INET", i(libc::AF_INET as i64)),
        ("STREAM_PF_INET6", i(libc::AF_INET6 as i64)),
        ("STREAM_PF_UNIX", i(libc::AF_UNIX as i64)),
        ("STREAM_IPPROTO_IP", i(libc::IPPROTO_IP as i64)),
        ("STREAM_IPPROTO_TCP", i(libc::IPPROTO_TCP as i64)),
        ("STREAM_IPPROTO_UDP", i(libc::IPPROTO_UDP as i64)),
        ("STREAM_IPPROTO_ICMP", i(libc::IPPROTO_ICMP as i64)),
        ("STREAM_IPPROTO_RAW", i(libc::IPPROTO_RAW as i64)),
        ("STREAM_SOCK_STREAM", i(libc::SOCK_STREAM as i64)),
        ("STREAM_SOCK_DGRAM", i(libc::SOCK_DGRAM as i64)),
        ("STREAM_SOCK_RAW", i(libc::SOCK_RAW as i64)),
        ("STREAM_SOCK_SEQPACKET", i(libc::SOCK_SEQPACKET as i64)),
        ("STREAM_SOCK_RDM", i(libc::SOCK_RDM as i64)),
        ("STREAM_PEEK", i(STREAM_PEEK)),
        ("STREAM_OOB", i(STREAM_OOB)),
        ("STREAM_CRYPTO_METHOD_ANY_CLIENT", i(127)),
        ("STREAM_CRYPTO_METHOD_SSLv2_CLIENT", i(3)),
        ("STREAM_CRYPTO_METHOD_SSLv3_CLIENT", i(5)),
        ("STREAM_CRYPTO_METHOD_SSLv23_CLIENT", i(57)),
        ("STREAM_CRYPTO_METHOD_TLS_CLIENT", i(121)),
        ("STREAM_CRYPTO_METHOD_TLSv1_0_CLIENT", i(9)),
        ("STREAM_CRYPTO_METHOD_TLSv1_1_CLIENT", i(17)),
        ("STREAM_CRYPTO_METHOD_TLSv1_2_CLIENT", i(33)),
        ("STREAM_CRYPTO_METHOD_TLSv1_3_CLIENT", i(65)),
        ("STREAM_CRYPTO_METHOD_ANY_SERVER", i(126)),
        ("STREAM_CRYPTO_METHOD_SSLv2_SERVER", i(2)),
        ("STREAM_CRYPTO_METHOD_SSLv3_SERVER", i(4)),
        ("STREAM_CRYPTO_METHOD_SSLv23_SERVER", i(120)),
        ("STREAM_CRYPTO_METHOD_TLS_SERVER", i(120)),
        ("STREAM_CRYPTO_METHOD_TLSv1_0_SERVER", i(8)),
        ("STREAM_CRYPTO_METHOD_TLSv1_1_SERVER", i(16)),
        ("STREAM_CRYPTO_METHOD_TLSv1_2_SERVER", i(32)),
        ("STREAM_CRYPTO_METHOD_TLSv1_3_SERVER", i(64)),
        ("STREAM_CRYPTO_PROTO_SSLv3", i(4)),
        ("STREAM_CRYPTO_PROTO_TLSv1_0", i(8)),
        ("STREAM_CRYPTO_PROTO_TLSv1_1", i(16)),
        ("STREAM_CRYPTO_PROTO_TLSv1_2", i(32)),
        ("STREAM_CRYPTO_PROTO_TLSv1_3", i(64)),
    ]
}
