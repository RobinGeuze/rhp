//! The transfer engine behind `ext/curl`: a `Send` request description
//! ([`Spec`]) is performed on any thread ([`perform`]) into an [`Outcome`] —
//! the received header blocks, the body, libcurl-style error code/message and
//! the `curl_getinfo()` numbers. Delivery of that outcome to PHP (streams,
//! callbacks, return buffers) happens on the VM thread in `mod.rs`.
//!
//! HTTP(S) is a small HTTP/1.1 client of our own over rustls (bundled Mozilla
//! roots), with redirects followed here so every hop's headers reach the
//! header handler and `redirect_count` is exact, the way libcurl reports them.
//! `file://` is served directly, mirroring libcurl's FILE protocol (including
//! its synthesized `Content-Length` / `Accept-ranges` / `Last-Modified` header
//! lines).

use std::collections::HashMap;
use std::io::Read;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex, OnceLock};
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
    /// CURLOPT_SSL_VERIFYPEER (default on).
    pub verify_peer: bool,
    /// CURLOPT_UNRESTRICTED_AUTH: keep credentials/cookies on redirects to
    /// another host.
    pub unrestricted_auth: bool,
    /// CURLOPT_AUTOREFERER.
    pub autoreferer: bool,
}

/// The transfer numbers `curl_getinfo()` reports.
#[derive(Clone, Debug)]
pub struct Info {
    pub effective_url: String,
    /// libcurl fails CURLINFO_EFFECTIVE_URL (an unparsable URL, before a transfer).
    pub no_effective_url: bool,
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
            no_effective_url: false,
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

// ---------------------------------------------------------------------------
// HTTP/1.1 transport
//
// A small client of our own instead of ureq, so the exchange is libcurl's:
// the exact request header set and order (no injected User-Agent or
// Accept-Encoding, `Name:` removes a default), the response header lines
// passed on byte-for-byte as the server sent them, and no content decoding
// unless CURLOPT_ENCODING asked for it.
// ---------------------------------------------------------------------------

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use ureq::rustls;

/// libcurl's ALL_CONTENT_ENCODINGS: what CURLOPT_ENCODING "" advertises.
pub const ALL_ENCODINGS: &str = "deflate, gzip, br, zstd";

/// libcurl's DEFAULT_CONNECT_TIMEOUT.
const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 300_000;

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(s) => s,
            Stream::Tls(t) => &t.sock,
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(t) => match t.read(buf) {
                // A peer that closes without close_notify: treat as EOF, as
                // libcurl's OpenSSL backend does for HTTP framing purposes.
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(0),
                r => r,
            },
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(t) => t.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(t) => t.flush(),
        }
    }
}

type Conn = BufReader<Stream>;

#[derive(Clone, PartialEq, Eq, Hash)]
struct TlsKey {
    /// CURLOPT_CAINFO in effect (None = bundled Mozilla roots).
    cainfo: Option<String>,
    /// CURLOPT_SSL_VERIFYPEER = 0.
    insecure: bool,
}

/// Accepts any server certificate (CURLOPT_SSL_VERIFYPEER = 0).
#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The TLS configuration for `key` (cached), or libcurl's (errno, message)
/// when the CA bundle cannot be used.
fn tls_config(key: &TlsKey) -> Result<Arc<rustls::ClientConfig>, (i64, String)> {
    static CACHE: OnceLock<Mutex<HashMap<TlsKey, Arc<rustls::ClientConfig>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(c) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
        return Ok(c.clone());
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
        .map_err(|e| (35, e.to_string()))?;
    let config = if key.insecure {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
            .with_no_client_auth()
    } else if let Some(path) = &key.cainfo {
        use rustls::pki_types::pem::PemObject;
        let bad = || (77, format!("error setting certificate file: {path}"));
        let mut roots = rustls::RootCertStore::empty();
        let iter = rustls::pki_types::CertificateDer::pem_file_iter(path).map_err(|_| bad())?;
        for cert in iter.flatten() {
            let _ = roots.add(cert);
        }
        if roots.is_empty() {
            return Err(bad());
        }
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    let config = Arc::new(config);
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.clone(), config.clone());
    Ok(config)
}

/// Where a pooled connection leads.
#[derive(Clone, PartialEq, Eq, Hash)]
struct PoolKey {
    scheme: String,
    host: String,
    port: u16,
    proxy: Option<(String, u16)>,
    tls: TlsKey,
    ipresolve: i64,
}

/// Idle keep-alive connections, shared by every transfer (and every multi
/// worker thread) like libcurl's connection cache.
fn pool() -> &'static Mutex<HashMap<PoolKey, Vec<Conn>>> {
    static POOL: OnceLock<Mutex<HashMap<PoolKey, Vec<Conn>>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Is an idle connection still usable (open, nothing unexpected pending)?
