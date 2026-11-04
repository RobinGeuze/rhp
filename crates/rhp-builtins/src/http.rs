//! The http:// and https:// stream wrapper: a port of
//! ext/standard/http_fopen_wrapper.c over the socket transports
//! (`socket_fns::xport_client`, TLS from ext/openssl/xp_ssl.rs). The opened
//! stream is the connection itself, positioned at the body, with the
//! response headers as its wrapper data and php's "dechunk" filter for a
//! chunked body.

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::ext::openssl::xp_ssl;
use crate::main::streams::plain_wrapper::FdStream;
use crate::socket_fns::{xport_client, Opened};
use crate::{Host, Thrown};

const HTTP_HEADER_BLOCK_SIZE: usize = 1024;
const HTTP_HEADER_MAX_LOCATION_SIZE: usize = 8182;
const PHP_URL_REDIRECT_MAX: i64 = 20;

const HEADER_USER_AGENT: u32 = 1;
const HEADER_HOST: u32 = 2;
const HEADER_AUTH: u32 = 4;
const HEADER_FROM: u32 = 8;
const HEADER_CONTENT_LENGTH: u32 = 16;
const HEADER_TYPE: u32 = 32;
const HEADER_CONNECTION: u32 = 64;

const WRAPPER_HEADER_INIT: u32 = 1;
const WRAPPER_REDIRECTED: u32 = 2;
const WRAPPER_KEEP_METHOD: u32 = 4;
const WRAPPER_STRIP_AUTH: u32 = 8;

// php_stream_notify codes (STREAM_NOTIFY_*) and severities
pub(crate) const NOTIFY_CONNECT: i64 = 2;
pub(crate) const NOTIFY_AUTH_REQUIRED: i64 = 3;
pub(crate) const NOTIFY_MIME_TYPE_IS: i64 = 4;
pub(crate) const NOTIFY_FILE_SIZE_IS: i64 = 5;
pub(crate) const NOTIFY_REDIRECTED: i64 = 6;
pub(crate) const NOTIFY_PROGRESS: i64 = 7;
pub(crate) const NOTIFY_FAILURE: i64 = 9;
pub(crate) const NOTIFY_COMPLETED: i64 = 8;
pub(crate) const NOTIFY_AUTH_RESULT: i64 = 10;
const SEVERITY_INFO: i64 = 0;
const SEVERITY_ERR: i64 = 2;

/// The state of one `fopen` of an http URL.
struct Cx<'a> {
    host: &'a mut dyn Host,
    fname: &'a str,
    mode: &'a [u8],
    /// The stream context (Null: none).
    ctx: Value,
    /// `STREAM_ONLY_GET_HEADERS` (`get_headers`).
    only_headers: bool,
    /// php_stream_wrapper_log_error's list, shown in "Failed to open stream".
    errors: Vec<String>,
    /// The response headers of every hop (`$http_response_header`); `None`
    /// until a request was sent (php's UNDEF).
    headers: Option<PhpArray>,
}

impl Cx<'_> {
    fn opt(&self, name: &str) -> Option<Value> {
        ctx_opt(self.host, &self.ctx, "http", name)
    }

    fn log_error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }

    fn notify(
        &mut self,
        code: i64,
        severity: i64,
        msg: Option<&[u8]>,
        msg_code: i64,
        bytes: i64,
        max: i64,
    ) -> Result<(), Thrown> {
        notify(
            self.host, &self.ctx, code, severity, msg, msg_code, bytes, max,
        )
    }
}

/// A context option (`php_stream_context_get_option`).
pub(crate) fn ctx_opt(host: &dyn Host, ctx: &Value, wrapper: &str, name: &str) -> Option<Value> {
    if matches!(ctx, Value::Null) {
        return None;
    }
    match host.stream_context_options(ctx) {
        Value::Array(a) => match a
            .get(&ArrayKey::from_bytes(wrapper.as_bytes()))
            .map(rhp_values::ops::deref)
        {
            Some(Value::Array(w)) => w
                .get(&ArrayKey::from_bytes(name.as_bytes()))
                .map(rhp_values::ops::deref),
            _ => None,
        },
        _ => None,
    }
}

/// `php_stream_notification_notify` through the context's `notification`
/// parameter.
#[allow(clippy::too_many_arguments)]
pub(crate) fn notify(
    host: &mut dyn Host,
    ctx: &Value,
    code: i64,
    severity: i64,
    msg: Option<&[u8]>,
    msg_code: i64,
    bytes: i64,
    max: i64,
) -> Result<(), Thrown> {
    if matches!(ctx, Value::Null) {
        return Ok(());
    }
    let Some(cb) = host.stream_context_notifier(ctx) else {
        return Ok(());
    };
    let args = vec![
        Value::Int(code),
        Value::Int(severity),
        msg.map_or(Value::Null, |m| Value::str(m.to_vec())),
        Value::Int(msg_code),
        Value::Int(bytes),
        Value::Int(max),
    ];
    host.call_callable(&cb, &args)?;
    Ok(())
}

/// The progress counters of each context's notifier (`progress`,
/// `progress_max`, and whether progress is reported).
#[derive(Default)]
struct Progress(std::collections::HashMap<u32, (i64, i64, bool)>);

fn ctx_key(host: &dyn Host, ctx: &Value) -> Option<u32> {
    let Value::Resource(r) = host.stream_context_of(ctx).unwrap_or_else(|| ctx.clone()) else {
        return None;
    };
    Some(r.id)
}

fn progress_state<'a>(host: &'a mut dyn Host, ctx: &Value) -> Option<&'a mut (i64, i64, bool)> {
    let key = ctx_key(host, ctx)?;
    host.module_state("stream_notifier_progress", || Box::<Progress>::default())
        .and_then(|st| st.downcast_mut::<Progress>())
        .map(|st| st.0.entry(key).or_default())
}

/// A new notifier for `ctx` (`stream_context_set_params`): counters reset.
pub(crate) fn reset_progress(host: &mut dyn Host, ctx: &Value) {
    if let Some(p) = progress_state(host, ctx) {
        *p = (0, 0, false);
    }
}

