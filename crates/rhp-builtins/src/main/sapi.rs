//! The server API layer shared by the engine and its front ends (php's
//! `main/SAPI.c` + `ext/standard/head.c`).
//!
//! - [`Sapi`]: what a server embedding the engine provides (php's
//!   `sapi_module_struct`): its name, ini defaults, the unbuffered body
//!   writer, header emission, flushing and logging.
//! - [`Response`]: the per-request header store (`SG(sapi_headers)` plus
//!   `SG(headers_sent)` and the output start location), driven by
//!   [`header_op`] exactly as `sapi_header_op` does.
//! - [`RequestInfo`]: the request input a SAPI hands the engine (php's
//!   `SG(request_info)` plus the server variables): `$_SERVER`, `$_GET`,
//!   `$_POST`, `$_COOKIE`, `php://input`, `getallheaders()`.
//! - The header builtins: `header`, `header_remove`, `headers_list`,
//!   `headers_sent`, `http_response_code`, `setcookie`, `setrawcookie`,
//!   `header_register_callback`, `flush`, `getallheaders` /
//!   `apache_request_headers` / `apache_response_headers`.
//!
//! The SAPI name and its ini defaults are per thread (one engine per thread
//! at a time, see [`set_sapi`]); everything else lives in the engine.

use std::cell::RefCell;
use std::collections::HashMap;

use rhp_values::ops::deref;
use rhp_values::{Thrown, Value};

use crate::Host;

pub use crate::main::php_variables::{post_form_vars, treat_data, url_decode, TreatData};

/// What a server embedding the engine provides for one request (php's
/// `sapi_module_struct`). Every method but [`Sapi::name`] and
/// [`Sapi::ub_write`] has the php-cli behaviour as its default.
pub trait Sapi {
    /// `php_sapi_name()` / `PHP_SAPI` (`"cli"`, `"cgi-fcgi"`, …).
    fn name(&self) -> &str;

    /// The SAPI's ini defaults (`sapi_module.ini_defaults`): applied over the
    /// built-in defaults, below `-d` settings and `ini_set`.
    fn ini_defaults(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }

    /// Whether `header()` lines are stored (`sapi_module.header_handler`:
    /// php-cli's returns 0, so its header list stays empty).
    fn stores_headers(&self) -> bool {
        true
    }

    /// The status before any `header()` call: php-cli leaves it 0
    /// (`http_response_code()` is false), the CGI SAPI starts at 200.
    fn initial_response_code(&self) -> i64 {
        200
    }

    /// `ub_write`: response body bytes, after the headers went out.
    fn ub_write(&mut self, data: &[u8]);

    /// `flush` (`flush()`, implicit_flush, request end).
    fn flush(&mut self) {}

    /// `send_headers`: called once, right before the first body byte (or at
    /// request end when there is no body) unless the request has
    /// `no_headers`. The status line and header list are the final ones.
    fn send_headers(&mut self, _response: &Response) {}

    /// `log_message`: one `error_log` / `log_errors` line (no trailing
    /// newline; the SAPI adds its own, like php's CLI and CGI do).
    fn log_message(&mut self, msg: &[u8]) {
        use std::io::Write;
        let mut e = std::io::stderr().lock();
        let _ = e.write_all(msg);
        let _ = e.write_all(b"\n");
    }

    /// `display_errors=stderr` output (honoured only by the `cli`, `cgi` and
    /// `phpdbg` SAPI names, as php's `php_error_cb` does).
    fn write_stderr(&mut self, data: &[u8]) {
        use std::io::Write;
        let _ = std::io::stderr().write_all(data);
    }

    /// A write to the `STDOUT` stream (php-cli): php writes the descriptor
    /// directly, past the output layer, so what the SAPI holds goes out
    /// first.
    fn write_raw_stdout(&mut self, data: &[u8]) {
        use std::io::Write;
        self.flush();
        let mut so = std::io::stdout().lock();
        let _ = so.write_all(data);
        let _ = so.flush();
    }

    /// Whether running past the hard memory limit
    /// (`memory_hard_limit_percent`) ends the process, as php does (message
    /// on stderr, exit 137). False: the request ends as a fatal error, so
    /// one script can't take its host down.
    fn exits_on_hard_limit(&self) -> bool {
        false
    }
}

