//! The ftp:// and ftps:// stream wrapper: a port of
//! ext/standard/ftp_fopen_wrapper.c over the socket transports. A file
//! stream is the data connection, with the control connection kept beside
//! it (`QUIT` on close); a directory is read from `NLST` when it is opened.

use rhp_values::{conv, Value};

use super::http_fopen_wrapper::{
    c_str, notify, progress_init, strip_url_passwd, NOTIFY_AUTH_REQUIRED, NOTIFY_AUTH_RESULT,
    NOTIFY_CONNECT, NOTIFY_FAILURE, NOTIFY_FILE_SIZE_IS,
};
use crate::ext::openssl::xp_ssl;
use crate::main::streams::plain_wrapper::FdStream;
use crate::main::streams::xp_socket::{default_socket_timeout, xport_client, Opened};
use crate::{Host, Thrown};

const SEVERITY_INFO: i64 = 0;
const SEVERITY_ERR: i64 = 2;
/// `sizeof(tmp_line)`.
const TMP_LINE: usize = 512;

/// Whether `url` is for this wrapper.
pub fn is_ftp(url: &[u8]) -> bool {
    let scheme = crate::main::streams::split_scheme(url).0;
    scheme.eq_ignore_ascii_case(b"ftp") || scheme.eq_ignore_ascii_case(b"ftps")
}

/// The control connection of an open ftp file stream: php's
/// `stream->wrapperthis`, sent `QUIT` when the data stream closes.
pub struct Control {
    stream: FdStream,
    /// Opened for writing: the transfer result is read first.
    write: bool,
}

impl Drop for Control {
    fn drop(&mut self) {
        // php_stream_ftp_stream_close
        if self.write {
            let mut line = Vec::new();
            let result = get_result(&mut self.stream, &mut line);
            if result != 226 && result != 250 {
                crate::main::streams::plain_wrapper::push_diag(
                    2,
                    true,
                    format!(
                        "FTP server error {result}:{}",
                        String::from_utf8_lossy(&line)
                    ),
                );
            }
        }
        self.stream.write(b"QUIT\r\n");
    }
}

/// One wrapper operation: the context and php_stream_wrapper_log_error's
/// list.
struct Op<'a> {
    host: &'a mut dyn Host,
    fname: &'a str,
    ctx: Value,
    errors: Vec<String>,
    /// REPORT_ERRORS: log the errors (the stat/mkdir/… openers pass 0).
    report: bool,
}

impl Op<'_> {
    fn log_error(&mut self, msg: impl Into<String>) {
        if self.report {
            self.errors.push(msg.into());
        }
    }

    fn opt(&self, name: &str) -> Option<Value> {
        super::http_fopen_wrapper::ctx_opt(self.host, &self.ctx, "ftp", name)
    }

    fn notify(
        &mut self,
        code: i64,
        severity: i64,
        msg: Option<&[u8]>,
        msg_code: i64,
    ) -> Result<(), Thrown> {
        notify(self.host, &self.ctx, code, severity, msg, msg_code, 0, 0)
    }

    /// `php_stream_xport_create("tcp://host:port")` with REPORT_ERRORS:
    /// `error` takes the failure text instead of the warning.
    fn connect(
        &mut self,
        host: &[u8],
        port: i64,
        error: Option<&mut Option<String>>,
    ) -> Result<Option<FdStream>, Thrown> {
        let mut name = b"tcp://".to_vec();
        name.extend_from_slice(c_str(host));
        name.extend_from_slice(format!(":{port}").as_bytes());
        let timeout = (default_socket_timeout(self.host), 0);
        let ctx = self.ctx.clone();
        match xport_client(self.host, self.fname, &name, true, false, timeout, &ctx)? {
            Opened::Ok(s) => {
                // the stream's resource
                self.host.consume_resource_ids(1);
                Ok(Some(*s))
            }
            Opened::Fail {
                text, allocated, ..
            } => {
                if allocated {
                    self.host.consume_resource_ids(1);
                }
                match error {
                    Some(e) => *e = text,
                    None => {
                        let t = text.as_deref().unwrap_or("Unspecified error");
                        crate::warn(self.host, self.fname, &format!("connect() failed: {t}"))?;
                    }
                }
                Ok(None)
            }
        }
    }

    /// `php_stream_xport_crypto_setup` + `_enable` (SSLv23 client).
    fn crypto(&mut self, s: &mut FdStream) -> Result<bool, Thrown> {
        let ctx = (!matches!(self.ctx, Value::Null)).then(|| self.ctx.clone());
        let ok = xp_ssl::crypto_on_connect(self.host, s, ctx.as_ref(), xp_ssl::SSLV23_CLIENT)?;
        crate::flush_stream_diags(self.host, self.fname)?;
        Ok(ok)
    }
}

