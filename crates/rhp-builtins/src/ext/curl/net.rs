//! The transfer engine behind `ext/curl`: a `Send` request description
//! ([`Spec`]) is performed on any thread ([`perform`]) into an [`Outcome`] —
//! the received header blocks, the body, libcurl-style error code/message and
//! the `curl_getinfo()` numbers. Delivery of that outcome to PHP (streams,
//! callbacks, return buffers) happens on the VM thread in `mod.rs`.
//!
//! HTTP(S) rides on `ureq` (rustls TLS with bundled Mozilla roots), with
//! redirects followed here (not by ureq) so every hop's headers reach the header
//! handler and `redirect_count` is exact, the way libcurl reports them.
//! `file://` is served directly, mirroring libcurl's FILE protocol (including
//! its synthesized `Content-Length` / `Accept-ranges` / `Last-Modified` header
//! lines).

use std::collections::HashMap;
use std::io::Read;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// CURLPROTO_* bits used here.
pub const PROTO_HTTP: i64 = 1;
pub const PROTO_HTTPS: i64 = 2;
pub const PROTO_FILE: i64 = 1024;

/// CURL_HTTP_VERSION_* as reported by CURLINFO_HTTP_VERSION.
const HTTP_VERSION_1_0: i64 = 1;
const HTTP_VERSION_1_1: i64 = 2;
const HTTP_VERSION_2: i64 = 3;

/// Hop-by-hop headers that HTTP/2 does not carry.
const H1_ONLY_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
];

/// Everything needed to perform one transfer, detached from the VM.
#[derive(Clone, Debug)]
pub struct Spec {
    pub url: String,
    pub custom_request: Option<String>,
    pub nobody: bool,
    /// POST requested (CURLOPT_POST or CURLOPT_POSTFIELDS).
    pub post: bool,
    /// PUT/upload requested with this body.
    pub upload: Option<Vec<u8>>,
    pub body: Option<Vec<u8>>,
    /// Content-Type to send with `body` unless the user supplied one.
    pub body_content_type: Option<String>,
    /// Raw CURLOPT_HTTPHEADER lines.
    pub headers: Vec<Vec<u8>>,
    pub follow: bool,
    pub max_redirs: i64,
    pub timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub proxy: Option<String>,
    pub userpwd: Option<String>,
    pub useragent: Option<String>,
    pub referer: Option<String>,
    pub cookie: Option<String>,
    pub range: Option<String>,
    /// CURLOPT_ENCODING: `None` = no Accept-Encoding requested.
    pub encoding: Option<String>,
    pub ipresolve: i64,
    pub protocols: i64,
    pub redir_protocols: i64,
    pub failonerror: bool,
    /// CURLOPT_HTTP_VERSION (0 = libcurl's default: HTTP/2 over TLS).
    pub http_version: i64,
    pub cainfo: Option<String>,
    pub capath: Option<String>,
}

/// The transfer numbers `curl_getinfo()` reports.
#[derive(Clone, Debug)]
pub struct Info {
    pub effective_url: String,
    pub content_type: Option<String>,
    pub http_code: i64,
    pub header_size: i64,
    pub request_size: i64,
    pub filetime: i64,
    pub redirect_count: i64,
    pub total_us: i64,
    pub namelookup_us: i64,
    pub connect_us: i64,
    pub appconnect_us: i64,
    pub pretransfer_us: i64,
    pub starttransfer_us: i64,
    pub posttransfer_us: i64,
    pub redirect_us: i64,
    pub queue_us: i64,
    pub size_upload: i64,
    pub size_download: i64,
    pub content_length_download: f64,
    pub content_length_upload: f64,
    pub redirect_url: Option<String>,
    pub primary_ip: String,
    pub primary_port: i64,
    pub local_ip: String,
    pub local_port: i64,
    pub http_version: i64,
    pub protocol: i64,
    pub scheme: Option<String>,
    pub effective_method: String,
    pub cainfo: Option<String>,
    pub capath: Option<String>,
    pub used_proxy: i64,
    pub conn_id: i64,
    pub referer: Option<String>,
}

