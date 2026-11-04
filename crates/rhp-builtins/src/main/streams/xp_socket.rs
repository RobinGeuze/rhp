//! Socket streams: php's `main/streams/xp_socket.c` and
//! `main/streams/transports.c`, the transports behind `stream_socket_*`
//! and `fsockopen` (ext/standard/streamsfuncs.rs, fsock.rs), with the
//! persistent-socket registry. tcp and the TLS transports carry openssl's
//! socket ops (`ext/openssl/xp_ssl.rs`), as in php. Address resolution and
//! connecting are in [`crate::main::network`].
//!
//! The socket descriptor is created and connected / bound here, as an
//! [`FdStream`] whose reads and writes follow `php_sockop_read` /
//! `php_sockop_write` (or the TLS layer's); the VM registers it with its
//! context ([`Host::stream_insert_fd`]).

use std::borrow::Cow;
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use rhp_values::{conv, ArrayKey, Value};

use crate::ext::openssl::xp_ssl;
use crate::ext::openssl::xp_ssl::SslSock;
use crate::ext::standard::streamsfuncs::{STREAM_SERVER_BIND, STREAM_SERVER_LISTEN};
use crate::main::network::{connect_socket, deadline, getaddresses, now, parse_ip_address, Addr};
use crate::main::streams::errno;
use crate::main::streams::plain_wrapper::{FdStream, SockInfo};
use crate::{Host, Thrown};

pub(crate) fn strerror(e: i32) -> String {
    crate::main::streams::strerror(e)
}

/// php's `PHP_TIMEOUT_ERROR_VALUE`.
pub(crate) const TIMEOUT_ERR: i32 = libc::ETIMEDOUT;

/// A relative Unix socket path the request's directory was prefixed to:
/// the path the kernel holds, and the name the script gave, which php,
/// binding from a real working directory, reports.
type Prefixed = (Vec<u8>, Vec<u8>);