/// php's `SG(sapi_headers)` + `SG(headers_sent)` + the output start
/// location: the response the request is building.
#[derive(Clone)]
pub struct Response {
    /// `http_response_code` (0 = none set, php-cli's start).
    pub code: i64,
    /// A `header("HTTP/1.1 404 Not Found")` line, until the headers went out.
    pub status_line: Option<Vec<u8>>,
    /// The header lines in order (`headers_list()`).
    pub headers: Vec<Vec<u8>>,
    /// No `Content-Type` header was set: the default one is added when the
    /// headers are sent.
    pub send_default_content_type: bool,
    /// The response's mime type once known.
    pub mimetype: Option<Vec<u8>>,
    /// `SG(headers_sent)`.
    pub sent: bool,
    /// `SG(request_info).no_headers` (`php-cgi -q`): headers are never sent
    /// and never refused.
    pub no_headers: bool,
    /// A HEAD request (`headers_only`): the body is discarded.
    pub headers_only: bool,
    /// The SAPI refused output (`PHP_OUTPUT_DISABLED`).
    pub output_disabled: bool,
    /// Where output started (`headers_sent($file, $line)`).
    pub output_start: Option<(Vec<u8>, i64)>,
    /// `header_register_callback`'s callable.
    pub callback: Option<Value>,
    /// Whether header lines are stored ([`Sapi::stores_headers`]).
    pub store: bool,
    /// `SG(request_info).request_method` (the Location → 303 rule).
    pub method: Option<Vec<u8>>,
    /// `SG(request_info).proto_num` (1000 = HTTP/1.0).
    pub proto_num: i64,
}

impl Default for Response {
    /// php-cli's response: nothing stored, no status.
    fn default() -> Self {
        Response {
            code: 0,
            status_line: None,
            headers: Vec::new(),
            send_default_content_type: true,
            mimetype: None,
            sent: false,
            no_headers: false,
            headers_only: false,
            output_disabled: false,
            output_start: None,
            callback: None,
            store: false,
            method: None,
            proto_num: 1000,
        }
    }
}

impl Response {
    /// `sapi_update_response_code`: a new code drops the status line.
    pub fn update_code(&mut self, code: i64) {
        if self.code == code {
            return;
        }
        self.status_line = None;
        self.code = code;
    }

    /// `sapi_remove_header`: drop every `name:` header (case-insensitive).
    pub fn remove_header(&mut self, name: &[u8]) {
        let len = name.len();
        self.headers
            .retain(|h| !(h.len() > len && h[len] == b':' && h[..len].eq_ignore_ascii_case(name)));
    }

    /// `sapi_header_add_op`: store a line (replacing same-name ones).
    fn add(&mut self, replace: bool, line: Vec<u8>) {
        if !self.store {
            return;
        }
        if replace {
            if let Some(c) = line.iter().position(|&b| b == b':') {
                let name = line[..c].to_vec();
                self.remove_header(&name);
            }
        }
        self.headers.push(line);
    }

    /// The `Status:` value the CGI SAPI derives: the reason phrase php's
    /// `http_status_map` has for `code`, if any.
    pub fn reason_phrase(code: i64) -> Option<&'static str> {
        Some(match code {
            100 => "Continue",
            101 => "Switching Protocols",
            200 => "OK",
            201 => "Created",
            202 => "Accepted",
            203 => "Non-Authoritative Information",
            204 => "No Content",
            205 => "Reset Content",
            206 => "Partial Content",
            300 => "Multiple Choices",
            301 => "Moved Permanently",
            302 => "Found",
            303 => "See Other",
            304 => "Not Modified",
            305 => "Use Proxy",
            307 => "Temporary Redirect",
            308 => "Permanent Redirect",
            400 => "Bad Request",
            401 => "Unauthorized",
            402 => "Payment Required",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            406 => "Not Acceptable",
            407 => "Proxy Authentication Required",
            408 => "Request Timeout",
            409 => "Conflict",
            410 => "Gone",
            411 => "Length Required",
            412 => "Precondition Failed",
            413 => "Request Entity Too Large",
            414 => "Request-URI Too Long",
            415 => "Unsupported Media Type",
            416 => "Requested Range Not Satisfiable",
            417 => "Expectation Failed",
            426 => "Upgrade Required",
            428 => "Precondition Required",
            429 => "Too Many Requests",
            431 => "Request Header Fields Too Large",
            451 => "Unavailable For Legal Reasons",
            500 => "Internal Server Error",
            501 => "Not Implemented",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            504 => "Gateway Timeout",
            505 => "HTTP Version Not Supported",
            506 => "Variant Also Negotiates",
            511 => "Network Authentication Required",
            _ => return None,
        })
    }
}

