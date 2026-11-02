//! php-cgi (`sapi/cgi/cgi_main.c`, plain CGI without FastCGI): the
//! request comes from the environment and stdin, the response goes to
//! stdout as `Status:` + headers + a blank line + the body.
//!
//! [`main`] is the `php-cgi` / `rhp --cgi` front end: php-cgi's option
//! handling, `init_request_info`'s PATH_INFO / SCRIPT_FILENAME fixing, the
//! `cgi.force_redirect` check, the chdir to the script's directory and the
//! command-line mode (`php-cgi script.php a=1 b=2`: a query string from
//! the arguments, no headers with `-q`).

use std::io::{Read, Write};

use rhp_builtins::sapi::{cgi_request_headers, RequestInfo, Response, Sapi};

use crate::{run_request, Outcome, Request, Script};

/// php-cgi's response writer: body bytes buffered like stdio's `stdout`
/// and flushed at `flush()` / request end.
#[derive(Default)]
pub struct CgiSapi {
    buf: Vec<u8>,
}

impl CgiSapi {
    /// A CGI SAPI writing to the process's stdout.
    pub fn new() -> Self {
        Self::default()
    }

    fn put(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= 1 << 16 {
            self.flush();
        }
    }
}

/// php-cgi's ini overrides of php's built-in defaults: the core defaults
/// (html_errors, implicit_flush, max_execution_time) instead of the CLI's,
/// and opcache built in but disabled.
pub const CGI_INI_DEFAULTS: &[(&str, &str)] = &[
    ("html_errors", "1"),
    ("implicit_flush", "0"),
    ("max_execution_time", "30"),
    ("opcache.enable", "0"),
];

fn ini_on(name: &[u8]) -> bool {
    let v = rhp_builtins::ini_value(name)
        .flatten()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(&v[..], b"1" | b"on" | b"yes" | b"true") || rhp_builtins::c_atol(&v) != 0
}

impl Sapi for CgiSapi {
    fn name(&self) -> &str {
        "cgi-fcgi"
    }

    fn ini_defaults(&self) -> Vec<(&'static str, &'static str)> {
        CGI_INI_DEFAULTS.to_vec()
    }

    fn ub_write(&mut self, data: &[u8]) {
        self.put(data);
    }

    fn flush(&mut self) {
        let mut so = std::io::stdout().lock();
        let _ = so.write_all(&self.buf);
        let _ = so.flush();
        self.buf.clear();
    }

    /// `sapi_cgi_send_headers`.
    fn send_headers(&mut self, r: &Response) {
        let mut out = Vec::new();
        let mut ignore_status = false;
        let mut response_status = r.code;
        if ini_on(b"cgi.nph") || r.code != 200 {
            let mut has_status = false;
            let mut line: Option<Vec<u8>> = None;
            if ini_on(b"cgi.rfc2616_headers") && r.status_line.is_some() {
                let sl = r.status_line.clone().unwrap_or_default();
                if let Some(sp) = sl.iter().position(|&b| b == b' ') {
                    response_status = rhp_builtins::c_atol(&sl[sp + 1..]);
                }
                line = Some(sl.into_iter().take(1024).collect());
            } else {
                let from_status_line = r.status_line.as_ref().and_then(|sl| {
                    let sp = sl.iter().position(|&b| b == b' ')?;
                    (sp >= 5 && sl[..5].eq_ignore_ascii_case(b"HTTP/")).then_some((sl, sp))
                });
                if let Some((sl, sp)) = from_status_line {
                    let mut l = b"Status:".to_vec();
                    l.extend_from_slice(&sl[sp..]);
                    response_status = rhp_builtins::c_atol(&sl[sp + 1..]);
                    line = Some(l);
                } else {
                    has_status = r
                        .headers
                        .iter()
                        .any(|h| h.len() > 7 && h[..7].eq_ignore_ascii_case(b"Status:"));
                    if !has_status {
                        line = Some(match Response::reason_phrase(r.code) {
                            Some(reason) => format!("Status: {} {reason}", r.code).into_bytes(),
                            None => format!("Status: {}", r.code).into_bytes(),
                        });
                    }
                }
            }
            if !has_status {
                if let Some(mut l) = line {
                    l.truncate(1023);
                    out.extend_from_slice(&l);
                    out.extend_from_slice(b"\r\n");
                    ignore_status = true;
                }
            }
        }
        for h in &r.headers {
            if h.is_empty() {
                continue;
            }
            if h.len() > 7 && h[..7].eq_ignore_ascii_case(b"Status:") {
                if !ignore_status {
                    ignore_status = true;
                    out.extend_from_slice(h);
                    out.extend_from_slice(b"\r\n");
                }
            } else if response_status == 304
                && h.len() > 13
                && h[..13].eq_ignore_ascii_case(b"Content-Type:")
            {
                continue;
            } else {
                out.extend_from_slice(h);
                out.extend_from_slice(b"\r\n");
            }
        }
        out.extend_from_slice(b"\r\n");
        self.put(&out);
    }
}

