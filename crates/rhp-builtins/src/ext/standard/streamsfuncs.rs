//! ext/standard/streamsfuncs.c: the `stream_*` functions over the `Host`
//! stream primitives: contexts, the socket functions (over the transports
//! in [`crate::main::streams::xp_socket`]), timeouts, buffers and
//! copy/wrapper listing. The resource state lives in the VM.

use crate::main::streams::userspace as us;
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::ext::openssl::xp_ssl;
use crate::ext::openssl::xp_ssl::SslSock;
use crate::ext::standard::file::{
    int_arg, path_param, resolve_include_path, str_arg, stream_param, stream_seek,
};
use crate::main::network::{parse_network_address, sock_name, sockaddr_text};
use crate::main::streams::plain_wrapper::{poll_fd, FdStream, SockInfo};
use crate::main::streams::xp_socket::{
    ctx_socket_opt, default_sock_tv, default_socket_timeout, escape_persistent_key,
    persistent_lookup, register_stream, setsockopt_int, strerror, timeout_tv, xport_client,
    xport_server, Opened, TIMEOUT_ERR,
};
use crate::main::streams::{errno, split_scheme};
use crate::zend::zpp::{zpp_double_or_null, zpp_long, zpp_string, zval_value_name};
use crate::{Host, StreamKind, Thrown};

fn set_blocking_impl(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let (res, _) = crate::ext::standard::file::stream_param(host, args, 0, fname, "stream")?;
    let on = crate::zend::zpp::bool_param(host, args, 1, fname, "enable", true)?;
    if let Some(r) = us::set_option(host, &res, us::OPTION_BLOCKING, on as i64, None) {
        return Ok(Value::Bool(r != us::OPTION_RETURN_ERR));
    }
    Ok(Value::Bool(host.stream_set_blocking(&res, on)))
}

fn fn_stream_set_blocking(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_blocking_impl(host, args, "stream_set_blocking")
}

pub(crate) fn fn_socket_set_blocking(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_blocking_impl(host, args, "socket_set_blocking")
}

/// `stream_set_write_buffer` / `set_file_buffer`: php's WRITE_BUFFER option
/// is only implemented by stdio streams backed by a `FILE*` (popen, the
/// CLI's STDIN/STDOUT/STDERR); every other stream (plain files, memory,
/// sockets, wrappers) answers -1.
fn write_buffer_impl(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let (res, _) = crate::ext::standard::file::stream_param(host, args, 0, fname, "stream")?;
    let size = crate::zend::zpp::zpp_long(host, args, 1, fname, "size")?;
    let (mode, ptr) = if size == 0 {
        (us::BUFFER_NONE, None)
    } else {
        (2, Some(size))
    };
    if let Some(r) = us::set_option(host, &res, us::OPTION_WRITE_BUFFER, mode, ptr) {
        return Ok(Value::Int(if r == 0 { 0 } else { -1 }));
    }
    let file_backed = host.stream_sock_info(&res).is_none()
        && host.stream_meta(&res).is_some_and(|m| {
            m.stream_type == "STDIO" && (m.wrapper_type.is_empty() || m.wrapper_type == "PHP")
        });
    Ok(Value::Int(if file_backed { 0 } else { -1 }))
}

fn fn_stream_set_write_buffer(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    write_buffer_impl(host, args, "stream_set_write_buffer")
}

fn fn_set_file_buffer(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    write_buffer_impl(host, args, "set_file_buffer")
}

fn fn_stream_context_create(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ctx = host.create_stream_context(Value::Array(rhp_values::PhpArray::new()));
    if let Some(Value::Array(options)) = args.first().map(deref) {
        parse_context_options(host, &ctx, &options)?;
    }
    if let Some(Value::Array(params)) = args.get(1).map(deref) {
        parse_context_params(host, "stream_context_create", &ctx, &params)?;
    }
    Ok(ctx)
}

/// php's `parse_context_options`: each `[wrapper][option]` value copied
/// into the context (a list of options is skipped, as php skips a packed
/// array).
fn parse_context_options(
    host: &mut dyn Host,
    ctx: &Value,
    options: &rhp_values::PhpArray,
) -> Result<(), Thrown> {
    use rhp_values::ArrayKey;
    for (wk, wv) in options.iter() {
        let wv = deref(wv);
        let (ArrayKey::Str(w), Value::Array(inner)) = (wk, &wv) else {
            return Err(Thrown::error(
                "ValueError",
                "Options should have the form [\"wrappername\"][\"optionname\"] = $value",
            ));
        };
        if !inner.is_empty() && inner.iter().all(|(k, _)| matches!(k, ArrayKey::Int(_))) {
            continue;
        }
        for (ok, ov) in inner.iter() {
            if let ArrayKey::Str(o) = ok {
                host.stream_context_set_option(ctx, w.as_bytes(), o.as_bytes(), deref(ov));
            }
        }
    }
    Ok(())
}

/// php's `parse_context_params`: the `notification` callback, then
/// `options`.
fn parse_context_params(
    host: &mut dyn Host,
    fname: &str,
    ctx: &Value,
    params: &rhp_values::PhpArray,
) -> Result<(), Thrown> {
    use rhp_values::ArrayKey;
    if let Some(cb) = params
        .get(&ArrayKey::from_bytes(b"notification"))
        .map(deref)
    {
        host.set_stream_context_notifier(ctx, None);
        if let Some(err) = host.callable_error(&cb) {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #1 ($context) must be an array with valid callbacks as values, {err}"
            )));
        }
        host.set_stream_context_notifier(ctx, Some(cb));
        crate::ext::standard::http_fopen_wrapper::reset_progress(host, ctx);
    }
    if let Some(opts) = params.get(&ArrayKey::from_bytes(b"options")).map(deref) {
        let Value::Array(opts) = opts else {
            return Err(Thrown::type_error("Invalid stream/context parameter"));
        };
        parse_context_options(host, ctx, &opts)?;
    }
    Ok(())
}