/// php's `SG(request_info)` plus the server variables: what a SAPI hands the
/// engine about the request. Byte strings throughout (php is binary-safe).
#[derive(Default)]
pub struct RequestInfo {
    /// The SAPI's `register_server_variables` result, in order (the CGI
    /// SAPI: the environment, then `PHP_SELF`). `REQUEST_TIME[_FLOAT]` and
    /// `argv`/`argc` are added by the engine.
    pub server: Vec<(Vec<u8>, Vec<u8>)>,
    /// The environment (`$_ENV`, `getenv()`), in order.
    pub env: Vec<(Vec<u8>, Vec<u8>)>,
    /// `REQUEST_METHOD`.
    pub method: Option<Vec<u8>>,
    /// `QUERY_STRING` (`$_GET`).
    pub query_string: Option<Vec<u8>>,
    /// The `Cookie` header (`$_COOKIE`).
    pub cookie: Option<Vec<u8>>,
    /// `CONTENT_TYPE` (empty when absent, as php's CGI SAPI leaves it).
    pub content_type: Option<Vec<u8>>,
    /// The request body (`php://input`, `$_POST`), already read.
    pub body: Vec<u8>,
    /// The declared `Content-Length` (`post_max_size` is checked against
    /// it); `None`: the body's length. A body longer than this (a chunked
    /// request: `Some(0)`) is read only until more than `post_max_size`
    /// bytes were read, as php's readers do (GHSA-2cv3-554h-jpmh).
    pub content_length: Option<i64>,
    /// The `Authorization` header (php-cgi: `HTTP_AUTHORIZATION`), which
    /// yields `PHP_AUTH_USER` / `PHP_AUTH_PW` / `PHP_AUTH_DIGEST`.
    pub authorization: Option<Vec<u8>>,
    /// The request start time (`REQUEST_TIME_FLOAT`), seconds since the epoch.
    pub request_time: f64,
    /// Request headers for `getallheaders()` / `apache_request_headers()`.
    pub headers: Vec<(Vec<u8>, Vec<u8>)>,
    /// `SG(request_info).argc/argv` (php-cgi run from a shell): registers
    /// `$argv`/`$argc` and `$_SERVER['argv'/'argc']`.
    pub argv: Option<Vec<Vec<u8>>>,
    /// `php-cgi -q`: never send headers.
    pub no_headers: bool,
    /// The status before any `header()` call (php-cgi: 200, or 404 when
    /// `init_request_info` found no script).
    pub response_code: i64,
    /// `SG(request_info).proto_num` (php's CGI SAPI leaves it at 1000).
    pub proto_num: i64,
}

thread_local! {
    /// The running engine's SAPI name (`php_sapi_name()`).
    static SAPI_NAME: RefCell<String> = RefCell::new(String::from("cli"));
    /// The SAPI's ini defaults (`sapi_module.ini_defaults`), by directive.
    static SAPI_INI: RefCell<HashMap<String, Vec<u8>>> = RefCell::new(HashMap::new());
}

/// Install the running engine's SAPI name and ini defaults (per thread: one
/// engine per thread at a time). [`crate::reset_request_state`] goes back
/// to php-cli's.
pub fn set_sapi(name: &str, ini_defaults: &[(&str, &str)]) {
    SAPI_NAME.with(|n| *n.borrow_mut() = name.to_string());
    SAPI_INI.with(|m| {
        let mut m = m.borrow_mut();
        m.clear();
        for (k, v) in ini_defaults {
            m.insert((*k).to_string(), v.as_bytes().to_vec());
        }
    });
}

/// The running engine's SAPI name.
pub fn sapi_name() -> String {
    SAPI_NAME.with(|n| n.borrow().clone())
}

/// Whether the running SAPI is php-cgi's `cgi-fcgi`.
pub fn is_cgi() -> bool {
    SAPI_NAME.with(|n| n.borrow().as_str() == "cgi-fcgi")
}