/// `php_stream_write_string` on the control connection.
fn send(s: &mut FdStream, cmd: &[u8]) {
    s.write(cmd);
}

/// `get_ftp_result`: read lines until one starts with three digits and a
/// space; its code (`strtol`), the line left in `line`.
fn get_result(s: &mut FdStream, line: &mut Vec<u8>) -> i64 {
    line.clear();
    while let Some(l) = s.gets(Some(TMP_LINE - 1)) {
        *line = c_str(&l).to_vec();
        if line.len() > 3 && line[..3].iter().all(u8::is_ascii_digit) && line[3] == b' ' {
            break;
        }
    }
    strtol(line)
}

/// `strtol(s, NULL, 10)`, saturated to the int range used here.
fn strtol(s: &[u8]) -> i64 {
    let mut i = s
        .iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
        .count();
    let neg = s.get(i) == Some(&b'-');
    if matches!(s.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let v = s[i..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .fold(0i64, |a, &d| {
            a.saturating_mul(10).saturating_add((d - b'0') as i64)
        });
    if neg {
        -v
    } else {
        v
    }
}

/// `atoi`.
fn atoi(s: &[u8]) -> i64 {
    strtol(s) as i32 as i64
}

/// The parts of the URL php's ftp wrapper uses.
struct Resource {
    scheme: Option<Vec<u8>>,
    host: Option<Vec<u8>>,
    port: i64,
    user: Option<Vec<u8>>,
    pass: Option<Vec<u8>>,
    path: Option<Vec<u8>>,
}

fn parse(url: &[u8]) -> Option<Resource> {
    let u = crate::ext::standard::url::parse(url)?;
    Some(Resource {
        scheme: u.scheme,
        host: u.host,
        port: u.port as i64,
        user: u.user,
        pass: u.pass,
        path: u.path,
    })
}

/// What a successful login hands back.
struct Connected {
    stream: FdStream,
    resource: Resource,
    use_ssl_on_data: bool,
}

/// The `uri_parser_class` option check.
fn check_uri_parser(op: &mut Op) -> Result<(), Thrown> {
    match op.opt("uri_parser_class") {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Str(s)) if s.as_bytes() == b"parse_url" => Ok(()),
        Some(Value::Str(s))
            if s.as_bytes() == b"Uri\\Rfc3986\\Uri" || s.as_bytes() == b"Uri\\WhatWg\\Url" =>
        {
            Err(Thrown::unsupported(
                "ftp stream context option uri_parser_class",
                "ext/uri is not implemented",
            ))
        }
        Some(_) => Err(Thrown::error(
            "ValueError",
            format!(
                "{}(): Provided stream context has invalid value for the \"uri_parser_class\" option",
                op.fname
            ),
        )),
    }
}

/// `php_ftp_fopen_connect`: connect and log in. `Err` only for a thrown
/// exception; `Ok(Err(resource))` for a failure (the parsed URL when it had
/// a path).
fn fopen_connect(op: &mut Op, path: &[u8]) -> Result<Option<Connected>, Thrown> {
    check_uri_parser(op)?;
    let Some(mut resource) = parse(path) else {
        return Ok(None);
    };
    if resource.path.is_none() {
        return Ok(None);
    }
    let use_ssl = resource
        .scheme
        .as_ref()
        .is_some_and(|s| s.len() > 3 && s[3] == b's');
    if resource.port == 0 {
        resource.port = 21;
    }
    let host = resource.host.clone().unwrap_or_default();
    let Some(mut stream) = op.connect(&host, resource.port, None)? else {
        return Ok(None);
    };
    op.notify(NOTIFY_CONNECT, SEVERITY_INFO, None, 0)?;
    let mut line = Vec::new();
    let mut result = get_result(&mut stream, &mut line);
    if !(200..=299).contains(&result) {
        op.notify(NOTIFY_FAILURE, SEVERITY_ERR, Some(&line), result)?;
        return Ok(None);
    }
    let mut use_ssl_on_data = false;
    if use_ssl {
        send(&mut stream, b"AUTH TLS\r\n");
        result = get_result(&mut stream, &mut line);
        let mut reuse = false;
        if result != 234 {
            send(&mut stream, b"AUTH SSL\r\n");
            result = get_result(&mut stream, &mut line);
            if result != 334 {
                op.log_error("Server doesn't support FTPS.");
                return Ok(None);
            }
            reuse = true;
        }
        if !op.crypto(&mut stream)? {
            op.log_error("Unable to activate SSL mode");
            return Ok(None);
        }
        send(&mut stream, b"PBSZ 0\r\n");
        get_result(&mut stream, &mut line);
        send(&mut stream, b"PROT P\r\n");
        result = get_result(&mut stream, &mut line);
        use_ssl_on_data = (200..=299).contains(&result) || reuse;
    }
    // PHP_FTP_CNTRL_CHK
    let has_cntrl = |v: &[u8]| v.iter().any(|c| c.is_ascii_control());
    match &resource.user {
        Some(u) => {
            let u = c_str(&crate::main::php_variables::raw_url_decode(u)).to_vec();
            if has_cntrl(&u) {
                op.log_error(format!("Invalid login {}", String::from_utf8_lossy(&u)));
                return Ok(None);
            }
            let mut cmd = b"USER ".to_vec();
            cmd.extend_from_slice(&u);
            cmd.extend_from_slice(b"\r\n");
            send(&mut stream, &cmd);
        }
        None => send(&mut stream, b"USER anonymous\r\n"),
    }
    result = get_result(&mut stream, &mut line);
    if (300..=399).contains(&result) {
        op.notify(NOTIFY_AUTH_REQUIRED, SEVERITY_INFO, Some(&line), 0)?;
        let mut cmd = b"PASS ".to_vec();
        match &resource.pass {
            Some(p) => {
                let p = c_str(&crate::main::php_variables::raw_url_decode(p)).to_vec();
                if has_cntrl(&p) {
                    op.log_error(format!("Invalid password {}", String::from_utf8_lossy(&p)));
                    return Ok(None);
                }
                cmd.extend_from_slice(&p);
            }
            None => match op.host.ini_get(b"from").filter(|v| !v.is_empty()) {
                Some(from) => cmd.extend_from_slice(&from),
                None => cmd.extend_from_slice(b"anonymous"),
            },
        }
        cmd.extend_from_slice(b"\r\n");
        send(&mut stream, &cmd);
        result = get_result(&mut stream, &mut line);
        let sev = if (200..=299).contains(&result) {
            SEVERITY_INFO
        } else {
            SEVERITY_ERR
        };
        op.notify(NOTIFY_AUTH_RESULT, sev, Some(&line), result)?;
    }
    if !(200..=299).contains(&result) {
        return Ok(None);
    }
    Ok(Some(Connected {
        stream,
        resource,
        use_ssl_on_data,
    }))
}

/// `php_fopen_do_pasv`: the data port (0 on failure) and the host the
/// server gave.
fn do_pasv(s: &mut FdStream) -> (u16, Option<Vec<u8>>) {
    let mut line = Vec::new();
    // EPSV first (php is built with IPv6)
    send(s, b"EPSV\r\n");
    let result = get_result(s, &mut line);
    if result == 229 {
        // |||6446|
        let mut i = 0;
        let mut p = 4;
        while p < line.len() {
            if line[p] == b'|' {
                i += 1;
                if i == 3 {
                    break;
                }
            }
            p += 1;
        }
        if i < 3 {
            return (0, None);
        }
        return (strtoul(&line[p + 1..]).0 as u16, None);
    }
    send(s, b"PASV\r\n");
    if get_result(s, &mut line) != 227 {
        return (0, None);
    }
    // "227 Entering Passive Mode (h1,h2,h3,h4,p1,p2)"
    let mut t = 4.min(line.len());
    while t < line.len() && !line[t].is_ascii_digit() {
        t += 1;
    }
    if t >= line.len() {
        return (0, None);
    }
    let start = t;
    let mut ip = Vec::new();
    for _ in 0..4 {
        while t < line.len() && line[t].is_ascii_digit() {
            t += 1;
        }
        if line.get(t) != Some(&b',') {
            return (0, None);
        }
        t += 1;
    }
    for (k, &c) in line[start..t - 1].iter().enumerate() {
        let _ = k;
        ip.push(if c == b',' { b'.' } else { c });
    }
    // `char ip[sizeof("123.123.123.123")]`
    ip.truncate(15);
    let (hi, n) = strtoul(&line[t..]);
    let mut port = (hi as u16).wrapping_mul(256);
    t += n;
    if line.get(t) != Some(&b',') {
        return (0, None);
    }
    t += 1;
    port = port.wrapping_add(strtoul(&line[t..]).0 as u16);
    (port, Some(ip))
}

/// `strtoul(s, &end, 10)`: the value and the bytes it took.
fn strtoul(s: &[u8]) -> (u64, usize) {
    let mut i = s
        .iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
        .count();
    if matches!(s.get(i), Some(b'+')) {
        i += 1;
    }
    let n = s[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    if n == 0 {
        return (0, 0);
    }
    let v = s[i..i + n].iter().fold(0u64, |a, &d| {
        a.wrapping_mul(10).wrapping_add((d - b'0') as u64)
    });
    (v, i + n)
}

/// php_stream_display_wrapper_errors: "fname(url): caption: errors".
fn display_errors(op: &mut Op, url: &[u8], caption: &str) -> Result<(), Thrown> {
    let msg = if op.errors.is_empty() {
        "operation failed".to_string()
    } else {
        let html = op
            .host
            .ini_get(b"html_errors")
            .is_some_and(|v| conv::to_bool(&Value::str(v)));
        op.errors.join(if html { "<br />\n" } else { "\n" })
    };
    crate::warn_arg(
        op.host,
        op.fname,
        &strip_url_passwd(url),
        &format!("{caption}: {msg}"),
    )
}

/// `php_stream_url_wrap_ftp`: the data stream, its control connection
/// attached, as a resource; `None` after php's "Failed to open stream".
pub fn open(
    host: &mut dyn Host,
    fname: &str,
    url: &[u8],
    mode: &[u8],
    ctx: &Value,
) -> Result<Option<Value>, Thrown> {
    let mut op = Op {
        host,
        fname,
        ctx: ctx.clone(),
        errors: Vec::new(),
        report: true,
    };
    let stream = wrap(&mut op, url, mode)?;
    let Some(s) = stream else {
        display_errors(&mut op, url, "Failed to open stream")?;
        return Ok(None);
    };
    let ctx = (!matches!(ctx, Value::Null)).then(|| ctx.clone());
    Ok(op.host.stream_insert_fd(s, ctx))
}

fn wrap(op: &mut Op, url: &[u8], mode: &[u8]) -> Result<Option<FdStream>, Thrown> {
    let has = |cs: &[u8]| mode.iter().any(|c| cs.contains(c));
    let mut read_write = 0;
    if has(b"r+") {
        read_write = 1;
    }
    if has(b"wa+") {
        if read_write != 0 {
            op.log_error("FTP does not support simultaneous read/write connections");
            return Ok(None);
        }
        read_write = if mode.contains(&b'a') { 3 } else { 2 };
    }
    if read_write == 0 {
        op.log_error("Unknown file open mode");
        return Ok(None);
    }
    if op.opt("proxy").is_some() {
        if read_write == 1 {
            return Err(Thrown::unsupported(
                "the ftp context option proxy",
                "ftp through an http proxy is not implemented",
            ));
        }
        op.log_error("FTP proxy may only be used in read mode");
        return Ok(None);
    }
    let Some(c) = fopen_connect(op, url)? else {
        return Ok(None);
    };
    let Connected {
        mut stream,
        resource,
        use_ssl_on_data,
    } = c;
    let mut line = Vec::new();
    let mut result;
    let mut error_message: Option<String> = None;
    let path = resource.path.clone().unwrap_or_default();
    let fail = |op: &mut Op,
                line: &[u8],
                result: i64,
                error_message: Option<String>|
     -> Result<Option<FdStream>, Thrown> {
        op.notify(NOTIFY_FAILURE, SEVERITY_ERR, Some(line), result)?;
        if !line.is_empty() {
            op.log_error(format!(
                "FTP server reports {}",
                String::from_utf8_lossy(line)
            ));
        }
        if let Some(e) = error_message {
            op.log_error(format!("Failed to set up data channel: {e}"));
        }
        Ok(None)
    };
    let mut file_size = 0i64;
    'body: {
        send(&mut stream, b"TYPE I\r\n");
        result = get_result(&mut stream, &mut line);
        if !(200..=299).contains(&result) {
            break 'body;
        }
        let mut cmd = b"SIZE ".to_vec();
        cmd.extend_from_slice(c_str(&path));
        cmd.extend_from_slice(b"\r\n");
        send(&mut stream, &cmd);
        result = get_result(&mut stream, &mut line);
        if read_write == 1 {
            if !(200..=299).contains(&result) {
                crate::main::streams::set_errno(libc::ENOENT);
                break 'body;
            }
            if let Some(sp) = line.iter().position(|&c| c == b' ') {
                file_size = atoi(&line[sp + 1..]);
                let ln = line.clone();
                notify(
                    op.host,
                    &op.ctx,
                    NOTIFY_FILE_SIZE_IS,
                    SEVERITY_INFO,
                    Some(&ln),
                    result,
                    0,
                    file_size,
                )?;
            }
        } else if read_write == 2 && (200..=299).contains(&result) {
            let overwrite = op.opt("overwrite").is_some_and(|v| conv::to_bool(&v));
            if overwrite {
                let mut cmd = b"DELE ".to_vec();
                cmd.extend_from_slice(c_str(&path));
                cmd.extend_from_slice(b"\r\n");
                send(&mut stream, &cmd);
                result = get_result(&mut stream, &mut line);
                if !(200..=299).contains(&result) {
                    break 'body;
                }
            } else {
                op.log_error(
                    "Remote file already exists and overwrite context option not specified",
                );
                crate::main::streams::set_errno(libc::EEXIST);
                break 'body;
            }
        }
        let (portno, hoststart) = do_pasv(&mut stream);
        if portno == 0 {
            break 'body;
        }
        let verb: &[u8] = if read_write == 1 {
            if let Some(Value::Int(pos)) = op.opt("resume_pos") {
                if pos > 0 {
                    send(&mut stream, format!("REST {pos}\r\n").as_bytes());
                    result = get_result(&mut stream, &mut line);
                    if !(300..=399).contains(&result) {
                        op.log_error(format!("Unable to resume from offset {pos}"));
                        break 'body;
                    }
                }
            }
            b"RETR"
        } else if read_write == 2 {
            b"STOR"
        } else {
            b"APPE"
        };
        // the verb goes through tmp_line
        line = verb.to_vec();
        let mut cmd = verb.to_vec();
        cmd.push(b' ');
        cmd.extend_from_slice(c_str(&path));
        cmd.extend_from_slice(b"\r\n");
        send(&mut stream, &cmd);
        let data_host = hoststart.unwrap_or_else(|| resource.host.clone().unwrap_or_default());
        let Some(mut data) = op.connect(&data_host, portno as i64, Some(&mut error_message))?
        else {
            line.clear();
            break 'body;
        };
        result = get_result(&mut stream, &mut line);
        if result != 150 && result != 125 {
            break 'body;
        }
        progress_init(op.host, &op.ctx.clone(), 0, file_size)?;
        if use_ssl_on_data && !op.crypto(&mut data)? {
            op.log_error("Unable to activate SSL mode");
            line.clear();
            break 'body;
        }
        if let Some(sock) = data.sock_mut() {
            sock.wrapper = Some("ftp");
            sock.uri = Some(url.to_vec());
            sock.mode = Some(mode.to_vec());
        }
        data.ftp_control = Some(Box::new(Control {
            stream,
            write: read_write != 1,
        }));
        return Ok(Some(data));
    }
    fail(op, &line, result, error_message)
}

/// `php_stream_ftp_opendir` + its reads: the entries of the `NLST`
/// listing, or `None` after php's "Failed to open directory" warning.
pub fn opendir(
    host: &mut dyn Host,
    fname: &str,
    url: &[u8],
    ctx: &Value,
) -> Result<Option<Vec<Vec<u8>>>, Thrown> {
    let mut op = Op {
        host,
        fname,
        ctx: ctx.clone(),
        errors: Vec::new(),
        report: true,
    };
    let r = opendir_ex(&mut op, url)?;
    if r.is_none() {
        display_errors(&mut op, url, "Failed to open directory")?;
    }
    Ok(r)
}

fn opendir_ex(op: &mut Op, url: &[u8]) -> Result<Option<Vec<Vec<u8>>>, Thrown> {
    let Some(c) = fopen_connect(op, url)? else {
        return Ok(None);
    };
    let Connected {
        mut stream,
        resource,
        use_ssl_on_data,
    } = c;
    let mut line = Vec::new();
    let mut result;
    'body: {
        send(&mut stream, b"TYPE A\r\n");
        result = get_result(&mut stream, &mut line);
        if !(200..=299).contains(&result) {
            break 'body;
        }
        line.clear();
        let (portno, hoststart) = do_pasv(&mut stream);
        if portno == 0 {
            break 'body;
        }
        let data_host = hoststart.unwrap_or_else(|| resource.host.clone().unwrap_or_default());
        // php_stream_sock_open_host
        let Some(mut data) = op.connect(&data_host, portno as i64, None)? else {
            break 'body;
        };
        let mut cmd = b"NLST ".to_vec();
        cmd.extend_from_slice(c_str(resource.path.as_deref().unwrap_or(b"/")));
        cmd.extend_from_slice(b"\r\n");
        send(&mut stream, &cmd);
        result = get_result(&mut stream, &mut line);
        if result != 150 && result != 125 {
            break 'body;
        }
        if use_ssl_on_data && !op.crypto(&mut data)? {
            op.log_error("Unable to activate SSL mode");
            break 'body;
        }
        // the dir stream's resource
        op.host.consume_resource_ids(1);
        let mut entries = Vec::new();
        // php_ftp_dirstream_read
        while !data.eof() {
            let Some(l) = data.gets(Some(4096)) else {
                break;
            };
            let base = crate::ext::standard::string::php_basename(&l);
            let mut name = c_str(&base[..base.len().min(4095)]).to_vec();
            while matches!(name.last(), Some(b'\n' | b'\r' | b'\t' | b' ')) {
                name.pop();
            }
            entries.push(name);
        }
        return Ok(Some(entries));
    }
    let ln = line.clone();
    op.notify(NOTIFY_FAILURE, SEVERITY_ERR, Some(&ln), result)?;
    drop(stream);
    if !line.is_empty() {
        op.log_error(format!(
            "FTP server reports {}",
            String::from_utf8_lossy(&line)
        ));
    }
    Ok(None)
}