impl Info {
    /// The state of a handle that has not performed a transfer yet.
    pub fn fresh(url: &str, method: &str) -> Info {
        Info {
            effective_url: url.to_string(),
            content_type: None,
            http_code: 0,
            header_size: 0,
            request_size: 0,
            filetime: -1,
            redirect_count: 0,
            total_us: 0,
            namelookup_us: 0,
            connect_us: 0,
            appconnect_us: 0,
            pretransfer_us: 0,
            starttransfer_us: 0,
            posttransfer_us: 0,
            redirect_us: 0,
            queue_us: 0,
            size_upload: 0,
            size_download: 0,
            content_length_download: -1.0,
            content_length_upload: -1.0,
            redirect_url: None,
            primary_ip: String::new(),
            primary_port: -1,
            local_ip: String::new(),
            local_port: -1,
            http_version: 0,
            protocol: 0,
            scheme: None,
            effective_method: method.to_string(),
            cainfo: None,
            capath: None,
            used_proxy: 0,
            conn_id: -1,
            referer: None,
        }
    }
}

/// The result of [`perform`].
pub struct Outcome {
    /// Each received header block (one per response hop), as raw lines
    /// including their trailing CRLF; the final blank line is included.
    pub header_blocks: Vec<Vec<Vec<u8>>>,
    pub body: Vec<u8>,
    pub errno: i64,
    pub error: String,
    pub info: Info,
}

impl Spec {
    /// The request method libcurl would use (CURLINFO_EFFECTIVE_METHOD).
    pub fn method(&self) -> String {
        if let Some(m) = &self.custom_request {
            return m.clone();
        }
        if self.nobody {
            "HEAD".into()
        } else if self.upload.is_some() {
            "PUT".into()
        } else if self.post {
            "POST".into()
        } else {
            "GET".into()
        }
    }
}

fn us(d: Duration) -> i64 {
    d.as_micros() as i64
}

/// Split `scheme://...` off a URL (lowercased scheme), like libcurl's URL
/// parser does first. `None` = no scheme present.
fn scheme_of(url: &str) -> Option<String> {
    let idx = url.find("://")?;
    let s = &url[..idx];
    if s.is_empty()
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
    {
        return None;
    }
    Some(s.to_ascii_lowercase())
}

const KNOWN_PROTOCOLS: &[(&str, i64)] = &[
    ("http", 1),
    ("https", 2),
    ("ftp", 4),
    ("ftps", 8),
    ("scp", 16),
    ("sftp", 32),
    ("telnet", 64),
    ("ldap", 128),
    ("ldaps", 256),
    ("dict", 512),
    ("file", 1024),
    ("tftp", 2048),
    ("imap", 4096),
    ("imaps", 8192),
    ("pop3", 16384),
    ("pop3s", 32768),
    ("smtp", 65536),
    ("smtps", 131072),
    ("rtsp", 262144),
    ("gopher", 33554432),
    ("gophers", 536870912),
    ("mqtt", 268435456),
    ("ws", 1073741824),
    ("wss", 2147483648),
];

fn fail(mut info: Info, errno: i64, error: impl Into<String>, start: Instant) -> Outcome {
    info.total_us = us(start.elapsed());
    Outcome {
        header_blocks: Vec::new(),
        body: Vec::new(),
        errno,
        error: error.into(),
        info,
    }
}