impl Drop for CgiSapi {
    fn drop(&mut self) {
        if !self.buf.is_empty() {
            self.flush();
        }
    }
}

/// The CGI environment as an ordered, editable list (php's `environ` plus
/// the `CGI_PUTENV` edits `init_request_info` makes).
#[derive(Clone, Default)]
pub struct CgiEnv(pub Vec<(Vec<u8>, Vec<u8>)>);

impl CgiEnv {
    /// The process environment.
    pub fn from_process() -> Self {
        use std::os::unix::ffi::OsStrExt;
        CgiEnv(
            std::env::vars_os()
                .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
                .collect(),
        )
    }

    pub fn get(&self, k: &str) -> Option<Vec<u8>> {
        self.0
            .iter()
            .find(|(n, _)| n == k.as_bytes())
            .map(|(_, v)| v.clone())
    }

    /// `setenv` (`Some`, replacing in place or appending) / `unsetenv`.
    pub fn put(&mut self, k: &str, v: Option<&[u8]>) -> Option<Vec<u8>> {
        match v {
            Some(v) => {
                if let Some(slot) = self.0.iter_mut().find(|(n, _)| n == k.as_bytes()) {
                    slot.1 = v.to_vec();
                } else {
                    self.0.push((k.as_bytes().to_vec(), v.to_vec()));
                }
                Some(v.to_vec())
            }
            None => {
                self.0.retain(|(n, _)| n != k.as_bytes());
                None
            }
        }
    }
}

/// `SG(request_info)` as `init_request_info` leaves it.
#[derive(Default, Debug)]
pub struct CgiRequest {
    /// The script to run (`SG(request_info).path_translated`).
    pub path_translated: Option<Vec<u8>>,
    /// `SG(request_info).request_uri` (PHP_SELF's base).
    pub request_uri: Option<Vec<u8>>,
    /// 404 when the path did not resolve to a file.
    pub response_code: i64,
    /// SCRIPT_FILENAME or PATH_TRANSLATED was set: only then does php
    /// take the method, query string and content type/length from the
    /// environment.
    pub from_env: bool,
}

fn is_regular_file(p: &[u8]) -> bool {
    use std::os::unix::ffi::OsStrExt;
    std::fs::metadata(std::ffi::OsStr::from_bytes(p)).is_ok_and(|m| m.is_file())
}

fn realpath_ok(p: &[u8]) -> bool {
    use std::os::unix::ffi::OsStrExt;
    std::fs::canonicalize(std::ffi::OsStr::from_bytes(p)).is_ok()
}

fn find(hay: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}

/// `is_valid_path`: refuse `..` path segments.
fn is_valid_path(p: &[u8]) -> bool {
    if p.starts_with(b"..") && (p.len() == 2 || p[2] == b'/') {
        return false;
    }
    let mut i = 0;
    while i < p.len() {
        if p[i] == b'/' {
            let rest = &p[i + 1..];
            if rest.starts_with(b"..") && (rest.len() == 2 || rest[2] == b'/') {
                return false;
            }
        }
        i += 1;
    }
    true
}