/// `php_stream_notify_progress_init`.
fn progress_init(host: &mut dyn Host, ctx: &Value, sofar: i64, max: i64) -> Result<(), Thrown> {
    if host.stream_context_notifier(ctx).is_none() {
        return Ok(());
    }
    if let Some(p) = progress_state(host, ctx) {
        *p = (sofar, max, true);
    }
    notify(
        host,
        ctx,
        NOTIFY_PROGRESS,
        SEVERITY_INFO,
        None,
        0,
        sofar,
        max,
    )
}

/// `php_stream_notify_progress_increment`.
fn progress_increment(
    host: &mut dyn Host,
    ctx: &Value,
    dsofar: i64,
    dmax: i64,
) -> Result<(), Thrown> {
    if host.stream_context_notifier(ctx).is_none() {
        return Ok(());
    }
    let Some(p) = progress_state(host, ctx) else {
        return Ok(());
    };
    if !p.2 {
        return Ok(());
    }
    p.0 += dsofar;
    p.1 += dmax;
    let (sofar, max) = (p.0, p.1);
    notify(
        host,
        ctx,
        NOTIFY_PROGRESS,
        SEVERITY_INFO,
        None,
        0,
        sofar,
        max,
    )
}

/// Deliver what stream operations on `res` owed its context's notifier.
pub fn deliver(
    host: &mut dyn Host,
    res: &Value,
    events: &[crate::main::streams::plain_wrapper::NotifyEvent],
) -> Result<(), Thrown> {
    let Some(ctx) = host.stream_context_of(res) else {
        return Ok(());
    };
    if events.contains(&crate::main::streams::plain_wrapper::NotifyEvent::RenegLimit) {
        // php_openssl_limit_handshake_reneg's callback
        let keep = match ctx_opt(host, &ctx, "ssl", "reneg_limit_callback") {
            Some(cb) => matches!(
                host.call_callable(&cb, std::slice::from_ref(res))?,
                Value::Bool(true)
            ),
            None => false,
        };
        if let Some(f) = host.stream_fd_mut(res) {
            let keep = f.tls.as_deref_mut().is_none_or(|t| t.reneg_answered(keep));
            if !keep {
                f.reneg_close();
            }
        }
    }
    deliver_ctx(host, &ctx, events)
}

fn deliver_ctx(
    host: &mut dyn Host,
    ctx: &Value,
    events: &[crate::main::streams::plain_wrapper::NotifyEvent],
) -> Result<(), Thrown> {
    use crate::main::streams::plain_wrapper::NotifyEvent;
    for ev in events {
        match *ev {
            NotifyEvent::Progress(n) => progress_increment(host, ctx, n as i64, 0)?,
            NotifyEvent::RenegLimit => {}
            NotifyEvent::Completed => {
                if host.stream_context_notifier(ctx).is_none() {
                    continue;
                }
                let (sofar, max) = progress_state(host, ctx).map_or((0, 0), |p| (p.0, p.1));
                notify(
                    host,
                    ctx,
                    NOTIFY_COMPLETED,
                    SEVERITY_INFO,
                    None,
                    0,
                    sofar,
                    max,
                )?;
            }
        }
    }
    Ok(())
}

/// `php_strip_url_passwd`.
fn strip_url_passwd(url: &[u8]) -> Vec<u8> {
    let mut out = url.to_vec();
    let Some(p) = out.windows(3).position(|w| w == b"://") else {
        return out;
    };
    let start = p + 3;
    let end = out.iter().position(|&c| c == 0).unwrap_or(out.len());
    if let Some(at) = out[start..end]
        .iter()
        .position(|&c| c == b'@')
        .map(|a| start + a)
    {
        let mut url_start = start;
        let mut i = 0;
        while i < 3 && url_start < at {
            out[url_start] = b'.';
            url_start += 1;
            i += 1;
        }
        let tail = out[at..end].to_vec();
        out.truncate(url_start);
        out.extend_from_slice(&tail);
    }
    out
}

/// `%.1F`.
fn format_f1(d: f64) -> String {
    if d.is_nan() {
        "NAN".into()
    } else if d.is_infinite() {
        if d > 0.0 { "INF" } else { "-INF" }.into()
    } else {
        format!("{d:.1}")
    }
}

/// `php_trim(s, NULL, 0, 3)`: php's default whitespace from both ends.
fn trim(s: &[u8]) -> &[u8] {
    let ws = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0);
    let start = s.iter().position(|c| !ws(c)).unwrap_or(s.len());
    let end = s.iter().rposition(|c| !ws(c)).map_or(start, |e| e + 1);
    &s[start..end]
}

/// `next_header_line`.
fn next_header_line(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && s[i] != b'\r' && s[i] != b'\n' {
        i += 1;
    }
    if i < s.len() && s[i] == b'\r' {
        i += 1;
    }
    if i < s.len() && s[i] == b'\n' {
        i += 1;
    }
    i
}

/// `strip_header`: drop every `name:` line (and its folded continuation
/// lines) from both the header bag and its lowercase copy.
fn strip_header(bag: &mut Vec<u8>, lc: &mut Vec<u8>, name: &[u8]) {
    let mut line = 0;
    while line < lc.len() {
        if !lc[line..].starts_with(name) {
            line = next_header_line(lc, line);
            continue;
        }
        let mut colon = line + name.len();
        while colon < lc.len() && (lc[colon] == b' ' || lc[colon] == b'\t') {
            colon += 1;
        }
        if colon >= lc.len() || lc[colon] != b':' {
            line = next_header_line(lc, line);
            continue;
        }
        let mut next = next_header_line(lc, line);
        while next < lc.len() && (lc[next] == b' ' || lc[next] == b'\t') {
            next = next_header_line(lc, next);
        }
        if next >= lc.len() {
            // drop the line break before it too
            while line > 0 && (lc[line - 1] == b'\r' || lc[line - 1] == b'\n') {
                line -= 1;
            }
        }
        bag.drain(line..next);
        lc.drain(line..next);
    }
}