/// A connection for the stat/unlink/rename/mkdir/rmdir operations (no
/// errors logged).
fn quiet_op<'a>(host: &'a mut dyn Host, fname: &'a str, ctx: &Value) -> Op<'a> {
    Op {
        host,
        fname,
        ctx: ctx.clone(),
        errors: Vec::new(),
        report: false,
    }
}

/// `php_stream_ftp_url_stat`.
pub fn url_stat(
    host: &mut dyn Host,
    fname: &str,
    url: &[u8],
    ctx: &Value,
) -> Result<Option<crate::main::streams::StatInfo>, Thrown> {
    let mut op = quiet_op(host, fname, ctx);
    let Some(c) = fopen_connect(&mut op, url)? else {
        return Ok(None);
    };
    let Connected {
        mut stream,
        resource,
        ..
    } = c;
    let path = resource.path.clone().unwrap_or_else(|| b"/".to_vec());
    let path = c_str(&path);
    let mut line = Vec::new();
    let cmd = |verb: &str| [verb.as_bytes(), b" ", path, b"\r\n"].concat();
    let mut mode: u32 = 0o644;
    send(&mut stream, &cmd("CWD"));
    let result = get_result(&mut stream, &mut line);
    if !(200..=299).contains(&result) {
        mode |= libc::S_IFREG as u32;
    } else {
        mode |= (libc::S_IFDIR | libc::S_IXUSR | libc::S_IXGRP | libc::S_IXOTH) as u32;
    }
    send(&mut stream, b"TYPE I\r\n");
    let result = get_result(&mut stream, &mut line);
    if !(200..=299).contains(&result) {
        return Ok(None);
    }
    send(&mut stream, &cmd("SIZE"));
    let result = get_result(&mut stream, &mut line);
    let size = if !(200..=299).contains(&result) {
        if mode & libc::S_IFDIR as u32 != 0 {
            0
        } else {
            return Ok(None);
        }
    } else {
        atoi(line.get(4..).unwrap_or_default())
    };
    send(&mut stream, &cmd("MDTM"));
    let result = get_result(&mut stream, &mut line);
    let mtime = if result == 213 {
        mdtm(&line).unwrap_or(-1)
    } else {
        -1
    };
    let blksize = 4096;
    Ok(Some(crate::main::streams::StatInfo {
        mode,
        size: size as u64,
        mtime,
        atime: -1,
        ctime: -1,
        nlink: 1,
        rdev: -1,
        blksize,
        blocks: (4095 + size) / blksize,
        is_dir: mode & libc::S_IFMT as u32 == libc::S_IFDIR as u32,
        is_regular: mode & libc::S_IFMT as u32 == libc::S_IFREG as u32,
        ..Default::default()
    }))
}