/// Perform the transfer described by `spec`.
pub fn perform(spec: &Spec) -> Outcome {
    let start = Instant::now();
    let mut info = Info::fresh(&spec.url, &spec.method());
    info.cainfo = spec.cainfo.clone();
    info.capath = spec.capath.clone();
    info.referer = spec.referer.clone();
    if spec.url.is_empty() {
        return fail(info, 3, "No URL set", start);
    }
    // libcurl guesses http:// for scheme-less URLs.
    let url = match scheme_of(&spec.url) {
        Some(_) => spec.url.clone(),
        None => {
            if spec.url.contains(char::is_whitespace) {
                return fail(
                    info,
                    3,
                    "URL rejected: Malformed input to a URL function",
                    start,
                );
            }
            format!("http://{}", spec.url)
        }
    };
    let scheme = scheme_of(&url).unwrap_or_default();
    let proto_bit = match KNOWN_PROTOCOLS.iter().find(|(n, _)| *n == scheme) {
        Some((_, b)) => *b,
        None => {
            return fail(
                info,
                1,
                format!("Protocol \"{scheme}\" not supported"),
                start,
            )
        }
    };
    if spec.protocols & proto_bit == 0 {
        return fail(info, 1, format!("Protocol \"{scheme}\" is disabled"), start);
    }
    match scheme.as_str() {
        "file" => perform_file(spec, &url, info, start),
        "http" | "https" => perform_http(spec, url, info, start),
        _ => fail(
            info,
            1,
            format!("Protocol \"{scheme}\" not supported"),
            start,
        ),
    }
}

/// php-style `Last-Modified` date (`Sat, 31 Oct 2026 21:15:47 GMT`).
fn http_date(secs: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[days.rem_euclid(7) as usize],
        d,
        MONTHS[(mo - 1) as usize],
        y,
        h,
        m,
        s
    )
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn perform_file(spec: &Spec, url: &str, mut info: Info, start: Instant) -> Outcome {
    info.protocol = PROTO_FILE;
    info.scheme = Some("file".into());
    // file://[host]/path — only an empty host or "localhost" is local.
    let rest = &url[url.find("://").map(|i| i + 3).unwrap_or(0)..];
    let path_part = match rest.find('/') {
        Some(0) => rest,
        Some(i) => {
            let host = &rest[..i];
            if !host.eq_ignore_ascii_case("localhost") && !host.eq_ignore_ascii_case("127.0.0.1") {
                return fail(
                    info,
                    3,
                    "URL rejected: Malformed input to a URL function",
                    start,
                );
            }
            &rest[i..]
        }
        None => {
            return fail(
                info,
                3,
                "URL rejected: Malformed input to a URL function",
                start,
            )
        }
    };
    let path_part = path_part.split(['?', '#']).next().unwrap_or("");
    let path_bytes = percent_decode(path_part);
    let path_str = String::from_utf8_lossy(&path_bytes).into_owned();
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStrExt;
        std::path::PathBuf::from(std::ffi::OsStr::from_bytes(&path_bytes))
    };
    #[cfg(not(unix))]
    let path = std::path::PathBuf::from(&path_str);
    info.namelookup_us = us(start.elapsed());
    let meta = std::fs::metadata(&path);
    let data = match (&meta, spec.nobody) {
        (Ok(m), _) if m.is_dir() => None,
        (Ok(_), true) => Some(Vec::new()),
        (Ok(_), false) => std::fs::read(&path).ok(),
        _ => None,
    };
    let (Ok(meta), Some(data)) = (meta, data) else {
        return fail(info, 37, format!("Could not open file {path_str}"), start);
    };
    let len = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    info.pretransfer_us = us(start.elapsed());
    info.starttransfer_us = info.pretransfer_us;
    let headers = vec![
        format!("Content-Length: {len}\r\n").into_bytes(),
        b"Accept-ranges: bytes\r\n".to_vec(),
        format!("Last-Modified: {}\r\n", http_date(mtime)).into_bytes(),
        b"\r\n".to_vec(),
    ];
    info.content_length_download = len as f64;
    info.filetime = mtime;
    info.size_download = data.len() as i64;
    info.total_us = us(start.elapsed());
    info.posttransfer_us = info.pretransfer_us;
    info.conn_id = 0;
    Outcome {
        header_blocks: vec![headers],
        body: data,
        errno: 0,
        error: String::new(),
        info,
    }
}

/// Filter resolved addresses by CURLOPT_IPRESOLVE.
struct FamilyResolver(i64);