fn alive(c: &Conn) -> bool {
    if !c.buffer().is_empty() {
        return false;
    }
    let tcp = c.get_ref().tcp();
    if tcp.set_nonblocking(true).is_err() {
        return false;
    }
    let mut b = [0u8; 1];
    let r = tcp.peek(&mut b);
    let _ = tcp.set_nonblocking(false);
    matches!(r, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
}

fn checkout(key: &PoolKey) -> Option<Conn> {
    let mut p = pool().lock().unwrap_or_else(|e| e.into_inner());
    let list = p.get_mut(key)?;
    while let Some(c) = list.pop() {
        if alive(&c) {
            return Some(c);
        }
    }
    None
}

fn checkin(key: PoolKey, c: Conn) {
    let mut p = pool().lock().unwrap_or_else(|e| e.into_inner());
    let list = p.entry(key).or_default();
    if list.len() < 8 {
        list.push(c);
    }
}

/// CURLOPT_PROXY, parsed.
#[derive(Clone)]
struct Proxy {
    host: String,
    port: u16,
    /// `Proxy-Authorization` credentials (`user:password`).
    auth: Option<String>,
}

fn parse_proxy(p: &str) -> Result<Option<Proxy>, (i64, String)> {
    if p.is_empty() {
        return Ok(None);
    }
    let full = if scheme_of(p).is_some() {
        p.to_string()
    } else {
        format!("http://{p}")
    };
    let u = url::Url::parse(&full).map_err(|_| (5, format!("Could not resolve proxy: {p}")))?;
    if !matches!(u.scheme(), "http") {
        return Err((4, format!("Unsupported proxy scheme for '{}'", u.scheme())));
    }
    let host = u
        .host_str()
        .unwrap_or("")
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    if host.is_empty() {
        return Err((5, format!("Could not resolve proxy: {p}")));
    }
    let auth = if u.username().is_empty() && u.password().is_none() {
        None
    } else {
        let user = String::from_utf8_lossy(&percent_decode(u.username())).into_owned();
        let pass =
            String::from_utf8_lossy(&percent_decode(u.password().unwrap_or(""))).into_owned();
        Some(format!("{user}:{pass}"))
    };
    // libcurl's default proxy port.
    Ok(Some(Proxy {
        host,
        port: u.port().unwrap_or(1080),
        auth,
    }))
}

/// Resolve and connect, honouring CURLOPT_IPRESOLVE and the connect timeout.
fn tcp_connect(
    host: &str,
    port: u16,
    ipresolve: i64,
    timeout: Duration,
    start: Instant,
    is_proxy: bool,
) -> Result<TcpStream, (i64, String)> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map(|a| {
            a.filter(|a| match ipresolve {
                1 => a.is_ipv4(),
                2 => a.is_ipv6(),
                _ => true,
            })
            .collect()
        })
        .unwrap_or_default();
    if addrs.is_empty() {
        return Err(if is_proxy {
            (5, format!("Could not resolve proxy: {host}"))
        } else {
            (6, format!("Could not resolve host: {host}"))
        });
    }
    let mut timed_out = false;
    for a in addrs {
        let left = timeout.saturating_sub(start.elapsed());
        if left.is_zero() {
            timed_out = true;
            break;
        }
        match TcpStream::connect_timeout(&a, left) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => {
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) {
                    timed_out = true;
                }
            }
        }
    }
    let ms = start.elapsed().as_millis();
    Err(if timed_out {
        (28, format!("Connection timed out after {ms} milliseconds"))
    } else {
        (
            7,
            format!(
                "Failed to connect to {host}:{port} after {ms} ms: Could not connect to server"
            ),
        )
    })
}

/// A TLS failure, as libcurl (OpenSSL backend) reports it: certificate
/// problems are CURLE_PEER_FAILED_VERIFICATION (60), other handshake failures
/// CURLE_SSL_CONNECT_ERROR (35).
fn tls_error(e: &std::io::Error, host: &str) -> (i64, String) {
    use rustls::{CertificateError, Error as TlsError};
    if matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        return (28, "SSL connection timeout".into());
    }
    let verify = |msg: &str, code: u32| {
        (
            60,
            format!("SSL certificate OpenSSL verify result: {msg} ({code})"),
        )
    };
    match e.get_ref().and_then(|i| i.downcast_ref::<TlsError>()) {
        Some(TlsError::InvalidCertificate(c)) => match c {
            CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                verify("certificate has expired", 10)
            }
            CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                verify("certificate is not yet valid", 9)
            }
            CertificateError::UnknownIssuer => verify("unable to get local issuer certificate", 20),
            CertificateError::Revoked => verify("certificate revoked", 23),
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
                (
                    60,
                    format!(
                    "SSL: no alternative certificate subject name matches target hostname '{host}'"
                ),
                )
            }
            other => (60, format!("SSL certificate problem: {other:?}")),
        },
        Some(other) => (35, format!("TLS connect error: {other}")),
        None => (35, format!("TLS connect error: {e}")),
    }
}