/// The MDTM reply's time (`%4d%2d%2d%2d%2d%2d`, UTC) as php computes it.
fn mdtm(line: &[u8]) -> Option<i64> {
    let mut p = 4.min(line.len());
    while p < line.len() && !line[p].is_ascii_digit() {
        p += 1;
    }
    let mut fields = [0i32; 6];
    let widths = [4, 2, 2, 2, 2, 2];
    for (f, w) in fields.iter_mut().zip(widths) {
        // sscanf's %Nd: optional white space and sign, then up to N chars
        let rest = &line[p.min(line.len())..];
        let n = rest
            .iter()
            .take(w)
            .take_while(|b| b.is_ascii_digit())
            .count();
        if n == 0 {
            return None;
        }
        *f = std::str::from_utf8(&rest[..n]).ok()?.parse().ok()?;
        p += n;
    }
    // SAFETY: plain libc time conversions on local values.
    unsafe {
        let stamp = libc::time(std::ptr::null_mut());
        let mut tmbuf: libc::tm = std::mem::zeroed();
        let gmt = libc::gmtime_r(&stamp, &mut tmbuf);
        if gmt.is_null() {
            return None;
        }
        tmbuf.tm_isdst = -1;
        let off = stamp - libc::mktime(&mut tmbuf);
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = fields[0] - 1900;
        tm.tm_mon = fields[1] - 1;
        tm.tm_mday = fields[2];
        tm.tm_hour = fields[3];
        tm.tm_min = fields[4];
        tm.tm_sec = fields[5] + off as i32;
        tm.tm_isdst = tmbuf.tm_isdst;
        Some(libc::mktime(&mut tm) as i64)
    }
}