/// `init_request_info` (no FastCGI), editing `env` as php's CGI_PUTENV does.
pub fn init_request_info(env: &mut CgiEnv, fix_pathinfo: bool, discard_path: bool) -> CgiRequest {
    let mut req = CgiRequest {
        response_code: 200,
        ..Default::default()
    };
    let env_script_filename = env.get("SCRIPT_FILENAME");
    let env_path_translated = env.get("PATH_TRANSLATED");
    let mut script_path_translated = env_script_filename.clone();
    if script_path_translated.is_none() && env_path_translated.is_some() {
        script_path_translated = env_path_translated.clone();
    }
    let Some(_) = script_path_translated else {
        return req;
    };
    req.from_env = true;
    let mut env_path_info = env.get("PATH_INFO");
    let mut env_script_name = env.get("SCRIPT_NAME");
    if fix_pathinfo {
        let env_redirect_url = env.get("REDIRECT_URL");
        let env_document_root = env.get("DOCUMENT_ROOT");
        let orig_path_translated = env_path_translated.clone();
        let orig_path_info = env_path_info.clone();
        let orig_script_name = env_script_name.clone();
        let orig_script_filename = env_script_filename.clone();
        let mut script_name_redirected = false;
        if let (Some(pt), Some(ru)) = (&env_path_translated, &env_redirect_url) {
            if Some(pt) != script_path_translated.as_ref() {
                script_path_translated = Some(pt.clone());
                env_script_name = Some(ru.clone());
                script_name_redirected = true;
            }
        }
        let spt = script_path_translated.clone().unwrap_or_default();
        if !spt.is_empty() && (spt.ends_with(b"/") || !realpath_ok(&spt)) {
            let mut pt = spt.clone();
            let len = spt.len();
            let mut found = false;
            while let Some(slash) = pt.iter().rposition(|&b| b == b'/') {
                pt.truncate(slash);
                if is_regular_file(&pt) {
                    found = true;
                    let slen = len - pt.len();
                    // php compares the PATH_INFO suffix pointer with the
                    // original: they differ unless the stripped part is all
                    // of PATH_INFO.
                    if let Some(opi) = orig_path_info.as_ref().filter(|p| p.len() != slen) {
                        let cut = opi.len().saturating_sub(slen);
                        let head = opi[..cut].to_vec();
                        let tail = opi[cut..].to_vec();
                        env.put("ORIG_PATH_INFO", Some(opi));
                        if orig_script_name.as_deref() != Some(head.as_slice()) {
                            if let Some(osn) = &orig_script_name {
                                env.put("ORIG_SCRIPT_NAME", Some(osn));
                            }
                            req.request_uri = env.put("SCRIPT_NAME", Some(&head));
                        } else {
                            req.request_uri = orig_script_name.clone();
                        }
                        env_path_info = env.put("PATH_INFO", Some(&tail));
                    }
                    if orig_script_filename.as_deref() != Some(pt.as_slice()) {
                        if let Some(osf) = &orig_script_filename {
                            env.put("ORIG_SCRIPT_FILENAME", Some(osf));
                        }
                        script_path_translated = env.put("SCRIPT_FILENAME", Some(&pt));
                    }
                    if let Some(dr) = &env_document_root {
                        let mut l = dr.len();
                        if l > 0 && dr[l - 1] == b'/' {
                            l -= 1;
                        }
                        let mut new_pt = dr[..l].to_vec();
                        if let Some(pi) = &env_path_info {
                            new_pt.extend_from_slice(pi);
                        }
                        if let Some(opt) = &orig_path_translated {
                            env.put("ORIG_PATH_TRANSLATED", Some(opt));
                        }
                        env.put("PATH_TRANSLATED", Some(&new_pt));
                    } else if let Some(sn) = env_script_name.as_ref().filter(|sn| find(&pt, sn)) {
                        let ptlen = pt.len() - sn.len();
                        let mut new_pt = pt[..ptlen].to_vec();
                        if let Some(pi) = &env_path_info {
                            new_pt.extend_from_slice(pi);
                        }
                        if let Some(opt) = &orig_path_translated {
                            env.put("ORIG_PATH_TRANSLATED", Some(opt));
                        }
                        env.put("PATH_TRANSLATED", Some(&new_pt));
                    }
                    break;
                }
            }
            if !found {
                if let Some(osf) = &orig_script_filename {
                    env.put("ORIG_SCRIPT_FILENAME", Some(osf));
                }
                script_path_translated = env.put("SCRIPT_FILENAME", None);
                req.response_code = 404;
            }
            if req.request_uri.is_none() {
                if orig_script_name.is_none() || orig_script_name != env_script_name {
                    if let Some(osn) = &orig_script_name {
                        env.put("ORIG_SCRIPT_NAME", Some(osn));
                    }
                    req.request_uri = match &env_script_name {
                        Some(sn) => env.put("SCRIPT_NAME", Some(sn)),
                        None => env.put("SCRIPT_NAME", None),
                    };
                } else {
                    req.request_uri = orig_script_name.clone();
                }
            }
        } else {
            if orig_script_filename.is_none() || script_path_translated != orig_script_filename {
                if let Some(osf) = &orig_script_filename {
                    env.put("ORIG_SCRIPT_FILENAME", Some(osf));
                }
                let spt = script_path_translated.clone();
                script_path_translated = env.put("SCRIPT_FILENAME", spt.as_deref());
            }
            if env_redirect_url.is_some() {
                if let Some(opi) = &orig_path_info {
                    env.put("ORIG_PATH_INFO", Some(opi));
                    env.put("PATH_INFO", None);
                }
                if let Some(opt) = &orig_path_translated {
                    env.put("ORIG_PATH_TRANSLATED", Some(opt));
                    env.put("PATH_TRANSLATED", None);
                }
            }
            if script_name_redirected {
                if let Some(osn) = &orig_script_name {
                    env.put("ORIG_SCRIPT_NAME", Some(osn));
                }
                req.request_uri = match &env_script_name {
                    Some(sn) => env.put("SCRIPT_NAME", Some(sn)),
                    None => env.put("SCRIPT_NAME", None),
                };
            } else {
                req.request_uri = env_script_name.clone();
            }
        }
    } else {
        req.request_uri = env_path_info.clone().or_else(|| env_script_name.clone());
        if !discard_path && env_path_translated.is_some() {
            script_path_translated = env_path_translated.clone();
        }
    }
    if let Some(p) = script_path_translated.filter(|p| is_valid_path(p)) {
        req.path_translated = Some(p);
    }
    req
}