fn tls_handshake(
    mut tcp: TcpStream,
    host: &str,
    cfg: Arc<rustls::ClientConfig>,
) -> Result<Stream, (i64, String)> {
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| (35, format!("TLS connect error: {e}")))?;
    let mut conn = rustls::ClientConnection::new(cfg, name)
        .map_err(|e| (35, format!("TLS connect error: {e}")))?;
    while conn.is_handshaking() {
        if let Err(e) = conn.complete_io(&mut tcp) {
            return Err(tls_error(&e, host));
        }
    }
    Ok(Stream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))))
}

/// Read one header block (status line .. blank line), raw. `Ok(None)` = the
/// peer closed before sending anything.
fn read_head(conn: &mut Conn) -> std::io::Result<Option<Vec<Vec<u8>>>> {
    let mut lines: Vec<Vec<u8>> = Vec::new();
    let mut total = 0usize;
    loop {
        let mut line = Vec::new();
        let n = conn.read_until(b'\n', &mut line)?;
        if n == 0 {
            if lines.is_empty() {
                return Ok(None);
            }
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        total += n;
        if total > 300 * 1024 {
            return Err(std::io::Error::other("header too large"));
        }
        let blank = line == b"\r\n" || line == b"\n";
        lines.push(line);
        if blank {
            return Ok(Some(lines));
        }
    }
}

/// `HTTP/1.1 200 OK` → ("HTTP/1.1", 200).
fn status_of(line: &[u8]) -> Option<(String, i64)> {
    let s = String::from_utf8_lossy(line);
    let s = s.trim_end();
    let mut it = s.splitn(3, ' ');
    let ver = it.next()?;
    if !ver.starts_with("HTTP/") {
        return None;
    }
    let code: i64 = it.next()?.trim().parse().ok()?;
    if !(100..1000).contains(&code) {
        return None;
    }
    Some((ver.to_string(), code))
}

/// A header line's (name, trimmed value).
fn split_header(line: &[u8]) -> Option<(String, String)> {
    let c = line.iter().position(|&b| b == b':')?;
    let name = String::from_utf8_lossy(&line[..c]).trim().to_string();
    let value = String::from_utf8_lossy(&line[c + 1..]).trim().to_string();
    Some((name, value))
}

/// The first value of header `name` in a raw block.
fn header_value(block: &[Vec<u8>], name: &str) -> Option<String> {
    block
        .iter()
        .skip(1)
        .filter_map(|l| split_header(l))
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

enum Framing {
    Empty,
    Length(u64),
    Chunked,
    Eof,
}

/// Read a response body per its framing. Returns whether the connection is
/// left at a message boundary (reusable).
fn read_body(conn: &mut Conn, framing: &Framing, out: &mut Vec<u8>) -> std::io::Result<bool> {
    match framing {
        Framing::Empty => Ok(true),
        Framing::Length(n) => {
            let got = conn.take(*n).read_to_end(out)?;
            if (got as u64) < *n {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            Ok(true)
        }
        Framing::Eof => {
            conn.read_to_end(out)?;
            Ok(false)
        }
        Framing::Chunked => loop {
            let mut line = Vec::new();
            if conn.read_until(b'\n', &mut line)? == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            let text = String::from_utf8_lossy(&line);
            let size_s = text.split(';').next().unwrap_or("").trim();
            let size = u64::from_str_radix(size_s, 16)
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "chunk"))?;
            if size == 0 {
                // trailers up to the blank line
                loop {
                    let mut t = Vec::new();
                    if conn.read_until(b'\n', &mut t)? == 0 || t == b"\r\n" || t == b"\n" {
                        return Ok(true);
                    }
                }
            }
            let got = (&mut *conn).take(size).read_to_end(out)?;
            if (got as u64) < size {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            let mut crlf = Vec::new();
            conn.read_until(b'\n', &mut crlf)?;
        },
    }
}

/// Undo `Content-Encoding` (only when CURLOPT_ENCODING is set), like
/// libcurl's content_encoding.c. Codings are listed in the order applied.
fn decode_body(body: Vec<u8>, codings: &str) -> Result<Vec<u8>, (i64, String)> {
    let mut data = body;
    for c in codings.split(',').map(|s| s.trim()).rev() {
        let lc = c.to_ascii_lowercase();
        let mut out = Vec::new();
        let r = match lc.as_str() {
            "" | "identity" | "none" => continue,
            "gzip" | "x-gzip" => flate2::read::MultiGzDecoder::new(&data[..])
                .read_to_end(&mut out)
                .map(|_| ()),
            "deflate" => flate2::read::ZlibDecoder::new(&data[..])
                .read_to_end(&mut out)
                .map(|_| ())
                .or_else(|_| {
                    out.clear();
                    flate2::read::DeflateDecoder::new(&data[..])
                        .read_to_end(&mut out)
                        .map(|_| ())
                }),
            "br" => brotli_decompressor::Decompressor::new(&data[..], 4096)
                .read_to_end(&mut out)
                .map(|_| ()),
            "zstd" => {
                let mut src = &data[..];
                let mut r = Ok(());
                while !src.is_empty() {
                    match ruzstd::decoding::StreamingDecoder::new(&mut src) {
                        Ok(mut d) => {
                            if let Err(e) = d.read_to_end(&mut out) {
                                r = Err(e);
                                break;
                            }
                        }
                        Err(e) => {
                            r = Err(std::io::Error::other(e.to_string()));
                            break;
                        }
                    }
                }
                r
            }
            _ => {
                return Err((
                    61,
                    "Unrecognized content encoding type. libcurl understands deflate, gzip, br, zstd content encodings.".into(),
                ))
            }
        };
        if let Err(e) = r {
            return Err((
                61,
                format!("Error while processing content unencoding: {e}"),
            ));
        }
        data = out;
    }
    Ok(data)
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

/// One CURLOPT_HTTPHEADER entry, classified like Curl_add_custom_headers.
struct Custom {
    /// The header name as written.
    name: String,
    /// The line to send (`Name;` becomes `Name:`), or `None` when the entry
    /// only suppresses a default (`Name:` with a blank value) or is not a
    /// header at all.
    line: Option<Vec<u8>>,
}

fn custom_headers(spec: &Spec) -> Vec<Custom> {
    let mut out = Vec::new();
    for raw in &spec.headers {
        let mut line = raw.clone();
        while matches!(line.last(), Some(b'\r' | b'\n')) {
            line.pop();
        }
        if let Some(c) = line.iter().position(|&b| b == b':') {
            if c == 0 {
                continue;
            }
            let blank = line[c + 1..].iter().all(|b| b.is_ascii_whitespace());
            out.push(Custom {
                name: String::from_utf8_lossy(&line[..c]).into_owned(),
                line: if blank { None } else { Some(line) },
            });
        } else if let Some(sc) = line.iter().position(|&b| b == b';') {
            if sc == 0 || !line[sc + 1..].iter().all(|b| b.is_ascii_whitespace()) {
                continue;
            }
            let name = String::from_utf8_lossy(&line[..sc]).into_owned();
            // `Name;` (nothing after the semicolon) sends an empty `Name:`.
            let send = if line.len() == sc + 1 {
                let mut l = line[..sc].to_vec();
                l.push(b':');
                Some(l)
            } else {
                None
            };
            out.push(Custom { name, line: send });
        }
    }
    out
}

/// What varies per request hop.
struct Hop<'a> {
    method: &'a str,
    /// Request target: origin-form, or the absolute URL through a proxy.
    target: String,
    /// `host[:port]` for the Host header.
    host_hdr: String,
    /// Credentials/cookies may go to this host (first host, or
    /// CURLOPT_UNRESTRICTED_AUTH).
    allowed: bool,
    /// A custom Host header applies (not a redirect to another host).
    custom_host_ok: bool,
    referer: Option<&'a str>,
    http_proxy: Option<&'a Proxy>,
    body: Option<&'a [u8]>,
    body_content_type: Option<&'a str>,
}

/// Build the request head the way libcurl's Curl_http() lays it out.
fn build_request(spec: &Spec, hop: &Hop, customs: &[Custom]) -> Vec<u8> {
    let has = |n: &str| customs.iter().any(|c| c.name.eq_ignore_ascii_case(n));
    let mut r: Vec<u8> = Vec::new();
    let ver = if spec.http_version == 1 { "1.0" } else { "1.1" };
    r.extend_from_slice(format!("{} {} HTTP/{ver}\r\n", hop.method, hop.target).as_bytes());
    let custom_host = customs.iter().find(|c| c.name.eq_ignore_ascii_case("host"));
    match custom_host {
        Some(c) if hop.custom_host_ok => {
            // `Host:` alone suppresses the header; otherwise "Host:" + the
            // rest of the user's line.
            if let Some(l) = &c.line {
                r.extend_from_slice(b"Host:");
                r.extend_from_slice(&l[c.name.len() + 1..]);
                r.extend_from_slice(b"\r\n");
            }
        }
        _ => r.extend_from_slice(format!("Host: {}\r\n", hop.host_hdr).as_bytes()),
    }
    if let Some(p) = hop.http_proxy {
        if let Some(a) = &p.auth {
            if !has("proxy-authorization") {
                r.extend_from_slice(
                    format!("Proxy-Authorization: Basic {}\r\n", base64(a.as_bytes())).as_bytes(),
                );
            }
        }
    }
    if let Some(up) = &spec.userpwd {
        if hop.allowed && !has("authorization") {
            r.extend_from_slice(
                format!("Authorization: Basic {}\r\n", base64(up.as_bytes())).as_bytes(),
            );
        }
    }
    if let Some(range) = &spec.range {
        if !range.is_empty() && !has("range") {
            r.extend_from_slice(format!("Range: bytes={range}\r\n").as_bytes());
        }
    }
    if let Some(ua) = &spec.useragent {
        if !ua.is_empty() && !has("user-agent") {
            r.extend_from_slice(format!("User-Agent: {ua}\r\n").as_bytes());
        }
    }
    if !has("accept") {
        r.extend_from_slice(b"Accept: */*\r\n");
    }
    if let Some(enc) = &spec.encoding {
        let v = if enc.is_empty() { ALL_ENCODINGS } else { enc };
        if !has("accept-encoding") {
            r.extend_from_slice(format!("Accept-Encoding: {v}\r\n").as_bytes());
        }
    }
    if let Some(rf) = hop.referer {
        if !has("referer") {
            r.extend_from_slice(format!("Referer: {rf}\r\n").as_bytes());
        }
    }
    if hop.http_proxy.is_some() && !has("proxy-connection") {
        r.extend_from_slice(b"Proxy-Connection: Keep-Alive\r\n");
    }
    if let Some(c) = &spec.cookie {
        if hop.allowed && !has("cookie") {
            r.extend_from_slice(format!("Cookie: {c}\r\n").as_bytes());
        }
    }
    let multipart = hop
        .body_content_type
        .is_some_and(|ct| ct.starts_with("multipart/"));
    for c in customs {
        let Some(line) = &c.line else { continue };
        let n = c.name.to_ascii_lowercase();
        if n == "host"
            || (!hop.allowed && (n == "authorization" || n == "cookie"))
            || (multipart && (n == "content-type" || n == "content-length"))
        {
            continue;
        }
        r.extend_from_slice(line);
        r.extend_from_slice(b"\r\n");
    }
    if let Some(b) = hop.body {
        if !has("content-length") || multipart {
            r.extend_from_slice(format!("Content-Length: {}\r\n", b.len()).as_bytes());
        }
        if let Some(ct) = hop.body_content_type {
            if multipart || !has("content-type") {
                r.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
            }
        }
    }
    r.extend_from_slice(b"\r\n");
    r
}

/// Present an HTTP/1.x header block the way libcurl reports an h2 exchange:
/// `HTTP/2 <code> ` status line, lowercase names, no hop-by-hop headers.
fn as_h2(block: &[Vec<u8>], code: i64) -> Vec<Vec<u8>> {
    let mut out = vec![format!("HTTP/2 {code} \r\n").into_bytes()];
    for l in block.iter().skip(1) {
        if let Some((n, v)) = split_header(l) {
            let n = n.to_ascii_lowercase();
            if !H1_ONLY_HEADERS.contains(&n.as_str()) {
                out.push(format!("{n}: {v}\r\n").into_bytes());
            }
        }
    }
    out.push(b"\r\n".to_vec());
    out
}

fn io_error_kind_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

fn recv_error(e: &std::io::Error) -> (i64, String) {
    if e.kind() == std::io::ErrorKind::ConnectionReset {
        (56, "Recv failure: Connection reset by peer".into())
    } else {
        (56, format!("Recv failure: {e}"))
    }
}

/// Open (or reuse) a connection for `key`. Returns the connection and
/// whether it came from the pool. CONNECT response headers through a proxy
/// are appended to `blocks`.
fn open_conn(
    spec: &Spec,
    key: &PoolKey,
    proxy: Option<&Proxy>,
    tls: Option<Arc<rustls::ClientConfig>>,
    start: Instant,
    allow_reuse: bool,
    blocks: &mut Vec<Vec<Vec<u8>>>,
) -> Result<(Conn, bool), (i64, String)> {
    if allow_reuse {
        if let Some(c) = checkout(key) {
            return Ok((c, true));
        }
    }
    let mut ct = Duration::from_millis(if spec.connect_timeout_ms > 0 {
        spec.connect_timeout_ms
    } else {
        DEFAULT_CONNECT_TIMEOUT_MS
    });
    if spec.timeout_ms > 0 {
        ct = ct.min(Duration::from_millis(spec.timeout_ms));
    }
    let (chost, cport) = match proxy {
        Some(p) => (p.host.as_str(), p.port),
        None => (key.host.as_str(), key.port),
    };
    let tcp = tcp_connect(chost, cport, key.ipresolve, ct, start, proxy.is_some())?;
    set_timeouts(&tcp, spec, start);
    let tcp = match (proxy, &tls) {
        (Some(p), Some(_)) => {
            // Tunnel through the proxy with CONNECT.
            let hp = host_hdr(&key.host, key.port);
            let mut req = format!("CONNECT {hp} HTTP/1.1\r\nHost: {hp}\r\n");
            if let Some(a) = &p.auth {
                req.push_str(&format!(
                    "Proxy-Authorization: Basic {}\r\n",
                    base64(a.as_bytes())
                ));
            }
            if let Some(ua) = spec.useragent.as_deref().filter(|u| !u.is_empty()) {
                req.push_str(&format!("User-Agent: {ua}\r\n"));
            }
            req.push_str("Proxy-Connection: Keep-Alive\r\n\r\n");
            let mut c = BufReader::new(Stream::Plain(tcp));
            c.get_mut()
                .write_all(req.as_bytes())
                .map_err(|e| (55, format!("Send failure: {e}")))?;
            let head = match read_head(&mut c) {
                Ok(Some(h)) => h,
                Ok(None) => return Err((56, "Proxy CONNECT aborted".into())),
                Err(e) => return Err(recv_error(&e)),
            };
            let code = status_of(&head[0]).map_or(0, |(_, c)| c);
            blocks.push(head);
            if code / 100 != 2 {
                return Err((56, format!("CONNECT tunnel failed, response {code}")));
            }
            match c.into_inner() {
                Stream::Plain(t) => t,
                Stream::Tls(_) => unreachable!(),
            }
        }
        _ => tcp,
    };
    let stream = match tls {
        Some(cfg) => tls_handshake(tcp, &key.host, cfg)?,
        None => Stream::Plain(tcp),
    };
    Ok((BufReader::new(stream), false))
}

/// Read/write timeouts from what is left of CURLOPT_TIMEOUT.
fn set_timeouts(tcp: &TcpStream, spec: &Spec, start: Instant) {
    let t = if spec.timeout_ms > 0 {
        let left = Duration::from_millis(spec.timeout_ms).saturating_sub(start.elapsed());
        Some(left.max(Duration::from_millis(1)))
    } else {
        None
    };
    let _ = tcp.set_read_timeout(t);
    let _ = tcp.set_write_timeout(t);
}

/// `host[:port]` with the scheme's default port left out, IPv6 bracketed.
fn host_hdr(host: &str, port: u16) -> String {
    let h = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    format!("{h}:{port}")
}

fn perform_http(spec: &Spec, url_s: String, mut info: Info, start: Instant) -> Outcome {
    let tls_key = TlsKey {
        cainfo: spec.cainfo.clone().filter(|c| !c.is_empty()),
        insecure: !spec.verify_peer,
    };
    let proxy = match spec.proxy.as_deref().map(parse_proxy).transpose() {
        Ok(p) => p.flatten(),
        Err((errno, msg)) => {
            info.scheme = scheme_of(&url_s);
            return fail(info, errno, msg, start);
        }
    };
    info.used_proxy = proxy.is_some() as i64;
    let customs = custom_headers(spec);
    let mut url_s = url_s;
    let mut method = spec.method();
    let mut body = spec.upload.clone().or_else(|| spec.body.clone());
    let mut body_ct = if spec.upload.is_some() {
        None
    } else {
        spec.body_content_type.clone()
    };
    let mut referer = spec.referer.clone();
    let mut header_blocks: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut header_size: i64 = 0;
    let mut request_size: i64 = 0;
    let mut redirects: i64 = 0;
    let mut redirect_us: i64 = 0;
    let mut first: Option<(String, u16, String)> = None;
    macro_rules! bail {
        ($errno:expr, $msg:expr) => {{
            info.header_size = header_size;
            info.request_size = request_size;
            info.redirect_count = redirects;
            let mut o = fail(info, $errno, $msg, start);
            o.header_blocks = header_blocks;
            return o;
        }};
    }
    loop {
        let parsed = match url::Url::parse(&url_s) {
            Ok(u) => u,
            Err(url::ParseError::EmptyHost) => bail!(3, "URL rejected: No host part in the URL"),
            Err(_) => bail!(3, "URL rejected: Malformed input to a URL function"),
        };
        if parsed.host_str().is_none_or(|h| h.is_empty()) {
            bail!(3, "URL rejected: No host part in the URL");
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
        let host = parsed
            .host_str()
            .unwrap_or("")
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let port = parsed.port_or_known_default().unwrap_or(80);
        let this = (host.to_ascii_lowercase(), port, scheme.clone());
        let first_ref = first.get_or_insert_with(|| this.clone());
        let allowed = spec.unrestricted_auth || *first_ref == this;
        let custom_host_ok = first_ref.0 == this.0;
        let tls = if scheme == "https" {
            match tls_config(&tls_key) {
                Ok(c) => Some(c),
                Err((e, m)) => bail!(e, m),
            }
        } else {
            None
        };
        let host_header = if parsed.port().is_some() {
            host_hdr(&host, port)
        } else if host.contains(':') {
            format!("[{host}]")
        } else {
            host.clone()
        };
        let http_proxy = proxy.as_ref().filter(|_| tls.is_none());
        let target = if http_proxy.is_some() {
            let mut u = parsed.clone();
            u.set_fragment(None);
            u.to_string()
        } else {
            let mut t = parsed.path().to_string();
            if let Some(q) = parsed.query() {
                t.push('?');
                t.push_str(q);
            }
            t
        };
        let send_body = body.as_deref();
        let req = build_request(
            spec,
            &Hop {
                method: &method,
                target,
                host_hdr: host_header,
                allowed,
                custom_host_ok,
                referer: referer.as_deref(),
                http_proxy,
                body: send_body,
                body_content_type: if send_body.is_some() {
                    body_ct.as_deref()
                } else {
                    None
                },
            },
            &customs,
        );
        let key = PoolKey {
            scheme: scheme.clone(),
            host: host.clone(),
            port,
            proxy: proxy.as_ref().map(|p| (p.host.clone(), p.port)),
            tls: tls_key.clone(),
            ipresolve: spec.ipresolve,
        };
        let hop_start = Instant::now();
        // Send; a pooled connection the server has meanwhile dropped is
        // retried once on a fresh one.
        let mut allow_reuse = true;
        let (mut conn, mut head) = loop {
            let (mut conn, reused) = match open_conn(
                spec,
                &key,
                proxy.as_ref(),
                tls.clone(),
                start,
                allow_reuse,
                &mut header_blocks,
            ) {
                Ok(c) => c,
                Err((e, m)) => bail!(e, m),
            };
            set_timeouts(conn.get_ref().tcp(), spec, start);
            let sent = conn.get_mut().write_all(&req).and_then(|_| {
                if let Some(b) = send_body {
                    conn.get_mut().write_all(b)?;
                }
                conn.get_mut().flush()
            });
            if let Err(e) = sent {
                if reused {
                    allow_reuse = false;
                    continue;
                }
                if io_error_kind_timeout(&e) {
                    let ms = start.elapsed().as_millis();
                    bail!(
                        28,
                        format!(
                            "Operation timed out after {ms} milliseconds with 0 bytes received"
                        )
                    );
                }
                bail!(55, format!("Send failure: {e}"));
            }
            match read_head(&mut conn) {
                Ok(Some(h)) => break (conn, h),
                Ok(None) if reused => allow_reuse = false,
                Err(_) if reused => allow_reuse = false,
                Ok(None) => bail!(52, "Empty reply from server"),
                Err(e) if io_error_kind_timeout(&e) => {
                    let ms = start.elapsed().as_millis();
                    bail!(
                        28,
                        format!(
                            "Operation timed out after {ms} milliseconds with 0 bytes received"
                        )
                    );
                }
                Err(e) => bail!(recv_error(&e).0, recv_error(&e).1),
            }
        };
        request_size += (req.len() + send_body.map_or(0, |b| b.len())) as i64;
        let h2 = scheme == "https" && !matches!(spec.http_version, 1 | 2);
        // Informational responses (100 Continue, ...) reach the header
        // handler too, then the real response follows.
        let (version, status) = loop {
            let Some((ver, code)) = status_of(&head[0]) else {
                bail!(1, "Received HTTP/0.9 when not allowed");
            };
            if (100..200).contains(&code) && code != 101 {
                let block = if h2 { as_h2(&head, code) } else { head };
                header_size += block.iter().map(|l| l.len() as i64).sum::<i64>();
                header_blocks.push(block);
                head = match read_head(&mut conn) {
                    Ok(Some(h)) => h,
                    Ok(None) => bail!(52, "Empty reply from server"),
                    Err(e) => bail!(recv_error(&e).0, recv_error(&e).1),
                };
                continue;
            }
            break (ver, code);
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
        let tcp = conn.get_ref().tcp();
        if let Ok(remote) = tcp.peer_addr() {
            info.primary_ip = remote.ip().to_string();
            info.primary_port = remote.port() as i64;
        }
        if let Ok(local) = tcp.local_addr() {
            info.local_ip = local.ip().to_string();
            info.local_port = local.port() as i64;
        }
        info.conn_id = 0;
        info.http_version = if h2 {
            HTTP_VERSION_2
        } else if version == "HTTP/1.0" {
            HTTP_VERSION_1_0
        } else {
            HTTP_VERSION_1_1
        };
        info.http_code = status;
        let content_type = header_value(&head, "content-type");
        let content_length = header_value(&head, "content-length");
        let chunked = header_value(&head, "transfer-encoding")
            .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
        let content_encoding = header_value(&head, "content-encoding");
        let location = header_value(&head, "location");
        let conn_close = header_value(&head, "connection")
            .is_some_and(|v| v.to_ascii_lowercase().contains("close"))
            || (version == "HTTP/1.0"
                && !header_value(&head, "connection")
                    .is_some_and(|v| v.to_ascii_lowercase().contains("keep-alive")));
        let block = if h2 { as_h2(&head, status) } else { head };
        header_size += block.iter().map(|l| l.len() as i64).sum::<i64>();
        header_blocks.push(block);
        info.header_size = header_size;
        info.request_size = request_size;
        info.content_type = content_type;
        info.content_length_download = content_length
            .as_deref()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(-1.0);
        info.size_upload = send_body.map_or(0, |b| b.len() as i64);
        info.content_length_upload = if send_body.is_some() {
            info.size_upload as f64
        } else {
            -1.0
        };
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

        let framing = if method == "HEAD" || status == 204 || status == 304 {
            Framing::Empty
        } else if chunked {
            Framing::Chunked
        } else if let Some(n) = content_length
            .as_deref()
            .and_then(|v| v.parse::<u64>().ok())
        {
            Framing::Length(n)
        } else {
            Framing::Eof
        };
        let follow = spec.follow && is_redirect;
        let mut data = Vec::new();
        match read_body(&mut conn, &framing, &mut data) {
            Ok(reusable) => {
                if reusable && !conn_close {
                    checkin(key, conn);
                }
            }
            Err(e) => {
                info.size_download = data.len() as i64;
                let ms = start.elapsed().as_millis();
                let (errno, msg) = if io_error_kind_timeout(&e) {
                    match framing {
                        Framing::Length(n) => (
                            28,
                            format!(
                                "Operation timed out after {ms} milliseconds with {} out of {n} bytes received",
                                data.len()
                            ),
                        ),
                        _ => (
                            28,
                            format!(
                                "Operation timed out after {ms} milliseconds with {} bytes received",
                                data.len()
                            ),
                        ),
                    }
                } else if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    match framing {
                        Framing::Length(n) => (
                            18,
                            format!(
                                "transfer closed with {} bytes remaining to read",
                                n.saturating_sub(data.len() as u64)
                            ),
                        ),
                        _ => (
                            18,
                            "transfer closed with outstanding read data remaining".into(),
                        ),
                    }
                } else {
                    recv_error(&e)
                };
                info.redirect_count = redirects;
                let mut o = fail(info, errno, msg, start);
                o.header_blocks = header_blocks;
                o.body = data;
                return o;
            }
        }

        if follow {
            if redirects >= spec.max_redirs && spec.max_redirs >= 0 {
                bail!(
                    47,
                    format!("Maximum ({}) redirects followed", spec.max_redirs)
                );
            }
            let next = next.unwrap_or_default();
            let next_scheme = scheme_of(&next).unwrap_or_default();
            let bit = KNOWN_PROTOCOLS
                .iter()
                .find(|(n, _)| *n == next_scheme)
                .map_or(0, |(_, b)| *b);
            if spec.redir_protocols & bit == 0 || (bit != PROTO_HTTP && bit != PROTO_HTTPS) {
                bail!(
                    1,
                    format!("Protocol \"{next_scheme}\" not supported or disabled in libcurl")
                );
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
                body_ct = None;
            }
            if spec.autoreferer {
                let mut u = parsed.clone();
                u.set_fragment(None);
                let _ = u.set_username("");
                let _ = u.set_password(None);
                referer = Some(u.to_string());
            }
            url_s = next;
            continue;
        }

        if let Some(ce) = content_encoding.filter(|_| spec.encoding.is_some()) {
            info.size_download = data.len() as i64;
            match decode_body(data, &ce) {
                Ok(d) => data = d,
                Err((e, m)) => {
                    info.redirect_count = redirects;
                    let mut o = fail(info, e, m, start);
                    o.header_blocks = header_blocks;
                    return o;
                }
            }
        } else {
            info.size_download = data.len() as i64;
        }
        info.redirect_count = redirects;
        info.redirect_us = redirect_us;
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
