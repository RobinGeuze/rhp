//! php's built-in web server (`rhp -S addr:port [-t docroot] [router]`),
//! a port of sapi/cli/php_cli_server.c: one process (or
//! `PHP_CLI_SERVER_WORKERS` forked ones) polling a listening socket and its
//! clients, php_http_parser (rhp-csrc) reading each request, then a PHP
//! request run on a fresh engine ([`run_request`]) with the `cli-server`
//! SAPI, or a static file sent with the mime type php's table gives its
//! extension. The router script runs first when there is one; returning
//! `false` hands the request back to the server. Log lines go to stderr
//! with php's timestamps and wording (`artisan serve` reads them).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ffi::CString;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};

use rhp_builtins::main::sapi::{RequestInfo, Response, Sapi, ScriptDone};
use rhp_csrc::cli_server::{method, method_str, Event, HttpParser};
use rhp_values::Value;

use crate::getopt::{Opt, PhpGetopt};
use crate::{run_request, IniFiles, Request, Script};

const LOG_PROCESS: i32 = 1;
const LOG_ERROR: i32 = 2;
const LOG_MESSAGE: i32 = 3;

/// `php_cli_server_log_level` (`-q` lowers it).
static LOG_LEVEL: AtomicI32 = AtomicI32::new(3);
/// `php_cli_server_workers_max`.
static WORKERS_MAX: AtomicI64 = AtomicI64::new(0);
/// `server.is_running` (SIGINT clears it).
static RUNNING: AtomicBool = AtomicBool::new(true);

const UNEXPECTED_EOF: &str = "Unexpected EOF";

/// `php_cli_server_css`, copied from ext/standard/info.c.
const CSS: &str = "<style>\nbody { background-color: #fcfcfc; color: #333333; margin: 0; padding:0; }\nh1 { font-size: 1.5em; font-weight: normal; background-color: #9999cc; min-height:2em; line-height:2em; border-bottom: 1px inset black; margin: 0; }\nh1, p { padding-left: 10px; }\ncode.url { background-color: #eeeeee; font-family:monospace; padding:0 2px;}\n</style>\n";

/// `php_cli_server_get_system_time`: asctime() of the local time, without
/// its newline.
fn system_time() -> Option<String> {
    // SAFETY: plain libc time calls on local buffers.
    unsafe {
        let mut tv: libc::timeval = std::mem::zeroed();
        libc::gettimeofday(&mut tv, std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&tv.tv_sec, &mut tm).is_null() {
            return None;
        }
        let mut buf = [0 as libc::c_char; 52];
        if libc::asctime_r(&tm, buf.as_mut_ptr()).is_null() {
            return None;
        }
        let s = std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy();
        Some(s.strip_suffix('\n').unwrap_or(&s).to_string())
    }
}

/// `sapi_cli_server_log_write`: `[time] msg` on stderr (with the pid first
/// when there are workers).
fn log_write(ty: i32, msg: &[u8]) {
    if LOG_LEVEL.load(Ordering::Relaxed) < ty {
        return;
    }
    let t = match system_time() {
        Some(t) if t.is_empty() => "unknown".to_string(),
        Some(t) => t,
        None => "unknown time, can't be fetched".to_string(),
    };
    let mut line = Vec::with_capacity(msg.len() + 40);
    if WORKERS_MAX.load(Ordering::Relaxed) > 1 {
        // SAFETY: getpid has no preconditions.
        let _ = write!(line, "[{}] ", unsafe { libc::getpid() });
    }
    let _ = write!(line, "[{t}] ");
    line.extend_from_slice(msg);
    line.push(b'\n');
    let _ = std::io::stderr().write_all(&line);
}

fn logf(ty: i32, msg: impl AsRef<[u8]>) {
    log_write(ty, msg.as_ref());
}

fn strerror(errno: i32) -> String {
    rhp_builtins::main::streams::strerror(errno)
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// `get_status_string`: php's reason phrase, else a placeholder.
fn status_string(code: i64) -> &'static str {
    Response::reason_phrase(code).unwrap_or("Unknown Status Code")
}

/// `append_http_status_line`.
fn status_line(out: &mut Vec<u8>, protocol_version: i32, code: i64) {
    let code = if code == 0 { 200 } else { code };
    let _ = write!(
        out,
        "HTTP/{}.{} {code} {}\r\n",
        protocol_version / 100,
        protocol_version % 100,
        status_string(code)
    );
}

/// `append_essential_headers`: `Host` as the client sent it, `Date`
/// unless the script set one, `Connection: close`.
fn essential_headers(out: &mut Vec<u8>, host: Option<&[u8]>, headers: Option<&[Vec<u8>]>) {
    let has_date = headers.is_some_and(|hs| {
        hs.iter()
            .any(|h| h.len() > 5 && h[..5].eq_ignore_ascii_case(b"Date:"))
    });
    if let Some(host) = host {
        out.extend_from_slice(b"Host: ");
        out.extend_from_slice(host);
        out.extend_from_slice(b"\r\n");
    }
    if !has_date {
        let _ = write!(out, "Date: {} GMT\r\n", http_date());
    }
    out.extend_from_slice(b"Connection: close\r\n");
}

/// `php_format_date("D, d M Y H:i:s", now, 0)`.
fn http_date() -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // SAFETY: plain libc time calls on local buffers.
    let tm = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02}",
        DAYS[tm.tm_wday.clamp(0, 6) as usize],
        tm.tm_mday,
        MONTHS[tm.tm_mon.clamp(0, 11) as usize],
        tm.tm_year + 1900,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// `php_raw_url_decode`: `%XX` escapes only.