/// `sapi_cgi_register_variables`' PHP_SELF.
fn php_self(env: &CgiEnv, request_uri: Option<&[u8]>, fix_pathinfo: bool) -> Vec<u8> {
    if fix_pathinfo {
        match (env.get("PATH_INFO"), request_uri) {
            (Some(pi), Some(sn)) => {
                let mut v = sn.to_vec();
                v.extend_from_slice(&pi);
                v
            }
            (Some(pi), None) => pi,
            (None, Some(sn)) => sn.to_vec(),
            (None, None) => Vec::new(),
        }
    } else {
        request_uri.map(<[u8]>::to_vec).unwrap_or_default()
    }
}

fn usage() -> String {
    "Usage: php-cgi [-q] [-h] [-s] [-v] [-i] [-f <file>]\n       php-cgi <file> [args...]\n  -a               Run interactively\n  -b <address:port>|<port> Bind Path for external FASTCGI Server mode\n  -C               Do not chdir to the script's directory\n  -c <path>|<file> Look for php.ini file in this directory\n  -n               No php.ini file will be used\n  -d foo[=bar]     Define INI entry foo with value 'bar'\n  -e               Generate extended information for debugger/profiler\n  -f <file>        Parse <file>.  Implies `-q'\n  -h               This help\n  -i               PHP information\n  -l               Syntax check only (lint)\n  -m               Show compiled in modules\n  -q               Quiet-mode.  Suppress HTTP Header output.\n  -s               Display colour syntax highlighted source.\n  -v               Version number\n  -w               Display source with stripped comments and whitespace.\n  -T <count>       Measure execution time of script repeated <count> times.\n".to_string()
}