/// `decode_context_param` for a function's first argument.
fn context_param(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    if !matches!(v, Value::Resource(_)) {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($context) must be of type resource, {} given",
            crate::zend::zpp::zval_value_name(host, &v)
        )));
    }
    host.resolve_stream_context(&v).ok_or_else(|| {
        Thrown::type_error(format!(
            "{fname}(): Argument #1 ($context) must be a valid stream/context"
        ))
    })
}

fn fn_stream_context_set_params(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_context_set_params";
    let ctx = context_param(host, args, fname)?;
    let params = crate::zend::zpp::array_param(host, args, 1, fname)?;
    parse_context_params(host, fname, &ctx, &params)?;
    Ok(Value::Bool(true))
}

fn fn_stream_context_get_params(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ctx = context_param(host, args, "stream_context_get_params")?;
    let mut a = rhp_values::PhpArray::new();
    if let Some(cb) = host.stream_context_notifier(&ctx) {
        a.insert(rhp_values::ArrayKey::from_bytes(b"notification"), cb);
    }
    let opts = match host.stream_context_options(&ctx) {
        Value::Null => Value::Array(rhp_values::PhpArray::new()),
        v => v,
    };
    a.insert(rhp_values::ArrayKey::from_bytes(b"options"), opts);
    Ok(Value::Array(a))
}

fn fn_stream_context_get_options(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match host.stream_context_options(&deref(&args[0])) {
        Value::Null => Ok(Value::Array(rhp_values::PhpArray::new())),
        v => Ok(v),
    }
}

fn fn_stream_context_set_option(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_context_set_option";
    if args.len() == 2 {
        host.emit_diagnostic(
            crate::E_DEPRECATED,
            b"Calling stream_context_set_option() with 2 arguments is deprecated, use stream_context_set_options() instead",
        )?;
    }
    let ctx = context_param(host, args, fname)?;
    match args.get(1).map(deref) {
        Some(Value::Array(opts)) => {
            if !matches!(args.get(2).map(deref), None | Some(Value::Null)) {
                return Err(crate::zend::zpp::value_error(
                    fname,
                    3,
                    "option_name",
                    "must be null when argument #2 ($wrapper_or_options) is an array",
                ));
            }
            if args.get(3).is_some() {
                return Err(crate::zend::zpp::value_error(
                    fname,
                    4,
                    "value",
                    "cannot be provided when argument #2 ($wrapper_or_options) is an array",
                ));
            }
            parse_context_options(host, &ctx, &opts)?;
        }
        w => {
            let wrapper = conv::to_string_bytes(&w.unwrap_or(Value::Null));
            let option = match args.get(2).map(deref) {
                None | Some(Value::Null) => {
                    return Err(crate::zend::zpp::value_error(
                        fname,
                        3,
                        "option_name",
                        "cannot be null when argument #2 ($wrapper_or_options) is a string",
                    ))
                }
                Some(v) => conv::to_string_bytes(&v),
            };
            let Some(value) = args.get(3).map(deref) else {
                return Err(crate::zend::zpp::value_error(
                    fname,
                    4,
                    "value",
                    "must be provided when argument #2 ($wrapper_or_options) is a string",
                ));
            };
            host.stream_context_set_option(&ctx, &wrapper, &option, value);
        }
    }
    Ok(Value::Bool(true))
}

fn fn_stream_context_set_options(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_context_set_options";
    let ctx = context_param(host, args, fname)?;
    let opts = crate::zend::zpp::array_param(host, args, 1, fname)?;
    parse_context_options(host, &ctx, &opts)?;
    Ok(Value::Bool(true))
}

/// `stream_copy_to_stream($from, $to, ?int $length = null, int $offset = 0)`:
/// seek `$from` to `$offset` (when > 0), then copy up to `$length` bytes
/// (null or negative = everything; php casts it to size_t).
fn fn_stream_copy_to_stream(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (src, _) =
        crate::ext::standard::file::stream_param(host, args, 0, "stream_copy_to_stream", "from")?;
    let (dst, _) =
        crate::ext::standard::file::stream_param(host, args, 1, "stream_copy_to_stream", "to")?;
    let maxlen = match args.get(2).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = conv::to_int(&v);
            (n >= 0).then_some(n as usize)
        }
    };
    let pos = args.get(3).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if pos > 0
        && !crate::ext::standard::file::stream_seek(
            host,
            "stream_copy_to_stream",
            &src,
            pos,
            libc::SEEK_SET,
        )?
    {
        crate::warn(
            host,
            "stream_copy_to_stream",
            &format!("Failed to seek to position {pos} in the stream"),
        )?;
        return Ok(Value::Bool(false));
    }
    if maxlen == Some(0) {
        return Ok(Value::Int(0));
    }
    let mut total = 0usize;
    loop {
        let chunk = match maxlen {
            Some(m) => (m - total).min(8192),
            None => 8192,
        };
        if chunk == 0 {
            break;
        }
        match host.stream_read(&src, chunk as i64) {
            Some(b) if !b.is_empty() => {
                host.stream_write(&dst, &b);
                total += b.len();
            }
            // a failed read fails the copy
            None => return Ok(Value::Bool(false)),
            _ => break,
        }
    }
    Ok(Value::Int(total as i64))
}