/// A one-command operation's connection, with php's "Unable to connect" /
/// "Invalid path provided" warnings: the stream and the URL's path.
fn op_connect(
    op: &mut Op,
    url: &[u8],
    report: bool,
) -> Result<Option<(FdStream, Vec<u8>)>, Thrown> {
    let Some(c) = fopen_connect(op, url)? else {
        if report {
            crate::warn(
                op.host,
                op.fname,
                &format!("Unable to connect to {}", String::from_utf8_lossy(url)),
            )?;
        }
        return Ok(None);
    };
    // fopen_connect fails without a path
    let path = c.resource.path.clone().unwrap_or_default();
    Ok(Some((c.stream, c_str(&path).to_vec())))
}

/// `php_stream_ftp_unlink`.
pub fn unlink(host: &mut dyn Host, fname: &str, url: &[u8], ctx: &Value) -> Result<bool, Thrown> {
    let mut op = quiet_op(host, fname, ctx);
    let Some((mut stream, path)) = op_connect(&mut op, url, true)? else {
        return Ok(false);
    };
    send(&mut stream, &[b"DELE ", &path[..], b"\r\n"].concat());
    let mut line = Vec::new();
    let result = get_result(&mut stream, &mut line);
    if !(200..=299).contains(&result) {
        crate::warn(
            op.host,
            fname,
            &format!("Error Deleting file: {}", String::from_utf8_lossy(&line)),
        )?;
        return Ok(false);
    }
    Ok(true)
}