/// php-cgi's option table (`OPTIONS` in cgi_main.c): (char, needs a
/// value, long name).
const OPTIONS: &[(u8, bool, &str)] = &[
    (b'a', false, "interactive"),
    (b'b', true, "bindpath"),
    (b'C', false, "no-chdir"),
    (b'c', true, "php-ini"),
    (b'd', true, "define"),
    (b'e', false, "profile-info"),
    (b'f', true, "file"),
    (b'h', false, "help"),
    (b'i', false, "info"),
    (b'l', false, "syntax-check"),
    (b'm', false, "modules"),
    (b'n', false, "no-php-ini"),
    (b'q', false, "no-header"),
    (b's', false, "syntax-highlight"),
    (b's', false, "syntax-highlighting"),
    (b'w', false, "strip"),
    (b'?', false, "usage"),
    (b'v', false, "version"),
    (b'T', true, "timing"),
];

/// One `php_getopt` result.
enum Opt {
    /// No more options (`php_optind` is the first operand).
    End,
    /// An invalid option (reported on stderr when asked to).
    Invalid,
    /// An option and its value.
    Char(u8, Option<String>),
}

/// A port of php's `php_getopt` (main/getopt.c): bundled short options
/// (`-qC`), `-dfoo` / `-d foo` / `-d=foo` values, `--long[=value]` names,
/// and its error messages.
struct PhpGetopt<'a> {
    args: &'a [String],
    optind: usize,
    optchr: usize,
    dash: bool,
}

impl<'a> PhpGetopt<'a> {
    fn new(args: &'a [String]) -> Self {
        PhpGetopt {
            args,
            optind: 1,
            optchr: 0,
            dash: false,
        }
    }

    /// Byte `i` of argument `ind`, 0 past its end (a C string).
    fn at(&self, ind: usize, i: usize) -> u8 {
        self.args
            .get(ind)
            .and_then(|a| a.as_bytes().get(i).copied())
            .unwrap_or(0)
    }

    fn error(&self, ind: usize, chr: usize, what: &str, show: bool) -> Opt {
        if show {
            let c = self.at(ind, chr) as char;
            let msg = match what {
                "colon" => ": in flags".to_string(),
                "nf" => format!("option not found {c}"),
                _ => format!("no argument for option {c}"),
            };
            eprintln!("Error in argument {ind}, char {}: {msg}", chr + 1);
        }
        Opt::Invalid
    }