/// The SAPI's default for a directive, if it overrides the built-in one.
pub(crate) fn sapi_ini_default(name: &str) -> Option<Vec<u8>> {
    SAPI_INI.with(|m| m.borrow().get(name).cloned())
}

pub(crate) fn reset_state() {
    set_sapi("cli", &[]);
}

/// Whether the running SAPI has pcntl: only the CLI and CGI. Signal
/// handlers and fork belong to the whole process, so a server that embeds
/// rhp never gets them. This is rhp's choice: php links pcntl into every
/// SAPI binary built with it (php-fpm and embed included).
pub fn has_pcntl() -> bool {
    SAPI_NAME.with(|n| matches!(n.borrow().as_str(), "cli" | "cgi-fcgi"))
}

/// Builtins that exist only under some SAPIs (php registers them as SAPI
/// `additional_functions`; pcntl's, see [`has_pcntl`]): hidden from lookup
/// elsewhere.
pub fn builtin_hidden(name_lc: &str) -> bool {
    match name_lc {
        "getallheaders"
        | "apache_request_headers"
        | "apache_response_headers"
        | "apache_child_terminate" => !is_cgi(),
        "cli_set_process_title" | "cli_get_process_title" => {
            SAPI_NAME.with(|n| n.borrow().as_str() != "cli")
        }
        n if n.starts_with("pcntl_") => !has_pcntl(),
        _ => false,
    }
}

/// php's `escape_html` for `html_errors`: `htmlspecialchars` with
/// ENT_COMPAT (`&`, `<`, `>`, `"`) in `default_charset`. Under UTF-8 (also
/// php's fallback for a charset it does not know) invalid sequences are
/// substituted (the retry with ENT_HTML_SUBSTITUTE_ERRORS); a single-byte
/// charset such as Windows-1251 passes every byte through.
pub fn escape_html(s: &[u8], charset: &[u8]) -> Vec<u8> {
    let owned;
    let s: &[u8] = match std::str::from_utf8(s) {
        Err(_) if !is_non_utf8_charset(charset) => {
            owned = String::from_utf8_lossy(s).into_owned().into_bytes();
            &owned
        }
        _ => s,
    };
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        match b {
            b'&' => out.extend_from_slice(b"&amp;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            _ => out.push(b),
        }
    }
    out
}

/// A charset name from html.c's `charset_map` other than UTF-8 (the
/// multi-byte ones are treated bytewise too).
fn is_non_utf8_charset(charset: &[u8]) -> bool {
    const NAMES: &[&str] = &[
        "iso-8859-1",
        "iso8859-1",
        "iso-8859-15",
        "iso8859-15",
        "iso-8859-5",
        "iso8859-5",
        "cp866",
        "866",
        "ibm866",
        "cp1251",
        "windows-1251",
        "win-1251",
        "1251",
        "cp1252",
        "windows-1252",
        "1252",
        "koi8-r",
        "koi8-ru",
        "koi8r",
        "big5",
        "950",
        "gb2312",
        "936",
        "big5-hkscs",
        "shift_jis",
        "sjis",
        "sjis-win",
        "cp932",
        "932",
        "euc-jp",
        "eucjp",
        "eucjp-win",
        "macroman",
    ];
    let c = String::from_utf8_lossy(charset).trim().to_ascii_lowercase();
    NAMES.contains(&c.as_str())
}

/// `get_default_content_type`: `default_mimetype` plus `; charset=` for
/// `text/*` types.
pub fn default_content_type(host: &dyn Host) -> Vec<u8> {
    let mut mime = host.ini_get(b"default_mimetype").unwrap_or_default();
    if mime.is_empty() {
        mime = b"text/html".to_vec();
    }
    let charset = host.ini_get(b"default_charset").unwrap_or_default();
    if !charset.is_empty() && mime.len() >= 5 && mime[..5].eq_ignore_ascii_case(b"text/") {
        mime.extend_from_slice(b"; charset=");
        mime.extend_from_slice(&charset);
    }
    mime
}