/// `check_has_header`: `header` at the start or right after a CRLF.
fn check_has_header(headers: &[u8], header: &[u8]) -> bool {
    let mut from = 0;
    while let Some(p) = headers[from..]
        .windows(header.len())
        .position(|w| w == header)
        .map(|p| from + p)
    {
        if p == 0 || (p >= 2 && headers[p - 1] == b'\n' && headers[p - 2] == b'\r') {
            return true;
        }
        from = p + 1;
    }
    false
}

/// `php_stream_handle_proxy_authorization_header`: the first
/// `Proxy-Authorization:` line of `s`, CRLF-terminated.
fn proxy_authorization_header(s: &[u8]) -> Option<Vec<u8>> {
    let mut i = 0;
    loop {
        while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
            i += 1;
        }
        let start = i;
        let mut p = i;
        while p < s.len() && !matches!(s[p], b':' | b'\r' | b'\n') {
            p += 1;
        }
        if p < s.len() && s[p] == b':' {
            p += 1;
            let is_pa =
                p - start == 20 && s[start..p].eq_ignore_ascii_case(b"Proxy-Authorization:");
            while p < s.len() && s[p] != b'\r' && s[p] != b'\n' {
                p += 1;
            }
            if is_pa {
                let mut v = s[start..p].to_vec();
                v.extend_from_slice(b"\r\n");
                return Some(v);
            }
        }
        i = p;
        while i < s.len() && (s[i] == b'\r' || s[i] == b'\n') {
            i += 1;
        }
        if i >= s.len() {
            return None;
        }
    }
}