    fn next(&mut self, show: bool) -> Opt {
        let argc = self.args.len();
        if self.optind >= argc {
            return Opt::End;
        }
        let ind = self.optind;
        if !self.dash && (self.at(ind, 0) != b'-' || self.at(ind, 1) == 0) {
            return Opt::End;
        }
        let arg = self.args[ind].as_bytes();
        let mut idx: Option<usize> = None;
        let mut arg_start;
        if arg.len() >= 2 && arg[0] == b'-' && arg[1] == b'-' {
            if arg.len() == 2 {
                self.optind += 1;
                return Opt::End;
            }
            arg_start = 2;
            let mut arg_end = arg.len() - 1;
            match arg[arg_start..arg_end].iter().position(|&b| b == b'=') {
                Some(p) => {
                    arg_end = p;
                    arg_start += 1;
                }
                None => arg_end -= 1,
            }
            let name = &arg[2..(2 + arg_end).min(arg.len())];
            match OPTIONS.iter().position(|o| o.2.as_bytes() == name) {
                Some(i) => idx = Some(i),
                None => {
                    self.optind += 1;
                    return self.error(self.optind - 1, self.optchr, "arg", show);
                }
            }
            self.optchr = 0;
            self.dash = false;
            arg_start += OPTIONS[idx.unwrap_or(0)].2.len();
        } else {
            if !self.dash {
                self.dash = true;
                self.optchr = 1;
            }
            if self.at(ind, self.optchr) == b':' {
                self.dash = false;
                self.optind += 1;
                return self.error(self.optind - 1, self.optchr, "colon", show);
            }
            arg_start = 1 + self.optchr;
        }
        let i = match idx {
            Some(i) => i,
            None => {
                let c = self.at(ind, self.optchr);
                match OPTIONS.iter().position(|o| o.0 == c) {
                    Some(i) => i,
                    None => {
                        let (errind, errchr) = (ind, self.optchr);
                        if self.at(ind, self.optchr + 1) == 0 {
                            self.dash = false;
                            self.optind += 1;
                        } else {
                            self.optchr += 1;
                        }
                        return self.error(errind, errchr, "nf", show);
                    }
                }
            }
        };
        let (c, need_param, _) = OPTIONS[i];
        if need_param {
            self.dash = false;
            if self.at(ind, arg_start) == 0 {
                self.optind += 1;
                if self.optind == argc {
                    return self.error(self.optind - 1, self.optchr, "arg", show);
                }
                let v = self.args[self.optind].clone();
                self.optind += 1;
                return Opt::Char(c, Some(v));
            }
            if self.at(ind, arg_start) == b'=' {
                arg_start += 1;
            }
            let v = String::from_utf8_lossy(&arg[arg_start.min(arg.len())..]).into_owned();
            self.optind += 1;
            return Opt::Char(c, Some(v));
        }
        let long = arg.len() >= 2 && arg[0] == b'-' && arg[1] == b'-';
        if arg_start >= 2 && !long {
            if self.at(ind, self.optchr + 1) == 0 {
                self.dash = false;
                self.optind += 1;
            } else {
                self.optchr += 1;
            }
        } else {
            self.optind += 1;
        }
        Opt::Char(c, None)
    }
}

/// php-cgi's command line after its three `php_getopt` passes.
#[derive(Default)]
struct Opts {
    ini: Vec<(String, String)>,
    no_chdir: bool,
    no_headers: bool,
    script_file: Option<String>,
    /// Index of the first non-option argument.
    optind: usize,
}

/// php-cgi's option passes: `-d` (skipped for a `-…` query string), the
/// usage check (`-h`, invalid options; same skip), then in command-line
/// mode `-C` / `-q` / `-f`. `Err((stdout text, exit code))` to stop.
fn parse_options(args: &[String], cgi: bool, skip: bool) -> Result<Opts, (String, i32)> {
    let mut o = Opts::default();
    if !skip {
        let mut g = PhpGetopt::new(args);
        loop {
            match g.next(false) {
                Opt::End => break,
                Opt::Char(b'd', Some(kv)) => {
                    let (k, v) = kv.split_once('=').unwrap_or((&kv, "1"));
                    o.ini
                        .push((k.trim().to_string(), crate::ini_define_value(v)));
                }
                _ => {}
            }
        }
        let mut g = PhpGetopt::new(args);
        loop {
            match g.next(true) {
                Opt::End => break,
                Opt::Invalid => return Err((usage(), 1)),
                Opt::Char(b'h' | b'?', _) => return Err((usage(), 0)),
                _ => {}
            }
        }
    }
    let mut g = PhpGetopt::new(args);
    if !cgi {
        loop {
            match g.next(false) {
                Opt::End => break,
                Opt::Char(b'C', _) => o.no_chdir = true,
                Opt::Char(b'q', _) => o.no_headers = true,
                Opt::Char(b'f', v) => {
                    o.script_file = v;
                    o.no_headers = true;
                }
                Opt::Char(c @ (b'a' | b'i' | b'l' | b'm' | b's' | b'v' | b'w'), _) => {
                    eprintln!("rhp --cgi: option -{} is not supported", c as char);
                    return Err((String::new(), 1));
                }
                _ => {}
            }
        }
    }
    o.optind = g.optind;
    Ok(o)
}