/// `sapi_header_op`'s operations.
pub enum HeaderOp<'a> {
    /// `header($line, true, $code)`.
    Replace(&'a [u8], i64),
    /// `header($line, false, $code)` / `setcookie`.
    Add(&'a [u8], i64),
    /// `header_remove($name)`.
    Delete(&'a [u8]),
    /// `header_remove()`.
    DeleteAll,
}

/// A warning raised through `sapi_error` (`zend_error`: no `func(): `
/// prefix, never HTML-escaped).
fn sapi_error(host: &mut dyn Host, msg: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(crate::E_WARNING, msg.as_bytes())
}

/// "(output started at FILE:LINE)" for the headers-already-sent warnings.
pub(crate) fn started_at(r: &Response) -> Option<String> {
    r.output_start
        .as_ref()
        .filter(|(f, _)| !f.is_empty())
        .map(|(f, l)| format!("{}:{l}", String::from_utf8_lossy(f)))
}

/// `sapi_header_op`. Returns php's SUCCESS.
pub fn header_op(host: &mut dyn Host, op: HeaderOp<'_>) -> Result<bool, Thrown> {
    let Some(r) = host.response() else {
        return Ok(false);
    };
    if r.sent && !r.no_headers {
        let msg = match started_at(r) {
            Some(at) => format!(
                "Cannot modify header information - headers already sent by (output started at {at})"
            ),
            None => "Cannot modify header information - headers already sent".to_string(),
        };
        sapi_error(host, &msg)?;
        return Ok(false);
    }
    let (line, code, replace, delete) = match op {
        HeaderOp::DeleteAll => {
            r.headers.clear();
            return Ok(true);
        }
        HeaderOp::Replace(l, c) => (l, c, true, false),
        HeaderOp::Add(l, c) => (l, c, false, false),
        HeaderOp::Delete(l) => (l, 0, false, true),
    };
    if line.is_empty() {
        return Ok(false);
    }
    let mut line = line.to_vec();
    while line
        .last()
        .is_some_and(|&b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
    {
        line.pop();
    }
    if delete {
        if line.contains(&b':') {
            sapi_error(host, "Header to delete may not contain colon.")?;
            return Ok(false);
        }
        if let Some(r) = host.response() {
            r.remove_header(&line);
        }
        return Ok(true);
    }
    if line.iter().any(|&b| b == b'\n' || b == b'\r') {
        sapi_error(
            host,
            "Header may not contain more than a single header, new line detected",
        )?;
        return Ok(false);
    }
    if line.contains(&0) {
        sapi_error(host, "Header may not contain NUL bytes")?;
        return Ok(false);
    }
    let charset = host.ini_get(b"default_charset").unwrap_or_default();
    let Some(r) = host.response() else {
        return Ok(false);
    };
    if line.len() >= 5 && line[..5].eq_ignore_ascii_case(b"HTTP/") {
        r.update_code(extract_response_code(&line));
        r.status_line = Some(line);
        return Ok(true);
    }
    if let Some(colon) = line.iter().position(|&b| b == b':') {
        let name = &line[..colon];
        if name.eq_ignore_ascii_case(b"Content-Type") {
            let mut p = colon + 1;
            while line.get(p) == Some(&b' ') {
                p += 1;
            }
            let mut mime = line[p..].to_vec();
            let changed = apply_default_charset(&mut mime, &charset);
            if r.mimetype.is_none() {
                r.mimetype = Some(mime.clone());
            }
            if changed {
                let mut nl = b"Content-type: ".to_vec();
                nl.extend_from_slice(&mime);
                line = nl;
            }
            r.send_default_content_type = false;
        } else if name.eq_ignore_ascii_case(b"Location") {
            if (r.code < 300 || r.code > 399) && r.code != 201 {
                if code != 0 {
                    r.update_code(code);
                } else if r.proto_num > 1000
                    && r.method
                        .as_deref()
                        .is_some_and(|m| m != b"HEAD" && m != b"GET")
                {
                    r.update_code(303);
                } else {
                    r.update_code(302);
                }
            }
        } else if name.eq_ignore_ascii_case(b"WWW-Authenticate") {
            r.update_code(401);
        }
    }
    if code != 0 {
        r.update_code(code);
    }
    r.add(replace, line);
    Ok(true)
}

/// `sapi_extract_response_code`: the number after the first space that is
/// not followed by another space (200 when none).
fn extract_response_code(line: &[u8]) -> i64 {
    for i in 0..line.len() {
        if line[i] == b' ' && line.get(i + 1) != Some(&b' ') {
            return crate::main::ini::c_atol(&line[i + 1..]);
        }
    }
    200
}

/// `sapi_apply_default_charset`: `text/*` without a charset gets
/// `;charset=<default_charset>`. Returns whether it changed.
fn apply_default_charset(mime: &mut Vec<u8>, charset: &[u8]) -> bool {
    if !charset.is_empty() && mime.starts_with(b"text/") && !contains(mime, b"charset=") {
        mime.extend_from_slice(b";charset=");
        mime.extend_from_slice(charset);
        return true;
    }
    false
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The default `Content-type` line `sapi_send_headers` adds when no
/// Content-Type header was set (and marks it set).
pub fn take_default_content_type(host: &mut dyn Host) {
    let want = host.response().is_some_and(|r| r.send_default_content_type);
    if !want {
        return;
    }
    let ct = default_content_type(host);
    if let Some(r) = host.response() {
        if !ct.is_empty() {
            r.mimetype = Some(ct.clone());
            let mut line = b"Content-type: ".to_vec();
            line.extend_from_slice(&ct);
            r.add(false, line);
        }
        r.send_default_content_type = false;
    }
}

fn fn_header_register_callback(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cb = args[0].clone();
    if let Some(err) = host.callable_error(&cb) {
        return Err(Thrown::type_error(format!(
            "header_register_callback(): Argument #1 ($callback) must be a valid callback, {err}"
        )));
    }
    if let Some(r) = host.response() {
        r.callback = None;
        if !r.sent {
            r.callback = Some(deref(&cb));
        }
    }
    Ok(Value::Bool(true))
}

/// The CGI SAPI's request headers from its environment
/// (`apache_request_headers` without FastCGI): `HTTP_FOO_BAR` → `Foo-Bar`,
/// plus `CONTENT_TYPE` / `CONTENT_LENGTH`.
pub fn cgi_request_headers(env: &[(Vec<u8>, Vec<u8>)]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    for (k, v) in env {
        let name = if k.len() > 5 && k.starts_with(b"HTTP_") {
            let p = &k[5..];
            let mut q = vec![p[0]];
            let mut i = 1;
            while i < p.len() {
                let c = p[i];
                if c == b'_' {
                    q.push(b'-');
                    i += 1;
                    if i < p.len() {
                        q.push(p[i]);
                        i += 1;
                    }
                } else {
                    q.push(c.to_ascii_lowercase());
                    i += 1;
                }
            }
            q
        } else if k == b"CONTENT_TYPE" {
            b"Content-Type".to_vec()
        } else if k == b"CONTENT_LENGTH" {
            b"Content-Length".to_vec()
        } else {
            continue;
        };
        out.push((name, v.clone()));
    }
    out
}

/// `php_handle_auth_data`: the `$_SERVER` entries an `Authorization` header
/// yields — `PHP_AUTH_USER` / `PHP_AUTH_PW` for `Basic` (loose base64, the
/// password only when non-empty), `PHP_AUTH_DIGEST` for `Digest`.
pub fn auth_server_vars(auth: &[u8]) -> Vec<(&'static [u8], Vec<u8>)> {
    // php keeps C strings: everything stops at the first NUL.
    fn cstr(s: &[u8]) -> Vec<u8> {
        s.iter().take_while(|&&b| b != 0).copied().collect()
    }
    let has_prefix = |p: &[u8]| auth.len() >= p.len() && auth[..p.len()].eq_ignore_ascii_case(p);
    if has_prefix(b"Basic ") {
        if let Some(user) = crate::ext::standard::base64::base64_decode(&auth[6..], false) {
            if let Some(colon) = user.iter().position(|&b| b == b':') {
                let mut out = vec![(&b"PHP_AUTH_USER"[..], cstr(&user[..colon]))];
                let pass = cstr(&user[colon + 1..]);
                if !pass.is_empty() {
                    out.push((&b"PHP_AUTH_PW"[..], pass));
                }
                return out;
            }
        }
    }
    if has_prefix(b"Digest ") {
        return vec![(&b"PHP_AUTH_DIGEST"[..], cstr(&auth[7..]))];
    }
    Vec::new()
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[("header_register_callback", fn_header_register_callback)],
    );
}