fn raw_url_decode(s: &[u8]) -> Vec<u8> {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' && i + 2 < s.len() {
            if let (Some(a), Some(b)) = (hex(s[i + 1]), hex(s[i + 2])) {
                out.push(a * 16 + b);
                i += 3;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// `normalize_vpath`: the decoded path with duplicate slashes, `.` and `..`
/// segments removed (`..` never above the root).
fn normalize_vpath(vpath: &[u8]) -> Vec<u8> {
    let mut d = raw_url_decode(vpath);
    let mut end = d.len();
    let mut p = 0;
    if p < end && d[p] == b'/' {
        let mut n = p;
        while n < end && d[n] == b'/' {
            n += 1;
        }
        p += 1;
        d.copy_within(n..end, p);
        end -= n - p;
    }
    while p < end {
        let mut n = p;
        while n < end && d[n] != b'/' {
            n += 1;
        }
        if n - p == 2 && d[p] == b'.' && d[p + 1] == b'.' {
            if p > 0 {
                p -= 1;
                loop {
                    if p == 0 {
                        if d[p] == b'/' {
                            p += 1;
                        }
                        break;
                    }
                    p -= 1;
                    if d[p] == b'/' {
                        p += 1;
                        break;
                    }
                }
            }
            while n < end && d[n] == b'/' {
                n += 1;
            }
            d.copy_within(n..end, p);
            end -= n - p;
        } else if n - p == 1 && d[p] == b'.' {
            while n < end && d[n] == b'/' {
                n += 1;
            }
            d.copy_within(n..end, p);
            end -= n - p;
        } else if n < end {
            let mut nn = n;
            while nn < end && d[nn] == b'/' {
                nn += 1;
            }
            p = n + 1;
            d.copy_within(nn..end, p);
            end -= nn - p;
        } else {
            p = n;
        }
    }
    d.truncate(end);
    d
}

/// `stat(2)` on a path: (is a directory, is a regular file, size).
fn stat(path: &[u8]) -> Option<(bool, bool, u64)> {
    let c = CString::new(path).ok()?;
    // SAFETY: c is a valid C string; sb is written by stat.
    unsafe {
        let mut sb: libc::stat = std::mem::zeroed();
        if libc::stat(c.as_ptr(), &mut sb) != 0 {
            return None;
        }
        let fmt = sb.st_mode & libc::S_IFMT;
        Some((
            fmt == libc::S_IFDIR,
            fmt == libc::S_IFREG,
            sb.st_size as u64,
        ))
    }
}

/// `php_cli_server_request`.
#[derive(Default)]
struct Req {
    method: u8,
    protocol_version: i32,
    request_uri: Option<Vec<u8>>,
    vpath: Option<Vec<u8>>,
    path_translated: Option<Vec<u8>>,
    path_info: Option<Vec<u8>>,
    query_string: Option<Vec<u8>>,
    /// Lowercased names, in first-seen order (a hash table updated in place).
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    /// The names as sent.
    headers_original_case: Vec<(Vec<u8>, Vec<u8>)>,
    content: Option<Vec<u8>>,
    ext: Vec<u8>,
    size: u64,
}

impl Req {
    fn header(&self, lc: &[u8]) -> Option<&[u8]> {
        self.headers
            .iter()
            .find(|(k, _)| k == lc)
            .map(|(_, v)| &v[..])
    }

    /// `php_cli_server_request_translate_vpath`: the file the path names
    /// (a directory's index.php or index.html), what follows it as
    /// PATH_INFO, walking up until something exists.
    fn translate_vpath(&mut self, document_root: &[u8]) {
        const INDEX_FILES: [&[u8]; 2] = [b"index.php", b"index.html"];
        let vpath = self.vpath.clone().unwrap_or_default();
        let mut buf = document_root.to_vec();
        let vstart = buf.len();
        if !vpath.is_empty() {
            if vpath[0] != b'/' {
                buf.push(b'/');
            }
            buf.extend_from_slice(&vpath);
        }
        let p = buf.len();
        let mut q = p;
        let mut prev_path: Option<Vec<u8>> = None;
        let mut size = 0;
        // buf[..q] is the path being tried.
        while q > 0 {
            if let Some((is_dir, _, sz)) = stat(&buf[..q]) {
                size = sz;
                if is_dir {
                    buf.truncate(q);
                    if buf[q - 1] != b'/' {
                        buf.push(b'/');
                        q += 1;
                    }
                    let mut found = false;
                    for file in INDEX_FILES {
                        buf.truncate(q);
                        buf.extend_from_slice(file);
                        if let Some((_, true, sz)) = stat(&buf) {
                            size = sz;
                            q += file.len();
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return;
                    }
                }
                break;
            }
            // The tail from the slash before q to the original end.
            while q > 0 {
                q -= 1;
                if buf[q] == b'/' {
                    break;
                }
            }
            prev_path = Some(buf[q..p].to_vec());
        }
        if let Some(prev) = prev_path {
            self.path_info = Some(prev);
        }
        let q = q.min(buf.len());
        self.vpath = Some(buf.get(vstart..q).unwrap_or_default().to_vec());
        self.path_translated = Some(buf[..q].to_vec());
        self.size = size;
    }
}

#[derive(PartialEq, Clone, Copy)]
enum HeaderElement {
    None,
    Field,
    Value,
}

/// `php_cli_server_client`.
struct Client {
    addr_str: String,
    parser: HttpParser,
    request_read: bool,
    current_header_name: Vec<u8>,
    current_header_value: Vec<u8>,
    last_header_element: HeaderElement,
    request: Req,
    /// The content sender's chunks, once a static file or error page is
    /// being sent.
    content: Option<VecDeque<Vec<u8>>>,
    file: Option<std::fs::File>,
}

impl Client {
    fn new(addr_str: String) -> Self {
        Client {
            addr_str,
            parser: HttpParser::new(),
            request_read: false,
            current_header_name: Vec::new(),
            current_header_value: Vec::new(),
            last_header_element: HeaderElement::None,
            request: Req::default(),
            content: None,
            file: None,
        }
    }

    /// `php_cli_server_client_save_header`.
    fn save_header(&mut self) {
        let name = std::mem::take(&mut self.current_header_name);
        let value = std::mem::take(&mut self.current_header_value);
        let lc = name.to_ascii_lowercase();
        let with_comma = lc != b"set-cookie";
        let r = &mut self.request;
        let value = match r.headers.iter().find(|(k, _)| *k == lc) {
            Some((_, cur)) if with_comma => {
                let mut v = cur.clone();
                v.extend_from_slice(b", ");
                v.extend_from_slice(&value);
                v
            }
            _ => value,
        };
        upsert(&mut r.headers, lc, value.clone());
        upsert(&mut r.headers_original_case, name, value);
    }

    /// One parser callback; false is an error.
    fn on_event(&mut self, ev: Event<'_>, document_root: &[u8]) -> bool {
        match ev {
            Event::MessageBegin | Event::Fragment(_) => {}
            Event::Path(at) => {
                if self.request.vpath.is_some() {
                    return false;
                }
                self.request.vpath = Some(normalize_vpath(at));
            }
            Event::QueryString(at) => self
                .request
                .query_string
                .get_or_insert_with(Vec::new)
                .extend_from_slice(at),
            Event::Url(at, method) => {
                if self.request.request_uri.is_none() {
                    self.request.method = method;
                }
                self.request
                    .request_uri
                    .get_or_insert_with(Vec::new)
                    .extend_from_slice(at);
            }
            Event::HeaderField(at) => {
                match self.last_header_element {
                    HeaderElement::Value => {
                        self.save_header();
                        self.current_header_name = at.to_vec();
                    }
                    HeaderElement::None => self.current_header_name = at.to_vec(),
                    HeaderElement::Field => self.current_header_name.extend_from_slice(at),
                }
                self.last_header_element = HeaderElement::Field;
            }
            Event::HeaderValue(at) => {
                match self.last_header_element {
                    HeaderElement::Field => self.current_header_value = at.to_vec(),
                    HeaderElement::Value => self.current_header_value.extend_from_slice(at),
                    HeaderElement::None => {}
                }
                self.last_header_element = HeaderElement::Value;
            }
            Event::HeadersComplete => {
                match self.last_header_element {
                    HeaderElement::None => {}
                    HeaderElement::Field => {
                        self.current_header_value.clear();
                        self.save_header();
                    }
                    HeaderElement::Value => self.save_header(),
                }
                self.last_header_element = HeaderElement::None;
            }
            Event::Body(at) => self
                .request
                .content
                .get_or_insert_with(Vec::new)
                .extend_from_slice(at),
            Event::MessageComplete(version) => {
                self.request.protocol_version = version;
                self.request.translate_vpath(document_root);
                if let Some(vpath) = &self.request.vpath {
                    self.request.ext = match vpath.iter().rposition(|&b| b == b'.') {
                        Some(dot) => vpath[dot + 1..].to_vec(),
                        None => Vec::new(),
                    };
                }
                self.request_read = true;
            }
        }
        true
    }
}

/// A hash table update: replace in place, else append.
fn upsert(list: &mut Vec<(Vec<u8>, Vec<u8>)>, k: Vec<u8>, v: Vec<u8>) {
    match list.iter_mut().find(|(n, _)| *n == k) {
        Some(slot) => slot.1 = v,
        None => list.push((k, v)),
    }
}

/// `php_cli_server_log_response`.
fn log_response(
    client_addr: &str,
    req: (&str, &[u8]),
    status: i64,
    message: Option<&str>,
    last_error: Option<(i64, &[u8], &[u8], i64)>,
) {
    const E_FATAL_ERRORS: i64 = 1 | 16 | 64 | 256 | 4096 | 4;
    let fatal = last_error.filter(|e| e.0 & E_FATAL_ERRORS != 0);
    let effective = if fatal.is_some() && status == 200 {
        500
    } else {
        status
    };
    let color = if color_enabled() && unsafe { libc::isatty(1) } == 1 {
        match effective {
            500.. => 1,
            400.. => 3,
            200.. => 2,
            _ => 0,
        }
    } else {
        0
    };
    let mut line = Vec::new();
    if color != 0 {
        let _ = write!(line, "\x1b[3{color}m");
    }
    let _ = write!(line, "{client_addr} [{status}]: {} ", req.0);
    line.extend_from_slice(req.1);
    if let Some(m) = message {
        let _ = write!(line, " - {m}");
    }
    if let Some((_, msg, file, lineno)) = fatal {
        line.extend_from_slice(b" - ");
        line.extend_from_slice(msg);
        line.extend_from_slice(b" in ");
        line.extend_from_slice(file);
        let _ = write!(line, " on line {lineno}");
    }
    if color != 0 {
        line.extend_from_slice(b"\x1b[0m");
    }
    log_write(LOG_MESSAGE, &line);
}

thread_local! {
    /// `cli_server.color` as php.ini or `-d` set it at startup.
    static COLOR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn color_enabled() -> bool {
    COLOR.with(|c| c.get())
}

/// What the router or script run tells the server loop.
#[derive(Default)]
struct Dispatch {
    /// The router is running (else the requested script).
    router: bool,
    /// The router returned false.
    declined: bool,
    /// The headers are dropped (the router declined a static file).
    discard_headers: bool,
    headers_written: bool,
    /// The request is for a static file.
    is_static: bool,
    /// The script to run once the router declines.
    script: Option<Vec<u8>>,
    /// `client->addr_str`, the method name and the request URI, for the log.
    addr: String,
    method: &'static str,
    uri: Vec<u8>,
}

/// The `cli-server` SAPI for one request on one client socket.
struct ServerSapi {
    sock: i32,
    protocol_version: i32,
    host: Option<Vec<u8>>,
    st: Rc<RefCell<Dispatch>>,
    /// The socket failed: later writes report the abort at once.
    closed: bool,
}

/// The `cli-server` SAPI's ini defaults: `sapi_cli_ini_defaults`, over
/// php's core defaults (php-cli's HARDCODED_INI is not applied).
const SERVER_INI_DEFAULTS: &[(&str, &str)] = &[
    ("display_errors", "1"),
    ("html_errors", "1"),
    ("implicit_flush", "0"),
    ("max_execution_time", "30"),
];

impl ServerSapi {
    /// `php_cli_server_client_send_through`: the whole buffer, waiting up to
    /// 10 seconds whenever the socket is full. False: the client is gone.
    fn send_through(&mut self, mut data: &[u8]) -> bool {
        if self.closed {
            return false;
        }
        while !data.is_empty() {
            // SAFETY: sends from a live slice.
            let n = unsafe { libc::send(self.sock, data.as_ptr().cast(), data.len(), 0) };
            if n < 0 {
                if errno() == libc::EAGAIN {
                    let mut p = libc::pollfd {
                        fd: self.sock,
                        events: libc::POLLOUT,
                        revents: 0,
                    };
                    // SAFETY: one live pollfd.
                    if unsafe { libc::poll(&mut p, 1, 10_000) } > 0 {
                        continue;
                    }
                }
                self.closed = true;
                return false;
            }
            data = &data[n as usize..];
        }
        true
    }
}

impl Sapi for ServerSapi {
    fn name(&self) -> &str {
        "cli-server"
    }

    fn ini_defaults(&self) -> Vec<(&'static str, &'static str)> {
        SERVER_INI_DEFAULTS.to_vec()
    }

    fn ub_write(&mut self, data: &[u8]) -> bool {
        self.send_through(data)
    }

    /// `sapi_cli_server_flush`: a closed client is an abort; the headers go
    /// out if nothing sent them.
    fn flush(&mut self) -> bool {
        !self.closed
    }

    fn flush_sends_headers(&self) -> bool {
        true
    }

    /// `sapi_cli_server_send_headers` (or `sapi_cli_server_discard_headers`
    /// once the router declined a static file).
    fn send_headers(&mut self, r: &Response) -> bool {
        {
            let st = self.st.borrow();
            if st.discard_headers || st.headers_written {
                return true;
            }
        }
        let mut out = Vec::new();
        match &r.status_line {
            Some(line) => {
                out.extend_from_slice(line);
                out.extend_from_slice(b"\r\n");
            }
            None => status_line(&mut out, self.protocol_version, r.code),
        }
        essential_headers(&mut out, self.host.as_deref(), Some(&r.headers));
        for h in r.headers.iter().filter(|h| !h.is_empty()) {
            out.extend_from_slice(h);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        let sent = self.send_through(&out);
        self.st.borrow_mut().headers_written = true;
        sent
    }

    fn log_message(&mut self, msg: &[u8]) {
        log_write(LOG_MESSAGE, msg);
    }

    fn script_done(&mut self, done: ScriptDone<'_>) -> Option<Vec<u8>> {
        let mut st = self.st.borrow_mut();
        if st.router {
            // php_cli_server_dispatch_router: only an explicit false
            // declines.
            st.declined = matches!(done.ret, Some(Value::Bool(false)));
            if !st.declined {
                return None;
            }
            if st.is_static {
                done.response.send_default_content_type = false;
                st.discard_headers = true;
                return None;
            }
            st.router = false;
            let script = st.script.clone()?;
            if script.contains(&0) {
                // Paths with NUL bytes can't be handled: the error page is
                // logged, the request then ends as usual.
                let e = strerror(errno());
                log_response(&st.addr, (st.method, &st.uri), 400, Some(&e), None);
                return None;
            }
            return Some(script);
        }
        // php_cli_server_dispatch_script's log line, then the 304 rule.
        log_response(
            &st.addr,
            (st.method, &st.uri),
            done.response.code,
            None,
            done.last_error,
        );
        if done.response.code == 304 {
            done.response.send_default_content_type = false;
        }
        None
    }
}

/// The server's settings (`do_cli_server`'s options).
struct Settings {
    ini: Vec<(String, String)>,
    ini_files: IniFiles,
    php_binary: Option<Vec<u8>>,
}

/// `php_cli_server`.
struct Server {
    sock: i32,
    host: String,
    port: u16,
    document_root: Vec<u8>,
    router: Option<Vec<u8>>,
    clients: BTreeMap<i32, Client>,
    rfds: BTreeSet<i32>,
    wfds: BTreeSet<i32>,
    mime: HashMap<&'static [u8], &'static [u8]>,
    settings: Settings,
}

/// `php_cli_server_parse_addr`: `host:port` or `[host]:port`.
fn parse_addr(addr: &str) -> Option<(String, u16)> {
    let (host, port) = if let Some(rest) = addr.strip_prefix('[') {
        let end = rest.find(']')?;
        let after = &rest[end + 1..];
        (&rest[..end], after.strip_prefix(':')?)
    } else {
        let end = addr.find(':')?;
        (&addr[..end], &addr[end + 1..])
    };
    // strtol: leading whitespace and a sign, then digits to the end.
    let t = port.trim_start();
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = if digits.is_empty() {
        0
    } else {
        t.parse().ok()?
    };
    if !(0..=65535).contains(&n) {
        return None;
    }
    Some((host.to_string(), n as u16))
}

/// `php_network_listen_socket`: the first of the host's addresses that
/// binds, listening; the port is filled in when it was 0.
fn listen_socket(host: &str, port: &mut u16) -> Result<i32, String> {
    let chost = CString::new(host).map_err(|_| "?".to_string())?;
    // SAFETY: getaddrinfo on a valid C string; the list is freed below.
    unsafe {
        let mut hints: libc::addrinfo = std::mem::zeroed();
        let probe = libc::socket(libc::PF_INET6, libc::SOCK_DGRAM, 0);
        hints.ai_family = if probe < 0 {
            libc::AF_INET
        } else {
            libc::close(probe);
            libc::AF_UNSPEC
        };
        hints.ai_socktype = libc::SOCK_STREAM;
        let mut res: *mut libc::addrinfo = std::ptr::null_mut();
        let n = libc::getaddrinfo(chost.as_ptr(), std::ptr::null(), &hints, &mut res);
        if n != 0 {
            let e = std::ffi::CStr::from_ptr(libc::gai_strerror(n)).to_string_lossy();
            let msg = format!("php_network_getaddresses: getaddrinfo for {host} failed: {e}");
            eprintln!("PHP Warning:  {msg} in Unknown on line 0");
            return Err(msg);
        }
        let mut ai = res;
        let mut sock = -1;
        let mut err = 0;
        while !ai.is_null() {
            let a = &*ai;
            ai = a.ai_next;
            let s = libc::socket(a.ai_family, libc::SOCK_STREAM, 0);
            if s < 0 {
                continue;
            }
            let mut ss: libc::sockaddr_storage = std::mem::zeroed();
            let len = match a.ai_family {
                libc::AF_INET6 => {
                    let mut sa = *(a.ai_addr as *const libc::sockaddr_in6);
                    sa.sin6_port = port.to_be();
                    *(&mut ss as *mut _ as *mut libc::sockaddr_in6) = sa;
                    std::mem::size_of::<libc::sockaddr_in6>()
                }
                libc::AF_INET => {
                    let mut sa = *(a.ai_addr as *const libc::sockaddr_in);
                    sa.sin_port = port.to_be();
                    *(&mut ss as *mut _ as *mut libc::sockaddr_in) = sa;
                    std::mem::size_of::<libc::sockaddr_in>()
                }
                _ => {
                    libc::close(s);
                    continue;
                }
            } as libc::socklen_t;
            let one: libc::c_int = 1;
            libc::setsockopt(
                s,
                libc::SOL_SOCKET,
                libc::SO_REUSEADDR,
                (&one as *const libc::c_int).cast(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
            if libc::bind(s, (&ss as *const libc::sockaddr_storage).cast(), len) != 0 {
                err = errno();
                if err == libc::EINVAL || err == libc::EADDRINUSE {
                    libc::close(s);
                    libc::freeaddrinfo(res);
                    return Err(strerror(err));
                }
                libc::close(s);
                continue;
            }
            err = 0;
            if *port == 0 {
                let mut l = len;
                if libc::getsockname(s, (&mut ss as *mut libc::sockaddr_storage).cast(), &mut l)
                    != 0
                {
                    err = errno();
                    libc::close(s);
                    libc::freeaddrinfo(res);
                    return Err(strerror(err));
                }
                *port = match a.ai_family {
                    libc::AF_INET6 => {
                        u16::from_be((*(&ss as *const _ as *const libc::sockaddr_in6)).sin6_port)
                    }
                    _ => u16::from_be((*(&ss as *const _ as *const libc::sockaddr_in)).sin_port),
                };
            }
            sock = s;
            break;
        }
        libc::freeaddrinfo(res);
        if sock < 0 {
            return Err(if err != 0 {
                strerror(err)
            } else {
                "?".to_string()
            });
        }
        if libc::listen(sock, libc::SOMAXCONN) != 0 {
            let e = errno();
            libc::close(sock);
            return Err(strerror(e));
        }
        Ok(sock)
    }
}

fn set_nonblocking(fd: i32) -> bool {
    // SAFETY: fcntl on an fd we own.
    unsafe {
        let fl = libc::fcntl(fd, libc::F_GETFL);
        fl >= 0 && libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK) == 0
    }
}

/// `php_network_populate_name_from_sockaddr`: `ip:port`, `[ip6]:port`.
fn sockaddr_name(ss: &libc::sockaddr_storage) -> String {
    // SAFETY: the family says which struct ss holds.
    unsafe {
        match ss.ss_family as i32 {
            libc::AF_INET => {
                let sa = &*(ss as *const _ as *const libc::sockaddr_in);
                let ip = std::net::Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr));
                format!("{ip}:{}", u16::from_be(sa.sin_port))
            }
            libc::AF_INET6 => {
                let sa = &*(ss as *const _ as *const libc::sockaddr_in6);
                let ip = std::net::Ipv6Addr::from(sa.sin6_addr.s6_addr);
                format!("[{ip}]:{}", u16::from_be(sa.sin6_port))
            }
            _ => "-".to_string(),
        }
    }
}

/// `php_escape_html_entities_ex(uri, ENT_QUOTES, no double encoding)` in
/// UTF-8: an invalid string escapes to nothing.
fn escape_html(s: &[u8]) -> Vec<u8> {
    if std::str::from_utf8(s).is_err() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'&' => {
                if is_entity(&s[i + 1..]) {
                    out.push(b'&');
                } else {
                    out.extend_from_slice(b"&amp;");
                }
            }
            b'"' => out.extend_from_slice(b"&quot;"),
            b'\'' => out.extend_from_slice(b"&#039;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// After a `&`: an entity reference that is kept as is (`#123;`, `#x1f;`,
/// `name;`).
fn is_entity(s: &[u8]) -> bool {
    let Some(semi) = s.iter().position(|&b| b == b';') else {
        return false;
    };
    let e = &s[..semi];
    if let Some(num) = e.strip_prefix(b"#") {
        return match num.strip_prefix(b"x").or_else(|| num.strip_prefix(b"X")) {
            Some(h) => !h.is_empty() && h.iter().all(u8::is_ascii_hexdigit),
            None => !num.is_empty() && num.iter().all(u8::is_ascii_digit),
        };
    }
    !e.is_empty() && e[0].is_ascii_alphabetic() && e.iter().all(u8::is_ascii_alphanumeric)
}

impl Server {
    /// `php_cli_server_close_connection`.
    fn close_connection(&mut self, fd: i32) {
        if let Some(c) = self.clients.get(&fd) {
            logf(LOG_MESSAGE, format!("{} Closing", c.addr_str));
        }
        if self.clients.remove(&fd).is_some() {
            // SAFETY: the client's socket, closed once.
            unsafe {
                libc::shutdown(fd, libc::SHUT_RDWR);
                libc::close(fd);
            }
            self.rfds.remove(&fd);
            self.wfds.remove(&fd);
        }
    }

    /// `php_cli_server_send_error_page`.
    fn send_error_page(&mut self, fd: i32, status: i64, errstr: &str) {
        let Some(c) = self.clients.get_mut(&fd) else {
            return;
        };
        let status_string = status_string(status);
        let template: Option<String> = match status {
            400 => Some(format!("<h1>{status_string}</h1><p>Your browser sent a request that this server could not understand.</p>")),
            404 => None,
            405 => Some(format!("<h1>{status_string}</h1><p>Requested method not allowed.</p>")),
            500 => Some(format!("<h1>{status_string}</h1><p>The server is temporarily unavailable.</p>")),
            _ => Some(format!("<h1>{status_string}</h1><p>Request method not supported.</p>")),
        };
        let mut body = Vec::new();
        if c.request.method != method::HEAD {
            let uri = c.request.request_uri.clone().unwrap_or_default();
            let escaped = escape_html(&uri);
            let _ = write!(body, "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{status} {status_string}</title>");
            body.extend_from_slice(CSS.as_bytes());
            body.extend_from_slice(b"</head><body>");
            match template {
                Some(t) => body.extend_from_slice(t.as_bytes()),
                None => {
                    let _ = write!(
                        body,
                        "<h1>{status_string}</h1><p>The requested resource <code class=\"url\">"
                    );
                    body.extend_from_slice(&escaped);
                    body.extend_from_slice(b"</code> was not found on this server.</p>");
                }
            }
            body.extend_from_slice(b"</body></html>");
        }
        let mut head = Vec::new();
        status_line(&mut head, c.request.protocol_version, status);
        essential_headers(&mut head, c.request.header(b"host"), None);
        let _ = write!(
            head,
            "X-Powered-By: PHP/{}\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: {}\r\n",
            rhp_builtins::PHP_VERSION,
            body.len()
        );
        if status == 405 {
            head.extend_from_slice(b"Allow: GET, HEAD, POST\r\n");
        }
        head.extend_from_slice(b"\r\n");
        let mut chunks = VecDeque::new();
        chunks.push_back(head);
        if !body.is_empty() {
            chunks.push_back(body);
        }
        c.content = Some(chunks);
        let uri = c.request.request_uri.clone().unwrap_or_default();
        log_response(
            &c.addr_str,
            (method_str(c.request.method), &uri),
            status,
            Some(errstr),
            None,
        );
        self.wfds.insert(fd);
    }

    /// `php_cli_server_begin_send_static`. False: the connection is to be
    /// closed.
    fn begin_send_static(&mut self, fd: i32) -> bool {
        let Some(c) = self.clients.get_mut(&fd) else {
            return false;
        };
        let m = c.request.method;
        if m == method::DELETE || m == method::PUT || m == method::PATCH {
            self.send_error_page(fd, 405, &strerror(errno()));
            return true;
        }
        if c.request
            .path_translated
            .as_ref()
            .is_some_and(|p| p.contains(&0))
        {
            self.send_error_page(fd, 400, &strerror(errno()));
            return true;
        }
        let file = match &c.request.path_translated {
            Some(p) => std::fs::File::open(std::ffi::OsStr::from_bytes(p))
                .map_err(|e| strerror(e.raw_os_error().unwrap_or(0))),
            None => Err(strerror(errno())),
        };
        let file = match file {
            Ok(f) => f,
            Err(e) => {
                self.send_error_page(fd, 404, &e);
                return true;
            }
        };
        if m != method::HEAD {
            c.file = Some(file);
        }
        let ext = c.request.ext.to_ascii_lowercase();
        let mut head = Vec::new();
        status_line(&mut head, c.request.protocol_version, 200);
        essential_headers(&mut head, c.request.header(b"host"), None);
        if let Some(mime) = self.mime.get(&ext[..]) {
            head.extend_from_slice(b"Content-Type: ");
            head.extend_from_slice(mime);
            if mime.starts_with(b"text/") {
                head.extend_from_slice(b"; charset=UTF-8");
            }
            head.extend_from_slice(b"\r\n");
        }
        let _ = write!(head, "Content-Length: {}\r\n\r\n", c.request.size);
        c.content = Some(VecDeque::from([head]));
        let uri = c.request.request_uri.clone().unwrap_or_default();
        log_response(&c.addr_str, (method_str(m), &uri), 200, None, None);
        self.wfds.insert(fd);
        true
    }

    /// `php_cli_server_dispatch`: the router and/or the script, or a static
    /// file.
    fn dispatch(&mut self, fd: i32) {
        let Some(c) = self.clients.get(&fd) else {
            return;
        };
        let r = &c.request;
        let is_static = !r.ext.eq_ignore_ascii_case(b"php") || r.path_translated.is_none();
        if self.router.is_none() && is_static {
            if !self.begin_send_static(fd) {
                self.close_connection(fd);
            }
            return;
        }
        let st = Rc::new(RefCell::new(Dispatch {
            router: self.router.is_some(),
            is_static,
            script: r.path_translated.clone(),
            addr: c.addr_str.clone(),
            method: method_str(r.method),
            uri: r.request_uri.clone().unwrap_or_default(),
            ..Dispatch::default()
        }));
        let sapi = ServerSapi {
            sock: fd,
            protocol_version: r.protocol_version,
            host: r.header(b"host").map(<[u8]>::to_vec),
            st: st.clone(),
            closed: false,
        };
        let info = self.request_info(c);
        let (script, cwd) = match &self.router {
            Some(router) => (Script::File(lossy(router)), None),
            None => {
                let path = r.path_translated.clone().unwrap_or_default();
                if path.contains(&0) {
                    // php_cli_server_dispatch_script refuses it after the
                    // request started.
                    let mut d = st.borrow_mut();
                    d.router = true;
                    d.declined = true;
                    let e = strerror(errno());
                    log_response(&d.addr, (d.method, &d.uri), 400, Some(&e), None);
                    drop(d);
                    (
                        Script::Source {
                            path: String::new(),
                            bytes: Vec::new(),
                        },
                        None,
                    )
                } else {
                    let dir = match path.iter().rposition(|&b| b == b'/') {
                        Some(0) => b"/".to_vec(),
                        Some(i) => path[..i].to_vec(),
                        None => Vec::new(),
                    };
                    (Script::File(lossy(&path)), Some(dir))
                }
            }
        };
        let req = Request {
            script,
            ini: self.settings.ini.clone(),
            args: Vec::new(),
            info: Some(info),
            php_binary: self.settings.php_binary.clone(),
            cwd,
            ini_files: Some(self.settings.ini_files.clone()),
        };
        let _ = run_request(Box::new(sapi), req);
        let (declined, is_static) = {
            let d = st.borrow();
            (d.declined, d.is_static)
        };
        if self.router.is_some() && declined && is_static {
            if !self.begin_send_static(fd) {
                self.close_connection(fd);
            }
            return;
        }
        self.close_connection(fd);
    }

    /// `sapi_cli_server_register_variables` and the request info.
    fn request_info(&self, c: &Client) -> RequestInfo {
        let r = &c.request;
        let mut server: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut known = |k: &str, v: &[u8]| server.push((k.as_bytes().to_vec(), v.to_vec()));
        known("DOCUMENT_ROOT", &self.document_root);
        match c.addr_str.rfind(':') {
            Some(colon) => {
                let a = c.addr_str[..colon].trim_start_matches('[');
                let a = a.strip_suffix(']').unwrap_or(a);
                let port: String = c.addr_str[colon + 1..].chars().take(7).collect();
                known("REMOTE_ADDR", a.as_bytes());
                known("REMOTE_PORT", port.as_bytes());
            }
            None => known("REMOTE_ADDR", c.addr_str.as_bytes()),
        }
        known(
            "SERVER_SOFTWARE",
            format!("PHP/{} (Development Server)", rhp_builtins::PHP_VERSION).as_bytes(),
        );
        known(
            "SERVER_PROTOCOL",
            format!(
                "HTTP/{}.{}",
                r.protocol_version / 100,
                r.protocol_version % 100
            )
            .as_bytes(),
        );
        known("SERVER_NAME", self.host.as_bytes());
        known("SERVER_PORT", self.port.to_string().as_bytes());
        let uri = r.request_uri.clone().unwrap_or_default();
        known("REQUEST_URI", &uri);
        known("REQUEST_METHOD", method_str(r.method).as_bytes());
        let vpath = r.vpath.clone().unwrap_or_default();
        known("SCRIPT_NAME", &vpath);
        if let Some(pt) = &r.path_translated {
            known("SCRIPT_FILENAME", until_nul(pt));
        } else if let Some(router) = &self.router {
            known("SCRIPT_FILENAME", router);
        }
        if let Some(pi) = &r.path_info {
            known("PATH_INFO", pi);
        }
        match &r.path_info {
            Some(pi) if !pi.is_empty() => {
                let mut s = vpath.clone();
                s.extend_from_slice(pi);
                known("PHP_SELF", &s);
            }
            _ => known("PHP_SELF", &vpath),
        }
        // Through the input filter and php_register_variable's name rules.
        if let Some(q) = &r.query_string {
            register_variable(&mut server, b"QUERY_STRING", q);
        }
        for (k, v) in &r.headers {
            let key: Vec<u8> = k
                .iter()
                .map(|&b| {
                    if b == b'-' {
                        b'_'
                    } else {
                        b.to_ascii_uppercase()
                    }
                })
                .collect();
            if key == b"CONTENT_TYPE" || key == b"CONTENT_LENGTH" {
                register_variable(&mut server, &key, v);
            }
            let mut real = b"HTTP_".to_vec();
            real.extend_from_slice(&key);
            register_variable(&mut server, &real, v);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        use std::os::unix::ffi::OsStringExt;
        RequestInfo {
            server,
            env: std::env::vars_os()
                .map(|(k, v)| (k.into_vec(), v.into_vec()))
                .collect(),
            method: Some(method_str(r.method).as_bytes().to_vec()),
            query_string: r.query_string.clone(),
            cookie: r.header(b"cookie").map(<[u8]>::to_vec),
            content_type: r.header(b"content-type").map(<[u8]>::to_vec),
            body: r.content.clone().unwrap_or_default(),
            content_length: Some(r.content.as_ref().map_or(0, Vec::len) as i64),
            authorization: r.header(b"authorization").map(<[u8]>::to_vec),
            request_time: now,
            headers: r.headers_original_case.clone(),
            argv: None,
            no_headers: false,
            response_code: 200,
            proto_num: r.protocol_version as i64,
        }
    }

    /// `php_cli_server_recv_event_read_request`.
    fn read_request(&mut self, fd: i32) {
        let doc_root = self.document_root.clone();
        let Some(c) = self.clients.get_mut(&fd) else {
            return;
        };
        let outcome = if c.request_read {
            Ok(true)
        } else {
            let mut buf = [0u8; 16384];
            // SAFETY: receives into a live buffer.
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len() - 1, 0) };
            if n < 0 {
                let e = errno();
                if e == libc::EAGAIN {
                    Ok(false)
                } else {
                    Err(strerror(e))
                }
            } else if n == 0 {
                Err(UNEXPECTED_EOF.to_string())
            } else {
                let data = &buf[..n as usize];
                let mut parser = std::mem::take(&mut c.parser);
                let consumed = parser.execute(data, &mut |ev| c.on_event(ev, &doc_root));
                c.parser = parser;
                if consumed != data.len() {
                    Err(if data[0] & 0x80 != 0 || data[0] == 0x16 {
                        "Unsupported SSL request".to_string()
                    } else {
                        "Malformed HTTP request".to_string()
                    })
                } else {
                    Ok(c.request_read)
                }
            }
        };
        match outcome {
            Err(e) => {
                if e == UNEXPECTED_EOF && c.parser.at_request_start() {
                    logf(
                        LOG_MESSAGE,
                        format!(
                            "{} Closed without sending a request; it was probably just an unused speculative preconnection",
                            c.addr_str
                        ),
                    );
                } else {
                    logf(LOG_ERROR, format!("{} Invalid request ({e})", c.addr_str));
                }
                self.close_connection(fd);
            }
            Ok(true) => {
                if c.request.method == method::NOT_IMPLEMENTED {
                    self.send_error_page(fd, 501, &strerror(errno()));
                    return;
                }
                self.rfds.remove(&fd);
                self.dispatch(fd);
            }
            Ok(false) => {
                self.rfds.insert(fd);
            }
        }
    }

    /// `php_cli_server_send_event`.
    fn send_event(&mut self, fd: i32) {
        let Some(c) = self.clients.get_mut(&fd) else {
            return;
        };
        let Some(chunks) = c.content.as_mut() else {
            return;
        };
        if let Some(f) = c.file.as_mut() {
            if chunks.is_empty() {
                let mut buf = vec![0u8; 131072];
                match std::io::Read::read(f, &mut buf) {
                    Ok(n) => {
                        buf.truncate(n);
                        chunks.push_back(buf);
                        if n == 0 {
                            c.file = None;
                        }
                    }
                    Err(e) => {
                        logf(LOG_ERROR, strerror(e.raw_os_error().unwrap_or(0)));
                        self.close_connection(fd);
                        return;
                    }
                }
            }
        }
        while let Some(front) = chunks.front_mut() {
            // SAFETY: sends from a live buffer.
            let n = unsafe { libc::send(fd, front.as_ptr().cast(), front.len(), 0) };
            if n < 0 {
                let e = errno();
                if e != libc::EAGAIN {
                    self.close_connection(fd);
                    return;
                }
                break;
            }
            if n as usize == front.len() {
                chunks.pop_front();
            } else {
                front.drain(..n as usize);
                break;
            }
        }
        if chunks.is_empty() && c.file.is_none() {
            self.close_connection(fd);
        }
    }

    /// An event on the listening socket: accept one client.
    fn accept(&mut self) {
        // SAFETY: accept into a sockaddr_storage.
        let (s, ss) = unsafe {
            let mut ss: libc::sockaddr_storage = std::mem::zeroed();
            let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
            let s = libc::accept(
                self.sock,
                (&mut ss as *mut libc::sockaddr_storage).cast(),
                &mut len,
            );
            (s, ss)
        };
        if s < 0 {
            let e = errno();
            if e != libc::EAGAIN {
                logf(
                    LOG_ERROR,
                    format!("Failed to accept a client (reason: {})", strerror(e)),
                );
            }
            return;
        }
        if !set_nonblocking(s) {
            // SAFETY: our fresh socket.
            unsafe { libc::close(s) };
            return;
        }
        let c = Client::new(sockaddr_name(&ss));
        logf(LOG_MESSAGE, format!("{} Accepted", c.addr_str));
        self.clients.insert(s, c);
        self.rfds.insert(s);
    }

    /// `php_cli_server_do_event_loop`.
    fn run(&mut self) -> i32 {
        while RUNNING.load(Ordering::Relaxed) {
            let mut fds: BTreeMap<i32, i16> = BTreeMap::new();
            fds.insert(self.sock, libc::POLLIN);
            for &fd in &self.rfds {
                *fds.entry(fd).or_default() |= libc::POLLIN;
            }
            for &fd in &self.wfds {
                *fds.entry(fd).or_default() |= libc::POLLOUT;
            }
            let mut pfds: Vec<libc::pollfd> = fds
                .iter()
                .map(|(&fd, &events)| libc::pollfd {
                    fd,
                    events,
                    revents: 0,
                })
                .collect();
            // SAFETY: a live pollfd array.
            let n = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, 1000) };
            if n < 0 {
                let e = errno();
                if e != libc::EINTR {
                    logf(LOG_ERROR, strerror(e));
                    return 1;
                }
                continue;
            }
            for p in pfds.iter().filter(|p| p.revents != 0) {
                let readable = p.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0;
                let writable = p.revents & (libc::POLLOUT | libc::POLLHUP | libc::POLLERR) != 0;
                if p.fd == self.sock {
                    if readable {
                        self.accept();
                    }
                    continue;
                }
                if readable && p.events & libc::POLLIN != 0 {
                    self.read_request(p.fd);
                }
                if writable && p.events & libc::POLLOUT != 0 {
                    self.send_event(p.fd);
                }
            }
        }
        0
    }
}

/// The C string a path is up to its first NUL.
fn until_nul(b: &[u8]) -> &[u8] {
    match b.iter().position(|&c| c == 0) {
        Some(i) => &b[..i],
        None => b,
    }
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// `php_register_variable_safe`'s name rules for a server variable:
/// leading spaces dropped, spaces and dots made underscores; a name with
/// `[` (an array) or none left is skipped.
fn register_variable(server: &mut Vec<(Vec<u8>, Vec<u8>)>, key: &[u8], val: &[u8]) {
    let key: Vec<u8> = key
        .iter()
        .skip_while(|&&b| b == b' ')
        .map(|&b| if b == b' ' || b == b'.' { b'_' } else { b })
        .collect();
    if key.is_empty() || key.contains(&b'[') {
        return;
    }
    let val = until_nul(val);
    upsert(server, key, val.to_vec());
}

extern "C" fn sigint(_: libc::c_int) {
    RUNNING.store(false, Ordering::Relaxed);
}

/// `php_cli_server_startup_workers`: fork `PHP_CLI_SERVER_WORKERS`
/// processes that all serve the socket (the parent too). Returns the
/// children's pids, for the parent to wait for.
fn startup_workers() -> Vec<libc::pid_t> {
    let Ok(w) = std::env::var("PHP_CLI_SERVER_WORKERS") else {
        return Vec::new();
    };
    let max = rhp_builtins::c_atol(w.as_bytes());
    WORKERS_MAX.store(max, Ordering::Relaxed);
    if max <= 1 {
        eprintln!("number of workers must be larger than 1");
        return Vec::new();
    }
    // SAFETY: getpid has no preconditions.
    let master = unsafe { libc::getpid() };
    let mut pids = Vec::new();
    for i in 0..max {
        // SAFETY: the server has not started any thread.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            WORKERS_MAX.store(i + 1, Ordering::Relaxed);
            return pids;
        }
        if pid == 0 {
            #[cfg(target_os = "linux")]
            // SAFETY: plain prctl/getppid calls.
            unsafe {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                if libc::getppid() != master {
                    libc::_exit(1);
                }
            }
            let _ = master;
            return Vec::new();
        }
        pids.push(pid);
    }
    pids
}

/// `do_cli_server`: the options it reads (`-S`, `-t`, `-q`), the document
/// root and router, then the server until SIGINT. Returns the exit status.
pub(crate) fn main(
    args: &[String],
    ini: Vec<(String, String)>,
    ini_files: IniFiles,
    php_binary: Option<Vec<u8>>,
) -> i32 {
    let mut addr = String::new();
    let mut document_root: Option<String> = None;
    let mut g = PhpGetopt::new(args, crate::cli::OPTIONS);
    loop {
        match g.next(false) {
            Opt::End => break,
            Opt::Char(b'S', Some(a)) => addr = a,
            Opt::Char(b't', Some(t)) => document_root = Some(t),
            Opt::Char(b'q', _) => {
                let l = LOG_LEVEL.load(Ordering::Relaxed);
                if l > 1 {
                    LOG_LEVEL.store(l - 1, Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
    let document_root: Vec<u8> = match document_root {
        Some(d) => match stat(d.as_bytes()) {
            None => {
                eprintln!("Directory {d} does not exist.");
                return 1;
            }
            Some((false, ..)) => {
                eprintln!("{d} is not a directory.");
                return 1;
            }
            Some(_) => match std::fs::canonicalize(&d) {
                Ok(p) => p.as_os_str().as_bytes().to_vec(),
                Err(_) => d.into_bytes(),
            },
        },
        None => std::env::current_dir()
            .map(|p| p.as_os_str().as_bytes().to_vec())
            .unwrap_or_else(|_| b".".to_vec()),
    };
    let router = args.get(g.optind).map(|r| r.as_bytes().to_vec());

    // php_cli_server_ctor.
    let Some((host, mut port)) = parse_addr(&addr) else {
        eprintln!("Invalid address: {addr}");
        return 1;
    };
    let sock = match listen_socket(&host, &mut port) {
        Ok(s) => s,
        Err(e) => {
            let at = if host.contains(':') {
                format!("[{host}]:{port}")
            } else {
                format!("{host}:{port}")
            };
            logf(LOG_ERROR, format!("Failed to listen on {at} (reason: {e})"));
            return 1;
        }
    };
    if !set_nonblocking(sock) {
        logf(LOG_ERROR, "Failed to make server socket non-blocking");
        // SAFETY: our socket.
        unsafe { libc::close(sock) };
        return 1;
    }
    let workers = startup_workers();
    let color = ini
        .iter()
        .rev()
        .find(|(k, _)| k == "cli_server.color")
        .is_some_and(|(_, v)| rhp_builtins::ini_parse_bool(v.as_bytes()));
    COLOR.with(|c| c.set(color));
    let mut server = Server {
        sock,
        host: host.clone(),
        port,
        document_root,
        router,
        clients: BTreeMap::new(),
        rfds: BTreeSet::new(),
        wfds: BTreeSet::new(),
        mime: rhp_csrc::cli_server::mime_types().collect(),
        settings: Settings {
            ini,
            ini_files,
            php_binary,
        },
    };
    let (open, close) = if host.contains(':') {
        ("[", "]")
    } else {
        ("", "")
    };
    logf(
        LOG_PROCESS,
        format!(
            "PHP {} Development Server (http://{open}{host}{close}:{port}) started",
            rhp_builtins::PHP_VERSION
        ),
    );
    // SAFETY: installs a handler that only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGINT, sigint as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let r = server.run();
    // php_cli_server_dtor: the clients, the socket, then the workers.
    let fds: Vec<i32> = server.clients.keys().copied().collect();
    for fd in fds {
        server.clients.remove(&fd);
        // SAFETY: the client's socket.
        unsafe {
            libc::shutdown(fd, libc::SHUT_RDWR);
            libc::close(fd);
        }
    }
    // SAFETY: our socket.
    unsafe { libc::close(server.sock) };
    for pid in workers {
        let mut status = 0;
        // SAFETY: waits for our own child.
        unsafe { libc::waitpid(pid, &mut status, 0) };
    }
    r
}

#[cfg(test)]
mod tests;