const SECURITY_ALERT: &str = "<b>Security Alert!</b> The PHP CGI cannot be accessed directly.\n\n<p>This PHP CGI binary was compiled with force-cgi-redirect enabled.  This\nmeans that a page will only be served up if the REDIRECT_STATUS CGI variable is\nset, e.g. via an Apache Action directive.</p>\n<p>For more information as to <i>why</i> this behaviour exists, see the <a href=\"https://www.php.net/security.cgi-bin\">manual page for CGI security</a>.</p>\n<p>For more information about changing this behaviour or re-enabling this webserver,\nconsult the installation file that came with this distribution, or visit \n<a href=\"https://www.php.net/install.windows\">the manual page</a>.</p>\n";

/// Read a boolean ini setting from `-d` overrides, else its default.
fn opt_bool(ini: &[(String, String)], name: &str, default: bool) -> bool {
    match ini.iter().rev().find(|(k, _)| k == name) {
        Some((_, v)) => {
            let v = v.to_ascii_lowercase();
            matches!(v.as_str(), "1" | "on" | "yes" | "true")
                || rhp_builtins::c_atol(v.as_bytes()) != 0
        }
        None => default,
    }
}

fn opt_str(ini: &[(String, String)], name: &str) -> Option<String> {
    ini.iter()
        .rev()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
}

/// A response with no script behind it (the security alert, "No input
/// file specified."): headers plus `body`, through a fresh engine so the
/// header defaults (X-Powered-By, Content-type) match php's.
fn bare_response(ini: &[(String, String)], code: i64, body: &[u8], no_headers: bool) {
    let mut vm = rhp_vm::Vm::new();
    vm.set_sapi(Box::new(CgiSapi::new()));
    for (k, v) in ini {
        vm.set_ini(k.as_bytes(), v.as_bytes());
    }
    vm.set_request(RequestInfo {
        response_code: code,
        no_headers,
        ..Default::default()
    });
    vm.start_request();
    vm.write_output(body);
    vm.finish_request();
}