/// `php_stream_ftp_rename`.
pub fn rename(
    host: &mut dyn Host,
    fname: &str,
    from: &[u8],
    to: &[u8],
    ctx: &Value,
) -> Result<bool, Thrown> {
    let mut op = quiet_op(host, fname, ctx);
    check_uri_parser(&mut op)?;
    let (Some(rf), Some(rt)) = (parse(from), parse(to)) else {
        return Ok(false);
    };
    let same = rf.scheme.is_some()
        && rf.scheme == rt.scheme
        && rf.host.is_some()
        && rf.host == rt.host
        && (rf.port == rt.port || rf.port * rt.port == 0 && rf.port + rt.port == 21)
        && rf.path.is_some()
        && rt.path.is_some();
    if !same {
        return Ok(false);
    }
    let Some(c) = fopen_connect(&mut op, from)? else {
        crate::warn(
            op.host,
            fname,
            &format!(
                "Unable to connect to {}",
                String::from_utf8_lossy(c_str(rf.host.as_deref().unwrap_or_default()))
            ),
        )?;
        return Ok(false);
    };
    let mut stream = c.stream;
    let mut line = Vec::new();
    send(
        &mut stream,
        &[
            b"RNFR ",
            c_str(rf.path.as_deref().unwrap_or_default()),
            b"\r\n",
        ]
        .concat(),
    );
    let result = get_result(&mut stream, &mut line);
    if !(300..=399).contains(&result) {
        crate::warn(
            op.host,
            fname,
            &format!("Error Renaming file: {}", String::from_utf8_lossy(&line)),
        )?;
        return Ok(false);
    }
    send(
        &mut stream,
        &[
            b"RNTO ",
            c_str(rt.path.as_deref().unwrap_or_default()),
            b"\r\n",
        ]
        .concat(),
    );
    let result = get_result(&mut stream, &mut line);
    if !(200..=299).contains(&result) {
        crate::warn(
            op.host,
            fname,
            &format!("Error Renaming file: {}", String::from_utf8_lossy(&line)),
        )?;
        return Ok(false);
    }
    Ok(true)
}