thread_local! {
    /// The Unix socket paths the request bound relative to its directory.
    static BOUND_UNIX: std::cell::RefCell<Vec<Prefixed>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A new request has bound no socket.
pub(crate) fn reset_state() {
    BOUND_UNIX.with(|b| b.borrow_mut().clear());
}

/// The name to report for the Unix socket path `path`: the one the script
/// gave when the request bound it relative to its directory.
pub(crate) fn unix_name(path: &[u8]) -> Vec<u8> {
    BOUND_UNIX.with(|b| {
        b.borrow()
            .iter()
            .rev()
            .find(|(bound, _)| bound == path)
            .map_or_else(|| path.to_vec(), |(_, given)| given.clone())
    })
}

pub(crate) type Tv = Option<(i64, i64)>;

/// php's timeout double → timeval (`None` when negative or too large).
pub(crate) fn timeout_tv(t: f64) -> Tv {
    if t < 0.0 || t >= u64::MAX as f64 / 1_000_000.0 {
        return None;
    }
    let conv = (t * 1_000_000.0) as u64;
    Some(((conv / 1_000_000) as i64, (conv % 1_000_000) as i64))
}

/// `FG(default_socket_timeout)`.
pub(crate) fn default_socket_timeout(host: &dyn Host) -> i64 {
    host.ini_get(b"default_socket_timeout")
        .map(|v| conv::to_int(&Value::str(v)))
        .unwrap_or(60)
}

/// The socket timeout a new socket stream starts with (`tv_sec ==
/// default_socket_timeout`; -1 means forever).
pub(crate) fn default_sock_tv(host: &dyn Host) -> Tv {
    let t = default_socket_timeout(host);
    (t >= 0).then_some((t, 0))
}

/// A `socket` context option of the stream context `ctx` (or of the
/// default context).
pub(crate) fn ctx_socket_opt(host: &dyn Host, ctx: &Value, name: &str) -> Option<Value> {
    let Value::Array(opts) = host.stream_context_options(ctx) else {
        return None;
    };
    let Some(Value::Array(sock)) = opts.get(&ArrayKey::from_bytes(b"socket")) else {
        return None;
    };
    sock.get(&ArrayKey::from_bytes(name.as_bytes())).cloned()
}

/// A `sockaddr_un` for `path` (php's `parse_unix_address`, truncating an
/// over-long path with a notice). A relative path is taken from the
/// request's working directory while that still fits; the path bound and
/// the name as given come back too then.
fn unix_addr(
    host: &mut dyn Host,
    fname: &str,
    path: &[u8],
) -> Result<(Addr, Option<Prefixed>), Thrown> {
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
    let resolved = if abstract_ns {
        Cow::Borrowed(&path[..n])
    } else {
        crate::zend::virtual_cwd::resolve(&path[..n])
    };
    let given = &path[..n];
    let (path, alias) = match &resolved {
        Cow::Owned(full) if full.len() <= max => (&full[..], Some((full.clone(), given.to_vec()))),
        _ => (given, None),
    };
    let n = path.len();
    for (d, &s) in un.sun_path.iter_mut().zip(path) {
        *d = s as libc::c_char;
    }
    let off = std::mem::size_of::<libc::sockaddr_un>() - cap;
    let addr = Addr {
        storage,
        len: (off + n) as libc::socklen_t,
    };
    Ok((addr, alias))
}

fn new_socket(family: i32, socktype: i32) -> Option<OwnedFd> {
    let fd = unsafe { libc::socket(family, socktype, 0) };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

pub(crate) fn setsockopt_int(fd: RawFd, level: i32, name: i32, val: i32) {
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

/// The outcome of opening a transport: the stream, or php's
/// `(errno, errstr)` (errstr `None` = "Unknown error") and whether a stream
/// had been allocated (a resource id consumed) before the failure.
pub(crate) enum Opened {
    Ok(Box<FdStream>),
    Fail {
        code: i32,
        text: Option<String>,
        allocated: bool,
    },
}

/// The transport factory for `proto`: the protocol and, for tcp and the
/// TLS transports (php's openssl socket ops), `(enable_on_connect, crypto
/// method)`. `Err` is the failure to report.
type Transport = (Proto, Option<(bool, i64)>);

fn transport(
    host: &mut dyn Host,
    fname: &str,
    proto: &[u8],
    ctx: &Value,
) -> Result<Result<Transport, Opened>, Thrown> {
    Ok(Ok(match proto {
        b"udp" => (Proto::Udp, None),
        b"unix" => (Proto::Unix, None),
        b"udg" => (Proto::Udg, None),
        other => {
            let ctx = (!matches!(ctx, Value::Null)).then_some(ctx);
            let opts = xp_ssl::stream_opts(host, ctx);
            match xp_ssl::transport(other, &opts) {
                Some(Ok(spec)) => (Proto::Tcp, Some(spec)),
                Some(Err(())) => {
                    crate::flush_stream_diags(host, fname)?;
                    return Ok(Err(Opened::Fail {
                        code: 0,
                        text: None,
                        allocated: true,
                    }));
                }
                None => {
                    return Ok(Err(Opened::Fail {
                        code: 0,
                        text: Some(unknown_transport(other)),
                        allocated: false,
                    }))
                }
            }
        }
    }))
}

/// A connected client socket as a stream, its TLS layer enabled when the
/// transport says so (php_openssl_sockop_set_option's CONNECT).
#[allow(clippy::too_many_arguments)]
fn client_stream(
    host: &mut dyn Host,
    fname: &str,
    fd: OwnedFd,
    proto: Proto,
    tls: Option<(bool, i64)>,
    name: &[u8],
    timeout: (i64, i64),
    ctx: &Value,
) -> Result<Opened, Thrown> {
    let mut info = SockInfo::new(proto.stream_type(), default_sock_tv(host));
    info.uri = Some(name.to_vec());
    info.connect_timeout = timeout;
    let mut s = FdStream::new_socket(fd, info);
    s.notify = !matches!(ctx, Value::Null) && host.stream_context_notifier(ctx).is_some();
    if let Some((enable, method)) = tls {
        let url_name = xp_ssl::url_name(split_transport(name).1);
        s.tls = Some(Box::new(SslSock::new(enable, method, url_name)));
        if enable {
            let ctx = (!matches!(ctx, Value::Null)).then_some(ctx);
            let ok = xp_ssl::crypto_on_connect(host, &mut s, ctx, method)?;
            crate::flush_stream_diags(host, fname)?;
            if !ok {
                crate::warn(host, fname, "Failed to enable crypto")?;
                return Ok(Opened::Fail {
                    code: 0,
                    text: None,
                    allocated: true,
                });
            }
        }
    }
    Ok(Opened::Ok(Box::new(s)))
}

/// `_php_stream_xport_create` for a client. `timeout` is php's timeval (a
/// negative one waits forever); `ctx` is Null for a stream without a context.
pub(crate) fn xport_client(
    host: &mut dyn Host,
    fname: &str,
    name: &[u8],
    connect: bool,
    asynchronous: bool,
    timeout: (i64, i64),
    ctx: &Value,
) -> Result<Opened, Thrown> {
    let (proto_name, rest) = split_transport(name);
    let (proto, tls) = match transport(host, fname, proto_name, ctx)? {
        Ok(t) => t,
        Err(fail) => return Ok(fail),
    };
    let tv = Some(timeout);
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
        let (addr, _) = unix_addr(host, fname, rest)?;
        return match connect_socket(fd.as_raw_fd(), &addr, asynchronous, tv) {
            Ok(_) => client_stream(host, fname, fd, proto, tls, name, timeout, ctx),
            Err(e) => Ok(fail(e, Some(strerror(e)))),
        };
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
    let limit = tv.and_then(deadline);
    let mut working = tv;
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
            Ok(_) => return client_stream(host, fname, fd, proto, tls, name, timeout, ctx),
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
    // `bindto` is a C string to php: it ends at the first NUL.
    let ip = &ip[..ip.iter().position(|&b| b == 0).unwrap_or(ip.len())];
    let text = String::from_utf8_lossy(ip).into_owned();
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let len = unsafe {
        if fam == libc::AF_INET {
            match crate::main::network::pton4(ip) {
                Some(a) => {
                    let sin = &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in);
                    sin.sin_family = libc::AF_INET as libc::sa_family_t;
                    sin.sin_port = (port as u16).to_be();
                    sin.sin_addr.s_addr = u32::from_ne_bytes(a);
                    std::mem::size_of::<libc::sockaddr_in>()
                }
                None => 0,
            }
        } else {
            match crate::main::network::pton6(ip) {
                Some(a) => {
                    let sin6 = &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in6);
                    sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                    sin6.sin6_port = (port as u16).to_be();
                    sin6.sin6_addr.s6_addr = a;
                    std::mem::size_of::<libc::sockaddr_in6>()
                }
                None => 0,
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
pub(crate) fn xport_server(
    host: &mut dyn Host,
    fname: &str,
    name: &[u8],
    flags: i64,
    ctx: &Value,
) -> Result<Opened, Thrown> {
    let (proto_name, rest) = split_transport(name);
    let (proto, tls) = match transport(host, fname, proto_name, ctx)? {
        Ok(t) => t,
        Err(fail) => return Ok(fail),
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
        let (addr, alias) = unix_addr(host, fname, rest)?;
        if unsafe { libc::bind(fd.as_raw_fd(), addr.ptr(), addr.len) } != 0 {
            return Ok(fail(None));
        }
        if let Some(alias) = alias {
            BOUND_UNIX.with(|b| b.borrow_mut().push(alias));
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
    let mut info = SockInfo::new(proto.stream_type(), default_sock_tv(host));
    info.uri = Some(name.to_vec());
    info.no_io = flags & STREAM_SERVER_BIND != 0;
    let mut s = FdStream::new_socket(fd, info);
    if let Some((enable, method)) = tls {
        s.tls = Some(Box::new(SslSock::new(
            enable,
            method,
            xp_ssl::url_name(rest),
        )));
    }
    Ok(Opened::Ok(Box::new(s)))
}

/// Register a socket stream with its context, under `persistent_key` (a
/// `persistent stream` resource) when given.
pub(crate) fn register_stream(
    host: &mut dyn Host,
    mut stream: FdStream,
    ctx: &Value,
    persistent_key: Option<&[u8]>,
) -> Value {
    if let Some(info) = stream.sock_mut() {
        info.persistent = persistent_key.is_some();
    }
    let ctx = (!matches!(ctx, Value::Null)).then(|| ctx.clone());
    let v = host
        .stream_insert_fd(stream, ctx)
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
pub(crate) fn persistent_lookup(host: &mut dyn Host, key: &[u8]) -> Option<Value> {
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

/// `php_stream_escape_persistent_key`.
pub(crate) fn escape_persistent_key(host: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(host.len());
    for &b in host {
        match b {
            0 => out.extend_from_slice(b"\\0"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            _ => out.push(b),
        }
    }
    out
}