/// The `php-cgi` front end. `args[0]` is the program name; `env` the CGI
/// environment. Returns the process exit status.
pub fn main(args: &[String], env: CgiEnv) -> i32 {
    let mut env = env;
    let cgi = [
        "SERVER_SOFTWARE",
        "SERVER_NAME",
        "GATEWAY_INTERFACE",
        "REQUEST_METHOD",
    ]
    .iter()
    .any(|k| env.get(k).is_some());
    // A query string without '=' starting with '-' never reaches getopt
    // (CVE-2012-1823).
    let mut skip_getopt = false;
    if let Some(qs) = env.get("QUERY_STRING") {
        if !qs.contains(&b'=') {
            let decoded = rhp_builtins::sapi::url_decode(&qs);
            let first = decoded.iter().find(|&&b| b > b' ');
            if first == Some(&b'-') {
                skip_getopt = true;
            }
        }
    }
    let opts = match parse_options(args, cgi, skip_getopt) {
        Ok(o) => o,
        Err((msg, code)) => {
            print!("{msg}");
            let _ = std::io::stdout().flush();
            return code;
        }
    };
    let ini = opts.ini.clone();

    // cgi.force_redirect: only behind a server that sets REDIRECT_STATUS.
    if cgi && opt_bool(&ini, "cgi.force_redirect", true) {
        let var = opt_str(&ini, "cgi.redirect_status_env")
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "REDIRECT_STATUS".to_string());
        if env.get(&var).is_none() {
            // Before request startup: the body alone, no headers.
            bare_response(&ini, 400, SECURITY_ALERT.as_bytes(), true);
            return 255;
        }
    }

    let fix_pathinfo = opt_bool(&ini, "cgi.fix_pathinfo", true);
    let discard_path = opt_bool(&ini, "cgi.discard_path", false);
    let info = init_request_info(&mut env, fix_pathinfo, discard_path);
    let mut path_translated = info.path_translated.clone();
    let mut argv: Option<Vec<Vec<u8>>> = None;
    let from_env = |k: &str| if info.from_env { env.get(k) } else { None };
    let mut query_string = from_env("QUERY_STRING");
    let mut no_headers = false;
    if !cgi {
        no_headers = opts.no_headers;
        let rest = &args[opts.optind.min(args.len())..];
        if let Some(f) = &opts.script_file {
            path_translated = Some(f.clone().into_bytes());
            let mut a = vec![f.clone().into_bytes()];
            a.extend(rest.iter().map(|s| s.clone().into_bytes()));
            argv = Some(a);
        } else if let Some(first) = rest.first() {
            path_translated = Some(first.clone().into_bytes());
            argv = Some(rest.iter().map(|s| s.clone().into_bytes()).collect());
        }
        // All remaining arguments (the script name included when it was
        // not given by -f) form the query string.
        let qargs = rest;
        if query_string.is_none() && !qargs.is_empty() {
            let sep = opt_str(&ini, "arg_separator.input")
                .and_then(|s| s.chars().next())
                .unwrap_or('&');
            let joined: Vec<&str> = qargs.iter().map(String::as_str).collect();
            query_string = Some(joined.join(&sep.to_string()).into_bytes());
        }
    }
    let Some(script) = path_translated
        .clone()
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(&p).into_owned())
    else {
        if cgi {
            bare_response(&ini, 404, b"No input file specified.\n", no_headers);
            return 255;
        }
        // php-cgi with no script reads one from stdin: not supported.
        eprintln!("rhp --cgi: no script given");
        return 1;
    };
    // php_fopen_primary_script: a directory is no script either.
    let opened = std::fs::File::open(&script).and_then(|f| {
        if f.metadata()?.is_dir() {
            return Err(std::io::ErrorKind::IsADirectory.into());
        }
        Ok(f)
    });
    if let Err(e) = opened {
        let (code, msg): (i64, &[u8]) = if e.kind() == std::io::ErrorKind::PermissionDenied {
            (403, b"Access denied.\n")
        } else {
            (404, b"No input file specified.\n")
        };
        bare_response(&ini, code, msg, no_headers);
        return 255;
    }
    // The script was opened relative to the starting directory; the chdir
    // below must not change which file runs.
    let script = std::path::absolute(&script)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(script);

    // The request body: CONTENT_LENGTH bytes from stdin.
    let content_length = from_env("CONTENT_LENGTH")
        .map(|v| rhp_builtins::c_atol(&v))
        .unwrap_or(0);
    let mut body = Vec::new();
    if content_length > 0 {
        let _ = std::io::stdin()
            .lock()
            .take(content_length as u64)
            .read_to_end(&mut body);
    }
    let mut server = env.0.clone();
    server.push((
        b"PHP_SELF".to_vec(),
        php_self(&env, info.request_uri.as_deref(), fix_pathinfo),
    ));
    let request_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let request = RequestInfo {
        server,
        headers: cgi_request_headers(&env.0),
        env: env.0.clone(),
        method: from_env("REQUEST_METHOD"),
        query_string,
        cookie: env.get("HTTP_COOKIE"),
        content_type: info
            .from_env
            .then(|| env.get("CONTENT_TYPE").unwrap_or_default()),
        body,
        content_length: Some(content_length),
        authorization: from_env("HTTP_AUTHORIZATION"),
        request_time,
        argv,
        no_headers,
        response_code: info.response_code,
        proto_num: 1000,
    };
    // php_execute_script: chdir to the script's directory (unless -C).
    if !opts.no_chdir {
        if let Some(dir) = std::path::Path::new(&script).parent() {
            if !dir.as_os_str().is_empty() {
                let _ = std::env::set_current_dir(dir);
            }
        }
    }
    let req = Request {
        script: Script::File(script),
        ini,
        args: Vec::new(),
        info: Some(request),
        php_binary: None,
    };
    match run_request(Box::new(CgiSapi::new()), req) {
        Outcome::Exit(code) => code,
        Outcome::CannotRead { .. } => 255,
    }
}