/// `apply_filter_to_stream`: stream_filter_append / stream_filter_prepend.
fn apply_filter_to_stream(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    append: bool,
) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, fname, "stream")?;
    let name = str_arg(args, 1);
    let mut read_write = int_arg(args, 2, 0);
    let params = args.get(3).map(deref);
    const READ: i64 = 1;
    const WRITE: i64 = 2;
    if read_write & (READ | WRITE) == 0 {
        // No chain given: the ones the stream's mode uses.
        let mode = host.stream_meta(&h).map(|m| m.mode).unwrap_or_default();
        if mode.contains(&b'r') || mode.contains(&b'+') {
            read_write |= READ;
        }
        if mode.contains(&b'w') || mode.contains(&b'+') || mode.contains(&b'a') {
            read_write |= WRITE;
        }
    }
    let mut res = None;
    for (bit, write) in [(READ, false), (WRITE, true)] {
        if read_write & bit == 0 {
            continue;
        }
        let Some(filter) = crate::main::streams::filter::create(host, &name, params.as_ref())
        else {
            return Ok(Value::Bool(false));
        };
        let register = write || read_write & WRITE == 0;
        match host.stream_filter_add(&h, filter, write, append, register) {
            Some(v) => res = Some(v),
            None => return Ok(Value::Bool(false)),
        }
    }
    Ok(res
        .filter(|v| !matches!(v, Value::Null))
        .unwrap_or(Value::Bool(false)))
}

fn fn_stream_filter_append(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    apply_filter_to_stream(host, args, "stream_filter_append", true)
}

fn fn_stream_filter_prepend(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    apply_filter_to_stream(host, args, "stream_filter_prepend", false)
}

/// `stream_filter_remove(resource $stream_filter): bool`.
fn fn_stream_filter_remove(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    Ok(Value::Bool(host.stream_filter_remove(&v)?))
}

fn fn_stream_get_wrappers(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = rhp_values::PhpArray::new();
    for scheme in host.stream_wrapper_schemes() {
        a.push(Value::str(scheme));
    }
    Ok(Value::Array(a))
}

pub(crate) fn write_ref(arg: Option<&Value>, value: Value) {
    if let Some(Value::Ref(rc)) = arg {
        *rc.borrow_mut() = value;
    }
}

// php's xport flags.
const STREAM_CLIENT_PERSISTENT: i64 = 1;
const STREAM_CLIENT_ASYNC_CONNECT: i64 = 2;
const STREAM_CLIENT_CONNECT: i64 = 4;
pub(crate) const STREAM_SERVER_BIND: i64 = 4;
pub(crate) const STREAM_SERVER_LISTEN: i64 = 8;
const STREAM_OOB: i64 = 1;
const STREAM_PEEK: i64 = 2;

/// The largest buffer `stream_socket_recvfrom` hands to one `recv`.
const RECV_CAP: usize = 16 << 20;