/// `php_stream_ftp_mkdir`.
pub fn mkdir(
    host: &mut dyn Host,
    fname: &str,
    url: &[u8],
    recursive: bool,
    ctx: &Value,
) -> Result<bool, Thrown> {
    let mut op = quiet_op(host, fname, ctx);
    let Some((mut stream, path)) = op_connect(&mut op, url, true)? else {
        return Ok(false);
    };
    let mut line = Vec::new();
    let mut result;
    if !recursive {
        send(&mut stream, &[b"MKD ", &path[..], b"\r\n"].concat());
        result = get_result(&mut stream, &mut line);
    } else {
        // buf with its '/' turned into NULs from the end, as php does
        let mut buf = path.clone();
        let e = buf.len();
        let cstr = |b: &[u8]| -> Vec<u8> { c_str(b).to_vec() };
        let mut p: Option<usize> = None;
        while let Some(i) = c_str(&buf).iter().rposition(|&c| c == b'/') {
            buf[i] = 0;
            let cur = cstr(&buf);
            let arg: &[u8] = if cur.is_empty() { b"/" } else { &cur };
            send(&mut stream, &[b"CWD ", arg, b"\r\n"].concat());
            result = get_result(&mut stream, &mut line);
            if (200..=299).contains(&result) {
                buf[i] = b'/';
                p = Some(i);
                break;
            }
            p = None;
        }
        let cur = cstr(&buf);
        let arg: &[u8] = if cur.is_empty() { b"/" } else { &cur };
        send(&mut stream, &[b"MKD ", arg, b"\r\n"].concat());
        result = get_result(&mut stream, &mut line);
        if (200..=299).contains(&result) {
            let mut q = p.unwrap_or(0);
            while q != e {
                if buf[q] == 0 && buf.get(q + 1).is_some_and(|&c| c != 0) {
                    buf[q] = b'/';
                    let cur = cstr(&buf);
                    send(&mut stream, &[b"MKD ", &cur[..], b"\r\n"].concat());
                    result = get_result(&mut stream, &mut line);
                    if !(200..=299).contains(&result) {
                        crate::warn(op.host, fname, &line[..])?;
                        break;
                    }
                }
                q += 1;
            }
        }
    }
    Ok((200..=299).contains(&result))
}

/// `php_stream_ftp_rmdir`.
pub fn rmdir(host: &mut dyn Host, fname: &str, url: &[u8], ctx: &Value) -> Result<bool, Thrown> {
    let mut op = quiet_op(host, fname, ctx);
    let Some((mut stream, path)) = op_connect(&mut op, url, true)? else {
        return Ok(false);
    };
    send(&mut stream, &[b"RMD ", &path[..], b"\r\n"].concat());
    let mut line = Vec::new();
    let result = get_result(&mut stream, &mut line);
    if !(200..=299).contains(&result) {
        crate::warn(op.host, fname, &line[..])?;
        return Ok(false);
    }
    Ok(true)
}