/// php's `%s` of a string: up to the first NUL.
fn c_str(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// The parts of the URL as `php_uri` holds them.
struct Resource {
    scheme: Vec<u8>,
    user: Option<Vec<u8>>,
    pass: Option<Vec<u8>>,
    host: Vec<u8>,
    /// 0 when there is none.
    port: i64,
    path: Option<Vec<u8>>,
    query: Option<Vec<u8>>,
}

fn parse_resource(path: &[u8]) -> Option<Resource> {
    let u = crate::url::parse(path)?;
    Some(Resource {
        scheme: u.scheme.unwrap_or_default(),
        user: u.user,
        pass: u.pass,
        host: u.host.unwrap_or_default(),
        port: u.port as i64,
        path: u.path,
        query: u.query,
    })
}

/// What the response headers said (`php_stream_http_response_header_info`).
struct HeaderInfo {
    dechunk: bool,
    file_size: i64,
    error: bool,
    follow_location: bool,
    location: Option<Vec<u8>>,
}

/// `php_stream_http_response_header_trim`: trailing CR/LF and whitespace
/// off; whether whitespace was trimmed.
fn header_trim(line: &mut Vec<u8>) -> bool {
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    let space = matches!(line.last(), Some(b' ' | b'\t'));
    if space {
        while matches!(line.last(), Some(b' ' | b'\t')) {
            line.pop();
        }
    }
    space
}

/// `php_stream_http_response_headers_parse`: fold `line` into the pending
/// `last` header (returned while still open), or finish `last`.
fn headers_parse(
    cx: &mut Cx,
    last: Vec<u8>,
    line: Option<&mut Vec<u8>>,
    response_code: i64,
    info: &mut HeaderInfo,
) -> Result<Option<Vec<u8>>, Thrown> {
    if let Some(line) = line {
        if !line.is_empty() && line[0] != b'\n' && line[0] != b'\r' {
            if header_trim(line) && line.is_empty() {
                // only spaces: an empty folding header
                return Ok(Some(last));
            }
            if line[0] == b' ' || line[0] == b'\t' {
                let start = line
                    .iter()
                    .position(|&c| c != b' ' && c != b'\t')
                    .unwrap_or(line.len());
                let mut ext = last;
                ext.push(b' ');
                ext.extend_from_slice(&line[start..]);
                return Ok(Some(ext));
            }
        }
    }
    let Some(colon) = last.iter().position(|&c| c == b':') else {
        info.error = true;
        cx.log_error("HTTP invalid response format (no colon in header line)!");
        return Ok(None);
    };
    if last[1.min(colon)..colon]
        .iter()
        .any(|&c| c == b' ' || c == b'\t')
    {
        info.error = true;
        cx.log_error("HTTP invalid response format (space in header name)!");
        return Ok(None);
    }
    let mut v = colon + 1;
    let end = last.len().saturating_sub(1);
    while v < end && (last[v] == b' ' || last[v] == b'\t') {
        v += 1;
    }
    let value = c_str(&last[v.min(last.len())..]).to_vec();
    let starts = |p: &[u8]| last.len() >= p.len() && last[..p.len()].eq_ignore_ascii_case(p);
    let mut store = true;
    if starts(b"Location:") {
        if let Some(f) = cx.opt("follow_location") {
            info.follow_location = conv::to_bool(&f);
        } else if !((300..304).contains(&response_code)
            || response_code == 307
            || response_code == 308)
        {
            info.follow_location = false;
        }
        if value.len() > HTTP_HEADER_MAX_LOCATION_SIZE {
            info.error = true;
            cx.log_error(format!(
                "HTTP Location header size is over the limit of {HTTP_HEADER_MAX_LOCATION_SIZE} bytes"
            ));
            return Ok(None);
        }
        info.location = Some(value);
    } else if starts(b"Content-Type:") {
        cx.notify(NOTIFY_MIME_TYPE_IS, SEVERITY_INFO, Some(&value), 0, 0, 0)?;
    } else if starts(b"Content-Length:") {
        if value.first().is_some_and(|c| c.is_ascii_digit())
            && value.iter().all(|c| c.is_ascii_digit())
        {
            let parsed = value
                .iter()
                .try_fold(0u64, |a, &d| {
                    a.checked_mul(10)?.checked_add((d - b'0') as u64)
                })
                .unwrap_or(u64::MAX);
            info.file_size = parsed.min(i64::MAX as u64) as i64;
            let line = c_str(&last).to_vec();
            // php_stream_notify_file_size: the size is bytes_max
            cx.notify(
                NOTIFY_FILE_SIZE_IS,
                SEVERITY_INFO,
                Some(&line),
                0,
                0,
                info.file_size,
            )?;
        }
    } else if starts(b"Transfer-Encoding:")
        && value.len() >= 7
        && value[..7].eq_ignore_ascii_case(b"Chunked")
        && !cx.only_headers
    {
        let decode = cx.opt("auto_decode").is_none_or(|v| conv::to_bool(&v));
        if decode {
            info.dechunk = true;
            store = false;
        }
    }
    if store {
        cx.headers
            .get_or_insert_with(PhpArray::new)
            .push(Value::str(last));
    }
    Ok(None)
}

/// php's `php_stream_get_line(stream, buf, maxlen)`: at most `maxlen - 1`
/// bytes (`None`: a whole line).
fn get_line(s: &mut FdStream, maxlen: Option<usize>) -> Option<Vec<u8>> {
    s.gets(maxlen)
}

/// `php_stream_url_wrap_http_ex`.
fn wrap_http_ex(
    cx: &mut Cx,
    path: &[u8],
    mut redirect_max: i64,
    flags: u32,
) -> Result<Option<FdStream>, Thrown> {
    let header_init = flags & WRAPPER_HEADER_INIT != 0;
    let redirected = flags & WRAPPER_REDIRECTED != 0;
    let redirect_keep_method = flags & WRAPPER_KEEP_METHOD != 0;
    if redirect_max < 1 {
        cx.log_error("Redirection limit reached, aborting");
        return Ok(None);
    }
    match cx.opt("uri_parser_class") {
        None | Some(Value::Null) => {}
        Some(Value::Str(s)) if s.as_bytes() == b"parse_url" => {}
        Some(Value::Str(s)) if s.as_bytes() == b"Uri\\Rfc3986\\Uri" || s.as_bytes() == b"Uri\\WhatWg\\Url" => {
            return Err(Thrown::unsupported(
                "http stream context option uri_parser_class",
                "ext/uri is not implemented",
            ));
        }
        Some(_) => {
            return Err(Thrown::error(
                "ValueError",
                format!(
                    "{}(): Provided stream context has invalid value for the \"uri_parser_class\" option",
                    cx.fname
                ),
            ))
        }
    }
    let Some(mut resource) = parse_resource(path) else {
        return Ok(None);
    };
    let is_http = resource.scheme.eq_ignore_ascii_case(b"http")
        || resource.scheme.eq_ignore_ascii_case(b"https");
    let use_ssl;
    let mut use_proxy = false;
    let mut request_fulluri = false;
    let transport: Vec<u8>;
    let proxy = match cx.opt("proxy") {
        Some(Value::Str(p)) if !p.is_empty() => Some(p.as_bytes().to_vec()),
        _ => None,
    };
    if !is_http {
        let Some(p) = proxy else {
            // a redirect to another scheme: that wrapper opens it
            crate::open_failed(cx.host, cx.fname, path)?;
            return Ok(None);
        };
        request_fulluri = true;
        use_ssl = false;
        use_proxy = true;
        transport = p;
    } else {
        if cx
            .mode
            .iter()
            .any(|&c| matches!(c, b'a' | b'w' | b'x' | b'+'))
        {
            cx.log_error("HTTP wrapper does not support writeable connections");
            return Ok(None);
        }
        if let Some(v) = cx.opt("request_fulluri") {
            request_fulluri = conv::to_bool(&v);
        }
        use_ssl = resource.scheme.len() > 4 && resource.scheme[4] == b's';
        if resource.port == 0 {
            resource.port = if use_ssl { 443 } else { 80 };
        }
        match proxy {
            Some(p) => {
                use_proxy = true;
                transport = p;
            }
            None => {
                let mut t = if use_ssl {
                    b"ssl://".to_vec()
                } else {
                    b"tcp://".to_vec()
                };
                t.extend_from_slice(c_str(&resource.host));
                t.extend_from_slice(format!(":{}", resource.port).as_bytes());
                transport = t;
            }
        }
    }
    if request_fulluri && path.iter().any(|&c| c == b'\n' || c == b'\r') {
        cx.log_error("HTTP wrapper full URI path does not allow CR or LF characters");
        return Ok(None);
    }
    let timeout = match cx.opt("timeout") {
        Some(v) => {
            let d = conv::to_float(&v);
            let max = u64::MAX as f64 / 1_000_000.0;
            if d > max {
                cx.log_error(format!("timeout must be lower than {}", max as u64));
                return Ok(None);
            }
            let sec = d as i64;
            (sec, ((d - sec as f64) * 1_000_000.0) as i64)
        }
        None => (default_socket_timeout(cx.host), 0),
    };
    let ctx = cx.ctx.clone();
    let mut stream = match xport_client(cx.host, cx.fname, &transport, true, false, timeout, &ctx)?
    {
        Opened::Ok(mut s) => {
            if let Some(sock) = s.sock_mut() {
                sock.timeout = Some(timeout);
            }
            Some(*s)
        }
        Opened::Fail {
            text, allocated, ..
        } => {
            if allocated {
                cx.host.consume_resource_ids(1);
            }
            if let Some(t) = text {
                cx.log_error(t);
            }
            None
        }
    };

    if let (Some(s), true, true) = (stream.as_mut(), use_proxy, use_ssl) {
        // CONNECT through the proxy, then TLS to the origin.
        let mut header = b"CONNECT ".to_vec();
        header.extend_from_slice(c_str(&resource.host));
        header.extend_from_slice(format!(":{} HTTP/1.0\r\n", resource.port).as_bytes());
        match cx.opt("header") {
            Some(Value::Array(a)) => {
                for (_, h) in a.iter() {
                    if let Value::Str(h) = rhp_values::ops::deref(h) {
                        if let Some(pa) = proxy_authorization_header(h.as_bytes()) {
                            header.extend_from_slice(&pa);
                            break;
                        }
                    }
                }
            }
            Some(Value::Str(h)) if !h.is_empty() => {
                if let Some(pa) = proxy_authorization_header(h.as_bytes()) {
                    header.extend_from_slice(&pa);
                }
            }
            _ => {}
        }
        header.extend_from_slice(b"\r\n");
        let mut ok = s.write(&header) == Some(header.len());
        crate::flush_stream_diags(cx.host, cx.fname)?;
        if ok {
            while let Some(line) = get_line(s, Some(HTTP_HEADER_BLOCK_SIZE - 1)) {
                if matches!(line.first(), None | Some(b'\n' | b'\r' | 0)) {
                    break;
                }
            }
            crate::flush_stream_diags(cx.host, cx.fname)?;
            // peer_name defaults to the origin host, not the proxy's
            let mut opts =
                xp_ssl::stream_opts(cx.host, (!matches!(ctx, Value::Null)).then_some(&ctx));
            if opts.get("peer_name").is_none() {
                opts = opts.with("peer_name", Value::str(c_str(&resource.host).to_vec()));
            }
            ok = xp_ssl::crypto_with_opts(cx.host, s, &ctx, &opts, xp_ssl::SSLV23_CLIENT)?;
            crate::flush_stream_diags(cx.host, cx.fname)?;
        }
        if !ok {
            cx.log_error("Cannot connect to HTTPS server through proxy");
            stream = None;
            cx.host.consume_resource_ids(1);
        }
    }

    let mut info = HeaderInfo {
        dechunk: false,
        file_size: 0,
        error: false,
        follow_location: true,
        location: None,
    };
    let Some(mut s) = stream else {
        return Ok(None);
    };
    // a stream allocated for this hop that does not come back
    let drop_stream = |cx: &mut Cx, s: FdStream| {
        drop(s);
        cx.host.consume_resource_ids(1);
    };

    cx.notify(NOTIFY_CONNECT, SEVERITY_INFO, None, 0, 0, 0)?;

    if header_init {
        if let Some(v) = cx.opt("max_redirects") {
            redirect_max = conv::to_int(&v) as i32 as i64;
        }
    }

    let mut req: Vec<u8> = Vec::new();
    let mut custom_method = false;
    if let Some(Value::Str(m)) = cx.opt("method") {
        if !m.is_empty()
            && (!redirected
                || redirect_keep_method
                || m.as_bytes() == b"GET"
                || m.as_bytes() == b"HEAD")
        {
            custom_method = true;
            req.extend_from_slice(m.as_bytes());
            req.push(b' ');
        }
    }
    if !custom_method {
        req.extend_from_slice(b"GET ");
    }
    if request_fulluri {
        req.extend_from_slice(c_str(path));
    } else {
        match &resource.path {
            Some(p) if !p.is_empty() => req.extend_from_slice(c_str(p)),
            _ => req.push(b'/'),
        }
        if let Some(q) = &resource.query {
            req.push(b'?');
            req.extend_from_slice(c_str(q));
        }
    }
    match cx.opt("protocol_version") {
        Some(v) => {
            req.extend_from_slice(
                format!(" HTTP/{}\r\n", format_f1(conv::to_float(&v))).as_bytes(),
            );
        }
        None => req.extend_from_slice(b" HTTP/1.1\r\n"),
    }

    let mut have_header = 0u32;
    let mut user_headers: Option<Vec<u8>> = None;
    if let Some(h) = cx.opt("header") {
        let tmp: Option<Vec<u8>> = match h {
            Value::Array(a) => {
                let mut joined = Vec::new();
                for (_, v) in a.iter() {
                    if let Value::Str(s) = rhp_values::ops::deref(v) {
                        joined.extend_from_slice(s.as_bytes());
                        joined.extend_from_slice(b"\r\n");
                    }
                }
                (!joined.is_empty()).then(|| trim(&joined).to_vec())
            }
            Value::Str(s) if !s.is_empty() => Some(trim(s.as_bytes()).to_vec()),
            _ => None,
        };
        if let Some(tmp) = tmp.filter(|t| !t.is_empty()) {
            // C string semantics from here on
            let tmp = c_str(&tmp).to_vec();
            let mut bag = tmp.clone();
            let mut t = tmp.to_ascii_lowercase();
            if !header_init && !redirect_keep_method {
                strip_header(&mut bag, &mut t, b"content-length");
                strip_header(&mut bag, &mut t, b"content-type");
            }
            if flags & WRAPPER_STRIP_AUTH != 0 {
                strip_header(&mut bag, &mut t, b"authorization");
                strip_header(&mut bag, &mut t, b"cookie");
                if !use_proxy {
                    strip_header(&mut bag, &mut t, b"proxy-authorization");
                }
            }
            for (name, bit) in [
                (&b"user-agent:"[..], HEADER_USER_AGENT),
                (b"host:", HEADER_HOST),
                (b"from:", HEADER_FROM),
                (b"authorization:", HEADER_AUTH),
                (b"content-length:", HEADER_CONTENT_LENGTH),
                (b"content-type:", HEADER_TYPE),
                (b"connection:", HEADER_CONNECTION),
            ] {
                if check_has_header(&t, name) {
                    have_header |= bit;
                }
            }
            let mut bag = Some(bag).filter(|b| !b.is_empty());
            // the Proxy-Authorization header goes to the proxy only
            if use_proxy && use_ssl {
                if let (Some(b), Some(sp)) = (bag.as_mut(), find(&t, b"proxy-authorization:")) {
                    if sp == 0 || t[sp - 1] == b'\n' {
                        let mut s0 = sp;
                        let mut p = sp + 20;
                        while s0 > 0 && (t[s0 - 1] == b' ' || t[s0 - 1] == b'\t') {
                            s0 -= 1;
                        }
                        while p < t.len() && t[p] != b'\r' && t[p] != b'\n' {
                            p += 1;
                        }
                        while p < t.len() && (t[p] == b'\r' || t[p] == b'\n') {
                            p += 1;
                        }
                        if p >= t.len() {
                            if s0 == 0 {
                                bag = None;
                            } else {
                                while s0 > 0 && (t[s0 - 1] == b'\r' || t[s0 - 1] == b'\n') {
                                    s0 -= 1;
                                }
                                b.truncate(s0);
                            }
                        } else {
                            b.drain(s0..p);
                        }
                    }
                }
            }
            user_headers = bag;
        }
    }

    if have_header & HEADER_AUTH == 0 {
        if let Some(user) = &resource.user {
            let mut scratch = crate::main::php_variables::url_decode(user);
            scratch.push(b':');
            if let Some(pass) = &resource.pass {
                scratch.extend_from_slice(&crate::main::php_variables::url_decode(pass));
            }
            req.extend_from_slice(b"Authorization: Basic ");
            req.extend_from_slice(&crate::ext::hash::primitives::base64_encode(&scratch));
            req.extend_from_slice(b"\r\n");
            cx.notify(NOTIFY_AUTH_REQUIRED, SEVERITY_INFO, None, 0, 0, 0)?;
        }
    }
    if have_header & HEADER_FROM == 0 {
        if let Some(from) = cx.host.ini_get(b"from").filter(|f| !f.is_empty()) {
            req.extend_from_slice(b"From: ");
            req.extend_from_slice(c_str(&from));
            req.extend_from_slice(b"\r\n");
        }
    }
    if have_header & HEADER_HOST == 0 {
        req.extend_from_slice(b"Host: ");
        req.extend_from_slice(c_str(&resource.host));
        if (use_ssl && resource.port != 443 && resource.port != 0)
            || (!use_ssl && resource.port != 80 && resource.port != 0)
        {
            req.extend_from_slice(format!(":{}", resource.port).as_bytes());
        }
        req.extend_from_slice(b"\r\n");
    }
    if have_header & HEADER_CONNECTION == 0 {
        req.extend_from_slice(b"Connection: close\r\n");
    }
    let ua: Option<Vec<u8>> = match cx.opt("user_agent") {
        Some(Value::Str(s)) => Some(s.as_bytes().to_vec()),
        _ => cx.host.ini_get(b"user_agent").filter(|u| !u.is_empty()),
    };
    if have_header & HEADER_USER_AGENT == 0 {
        if let Some(ua) = ua.map(|u| c_str(&u).to_vec()).filter(|u| !u.is_empty()) {
            req.extend_from_slice(b"User-Agent: ");
            req.extend_from_slice(&ua);
            req.extend_from_slice(b"\r\n");
        }
    }
    let content = match cx.opt("content") {
        Some(Value::Str(c)) if !c.is_empty() && (header_init || redirect_keep_method) => {
            Some(c.as_bytes().to_vec())
        }
        _ => None,
    };
    if let Some(uh) = user_headers {
        if let Some(c) = &content {
            if have_header & HEADER_CONTENT_LENGTH == 0 {
                req.extend_from_slice(format!("Content-Length: {}\r\n", c.len()).as_bytes());
                have_header |= HEADER_CONTENT_LENGTH;
            }
        }
        req.extend_from_slice(c_str(&uh));
        req.extend_from_slice(b"\r\n");
    }
    if let Some(c) = &content {
        if have_header & HEADER_CONTENT_LENGTH == 0 {
            req.extend_from_slice(format!("Content-Length: {}\r\n", c.len()).as_bytes());
        }
        if have_header & HEADER_TYPE == 0 {
            req.extend_from_slice(b"Content-Type: application/x-www-form-urlencoded\r\n");
            crate::notice(
                cx.host,
                cx.fname,
                "Content-type not specified assuming application/x-www-form-urlencoded",
            )?;
        }
        req.extend_from_slice(b"\r\n");
        req.extend_from_slice(c);
    } else {
        req.extend_from_slice(b"\r\n");
    }

    s.write(&req);
    let ev = s.take_events();
    deliver_ctx(cx.host, &ctx, &ev)?;
    if let Some(e) = s.take_send_error() {
        crate::notice(
            cx.host,
            cx.fname,
            &format!(
                "Send of {} bytes failed with errno={} {}",
                e.count,
                e.errno,
                crate::main::streams::strerror(e.errno)
            ),
        )?;
    }
    crate::flush_stream_diags(cx.host, cx.fname)?;
    cx.headers.get_or_insert_with(PhpArray::new);

    // the status line
    let mut response_code;
    let mut reqok = false;
    let mut ignore_errors = false;
    let status = if !s.eof() {
        get_line(&mut s, Some(128 - 1))
    } else {
        None
    };
    crate::flush_stream_diags(cx.host, cx.fname)?;
    let ev = s.take_events();
    deliver_ctx(cx.host, &ctx, &ev)?;
    let Some(mut tmp_line) = status else {
        drop_stream(cx, s);
        cx.log_error("HTTP request failed!");
        return Ok(None);
    };
    response_code = if tmp_line.len() > 9 {
        i64::from(crate::c_atoi(&tmp_line[9..]))
    } else {
        0
    };
    if let Some(v) = cx.opt("ignore_errors") {
        ignore_errors = conv::to_bool(&v);
    }
    if cx.only_headers || ignore_errors {
        reqok = true;
    }
    if (100..200).contains(&response_code) && response_code != 101 {
        // skip informational responses up to the real status line
        loop {
            if s.eof() {
                break;
            }
            match get_line(&mut s, Some(128 - 1)) {
                Some(l) => {
                    tmp_line = l;
                    if tmp_line.len() >= 6 && tmp_line[..6].eq_ignore_ascii_case(b"HTTP/1") {
                        break;
                    }
                }
                None => break,
            }
        }
        response_code = if tmp_line.len() > 9 {
            i64::from(crate::c_atoi(&tmp_line[9..]))
        } else {
            0
        };
    }
    if (200..400).contains(&response_code) {
        reqok = true;
    } else {
        let code = if response_code == 403 {
            NOTIFY_AUTH_RESULT
        } else {
            NOTIFY_FAILURE
        };
        let text = c_str(&tmp_line).to_vec();
        cx.notify(code, SEVERITY_ERR, Some(&text), response_code, 0, 0)?;
    }
    // the C buffer keeps its line end; only the stored copy loses it
    let status_raw = tmp_line.clone();
    if tmp_line.last() == Some(&b'\n') {
        tmp_line.pop();
        if tmp_line.last() == Some(&b'\r') {
            tmp_line.pop();
        }
    } else {
        // read and discard the rest of the status line
        get_line(&mut s, None);
    }
    cx.headers
        .get_or_insert_with(PhpArray::new)
        .push(Value::str(tmp_line.clone()));

    // the headers
    let mut last: Option<Vec<u8>> = None;
    while !s.eof() {
        let Some(mut line) = get_line(&mut s, None) else {
            break;
        };
        let last_line = match line.first() {
            Some(b'\r') => {
                if line.get(1) != Some(&b'\n') {
                    drop_stream(cx, s);
                    cx.log_error("HTTP invalid header name (cannot start with CR character)!");
                    return Ok(None);
                }
                true
            }
            Some(b'\n') => true,
            _ => false,
        };
        if let Some(l) = last.take() {
            last = headers_parse(cx, l, Some(&mut line), response_code, &mut info)?;
            if last.is_none() {
                if info.error {
                    drop_stream(cx, s);
                    return Ok(None);
                }
            } else {
                continue;
            }
        } else if !last_line {
            if line.first().is_some_and(|&c| c == b' ' || c == b'\t') {
                drop_stream(cx, s);
                cx.log_error("HTTP invalid response format (folding header at the start)!");
                return Ok(None);
            }
            header_trim(&mut line);
        }
        if last_line {
            break;
        }
        last = Some(line);
    }
    crate::flush_stream_diags(cx.host, cx.fname)?;
    let ev = s.take_events();
    deliver_ctx(cx.host, &ctx, &ev)?;
    if let Some(l) = last.take() {
        headers_parse(cx, l, None, response_code, &mut info)?;
    }

    if !reqok || (info.location.is_some() && info.follow_location) {
        if !info.follow_location || ((cx.only_headers || ignore_errors) && redirect_max <= 1) {
            return finish(cx, s, &info, header_init);
        }
        if let Some(loc) = info.location.clone() {
            cx.notify(NOTIFY_REDIRECTED, SEVERITY_INFO, Some(&loc), 0, 0, 0)?;
        }
        drop_stream(cx, s);
        let Some(location) = info.location.take() else {
            cx.log_error(format!(
                "HTTP request failed! {}",
                String::from_utf8_lossy(c_str(&status_raw))
            ));
            return Ok(None);
        };
        let lower_starts =
            |p: &[u8]| location.len() >= p.len() && location[..p.len()].eq_ignore_ascii_case(p);
        let new_path = if location.len() < 8
            || !(lower_starts(b"http://")
                || lower_starts(b"https://")
                || lower_starts(b"ftp://")
                || lower_starts(b"ftps://"))
        {
            let loc_path = if location.first() != Some(&b'/') {
                match resource.path.as_mut() {
                    Some(rp) if location.len() > 1 => {
                        let rp_c = c_str(rp).to_vec();
                        let base = match rp_c.iter().rposition(|&c| c == b'/') {
                            Some(i) => rp_c[..=i].to_vec(),
                            None if rp_c.is_empty() => b"/".to_vec(),
                            None => b"/".to_vec(),
                        };
                        let mut lp = base.clone();
                        if base != b"/" {
                            lp.push(b'/');
                        }
                        lp.extend_from_slice(&location);
                        lp
                    }
                    _ => {
                        let mut lp = b"/".to_vec();
                        lp.extend_from_slice(&location);
                        lp
                    }
                }
            } else {
                location.clone()
            };
            let mut np = c_str(&resource.scheme).to_vec();
            np.extend_from_slice(b"://");
            np.extend_from_slice(c_str(&resource.host));
            if (use_ssl && resource.port != 443) || (!use_ssl && resource.port != 80) {
                np.extend_from_slice(format!(":{}", resource.port).as_bytes());
            }
            np.extend_from_slice(&loc_path);
            np
        } else {
            location.clone()
        };
        let Some(mut new_resource) = parse_resource(&new_path) else {
            cx.log_error(format!(
                "Invalid redirect URL! {}",
                String::from_utf8_lossy(c_str(&new_path))
            ));
            return Ok(None);
        };
        let default_port = if use_ssl { 443 } else { 80 };
        let port_or = |p: i64| if p != 0 { p } else { default_port };
        let same_origin = !resource.scheme.is_empty()
            && !new_resource.scheme.is_empty()
            && resource.scheme.eq_ignore_ascii_case(&new_resource.scheme)
            && !resource.host.is_empty()
            && !new_resource.host.is_empty()
            && resource.host.eq_ignore_ascii_case(&new_resource.host)
            && port_or(resource.port) == port_or(new_resource.port);
        // control characters in the user, password and path
        let starts_ci =
            |p: &[u8]| new_path.len() >= p.len() && new_path[..p.len()].eq_ignore_ascii_case(p);
        if !starts_ci(b"http://") || !starts_ci(b"https://") {
            for v in [
                &mut new_resource.user,
                &mut new_resource.pass,
                &mut new_resource.path,
            ]
            .into_iter()
            .flatten()
            {
                {
                    *v = crate::main::php_variables::url_decode(v);
                    if v.iter().any(|c| c.is_ascii_control()) {
                        cx.log_error(format!(
                            "Invalid redirect URL! {}",
                            String::from_utf8_lossy(c_str(&new_path))
                        ));
                        return Ok(None);
                    }
                }
            }
        }
        let mut new_flags = WRAPPER_REDIRECTED | (flags & WRAPPER_STRIP_AUTH);
        if !same_origin {
            new_flags |= WRAPPER_STRIP_AUTH;
        }
        if response_code == 307 || response_code == 308 {
            new_flags |= WRAPPER_KEEP_METHOD;
        }
        // php's `out:` tail runs in this frame too, for the stream the
        // redirect returned (its chunked filter was dropped above)
        info.dechunk = false;
        return match wrap_http_ex(cx, &new_path, redirect_max - 1, new_flags)? {
            Some(s) => finish(cx, s, &info, header_init),
            None => Ok(None),
        };
    }
    finish(cx, s, &info, header_init)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The `out:` tail of `php_stream_url_wrap_http_ex` for a stream that is
/// handed back.
fn finish(
    cx: &mut Cx,
    mut s: FdStream,
    info: &HeaderInfo,
    header_init: bool,
) -> Result<Option<FdStream>, Thrown> {
    if let Some(sock) = s.sock_mut() {
        if header_init {
            sock.wrapper_data = Some(Value::Array(cx.headers.clone().unwrap_or_default()));
        }
        sock.mode = Some(cx.mode.to_vec());
    }
    // the progress notifications start
    let ctx = cx.ctx.clone();
    progress_init(cx.host, &ctx, 0, info.file_size)?;
    s.set_position(0);
    if info.dechunk {
        s.add_dechunk();
    }
    // body bytes that came in with the headers
    if s.unread() > 0 {
        progress_increment(cx.host, &ctx, s.unread() as i64, 0)?;
    }
    // php_openssl_is_http_stream_talking_to_iis
    if let Some(t) = s.tls.as_deref_mut() {
        t.talking_to_iis = cx.headers.as_ref().is_some_and(|h| {
            h.iter().any(|(_, v)| match v {
                Value::Str(s) => {
                    s.as_bytes().eq_ignore_ascii_case(b"Server: Microsoft-IIS")
                        || s.as_bytes().eq_ignore_ascii_case(b"Server: GFE/")
                }
                _ => false,
            })
        });
    }
    Ok(Some(s))
}

fn default_socket_timeout(host: &dyn Host) -> i64 {
    host.ini_get(b"default_socket_timeout")
        .map(|v| conv::to_int(&Value::str(v)))
        .unwrap_or(60)
}

/// `php_stream_url_wrap_http` from builtin `fname`: the opened stream
/// resource, or `None` after php's "Failed to open stream" warning. Sets the
/// last response headers and the caller's `$http_response_header`.
pub fn open(
    host: &mut dyn Host,
    fname: &str,
    url: &[u8],
    mode: &[u8],
    ctx: &Value,
    only_headers: bool,
) -> Result<Option<Value>, Thrown> {
    host.set_http_response_headers(None);
    let mut cx = Cx {
        host,
        fname,
        mode,
        ctx: ctx.clone(),
        only_headers,
        errors: Vec::new(),
        headers: None,
    };
    let r = wrap_http_ex(&mut cx, url, PHP_URL_REDIRECT_MAX, WRAPPER_HEADER_INIT);
    let Cx {
        host,
        errors,
        headers,
        ..
    } = cx;
    if let Some(h) = &headers {
        let lines = h.iter().map(|(_, v)| conv::to_string_bytes(v)).collect();
        host.set_http_response_headers(Some(lines));
        host.set_local_var(b"http_response_header", Value::Array(h.clone()));
    }
    let stream = r?;
    let Some(mut s) = stream else {
        // php_stream_display_wrapper_errors
        let msg = if errors.is_empty() {
            "operation failed".to_string()
        } else {
            let html = host
                .ini_get(b"html_errors")
                .is_some_and(|v| conv::to_bool(&Value::str(v)));
            errors.join(if html { "<br />\n" } else { "\n" })
        };
        crate::warn_arg(
            host,
            fname,
            &strip_url_passwd(url),
            &format!("Failed to open stream: {msg}"),
        )?;
        return Ok(None);
    };
    if let Some(sock) = s.sock_mut() {
        sock.wrapper = Some("http");
        sock.uri = Some(url.to_vec());
    }
    let ctx = (!matches!(ctx, Value::Null)).then(|| ctx.clone());
    Ok(host.stream_insert_fd(s, ctx))
}

/// `fopen`-style open from builtin `fname`: the context argument (the
/// default context when absent) and php's `allow_url_fopen` check.
pub fn open_arg(
    host: &mut dyn Host,
    fname: &str,
    url: &[u8],
    mode: &[u8],
    ctx_arg: Option<&Value>,
) -> Result<Option<Value>, Thrown> {
    let ctx = match ctx_arg.map(rhp_values::ops::deref) {
        Some(v @ Value::Resource(_)) => v,
        _ => host.default_stream_context(),
    };
    let allowed = host
        .ini_get(b"allow_url_fopen")
        .is_none_or(|v| conv::to_bool(&Value::str(v)));
    if !allowed {
        let scheme = crate::main::streams::split_scheme(url).0.to_vec();
        crate::warn(
            host,
            fname,
            &format!(
                "{}:// wrapper is disabled in the server configuration by allow_url_fopen=0",
                String::from_utf8_lossy(&scheme)
            ),
        )?;
        crate::warn_arg(
            host,
            fname,
            &strip_url_passwd(url),
            "Failed to open stream: no suitable wrapper could be found",
        )?;
        return Ok(None);
    }
    open(host, fname, url, mode, &ctx, false)
}

/// Open `url` and read it to the end, after seeking to `offset` and at
/// most `maxlen` bytes (`file_get_contents`, `file`, `readfile`, `copy`).
/// `None` once php's warnings were given.
pub fn get_contents(
    host: &mut dyn Host,
    fname: &str,
    url: &[u8],
    ctx_arg: Option<&Value>,
    offset: i64,
    maxlen: Option<usize>,
) -> Result<Option<Vec<u8>>, Thrown> {
    let Some(res) = open_arg(host, fname, url, b"rb", ctx_arg)? else {
        return Ok(None);
    };
    let whence = if offset > 0 {
        libc::SEEK_SET
    } else {
        libc::SEEK_END
    };
    if offset != 0 && !host.stream_seek(&res, offset, whence) {
        crate::warn(host, fname, "Stream does not support seeking")?;
        crate::warn(
            host,
            fname,
            &format!("Failed to seek to position {offset} in the stream"),
        )?;
        host.stream_close(&res);
        return Ok(None);
    }
    let mut out = Vec::new();
    match maxlen {
        None => out = host.stream_read(&res, -1).unwrap_or_default(),
        Some(max) => {
            while out.len() < max && !host.stream_eof(&res) {
                match host.stream_read(&res, (max - out.len()) as i64) {
                    Some(b) if !b.is_empty() => out.extend_from_slice(&b),
                    _ => break,
                }
            }
        }
    }
    crate::flush_stream_diags(host, fname)?;
    host.stream_close(&res);
    Ok(Some(out))
}

/// Whether `url` is for this wrapper.
pub fn is_http(url: &[u8]) -> bool {
    let scheme = crate::main::streams::split_scheme(url).0;
    scheme.eq_ignore_ascii_case(b"http") || scheme.eq_ignore_ascii_case(b"https")
}

#[cfg(test)]
mod tests;