/// A stream argument (`PHP_Z_PARAM_STREAM`).
fn stream_arg(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Value, Thrown> {
    crate::ext::standard::file::stream_param(host, args, i, fname, pname).map(|(v, _)| v)
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

fn fn_stream_socket_server(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_server";
    let address = zpp_string(host, args, 0, fname, "address")?;
    let flags = match args.get(3) {
        Some(_) => zpp_long(host, args, 3, fname, "flags")?,
        None => STREAM_SERVER_BIND | STREAM_SERVER_LISTEN,
    };
    let ctx = context_arg(host, args, 4);
    write_ref(args.get(1), Value::Int(0));
    write_ref(args.get(2), Value::str(""));
    match xport_server(host, fname, &address, flags, &ctx)? {
        Opened::Ok(stream) => Ok(register_stream(host, *stream, &ctx, None)),
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
pub(crate) fn c_str(s: &[u8]) -> String {
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
        k.extend_from_slice(&escape_persistent_key(&address));
        k
    });
    let ctx = context_arg(host, args, 5);
    // _php_stream_xport_create's default for no timeout
    let tv = timeout_tv(timeout).unwrap_or((default_socket_timeout(host), 0));
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
        Opened::Ok(stream) => Ok(register_stream(
            host,
            *stream,
            &ctx,
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
    let srv_ctx = host.stream_context_of(&srv);
    let nodelay = srv_ctx
        .as_ref()
        .is_some_and(|c| ctx_socket_opt(host, c, "tcp_nodelay").is_some_and(|v| conv::to_bool(&v)));
    // php_openssl_tcp_sockop_accept: the client bit leaves the server's
    // method once a client is accepted with enable_on_connect.
    let tls = host.stream_fd_mut(&srv).and_then(|f| {
        f.tls.as_mut().map(|t| {
            if t.enable_on_connect {
                t.method &= !xp_ssl::IS_CLIENT;
            }
            (t.enable_on_connect, t.method)
        })
    });
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
    if nodelay {
        setsockopt_int(c.as_raw_fd(), libc::IPPROTO_TCP, libc::TCP_NODELAY, 1);
    }
    let mut ci = SockInfo::new(info.stream_type, info.timeout);
    ci.blocked = info.blocked;
    if cfg!(target_os = "linux") {
        // O_NONBLOCK is not inherited on Linux.
        ci.blocked = true;
    }
    let mut cs = FdStream::new_socket(c, ci);
    if let Some((enable, method)) = tls {
        cs.tls = Some(Box::new(SslSock::new(false, method, None)));
        if enable {
            let ok = xp_ssl::crypto_on_connect(host, &mut cs, srv_ctx.as_ref(), method)?;
            crate::flush_stream_diags(host, fname)?;
            if !ok {
                crate::warn(host, fname, "Failed to enable crypto")?;
                // php allocated the client stream before closing it
                host.consume_resource_ids(1);
                crate::warn(host, fname, "Accept failed: Cannot enable crypto")?;
                return Ok(Value::Bool(false));
            }
        }
    }
    if let Some(p) = peer {
        write_ref(args.get(2), Value::str(p));
    }
    Ok(host
        .stream_insert_fd(cs, srv_ctx)
        .unwrap_or(Value::Bool(false)))
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

/// `stream_socket_enable_crypto`.
fn fn_stream_socket_enable_crypto(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_socket_enable_crypto";
    let s = stream_arg(host, args, 0, fname, "stream")?;
    let enable = match args.get(1).map(deref) {
        Some(v @ (Value::Array(_) | Value::Object(_) | Value::Resource(_))) => {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #2 ($enable) must be of type bool, {} given",
                zval_value_name(host, &v)
            )))
        }
        Some(v) => conv::to_bool(&v),
        None => false,
    };
    let method = match args.get(2).map(deref) {
        None | Some(Value::Null) => None,
        Some(_) => Some(zpp_long(host, args, 2, fname, "crypto_method")?),
    };
    let session = match args.get(3).map(deref) {
        None | Some(Value::Null) => None,
        Some(_) => Some(
            crate::ext::standard::file::stream_param(host, args, 3, fname, "session_stream")
                .map_err(|e| {
                    let v = args.get(3).map(deref).unwrap_or(Value::Null);
                    if matches!(v, Value::Resource(_)) {
                        e
                    } else {
                        Thrown::type_error(format!(
                            "{fname}(): Argument #4 ($session_stream) must be of type resource or null, {} given",
                            zval_value_name(host, &v)
                        ))
                    }
                })?
                .0,
        ),
    };
    if enable {
        let method = match method {
            Some(m) => m,
            None => {
                let opt = host.stream_context_of(&s).and_then(|c| {
                    match host.stream_context_options(&c) {
                        Value::Array(o) => match o.get(&ArrayKey::from_bytes(b"ssl")).map(deref) {
                            Some(Value::Array(ssl)) => {
                                ssl.get(&ArrayKey::from_bytes(b"crypto_method")).map(deref)
                            }
                            _ => None,
                        },
                        _ => None,
                    }
                });
                match opt {
                    Some(v) => conv::to_int(&v),
                    None => {
                        return Err(Thrown::error(
                            "ValueError",
                            format!("{fname}(): Argument #3 ($crypto_method) must be specified when enabling encryption"),
                        ))
                    }
                }
            }
        };
        if xp_ssl::crypto_setup(host, fname, &s, method, session.as_ref())? < 0 {
            return Ok(Value::Bool(false));
        }
    }
    Ok(match xp_ssl::crypto_enable(host, fname, &s, enable)? {
        -1 => Value::Bool(false),
        0 => Value::Int(0),
        _ => Value::Bool(true),
    })
}

fn fn_stream_set_timeout(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_timeout_impl(host, args, "stream_set_timeout")
}

fn fn_socket_set_timeout(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
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
    if let Some(r) = us::set_option(host, &s, us::OPTION_READ_TIMEOUT, sec, Some(usec)) {
        return Ok(Value::Bool(r == us::OPTION_RETURN_OK));
    }
    Ok(Value::Bool(host.stream_set_timeout(&s, tv)))
}

fn fn_stream_set_read_buffer(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_set_read_buffer";
    let s = stream_arg(host, args, 0, fname, "stream")?;
    let size = zpp_long(host, args, 1, fname, "size")?;
    let (mode, ptr) = if size == 0 {
        (us::BUFFER_NONE, None)
    } else {
        (2, Some(size))
    };
    if let Some(r) = us::set_option(host, &s, us::OPTION_READ_BUFFER, mode, ptr) {
        return Ok(Value::Int(if r == 0 { 0 } else { -1 }));
    }
    Ok(Value::Int(0))
}

/// Per-stream chunk sizes (`stream_set_chunk_size` returns the previous one).
#[derive(Default)]
struct ChunkSizes(HashMap<u32, i64>);

/// php fills a stream's read buffer a chunk at a time, allocating the chunk
/// fallibly: a read on a stream whose chunk size `stream_set_chunk_size`
/// raised past the memory left gives up (`false`).
pub(crate) fn reserve_read_chunk(host: &mut dyn Host, stream: &Value) -> bool {
    let Value::Resource(r) = stream else {
        return true;
    };
    let size = host
        .module_state("stream_chunk_sizes", || Box::<ChunkSizes>::default())
        .and_then(|st| st.downcast_mut::<ChunkSizes>())
        .and_then(|st| st.0.get(&r.id).copied());
    match size {
        Some(n) if n > 8192 => rhp_values::mem::reserve(n as usize),
        _ => true,
    }
}

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
    if let Some(prev) = us::set_option(host, &s, us::OPTION_SET_CHUNK_SIZE, size, None) {
        return Ok(Value::Int(prev as i64));
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

/// `stream_get_transports()`: the socket transports, then openssl's.
fn fn_stream_get_transports(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for t in ["tcp", "udp", "unix", "udg"]
        .into_iter()
        .chain(xp_ssl::transport_names())
    {
        a.push(Value::str(t));
    }
    Ok(Value::Array(a))
}

/// The stream / socket transport constants (php's `file.c` registrations).
pub fn constants() -> Vec<(&'static str, Value)> {
    let i = |n: i64| Value::Int(n);
    vec![
        // file.c / user_filters.c (php_stream.h, php_stream_filter_api.h).
        ("STREAM_FILTER_READ", i(1)),
        ("STREAM_FILTER_WRITE", i(2)),
        ("STREAM_FILTER_ALL", i(3)),
        ("STREAM_USE_PATH", i(1)),
        ("STREAM_IGNORE_URL", i(2)),
        ("STREAM_REPORT_ERRORS", i(8)),
        ("STREAM_MUST_SEEK", i(16)),
        ("STREAM_URL_STAT_LINK", i(1)),
        ("STREAM_URL_STAT_QUIET", i(2)),
        ("STREAM_MKDIR_RECURSIVE", i(1)),
        ("STREAM_IS_URL", i(1)),
        ("STREAM_OPTION_BLOCKING", i(1)),
        ("STREAM_OPTION_READ_TIMEOUT", i(4)),
        ("STREAM_OPTION_READ_BUFFER", i(2)),
        ("STREAM_OPTION_WRITE_BUFFER", i(3)),
        ("STREAM_BUFFER_NONE", i(0)),
        ("STREAM_BUFFER_LINE", i(1)),
        ("STREAM_BUFFER_FULL", i(2)),
        ("STREAM_CAST_AS_STREAM", i(0)),
        ("STREAM_CAST_FOR_SELECT", i(3)),
        ("STREAM_META_TOUCH", i(1)),
        ("STREAM_META_OWNER_NAME", i(2)),
        ("STREAM_META_OWNER", i(3)),
        ("STREAM_META_GROUP_NAME", i(4)),
        ("STREAM_META_GROUP", i(5)),
        ("STREAM_META_ACCESS", i(6)),
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
        (
            "STREAM_NOTIFY_CONNECT",
            i(crate::ext::standard::http_fopen_wrapper::NOTIFY_CONNECT),
        ),
        (
            "STREAM_NOTIFY_AUTH_REQUIRED",
            i(crate::ext::standard::http_fopen_wrapper::NOTIFY_AUTH_REQUIRED),
        ),
        (
            "STREAM_NOTIFY_AUTH_RESULT",
            i(crate::ext::standard::http_fopen_wrapper::NOTIFY_AUTH_RESULT),
        ),
        (
            "STREAM_NOTIFY_MIME_TYPE_IS",
            i(crate::ext::standard::http_fopen_wrapper::NOTIFY_MIME_TYPE_IS),
        ),
        (
            "STREAM_NOTIFY_FILE_SIZE_IS",
            i(crate::ext::standard::http_fopen_wrapper::NOTIFY_FILE_SIZE_IS),
        ),
        (
            "STREAM_NOTIFY_REDIRECTED",
            i(crate::ext::standard::http_fopen_wrapper::NOTIFY_REDIRECTED),
        ),
        (
            "STREAM_NOTIFY_PROGRESS",
            i(crate::ext::standard::http_fopen_wrapper::NOTIFY_PROGRESS),
        ),
        (
            "STREAM_NOTIFY_FAILURE",
            i(crate::ext::standard::http_fopen_wrapper::NOTIFY_FAILURE),
        ),
        (
            "STREAM_NOTIFY_COMPLETED",
            i(crate::ext::standard::http_fopen_wrapper::NOTIFY_COMPLETED),
        ),
        ("STREAM_NOTIFY_RESOLVE", i(1)),
        ("STREAM_NOTIFY_SEVERITY_INFO", i(0)),
        ("STREAM_NOTIFY_SEVERITY_WARN", i(1)),
        ("STREAM_NOTIFY_SEVERITY_ERR", i(2)),
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

// ---------------------------------------------------------------------------
// stream_select
// ---------------------------------------------------------------------------

/// A by-ref array argument of stream_select (None when null).
fn select_array(v: Option<&Value>) -> Option<PhpArray> {
    match v.map(deref) {
        Some(Value::Array(a)) => Some(a),
        _ => None,
    }
}

fn fn_stream_select(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arrays = [
        select_array(args.first()),
        select_array(args.get(1)),
        select_array(args.get(2)),
    ];
    // Z_PARAM_LONG_OR_NULL: a float with a fraction is deprecated.
    let mut long_or_null = |v: Option<Value>| -> Result<Option<i64>, Thrown> {
        Ok(match v {
            None | Some(Value::Null) => None,
            Some(Value::Float(f)) if f.is_finite() && f.fract() != 0.0 => {
                crate::zend::operators::incompatible_double_to_long_error(host, f)?;
                Some(f as i64)
            }
            Some(v) => Some(conv::to_int(&v)),
        })
    };
    let sec = long_or_null(args.get(3).map(deref))?;
    let usec = long_or_null(args.get(4).map(deref))?;

    let mut sets: [libc::fd_set; 3] = unsafe { std::mem::zeroed() };
    for s in sets.iter_mut() {
        unsafe { libc::FD_ZERO(s) };
    }
    let mut max_fd: RawFd = 0;
    let mut nsets = 0;
    // php's `php_stream_from_zval_no_verify` on an element that is not a
    // stream raises a TypeError but carries on: the select still runs (and
    // rewrites the arrays) before the exception surfaces.
    let mut pending: Option<Thrown> = None;
    for (i, a) in arrays.iter().enumerate() {
        if let Some(a) = a {
            for (_, el) in a.iter() {
                let el = deref(el);
                if let Some((fd, _)) = select_fd(host, &el)? {
                    if (fd as usize) < libc::FD_SETSIZE {
                        unsafe { libc::FD_SET(fd, &mut sets[i]) };
                        max_fd = max_fd.max(fd);
                        nsets += 1;
                    }
                } else if !matches!(el, Value::Resource(_)) {
                    pending.get_or_insert_with(|| {
                        Thrown::type_error(
                            "stream_select(): supplied argument is not a valid stream resource",
                        )
                    });
                } else if let Some(m) = host.stream_meta(&el) {
                    crate::warn(
                        host,
                        "stream_select",
                        &format!(
                            "Cannot represent a stream of type {} as a select()able descriptor",
                            m.stream_type
                        ),
                    )?;
                } else {
                    // A closed stream or another kind of resource.
                    pending.get_or_insert_with(|| {
                        Thrown::type_error(
                            "stream_select(): supplied resource is not a valid stream resource",
                        )
                    });
                }
            }
        }
    }
    // (php chains the TypeError as the ValueError's previous exception.)
    if nsets == 0 {
        return Err(Thrown::error("ValueError", "No stream arrays were passed"));
    }
    if sec.is_none() && usec.is_some_and(|u| u != 0) {
        return Err(Thrown::error(
            "ValueError",
            "stream_select(): Argument #5 ($microseconds) must be null when argument #4 ($seconds) is null",
        ));
    }
    let mut tv = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let mut tvp: *mut libc::timeval = std::ptr::null_mut();
    if let Some(sec) = sec {
        let usec = usec.unwrap_or(0);
        if sec < 0 {
            return Err(Thrown::error(
                "ValueError",
                "stream_select(): Argument #4 ($seconds) must be greater than or equal to 0",
            ));
        }
        if usec < 0 {
            return Err(Thrown::error(
                "ValueError",
                "stream_select(): Argument #5 ($microseconds) must be greater than or equal to 0",
            ));
        }
        tv.tv_sec = (sec + usec / 1_000_000) as libc::time_t;
        tv.tv_usec = (usec % 1_000_000) as libc::suseconds_t;
        tvp = &mut tv;
    }

    // Streams with buffered unread data count as readable immediately.
    if let Some(r) = &arrays[0] {
        let mut ready = PhpArray::new();
        for (k, el) in r.iter() {
            if let Some((_, true)) = host.stream_select_fd(&deref(el)) {
                ready.insert(k.clone(), el.clone());
            }
        }
        if !ready.is_empty() {
            let n = ready.len() as i64;
            write_ref(args.first(), Value::Array(ready));
            for (i, a) in arrays.iter().enumerate().skip(1) {
                if a.is_some() {
                    write_ref(args.get(i), Value::Array(PhpArray::new()));
                }
            }
            return pending.map_or(Ok(Value::Int(n)), Err);
        }
    }

    let (rp, rest) = sets.split_at_mut(1);
    let (wp, ep) = rest.split_at_mut(1);
    // No retry on EINTR: php reports an interrupted select (a signal
    // arrived) as a warning and false, so the handler runs right after.
    let n = unsafe { libc::select(max_fd + 1, &mut rp[0], &mut wp[0], &mut ep[0], tvp) };
    if n == -1 {
        let e = errno();
        crate::warn(
            host,
            "stream_select",
            &format!(
                "Unable to select [{e}]: {} (max_fd={max_fd})",
                crate::main::streams::strerror(e)
            ),
        )?;
        return pending.map_or(Ok(Value::Bool(false)), Err);
    }
    for (i, a) in arrays.iter().enumerate() {
        let Some(a) = a else { continue };
        let mut kept = PhpArray::new();
        for (k, el) in a.iter() {
            if let Some((fd, _)) = select_fd(host, &deref(el))? {
                if (fd as usize) < libc::FD_SETSIZE && unsafe { libc::FD_ISSET(fd, &sets[i]) } {
                    kept.insert(k.clone(), el.clone());
                }
            }
        }
        write_ref(args.get(i), Value::Array(kept));
    }
    pending.map_or(Ok(Value::Int(n as i64)), Err)
}

/// php_stream_cast(PHP_STREAM_AS_FD_FOR_SELECT) with errors shown: a user
/// stream casts to the stream its `stream_cast` returns.
fn select_fd(host: &mut dyn Host, el: &Value) -> Result<Option<(RawFd, bool)>, Thrown> {
    let Value::Resource(r) = el else {
        return Ok(None);
    };
    let me = r.id;
    let was = host.stream_no_fclose(el, true);
    let inner = us::with(host, el, |s, h| s.cast(h, us::CAST_FOR_SELECT, true, me));
    host.stream_no_fclose(el, was);
    let Some(inner) = inner else {
        return Ok(host.stream_select_fd(el));
    };
    let Some(inner) = inner else {
        return Ok(None);
    };
    let fd = select_fd(host, &inner)?;
    if fd.is_none() {
        if let Some(m) = host.stream_meta(&inner) {
            crate::warn(
                host,
                "stream_select",
                &format!(
                    "Cannot represent a stream of type {} as a select()able descriptor",
                    m.stream_type
                ),
            )?;
        }
    }
    Ok(fd)
}

/// `stream_isatty($stream)`: isatty(2) on the stream's fd.
fn fn_stream_isatty(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    Ok(Value::Bool(match host.stream_select_fd(&v) {
        Some((fd, _)) => unsafe { libc::isatty(fd) == 1 },
        None => false,
    }))
}

fn fn_stream_context_get_default(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ctx = host.default_stream_context();
    if let Some(Value::Array(opts)) = args.first().map(deref) {
        parse_context_options(host, &ctx, &opts)?;
    }
    Ok(ctx)
}

/// `stream_context_set_default(array $options)`: the default context, with
/// `$options` applied.
fn fn_stream_context_set_default(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    fn_stream_context_get_default(host, args)
}

fn fn_stream_get_contents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, kind) = stream_param(host, args, 0, "stream_get_contents", "stream")?;
    let maxlen = int_arg(args, 1, -1);
    let offset = int_arg(args, 2, -1);
    if maxlen < -1 {
        return Err(Thrown::error(
            "ValueError",
            "stream_get_contents(): Argument #2 ($length) must be greater than or equal to -1",
        ));
    }
    if offset >= 0 {
        // A forward seek is relative, so an unseekable stream can emulate it.
        let ok = match host.stream_tell(&h) {
            Some(pos) if offset > pos => stream_seek(
                host,
                "stream_get_contents",
                &h,
                offset - pos,
                libc::SEEK_CUR,
            )?,
            Some(pos) if offset < pos => {
                stream_seek(host, "stream_get_contents", &h, offset, libc::SEEK_SET)?
            }
            _ => true,
        };
        if !ok {
            crate::warn(
                host,
                "stream_get_contents",
                &format!("Failed to seek to position {offset} in the stream"),
            )?;
            return Ok(Value::Bool(false));
        }
    }
    if maxlen == 0 {
        return Ok(Value::str(""));
    }
    if maxlen > 0 && kind == StreamKind::Memory {
        // _php_stream_copy_to_mem: one read returns at most a buffer's worth
        // from an extension's stream (zip entries), so read on to `$length`.
        let mut out = Vec::new();
        while out.len() < maxlen as usize && !host.stream_eof(&h) {
            match host.stream_read(&h, maxlen - out.len() as i64) {
                Some(b) if !b.is_empty() => out.extend_from_slice(&b),
                _ => break,
            }
        }
        return Ok(Value::str(out));
    }
    // php_stream_copy_to_mem failing still returns ""
    Ok(Value::str(host.stream_read(&h, maxlen).unwrap_or_default()))
}

fn fn_stream_get_meta_data(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    stream_get_meta_data_impl(host, args, "stream_get_meta_data")
}

/// `socket_get_status`: php's alias of `stream_get_meta_data`.
pub(crate) fn fn_socket_get_status(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    stream_get_meta_data_impl(host, args, "socket_get_status")
}

fn stream_get_meta_data_impl(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, fname, "stream")?;
    let Some(m) = host.stream_meta(&h) else {
        return Ok(Value::Bool(false));
    };
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: Value| {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    };
    // The TLS layer's META_DATA_API answer comes first.
    if let Some(c) = m.crypto {
        put("crypto", c);
    }
    let own_meta = m.rfc2397.is_some();
    if let Some(Value::Array(meta)) = m.rfc2397 {
        for (k, v) in meta.iter() {
            a.insert(k.clone(), v.clone());
        }
    }
    let mut put = |k: &str, v: Value| {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    };
    // A TEMP stream answers META_DATA_API itself, so php skips these three.
    if m.stream_type != "TEMP" && !own_meta {
        put("timed_out", Value::Bool(m.timed_out));
        put("blocked", Value::Bool(m.blocked));
        put("eof", Value::Bool(m.eof));
    }
    if let Some(d) = m.wrapper_data {
        put("wrapper_data", d);
    }
    // Pipes have no wrapper and no uri; directory streams no uri.
    if !m.wrapper_type.is_empty() {
        put("wrapper_type", Value::str(m.wrapper_type));
    }
    put("stream_type", Value::str(m.stream_type));
    put("mode", Value::str(m.mode));
    put("unread_bytes", Value::Int(m.unread_bytes));
    put("seekable", Value::Bool(m.seekable));
    if !m.uri.is_empty() {
        put("uri", Value::str(m.uri));
    }
    Ok(Value::Array(a))
}

/// `stream_get_line($stream, $length, $ending = "")`: read up to `$length`
/// bytes (0 = the default chunk), stopping before `$ending` (consumed).
fn fn_stream_get_line(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "stream_get_line", "stream")?;
    let mut max = int_arg(args, 1, 0);
    if max < 0 {
        return Err(Thrown::error(
            "ValueError",
            "stream_get_line(): Argument #2 ($length) must be greater than or equal to 0",
        ));
    }
    if max == 0 {
        max = 8192;
    }
    if !reserve_read_chunk(host, &h) {
        return Ok(Value::Null);
    }
    let ending = str_arg(args, 2);
    if let Some(r) = host.stream_get_record(&h, max as usize, &ending) {
        return Ok(r.map_or(Value::Bool(false), Value::str));
    }
    host.stream_note_line_read(&h, false);
    let r = get_line(host, &h, max, &ending);
    host.stream_note_line_read(&h, true);
    r
}

fn get_line(host: &mut dyn Host, h: &Value, max: i64, ending: &[u8]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    while (out.len() as i64) < max {
        let Some(b) = host.stream_read(h, 1) else {
            break;
        };
        if b.is_empty() {
            break;
        }
        out.extend_from_slice(&b);
        if !ending.is_empty() && out.ends_with(ending) {
            out.truncate(out.len() - ending.len());
            return Ok(Value::str(out));
        }
    }
    // php_stream_get_record: no delimiter, short of $length and not at
    // eof (a non-blocking stream out of data) returns false and leaves the
    // data buffered; so does nothing at eof
    let eof = host.stream_eof(h);
    if (out.len() as i64) < max && !eof && host.stream_push_back(h, &out) {
        return Ok(Value::Bool(false));
    }
    if out.is_empty() && eof {
        return Ok(Value::Bool(false));
    }
    Ok(Value::str(out))
}

fn fn_stream_is_local(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    if let Value::Resource(_) = v {
        let (h, _) = stream_param(host, args, 0, "stream_is_local", "stream")?;
        // A stream without a wrapper (pipe, socket) is not local in php.
        let wrapperless = host
            .stream_meta(&h)
            .is_some_and(|m| m.wrapper_type.is_empty());
        return Ok(Value::Bool(!wrapperless));
    }
    let url = conv::to_string_bytes(&v);
    let (scheme, _) = split_scheme(&url);
    let lower = scheme.to_ascii_lowercase();
    // Wrappers with is_url=1 in php: http, https, ftp, ftps, data (rfc2397).
    let is_url = matches!(
        lower.as_slice(),
        b"http" | b"https" | b"ftp" | b"ftps" | b"data"
    );
    // php also treats "data:" (no slashes) as the rfc2397 wrapper.
    let is_data = url.len() >= 5 && url[..5].eq_ignore_ascii_case(b"data:");
    Ok(Value::Bool(!(is_url || is_data)))
}

/// `stream_supports_lock`: plain-file and fd streams lock (flock).
fn fn_stream_supports_lock(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = stream_param(host, args, 0, "stream_supports_lock", "stream")?;
    if let Some(r) = us::set_option(host, &h, us::OPTION_LOCKING, 0, Some(0)) {
        return Ok(Value::Bool(r == 0));
    }
    Ok(Value::Bool(
        crate::ext::standard::file::plain_fd(host, &h).is_some(),
    ))
}

fn fn_stream_resolve_include_path(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = path_param(args, 0, "stream_resolve_include_path", "filename")?;
    Ok(resolve_include_path(host, &name)
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

/// `posix_isatty` is in ext/posix; sapi_windows_vt100_support exists on
/// Windows only.
pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("stream_get_wrappers", fn_stream_get_wrappers),
            ("stream_filter_append", fn_stream_filter_append),
            ("stream_filter_prepend", fn_stream_filter_prepend),
            ("stream_filter_remove", fn_stream_filter_remove),
            ("stream_copy_to_stream", fn_stream_copy_to_stream),
            ("stream_context_create", fn_stream_context_create),
            ("stream_context_get_options", fn_stream_context_get_options),
            ("stream_context_set_params", fn_stream_context_set_params),
            ("stream_context_get_params", fn_stream_context_get_params),
            ("stream_context_set_option", fn_stream_context_set_option),
            ("stream_context_set_options", fn_stream_context_set_options),
            ("stream_set_blocking", fn_stream_set_blocking),
            ("stream_set_write_buffer", fn_stream_set_write_buffer),
            ("stream_supports_lock", fn_stream_supports_lock),
            ("set_file_buffer", fn_set_file_buffer),
            ("stream_socket_server", fn_stream_socket_server),
            ("stream_socket_client", fn_stream_socket_client),
            ("stream_socket_accept", fn_stream_socket_accept),
            ("stream_socket_get_name", fn_stream_socket_get_name),
            ("stream_socket_shutdown", fn_stream_socket_shutdown),
            ("stream_socket_pair", fn_stream_socket_pair),
            ("stream_socket_sendto", fn_stream_socket_sendto),
            ("stream_socket_recvfrom", fn_stream_socket_recvfrom),
            (
                "stream_socket_enable_crypto",
                fn_stream_socket_enable_crypto,
            ),
            ("stream_get_transports", fn_stream_get_transports),
            ("stream_set_timeout", fn_stream_set_timeout),
            ("socket_set_timeout", fn_socket_set_timeout),
            ("stream_set_read_buffer", fn_stream_set_read_buffer),
            ("stream_set_chunk_size", fn_stream_set_chunk_size),
            ("socket_get_status", fn_socket_get_status),
            ("socket_set_blocking", fn_socket_set_blocking),
            ("stream_select", fn_stream_select),
            ("stream_isatty", fn_stream_isatty),
            ("stream_context_get_default", fn_stream_context_get_default),
            ("stream_context_set_default", fn_stream_context_set_default),
            ("stream_get_contents", fn_stream_get_contents),
            ("stream_get_meta_data", fn_stream_get_meta_data),
            ("stream_get_line", fn_stream_get_line),
            ("stream_is_local", fn_stream_is_local),
            (
                "stream_resolve_include_path",
                fn_stream_resolve_include_path,
            ),
        ],
    );
}