impl ureq::Resolver for FamilyResolver {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        let addrs: Vec<SocketAddr> = netloc.to_socket_addrs()?.collect();
        Ok(addrs
            .into_iter()
            .filter(|a| match self.0 {
                1 => a.is_ipv4(),
                2 => a.is_ipv6(),
                _ => true,
            })
            .collect())
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct AgentKey {
    proxy: Option<String>,
    connect_timeout_ms: u64,
    ipresolve: i64,
}

/// One pooled agent per distinct connection configuration, shared by every
/// transfer (and every multi worker thread), so keep-alive connections are
/// reused across handles — like libcurl's shared connection cache.
fn agent_for(spec: &Spec) -> ureq::Agent {
    static AGENTS: OnceLock<Mutex<HashMap<AgentKey, ureq::Agent>>> = OnceLock::new();
    let key = AgentKey {
        proxy: spec.proxy.clone().filter(|p| !p.is_empty()),
        connect_timeout_ms: spec.connect_timeout_ms,
        ipresolve: spec.ipresolve,
    };
    let map = AGENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = map.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(a) = map.get(&key) {
        return a.clone();
    }
    let mut b = ureq::AgentBuilder::new()
        .redirects(0)
        .user_agent("")
        .max_idle_connections_per_host(8)
        .timeout_connect(Duration::from_millis(if key.connect_timeout_ms > 0 {
            key.connect_timeout_ms
        } else {
            300_000
        }))
        .resolver(FamilyResolver(key.ipresolve));
    if let Some(p) = &key.proxy {
        if let Ok(proxy) = ureq::Proxy::new(p) {
            b = b.proxy(proxy);
        }
    }
    let agent = b.build();
    map.insert(key, agent.clone());
    agent
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in input.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The request headers to send: libcurl's defaults overlaid by the user's
/// CURLOPT_HTTPHEADER lines (`Name:` removes a header, `Name;` sends it empty).
fn request_headers(spec: &Spec) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    for line in &spec.headers {
        let line = String::from_utf8_lossy(line).into_owned();
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(c) = line.find(':') {
            let name = line[..c].trim().to_string();
            let value = line[c + 1..].trim().to_string();
            if name.is_empty() {
                continue;
            }
            if value.is_empty() {
                removed.push(name.to_ascii_lowercase());
            } else {
                out.push((name, value));
            }
        } else if let Some(name) = line.strip_suffix(';') {
            let name = name.trim();
            if !name.is_empty() {
                out.push((name.to_string(), String::new()));
            }
        }
    }
    let has = |out: &Vec<(String, String)>, n: &str| {
        out.iter().any(|(k, _)| k.eq_ignore_ascii_case(n)) || removed.iter().any(|r| r == n)
    };
    let mut defaults: Vec<(String, String)> = Vec::new();
    if let Some(ua) = &spec.useragent {
        if !has(&out, "user-agent") {
            defaults.push(("User-Agent".into(), ua.clone()));
        }
    }
    if let Some(up) = &spec.userpwd {
        if !has(&out, "authorization") {
            defaults.push((
                "Authorization".into(),
                format!("Basic {}", base64(up.as_bytes())),
            ));
        }
    }
    if let Some(r) = &spec.referer {
        if !has(&out, "referer") {
            defaults.push(("Referer".into(), r.clone()));
        }
    }
    if let Some(c) = &spec.cookie {
        if !has(&out, "cookie") {
            defaults.push(("Cookie".into(), c.clone()));
        }
    }
    if let Some(r) = &spec.range {
        if !has(&out, "range") {
            defaults.push(("Range".into(), format!("bytes={r}")));
        }
    }
    match &spec.encoding {
        Some(e) if !has(&out, "accept-encoding") => {
            let v = if e.is_empty() {
                "gzip".to_string()
            } else {
                e.clone()
            };
            defaults.push(("Accept-Encoding".into(), v));
        }
        None if !has(&out, "accept-encoding") => {
            // libcurl sends no Accept-Encoding without CURLOPT_ENCODING; ask for
            // the identity coding so the body arrives exactly as libcurl's would.
            defaults.push(("Accept-Encoding".into(), "identity".into()));
        }
        _ => {}
    }
    if spec.body.is_some() || spec.post {
        if let Some(ct) = &spec.body_content_type {
            if !has(&out, "content-type") {
                defaults.push(("Content-Type".into(), ct.clone()));
            }
        }
    }
    defaults.extend(out);
    defaults
}

fn is_timeout(t: &ureq::Transport) -> bool {
    use std::error::Error;
    let mut src: Option<&(dyn Error + 'static)> = t.source();
    while let Some(e) = src {
        if let Some(io) = e.downcast_ref::<std::io::Error>() {
            if matches!(
                io.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                return true;
            }
        }
        src = e.source();
    }
    let s = t.to_string().to_ascii_lowercase();
    s.contains("timed out") || s.contains("timeout")
}

fn host_port(url: &url::Url) -> (String, u16) {
    let host = url.host_str().unwrap_or("").to_string();
    let port = url.port_or_known_default().unwrap_or(0);
    (host, port)
}

/// Map a ureq transport failure onto libcurl's (errno, message).
fn transport_error(
    t: &ureq::Transport,
    url: &url::Url,
    elapsed: Duration,
    got: usize,
) -> (i64, String) {
    let ms = elapsed.as_millis();
    let (host, port) = host_port(url);
    let host_disp = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    match t.kind() {
        ureq::ErrorKind::Dns => (6, format!("Could not resolve host: {host_disp}")),
        ureq::ErrorKind::ConnectionFailed => {
            if is_timeout(t) {
                (28, format!("Connection timed out after {ms} milliseconds"))
            } else {
                (
                    7,
                    format!(
                        "Failed to connect to {host}:{port} after {ms} ms: Could not connect to server"
                    ),
                )
            }
        }
        ureq::ErrorKind::InvalidUrl => {
            (3, "URL rejected: Malformed input to a URL function".into())
        }
        ureq::ErrorKind::UnknownScheme => {
            (1, format!("Protocol \"{}\" not supported", url.scheme()))
        }
        ureq::ErrorKind::InvalidProxyUrl => (5, "Could not resolve proxy".into()),
        ureq::ErrorKind::ProxyConnect | ureq::ErrorKind::ProxyUnauthorized => {
            (56, "CONNECT tunnel failed".into())
        }
        ureq::ErrorKind::BadStatus | ureq::ErrorKind::BadHeader => (8, "Weird server reply".into()),
        _ => {
            if is_timeout(t) {
                (
                    28,
                    format!(
                        "Operation timed out after {ms} milliseconds with {got} bytes received"
                    ),
                )
            } else {
                let msg = t.to_string();
                let lower = msg.to_ascii_lowercase();
                if lower.contains("certificate") || lower.contains("invalidcertificate") {
                    (
                        60,
                        format!("SSL certificate problem: {}", t.message().unwrap_or(&msg)),
                    )
                } else if lower.contains("tls")
                    || lower.contains("handshake")
                    || lower.contains("alert")
                {
                    (
                        35,
                        format!("TLS connect error: {}", t.message().unwrap_or(&msg)),
                    )
                } else if lower.contains("reset") {
                    (56, "Recv failure: Connection reset by peer".into())
                } else {
                    (56, format!("Recv failure: {}", t.message().unwrap_or(&msg)))
                }
            }
        }
    }
}

fn perform_http(spec: &Spec, url_s: String, mut info: Info, start: Instant) -> Outcome {
    let agent = agent_for(spec);
    let mut url_s = url_s;
    let mut method = spec.method();
    let mut body = spec.upload.clone().or_else(|| spec.body.clone());
    let mut header_blocks: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut header_size: i64 = 0;
    let mut request_size: i64 = 0;
    let mut redirects: i64 = 0;
    let mut redirect_us: i64 = 0;
    info.used_proxy = spec.proxy.as_deref().map_or(0, |p| !p.is_empty() as i64);
    let headers = request_headers(spec);
    loop {
        let parsed = match url::Url::parse(&url_s) {
            Ok(u) => u,
            Err(url::ParseError::EmptyHost) => {
                return fail(info, 3, "URL rejected: No host part in the URL", start)
            }
            Err(_) => {
                return fail(
                    info,
                    3,
                    "URL rejected: Malformed input to a URL function",
                    start,
                )
            }
        };
        if parsed.host_str().is_none_or(|h| h.is_empty()) {
            return fail(info, 3, "URL rejected: No host part in the URL", start);
        }
        let scheme = parsed.scheme().to_string();
        info.scheme = Some(scheme.clone());
        info.protocol = if scheme == "https" {
            PROTO_HTTPS
        } else {
            PROTO_HTTP
        };
        info.effective_url = url_s.clone();
        info.effective_method = method.clone();
        let hop_start = Instant::now();
        let mut req = agent.request(&method, &url_s);
        if spec.timeout_ms > 0 {
            let spent = start.elapsed();
            let total = Duration::from_millis(spec.timeout_ms);
            req = req.timeout(total.saturating_sub(spent).max(Duration::from_millis(1)));
        }
        for (k, v) in &headers {
            req = req.set(k, v);
        }
        // Approximate request size: request line + headers.
        request_size += (method.len() + parsed.path().len() + 12) as i64
            + headers
                .iter()
                .map(|(k, v)| (k.len() + v.len() + 4) as i64)
                .sum::<i64>()
            + 2;
        let result = match &body {
            Some(b) => req.send_bytes(b),
            None if method == "POST" || method == "PUT" => req.send_bytes(&[]),
            None => req.call(),
        };
        let resp = match result {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => {
                let (errno, msg) = transport_error(&t, &parsed, start.elapsed(), 0);
                info.header_size = header_size;
                info.request_size = request_size;
                info.redirect_count = redirects;
                let mut o = fail(info, errno, msg, start);
                o.header_blocks = header_blocks;
                return o;
            }
        };
        let elapsed = start.elapsed();
        info.namelookup_us = us(elapsed);
        info.connect_us = us(elapsed);
        info.pretransfer_us = us(elapsed);
        info.starttransfer_us = us(elapsed);
        info.posttransfer_us = us(elapsed);
        if scheme == "https" {
            info.appconnect_us = us(elapsed);
        }
        let remote = resp.remote_addr();
        info.primary_ip = remote.ip().to_string();
        info.primary_port = remote.port() as i64;
        let local = resp.local_addr();
        info.local_ip = local.ip().to_string();
        info.local_port = local.port() as i64;
        info.conn_id = 0;
        // libcurl negotiates HTTP/2 over TLS by default (CURL_HTTP_VERSION_2TLS)
        // and every mainstream package host speaks it; ureq is HTTP/1.1-only, so
        // present https responses the way libcurl reports an h2 exchange:
        // `HTTP/2 <code> ` status line (no reason phrase), lowercase header
        // names, no hop-by-hop headers, CURLINFO_HTTP_VERSION = 3. Forcing
        // CURL_HTTP_VERSION_1_0/1_1 keeps the HTTP/1.x presentation.
        let h2 = scheme == "https" && !matches!(spec.http_version, 1 | 2);
        info.http_version = if h2 {
            HTTP_VERSION_2
        } else if resp.http_version() == "HTTP/1.0" {
            HTTP_VERSION_1_0
        } else {
            HTTP_VERSION_1_1
        };
        let status = resp.status() as i64;
        info.http_code = status;
        // Rebuild the raw header block, preserving order and duplicates.
        let mut block: Vec<Vec<u8>> = Vec::new();
        block.push(if h2 {
            format!("HTTP/2 {} \r\n", resp.status()).into_bytes()
        } else {
            format!(
                "{} {} {}\r\n",
                resp.http_version(),
                resp.status(),
                resp.status_text()
            )
            .into_bytes()
        });
        let names = resp.headers_names();
        let mut seen: HashMap<String, usize> = HashMap::new();
        for n in &names {
            let k = seen.entry(n.clone()).or_insert(0);
            if let Some(v) = resp.all(n).get(*k) {
                if h2 {
                    if !H1_ONLY_HEADERS.contains(&n.as_str()) {
                        block.push(format!("{n}: {v}\r\n").into_bytes());
                    }
                } else {
                    block.push(format!("{}: {}\r\n", display_name(n), v).into_bytes());
                }
            }
            *k += 1;
        }
        block.push(b"\r\n".to_vec());
        header_size += block.iter().map(|l| l.len() as i64).sum::<i64>();
        header_blocks.push(block);
        info.header_size = header_size;
        info.request_size = request_size;
        info.content_type = resp.header("content-type").map(|s| s.to_string());
        info.content_length_download = resp
            .header("content-length")
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(-1.0);
        info.size_upload = body.as_ref().map_or(0, |b| b.len() as i64);
        info.content_length_upload = if body.is_some() {
            info.size_upload as f64
        } else {
            -1.0
        };
        let location = resp.header("location").map(|s| s.to_string());
        let next = location
            .as_deref()
            .and_then(|l| parsed.join(l).ok())
            .map(|u| u.to_string());
        let is_redirect = (300..400).contains(&status) && next.is_some();
        info.redirect_url = if is_redirect { next.clone() } else { None };

        if spec.failonerror && status >= 400 {
            info.redirect_count = redirects;
            info.total_us = us(start.elapsed());
            return Outcome {
                header_blocks,
                body: Vec::new(),
                errno: 22,
                error: format!("The requested URL returned error: {status}"),
                info,
            };
        }

        let follow = spec.follow && is_redirect;
        let mut data = Vec::new();
        if !(method == "HEAD" || spec.nobody) || follow {
            let mut reader = resp.into_reader();
            if let Err(e) = (&mut reader).take(1 << 34).read_to_end(&mut data) {
                info.size_download = data.len() as i64;
                info.redirect_count = redirects;
                let timed_out = matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                );
                let ms = start.elapsed().as_millis();
                let (errno, msg) = if timed_out {
                    (
                        28,
                        format!(
                            "Operation timed out after {ms} milliseconds with {} bytes received",
                            data.len()
                        ),
                    )
                } else {
                    (56, format!("Recv failure: {e}"))
                };
                let mut o = fail(info, errno, msg, start);
                o.header_blocks = header_blocks;
                o.body = data;
                return o;
            }
        }

        if follow {
            if redirects >= spec.max_redirs && spec.max_redirs >= 0 {
                info.redirect_count = redirects;
                let mut o = fail(
                    info,
                    47,
                    format!("Maximum ({}) redirects followed", spec.max_redirs),
                    start,
                );
                o.header_blocks = header_blocks;
                return o;
            }
            let next = next.unwrap_or_default();
            let next_scheme = scheme_of(&next).unwrap_or_default();
            let bit = KNOWN_PROTOCOLS
                .iter()
                .find(|(n, _)| *n == next_scheme)
                .map_or(0, |(_, b)| *b);
            if spec.redir_protocols & bit == 0 || (bit != PROTO_HTTP && bit != PROTO_HTTPS) {
                info.redirect_count = redirects;
                let mut o = fail(
                    info,
                    1,
                    format!("Protocol \"{next_scheme}\" not supported or disabled in libcurl"),
                    start,
                );
                o.header_blocks = header_blocks;
                return o;
            }
            redirects += 1;
            redirect_us += us(hop_start.elapsed());
            // 301/302/303 turn a POST into a GET (libcurl's default POSTREDIR).
            if (status == 303 && method != "HEAD")
                || ((status == 301 || status == 302) && method == "POST")
            {
                if spec.custom_request.is_none() {
                    method = "GET".into();
                }
                body = None;
            }
            url_s = next;
            continue;
        }

        info.redirect_count = redirects;
        info.redirect_us = redirect_us;
        info.size_download = data.len() as i64;
        info.total_us = us(start.elapsed());
        return Outcome {
            header_blocks,
            body: data,
            errno: 0,
            error: String::new(),
            info,
        };
    }
}

/// ureq lowercases header names; restore the conventional `Title-Case` that
/// HTTP/1.1 servers send (header names are case-insensitive either way).
fn display_name(n: &str) -> String {
    let mut out = String::with_capacity(n.len());
    let mut up = true;
    for c in n.chars() {
        if up {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        up = c == '-';
    }
    out
}
