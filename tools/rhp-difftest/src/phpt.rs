//! Differential runner over php-src's `.phpt` test corpus, with mutation.
//!
//! Each test's `--FILE--` runs through the real `php` (with run-tests.php's INI
//! defaults) and through the built `rhp` binary, in place inside a scratch copy
//! of the test tree so `include "foo.inc"` and friends resolve. Outputs are
//! compared the way run-tests.php compares them (CRLF-normalised, trimmed).
//! php — not the test's `--EXPECT--` — is the oracle, so `EXPECTF` patterns,
//! platform quirks and the host php's exact build are all handled for free.
//!
//! Results are classified and clustered so the biggest gaps surface first. A
//! *known-passing* list ratchets progress: `--check-known` fails if a listed
//! test stops matching. On top of that, `--mutate N` takes every passing test
//! and runs `N` first-order mutants of it ([`crate::mutate`]) through both
//! engines — probing the edges of features that already work — and
//! `--emit-fixtures` writes minimised, self-contained repros of mutant
//! divergences for triage into the golden suite.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::{mutate, php_bin, read_known, Rng};

// ---------------------------------------------------------------------------
// .phpt parsing
// ---------------------------------------------------------------------------

/// A parsed `.phpt` file: section name → raw body bytes.
pub struct Phpt {
    sections: HashMap<String, Vec<u8>>,
}

impl Phpt {
    pub fn parse(data: &[u8]) -> Phpt {
        let mut sections = HashMap::new();
        let mut current: Option<String> = None;
        let mut body = Vec::new();
        for line in data.split_inclusive(|&b| b == b'\n') {
            if let Some(name) = section_header(line) {
                if let Some(cur) = current.take() {
                    sections.insert(cur, std::mem::take(&mut body));
                }
                current = Some(name);
            } else if current.is_some() {
                body.extend_from_slice(line);
            }
        }
        if let Some(cur) = current {
            sections.insert(cur, body);
        }
        Phpt { sections }
    }

    pub fn section(&self, name: &str) -> Option<&[u8]> {
        self.sections.get(name).map(Vec::as_slice)
    }

    pub fn has(&self, name: &str) -> bool {
        self.sections.contains_key(name)
    }

    /// run-tests.php's `sectionNotEmpty()`: PHP's `!empty()` on the raw body.
    fn not_empty(&self, name: &str) -> bool {
        self.section(name)
            .is_some_and(|b| !b.is_empty() && b != b"0")
    }

    /// run-tests.php's `TestFile::isCGI()`: the test runs under `php-cgi -C`.
    pub fn is_cgi(&self) -> bool {
        self.has("CGI") || CGI_SECTIONS.iter().any(|s| self.not_empty(s))
    }
}

/// Sections whose non-empty presence makes run-tests.php use the CGI SAPI.
const CGI_SECTIONS: &[&str] = &[
    "GET",
    "POST",
    "GZIP_POST",
    "DEFLATE_POST",
    "POST_RAW",
    "PUT",
    "COOKIE",
    "EXPECTHEADERS",
];

/// `--NAME--` on a line of its own (trailing whitespace allowed).
fn section_header(line: &[u8]) -> Option<String> {
    let inner = line
        .trim_ascii_end()
        .strip_prefix(b"--")?
        .strip_suffix(b"--")?;
    (!inner.is_empty() && inner.iter().all(|&b| b.is_ascii_uppercase() || b == b'_'))
        .then(|| String::from_utf8_lossy(inner).into_owned())
}

/// Sections whose semantics this runner doesn't reproduce (debugger,
/// redirection, timing-sensitive or resource-conflicting tests). Web input
/// (`--GET--`, `--POST--`, `--CGI--`, ...) runs through the CGI SAPI instead;
/// see [`Phpt::is_cgi`] and [`CgiRequest`].
const UNSUPPORTED_SECTIONS: &[&str] = &[
    "HEADERS",
    "PHPDBG",
    "REDIRECTTEST",
    "CAPTURE_STDIO",
    "FLAKY",
    "CONFLICTS",
];

/// run-tests.php's `$ini_overwrites` (php 8.5), verbatim.
const PHP_DEFAULT_INI: &[&str] = &[
    "output_handler=",
    "open_basedir=",
    "disable_functions=",
    "output_buffering=Off",
    "error_reporting=32767",
    "fatal_error_backtraces=Off",
    "display_errors=1",
    "display_startup_errors=1",
    "log_errors=0",
    "html_errors=0",
    "track_errors=0",
    "report_zend_debug=0",
    "docref_root=",
    "docref_ext=.html",
    "error_prepend_string=",
    "error_append_string=",
    "auto_prepend_file=",
    "auto_append_file=",
    "ignore_repeated_errors=0",
    "precision=14",
    "serialize_precision=-1",
    "memory_limit=128M",
    "opcache.fast_shutdown=0",
    "opcache.file_update_protection=0",
    "opcache.revalidate_freq=0",
    "opcache.jit_hot_loop=1",
    "opcache.jit_hot_func=1",
    "opcache.jit_hot_return=1",
    "opcache.jit_hot_side_exit=1",
    "opcache.jit_max_root_traces=100000",
    "opcache.jit_max_side_traces=100000",
    "opcache.jit_max_exit_counters=100000",
    "opcache.protect_memory=1",
    "zend.assertions=1",
    "zend.exception_ignore_args=0",
    "zend.exception_string_param_max_len=15",
    "short_open_tag=0",
    "date.timezone=UTC",
];

/// Environment run-tests.php provides by default (offline, non-interactive),
/// plus skipping slow and timing-sensitive tests, which would make a CI
/// ratchet flaky, and one locale for every machine: libc and ICU's default
/// locale (intl) come from it.
const BASE_ENV: &[(&str, &str)] = &[
    ("NO_INTERACTION", "1"),
    ("SKIP_ONLINE_TESTS", "1"),
    ("SKIP_SLOW_TESTS", "1"),
    ("SKIP_PERF_SENSITIVE", "1"),
    ("LC_ALL", "C"),
];

/// Tests that spawn "the php under test" as a subprocess. rhp's CLI doesn't
/// take php's flags, and pointing it at the real php would let rhp's run
/// delegate the work to php, so such tests are skipped rather than compared.
const SUBPROCESS_MARKERS: &[&str] = &[
    "TEST_PHP_EXECUTABLE",
    "TEST_PHP_CGI_EXECUTABLE",
    "TEST_PHP_EXTRA_ARGS",
    "PHP_BINARY",
];

/// Lines of output php's error display produces (stripped for the
/// diagnostics-insensitive comparison).
const DIAGNOSTIC_PREFIXES: &[&str] = &[
    "Warning: ",
    "Notice: ",
    "Deprecated: ",
    "Fatal error: ",
    "Parse error: ",
    "Recoverable fatal error: ",
    "PHP Warning:  ",
    "PHP Notice:  ",
    "PHP Deprecated:  ",
    "PHP Fatal error:  ",
    "PHP Parse error:  ",
];

/// Directives whose effect rhp implements: a test that sets one runs, and
/// rhp gets the setting as `-d` too. Every `opcache.*` directive also
/// reaches rhp (see [`opcache_skip`] for the ones whose tests can't match).
const RHP_INI: &[&str] = &[
    "date.timezone",
    "default_charset",
    "display_errors",
    "error_reporting",
    "html_errors",
    "iconv.input_encoding",
    "iconv.internal_encoding",
    "iconv.output_encoding",
    "include_path",
    "input_encoding",
    "internal_encoding",
    "max_memory_limit",
    "memory_hard_limit_percent",
    "memory_limit",
    "output_encoding",
    "pcre.backtrack_limit",
    "pcre.jit",
    "pcre.recursion_limit",
    "precision",
    "serialize_precision",
    // Removed from php long ago, still set by ext/iconv tests: php 8.5 and
    // rhp both ignore them.
    "iconv.http_input",
    "iconv.http_output",
    "iconv.internal_charset",
];

/// The `name` of a `name=value` setting.
fn ini_key(kv: &str) -> &str {
    kv.split_once('=').map_or(kv, |(k, _)| k).trim()
}

/// Why a test with this opcache setting can't be compared, if it can't.
/// rhp has a compiled-script cache but no JIT and no file cache. Opcode
/// dumps (`opcache.opt_debug_level`) belong on the ignore list; one that
/// isn't there yet (a new php-src test) gets its own skip reason.
fn opcache_skip(k: &str, v: &str) -> Option<&'static str> {
    let on = !matches!(v, "" | "0");
    match k {
        "opcache.opt_debug_level" if on => Some("opcode dump not on tests/phpt/ignored.txt"),
        "opcache.jit_debug" if on => Some("opcache:jit_debug"),
        "opcache.file_cache" | "opcache.file_cache_only" if on => Some("opcache:file_cache"),
        _ => None,
    }
}

/// `tests/phpt/ignored.txt`: tests whose expected output rhp can never
/// print. They never run.
const IGNORED: &str = include_str!("../../../tests/phpt/ignored.txt");

/// An ignore list: test path → reason. One test per line, the path, then
/// whitespace and the reason; blank lines and `#` lines are skipped.
fn parse_ignored(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (path, reason) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(p, r)| (p, r.trim()));
        if reason.is_empty() {
            return Err(format!("line {}: {path} has no reason", i + 1));
        }
        if out.insert(path.to_string(), reason.to_string()).is_some() {
            return Err(format!("line {}: {path} is listed twice", i + 1));
        }
    }
    Ok(out)
}

/// How a test's `--INI--` settings are judged, and which reach rhp.
struct IniRules {
    probe: IniProbe,
    /// `--rhp-ini`: directives treated as in [`RHP_INI`] for this run.
    extra: Vec<String>,
}

impl IniRules {
    /// Whether `kv` (`name=value`) is passed to rhp too.
    fn rhp_applies(&self, kv: &str) -> bool {
        let k = ini_key(kv);
        RHP_INI.contains(&k) || k.starts_with("opcache.") || self.extra.iter().any(|e| e == k)
    }

    /// A test's `--INI--` as php `-d` args, or why it can't run. A setting
    /// is fine when it is a no-op (it leaves php's effective value as
    /// run-tests' defaults have it) or rhp applies it. With `pass_through`
    /// (CGI tests, whose request layer takes `-d`), every setting is passed
    /// to both engines instead.
    fn test_ini(&self, phpt: &Phpt, dir: &Path, pass_through: bool) -> Result<Vec<String>, String> {
        let Some(body) = phpt.section("INI") else {
            return Ok(Vec::new());
        };
        let defaults: HashMap<&str, &str> = PHP_DEFAULT_INI
            .iter()
            .filter_map(|kv| kv.split_once('='))
            .collect();
        // run-tests' settings2array: `name=value` lines only, both sides
        // trimmed, the last value of a name kept.
        let mut settings: Vec<(String, String)> = Vec::new();
        for line in String::from_utf8_lossy(body).lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            let line = substitute_ini(line, dir)?;
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (php_trim_str(k), php_trim_str(v));
            match settings.iter_mut().find(|(n, _)| n == k) {
                Some(s) if k != "extension" && k != "zend_extension" => s.1 = v.to_string(),
                _ => settings.push((k.to_string(), v.to_string())),
            }
        }
        let mut out = Vec::new();
        for (k, v) in &settings {
            let line = format!("{k}={v}");
            let (k, v) = (k.as_str(), v.trim_matches('"'));
            if let Some(reason) = opcache_skip(k, v) {
                return Err(reason.into());
            }
            let noop = || {
                defaults.get(k) == Some(&v)
                    || defaults
                        .get(k)
                        .is_some_and(|d| ini_bool(d).is_some() && ini_bool(d) == ini_bool(v))
                    || self.probe.is_noop(k, &line)
            };
            if !pass_through && !self.rhp_applies(&line) && !noop() {
                return Err(format!("ini:{k}"));
            }
            out.push(line);
        }
        Ok(out)
    }
}

/// run-tests.php's `--INI--` substitutions: `{PWD}` (the test's directory),
/// `{TMP}` (the temp dir) and `{ENV:NAME}` (a test whose variable is unset
/// is skipped).
fn substitute_ini(line: &str, dir: &Path) -> Result<String, String> {
    let tmp = std::env::temp_dir();
    let tmp = tmp.to_string_lossy();
    let mut line = line
        .replace("{PWD}", &dir.to_string_lossy())
        .replace("{TMP}", tmp.trim_end_matches('/'));
    while let Some(start) = line.find("{ENV:") {
        let Some(len) = line[start..].find('}') else {
            break;
        };
        let name = &line[start + 5..start + len];
        let value = std::env::var(name).map_err(|_| format!("ini:env:{name}"))?;
        line.replace_range(start..start + len + 1, &value);
    }
    Ok(line)
}

/// php's effective value of a directive (`ini_get()`) under run-tests'
/// defaults, with and without a test's setting. A setting that leaves it
/// unchanged is a no-op: php behaves as without it, so rhp needn't get it.
/// The answer comes from the php under test (one `php -r` per directive and
/// per setting, cached for the run), so it is deterministic for a given
/// php. A directive php doesn't know (`ini_get()` is false) is never a
/// no-op: an extension php lacks may own it.
struct IniProbe {
    /// `None` in unit tests: no setting is a no-op by probing.
    php: Option<PathBuf>,
    defaults: Vec<String>,
    /// `directive` (the defaults) or `directive\nsetting line` → `ini_get()`
    /// as serialized by php.
    cache: Mutex<HashMap<String, Vec<u8>>>,
}

impl IniProbe {
    fn new(php: Option<PathBuf>, defaults: &[String]) -> IniProbe {
        IniProbe {
            php,
            defaults: defaults.to_vec(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn is_noop(&self, k: &str, line: &str) -> bool {
        if self.php.is_none() || k.is_empty() {
            return false;
        }
        let base = self.value(k, None);
        base.starts_with(b"s:") && base == self.value(k, Some(line))
    }

    fn value(&self, k: &str, line: Option<&str>) -> Vec<u8> {
        let key = match line {
            None => k.to_string(),
            Some(l) => format!("{k}\n{l}"),
        };
        if let Some(v) = self.cache.lock().unwrap().get(&key) {
            return v.clone();
        }
        let mut cmd = Command::new(self.php.as_ref().unwrap());
        cmd.arg("-n");
        for kv in self.defaults.iter().map(String::as_str).chain(line) {
            cmd.arg("-d").arg(kv);
        }
        cmd.args(["-r", "echo serialize(ini_get($argv[1]));", "--", k]);
        let v = cmd
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| o.stdout)
            .unwrap_or_default();
        self.cache.lock().unwrap().insert(key, v.clone());
        v
    }
}

/// A boolean-looking ini value (`0`/`Off`/`false`/`no`, `1`/`On`/…), so
/// `output_buffering=0` counts as the default `Off`.
fn ini_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "0" | "off" | "false" | "no" => Some(false),
        "1" | "on" | "true" | "yes" => Some(true),
        _ => None,
    }
}

/// A CGI test's request, built exactly as run-tests.php builds it
/// (`run-tests.php` around `$env['REDIRECT_STATUS'] = '1'`): the CGI
/// environment and the request body fed on stdin.
pub struct CgiRequest {
    pub env: BTreeMap<String, String>,
    pub body: Option<Vec<u8>>,
}

/// [`php_trim`] on a `str`.
fn php_trim_str(s: &str) -> &str {
    s.trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\0' | '\x0b'))
}

/// PHP's `trim()` character set.
fn php_trim(b: &[u8]) -> &[u8] {
    let ws = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\0' | 0x0b);
    let start = b.iter().position(|c| !ws(c)).unwrap_or(b.len());
    let end = b.iter().rposition(|c| !ws(c)).map_or(start, |e| e + 1);
    &b[start..end]
}

/// `--POST_RAW--` / `--PUT--`: a leading `Content-Type:` line becomes the
/// CONTENT_TYPE (unless the test's ENV set one); every other line is the body.
fn raw_request(section: &[u8], env: &mut BTreeMap<String, String>) -> Vec<u8> {
    let mut request = Vec::new();
    let mut started = false;
    for line in php_trim(section).split(|&b| b == b'\n') {
        if env.get("CONTENT_TYPE").is_none_or(|v| v.is_empty())
            && line.len() >= 13
            && line[..13].eq_ignore_ascii_case(b"content-type:")
        {
            let v = String::from_utf8_lossy(&line[13..]).replace('\r', "");
            env.insert("CONTENT_TYPE".into(), v.trim().to_string());
            continue;
        }
        if started {
            request.push(b'\n');
        }
        started = true;
        request.extend_from_slice(line);
    }
    request
}

/// A test's `--ENV--` exactly as run-tests.php reads it: `{PWD}` is the
/// test's directory, each line is trimmed and split at the first `=`.
fn run_tests_env(phpt: &Phpt, dir: &Path) -> Vec<(String, String)> {
    let Some(body) = phpt.section("ENV") else {
        return Vec::new();
    };
    String::from_utf8_lossy(body)
        .replace("{PWD}", &dir.to_string_lossy())
        .split('\n')
        .filter_map(|l| {
            let (k, v) = l.trim().split_once('=')?;
            (!k.is_empty() && k != "0").then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

/// Build a CGI test's environment and stdin. `test_env` is its `--ENV--`
/// section, which run-tests applies first (an ENV value wins wherever
/// run-tests only fills an empty variable).
pub fn cgi_request(
    phpt: &Phpt,
    script: &Path,
    test_env: &[(String, String)],
) -> Result<CgiRequest, String> {
    use flate2::write::{GzEncoder, ZlibEncoder};
    let mut env: BTreeMap<String, String> = [
        "REDIRECT_STATUS",
        "QUERY_STRING",
        "PATH_TRANSLATED",
        "SCRIPT_FILENAME",
        "REQUEST_METHOD",
        "CONTENT_TYPE",
        "CONTENT_LENGTH",
    ]
    .iter()
    .map(|k| (k.to_string(), String::new()))
    .collect();
    env.extend(test_env.iter().cloned());
    let empty = |env: &BTreeMap<String, String>, k: &str| env.get(k).is_none_or(|v| v.is_empty());
    let set = |env: &mut BTreeMap<String, String>, k: &str, v: String| {
        env.insert(k.to_string(), v);
    };
    let script = script.to_string_lossy().into_owned();
    set(&mut env, "REDIRECT_STATUS", "1".into());
    if empty(&env, "QUERY_STRING") {
        let q = phpt.section("GET").map(php_trim).unwrap_or_default();
        set(
            &mut env,
            "QUERY_STRING",
            String::from_utf8_lossy(q).into_owned(),
        );
    }
    for k in ["PATH_TRANSLATED", "SCRIPT_FILENAME"] {
        if empty(&env, k) {
            set(&mut env, k, script.clone());
        }
    }
    let cookie = phpt.section("COOKIE").map(php_trim).unwrap_or_default();
    set(
        &mut env,
        "HTTP_COOKIE",
        String::from_utf8_lossy(cookie).into_owned(),
    );

    let compress = |raw: &[u8], gzip: bool| -> Vec<u8> {
        let level = flate2::Compression::new(9);
        if gzip {
            let mut e = GzEncoder::new(Vec::new(), level);
            let _ = e.write_all(raw);
            e.finish().unwrap_or_default()
        } else {
            let mut e = ZlibEncoder::new(Vec::new(), level);
            let _ = e.write_all(raw);
            e.finish().unwrap_or_default()
        }
    };
    let urlencoded = "application/x-www-form-urlencoded".to_string();
    let body = if phpt.not_empty("POST_RAW") || phpt.not_empty("PUT") {
        let put = !phpt.not_empty("POST_RAW");
        let request = raw_request(
            phpt.section(if put { "PUT" } else { "POST_RAW" }).unwrap(),
            &mut env,
        );
        set(&mut env, "CONTENT_LENGTH", request.len().to_string());
        if put {
            set(&mut env, "REQUEST_METHOD", "PUT".into());
        } else if empty(&env, "REQUEST_METHOD") {
            set(&mut env, "REQUEST_METHOD", "POST".into());
        }
        if request.is_empty() {
            return Err("borked: empty request".into());
        }
        Some(request)
    } else if phpt.not_empty("POST") {
        let post = php_trim(phpt.section("POST").unwrap()).to_vec();
        set(&mut env, "REQUEST_METHOD", "POST".into());
        if empty(&env, "CONTENT_TYPE") {
            set(&mut env, "CONTENT_TYPE", urlencoded);
        }
        if empty(&env, "CONTENT_LENGTH") {
            set(&mut env, "CONTENT_LENGTH", post.len().to_string());
        }
        Some(post)
    } else if phpt.not_empty("GZIP_POST") || phpt.not_empty("DEFLATE_POST") {
        let gzip = phpt.not_empty("GZIP_POST");
        let raw = phpt
            .section(if gzip { "GZIP_POST" } else { "DEFLATE_POST" })
            .unwrap();
        let post = compress(php_trim(raw), gzip);
        let encoding = if gzip { "gzip" } else { "deflate" };
        set(&mut env, "HTTP_CONTENT_ENCODING", encoding.into());
        set(&mut env, "REQUEST_METHOD", "POST".into());
        set(&mut env, "CONTENT_TYPE", urlencoded);
        set(&mut env, "CONTENT_LENGTH", post.len().to_string());
        Some(post)
    } else {
        set(&mut env, "REQUEST_METHOD", "GET".into());
        set(&mut env, "CONTENT_TYPE", String::new());
        set(&mut env, "CONTENT_LENGTH", String::new());
        None
    };
    Ok(CgiRequest { env, body })
}

/// run-tests.php appends `-- <ARGS>` only to a GET request's command line;
/// a request with a body (POST, PUT, ...) gets the script alone.
fn cgi_passes_args(req: &CgiRequest) -> bool {
    req.body.is_none()
}

/// A CGI response split the way run-tests.php splits it
/// (`/^(.*?)\r?\n\r?\n(.*)/s`): `(headers, body)`, or `None` when there is
/// no blank line.
pub fn split_cgi(out: &[u8]) -> Option<(&[u8], &[u8])> {
    (0..out.len()).find_map(|i| {
        if out[i] != b'\n' {
            return None;
        }
        let head = &out[..i];
        let head = head.strip_suffix(b"\r").unwrap_or(head);
        match out.get(i + 1) {
            Some(b'\n') => Some((head, &out[i + 2..])),
            Some(b'\r') if out.get(i + 2) == Some(&b'\n') => Some((head, &out[i + 3..])),
            _ => None,
        }
    })
}

/// Rewrite php's `X-Powered-By: PHP/<php>` header to rhp's version, so a
/// patch-level skew between the host php and rhp doesn't fail every CGI test.
fn normalise_powered_by(out: &mut Vec<u8>, php: &str, rhp: &str) {
    let Some((head, _)) = split_cgi(out) else {
        return;
    };
    let from = format!("X-Powered-By: PHP/{php}");
    let to = format!("X-Powered-By: PHP/{rhp}");
    let head_len = head.len();
    if let Some(at) = out[..head_len]
        .windows(from.len())
        .position(|w| w == from.as_bytes())
    {
        out.splice(at..at + from.len(), to.bytes());
    }
}

// ---------------------------------------------------------------------------
// process execution
// ---------------------------------------------------------------------------

/// Cap on captured output per stream (the rest is drained and dropped).
const OUTPUT_CAP: usize = 8 << 20;

/// One engine run. `out` is stdout and stderr merged into one stream, exactly
/// like run-tests.php's `2>&1`.
struct Outcome {
    out: Vec<u8>,
    timed_out: bool,
    signaled: bool,
    /// The terminating signal, when the process was killed by one.
    signal: Option<i32>,
}

fn read_capped(mut r: impl Read, sink: &Mutex<Vec<u8>>) {
    let mut buf = [0u8; 64 * 1024];
    while let Ok(n) = r.read(&mut buf) {
        if n == 0 {
            break;
        }
        let mut out = sink.lock().unwrap();
        if out.len() < OUTPUT_CAP {
            let room = OUTPUT_CAP - out.len();
            out.extend_from_slice(&buf[..n.min(room)]);
        }
    }
}

fn kill_group(pgid: u32) {
    // SAFETY: kill(2) on a process group we created (pgid == the child's pid).
    unsafe { libc::kill(-(pgid as libc::pid_t), libc::SIGKILL) };
}

/// Run `cmd` to completion or `timeout`, with stdout and stderr on one pipe,
/// in its own process group. A process that outlives the child while holding
/// the pipe (a backgrounded grandchild) is killed with the group, and the
/// reader is only waited on briefly, so nothing can stall the harness.
fn run_proc(mut cmd: Command, stdin: Option<&[u8]>, timeout: Duration) -> Outcome {
    let failed = |e: std::io::Error| Outcome {
        out: format!("spawn failed: {e}").into_bytes(),
        timed_out: false,
        signaled: false,
        signal: None,
    };
    let (reader, writer) = match std::io::pipe() {
        Ok(p) => p,
        Err(e) => return failed(e),
    };
    let writer2 = match writer.try_clone() {
        Ok(w) => w,
        Err(e) => return failed(e),
    };
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(writer)
    .stderr(writer2)
    .process_group(0);
    let spawned = cmd.spawn();
    drop(cmd); // release our copies of the pipe's write end, or EOF never comes
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return failed(e),
    };
    let pgid = child.id();
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let input = input.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&input);
        });
    }
    let sink = std::sync::Arc::new(Mutex::new(Vec::new()));
    let reader_thread = {
        let sink = sink.clone();
        std::thread::spawn(move || read_capped(reader, &sink))
    };
    let deadline = Instant::now() + timeout;
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(st)) => break (Some(st), false),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            _ => {
                kill_group(pgid);
                break (child.wait().ok(), true);
            }
        }
    };
    let wait_reader = |grace: Duration| {
        let until = Instant::now() + grace;
        while !reader_thread.is_finished() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
        reader_thread.is_finished()
    };
    if !wait_reader(Duration::from_millis(200)) {
        // Something in the group still holds the pipe open.
        kill_group(pgid);
        wait_reader(Duration::from_millis(300)); // then give up on it
    }
    let out = sink.lock().unwrap().clone();
    let signal = if timed_out {
        None
    } else {
        status.and_then(|s| s.signal())
    };
    Outcome {
        out,
        timed_out,
        signaled: signal.is_some(),
        signal,
    }
}

// ---------------------------------------------------------------------------
// comparison + classification
// ---------------------------------------------------------------------------

/// Evidence that php itself misbehaved on a program: killed by a signal, a
/// sanitizer report (ASan/UBSan, from a sanitizer build), or the Zend
/// allocator detecting heap corruption. Returns a cluster key that groups
/// runs hitting the same underlying bug. A timeout is *not* evidence — a
/// mutant can legitimately loop forever.
fn php_anomaly(o: &Outcome) -> Option<String> {
    let text = String::from_utf8_lossy(&o.out);
    // Sanitizer reports are recognised by their exact shape, so a program that
    // merely prints the words (e.g. a test titled "zend_mm_heap corrupted")
    // isn't a finding: ASan's "==<pid>==ERROR: AddressSanitizer: ", UBSan's
    // "<file>.<ext>:<line>:<col>: runtime error: ".
    let asan = text
        .match_indices("==ERROR: AddressSanitizer: ")
        .find(|&(at, _)| {
            let line = text[..at].rsplit('\n').next().unwrap_or("");
            line.len() > 2
                && line.starts_with("==")
                && line[2..].bytes().all(|b| b.is_ascii_digit())
        });
    if let Some((at, m)) = asan {
        let report = &text[at + m.len()..];
        let kind = report.split_whitespace().next().unwrap_or("error");
        // Where a stack overflow is detected is arbitrary (it varies run to
        // run), so its top frame would split the cluster and the repro would
        // never "reproduce"; key it by kind alone.
        if kind == "stack-overflow" {
            return Some("asan stack-overflow".into());
        }
        // First stack frame outside the sanitizer runtime / libc wrappers.
        let runtime = |f: &str| {
            [
                "__asan",
                "__interceptor",
                "___interceptor",
                "__sanitizer",
                "__ubsan",
                "__libc",
            ]
            .iter()
            .any(|p| f.starts_with(p))
                || matches!(
                    f,
                    "malloc"
                        | "free"
                        | "calloc"
                        | "realloc"
                        | "memcpy"
                        | "memmove"
                        | "memset"
                        | "memcmp"
                        | "strlen"
                        | "strcmp"
                        | "strncmp"
                )
        };
        // (function, source file) of each stack frame.
        let frames: Vec<(&str, &str)> = report
            .lines()
            .filter_map(|l| {
                let l = l.trim_start();
                l.starts_with('#').then(|| l.split(" in ").nth(1))?
            })
            .filter_map(|rest| {
                let mut w = rest.split_whitespace();
                Some((w.next()?, w.next().unwrap_or("")))
            })
            .collect();
        // Prefer the first frame in php's own sources: interceptors (memchr,
        // strstr, ...) are too many to list and would merge unrelated bugs.
        let in_php = |file: &str| {
            ["/Zend/", "/ext/", "/main/", "/sapi/", "/TSRM/"]
                .iter()
                .any(|d| file.contains(d))
        };
        let frame = frames
            .iter()
            .find(|(_, file)| in_php(file))
            .or_else(|| frames.iter().find(|(f, _)| !runtime(f)))
            .map_or("?", |(f, _)| f);
        return Some(format!("asan {kind} in {frame}"));
    }
    let ubsan = text.lines().find_map(|l| {
        let (loc, msg) = l.split_once(": runtime error: ")?;
        let mut parts = loc.rsplitn(3, ':');
        let (col, line, file) = (parts.next()?, parts.next()?, parts.next()?);
        let num = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        let source = [".c", ".h", ".cc", ".cpp", ".y", ".re", ".l"]
            .iter()
            .any(|e| file.ends_with(e));
        (num(col) && num(line) && source).then_some((file, line, msg))
    });
    if let Some((file, line, msg)) = ubsan {
        let file = file.rsplit(['/', ' ']).next().unwrap_or(file);
        return Some(cluster_key(&format!("ubsan {file}:{line}: "), msg));
    }
    if text
        .lines()
        .any(|l| l.trim_end() == "zend_mm_heap corrupted")
    {
        return Some("zend_mm_heap corrupted".into());
    }
    o.signal.map(|sig| format!("php killed by signal {sig}"))
}

/// run-tests.php normalisation: CRLF → LF, then PHP `trim()`.
fn normalise(out: &[u8]) -> Vec<u8> {
    let mut s = Vec::with_capacity(out.len());
    let mut i = 0;
    while i < out.len() {
        if out[i] == b'\r' && out.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        s.push(out[i]);
        i += 1;
    }
    let ws = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\0' | 0x0b);
    let start = s.iter().position(|b| !ws(b)).unwrap_or(s.len());
    let end = s.iter().rposition(|b| !ws(b)).map_or(start, |e| e + 1);
    s[start..end].to_vec()
}

/// PHP's `preg_quote($s, '/')`.
fn preg_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '.' | '\\' | '+' | '*' | '?' | '[' | '^' | ']' | '$' | '(' | ')' | '{' | '}' | '='
            | '!' | '<' | '>' | '|' | ':' | '-' | '#' | '/' => {
                out.push('\\');
                out.push(c);
            }
            '\0' => out.push_str("\\000"),
            c => out.push(c),
        }
    }
    out
}

/// run-tests.php's `expectf_to_regex()`, verbatim: quote everything outside
/// `%r...%r` sections, then expand the `%` placeholders (one left-to-right
/// pass, like `strtr`).
fn expectf_to_regex(wanted: &str) -> String {
    let wanted = wanted.replace("\r\n", "\n");
    let mut quoted = String::new();
    let mut rest = wanted.as_str();
    while !rest.is_empty() {
        match rest.find("%r") {
            Some(start) => match rest[start + 2..].find("%r") {
                Some(len) => {
                    quoted.push_str(&preg_quote(&rest[..start]));
                    quoted.push('(');
                    quoted.push_str(&rest[start + 2..start + 2 + len]);
                    quoted.push(')');
                    rest = &rest[start + 2 + len + 2..];
                }
                None => {
                    // Unbalanced tag: run-tests quotes the rest, tag included.
                    quoted.push_str(&preg_quote(rest));
                    rest = "";
                }
            },
            None => {
                quoted.push_str(&preg_quote(rest));
                rest = "";
            }
        }
    }
    let mut out = String::with_capacity(quoted.len() * 2);
    let mut chars = quoted.chars().peekable();
    while let Some(c) = chars.next() {
        let expansion = match (c, chars.peek()) {
            ('%', Some('e')) => Some("\\/"),
            ('%', Some('s')) => Some("[^\\r\\n]+"),
            ('%', Some('S')) => Some("[^\\r\\n]*"),
            ('%', Some('a')) => Some(".+?"),
            ('%', Some('A')) => Some(".*?"),
            ('%', Some('w')) => Some("\\s*"),
            ('%', Some('i')) => Some("[+-]?\\d+"),
            ('%', Some('d')) => Some("\\d+"),
            ('%', Some('x')) => Some("[0-9a-fA-F]+"),
            ('%', Some('f')) => Some("[+-]?(?:\\d+|(?=\\.\\d))(?:\\.\\d+)?(?:[Ee][+-]?\\d+)?"),
            ('%', Some('c')) => Some("."),
            ('%', Some('0')) => Some("\\x00"),
            _ => None,
        };
        match expansion {
            Some(e) => {
                out.push_str(e);
                chars.next();
            }
            None => out.push(c),
        }
    }
    out
}

/// Does `out` satisfy the test's own `--EXPECTF--` / `--EXPECTREGEX--`, the
/// way run-tests.php checks it (`/^...$/s` on the trimmed output)? `None`
/// when the test has neither section or the pattern doesn't compile.
fn spec_matches(phpt: &Phpt, out: &[u8]) -> Option<bool> {
    let (wanted, is_f) = match (phpt.section("EXPECTF"), phpt.section("EXPECTREGEX")) {
        (Some(w), _) => (w, true),
        (None, Some(w)) => (w, false),
        (None, None) => return None,
    };
    let wanted = String::from_utf8_lossy(&normalise(wanted)).into_owned();
    let re = if is_f {
        expectf_to_regex(&wanted)
    } else {
        wanted
    };
    let re =
        rhp_builtins::pcre::Pattern::new(&format!("^{re}$"), rhp_builtins::pcre::DOTALL).ok()?;
    re.is_match(&normalise(out)).ok()
}

fn is_diagnostic(line: &str) -> bool {
    let l = line.trim_start();
    DIAGNOSTIC_PREFIXES.iter().any(|p| l.starts_with(p))
}

/// Output with php's diagnostic lines (and the blank lines around them) gone.
///
/// An uncaught exception's `Fatal error: Uncaught ...` is followed by its
/// `Stack trace:` / `#N ...` / `  thrown in ...` tail (and any chained `Next
/// ...` exceptions); those lines belong to the diagnostic too.
fn strip_diagnostics(out: &[u8]) -> String {
    let text = String::from_utf8_lossy(out);
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::new();
    let mut in_trace = false;
    for (i, l) in lines.iter().copied().enumerate() {
        // php prints a blank line before each diagnostic (`\nWarning: ...`);
        // any other blank line is real output.
        if l.trim().is_empty() {
            if in_trace || lines.get(i + 1).is_some_and(|n| is_diagnostic(n)) {
                continue;
            }
            kept.push(l);
            continue;
        }
        if is_diagnostic(l) {
            in_trace = l
                .trim_start()
                .trim_start_matches("PHP ")
                .starts_with("Fatal error:  Uncaught")
                || l.trim_start().starts_with("Fatal error: Uncaught");
            continue;
        }
        if in_trace
            && (l.starts_with("Stack trace:")
                || l.starts_with('#')
                || l.starts_with("  thrown in ")
                || l.starts_with("Next "))
        {
            continue;
        }
        in_trace = false;
        kept.push(l);
    }
    kept.join("\n")
}

/// Collapse paths and numbers so similar failures share a cluster key.
fn cluster_key(prefix: &str, s: &str) -> String {
    let mut out = String::from(prefix);
    for word in s.split(' ') {
        let mut last_digit = false;
        // Addresses (ASLR) differ on every run.
        if word.starts_with("0x") && word.len() > 2 {
            out.push_str("ADDR ");
            continue;
        }
        if word.contains('/') && word.len() > 1 {
            out.push_str("PATH ");
            continue;
        }
        for c in word.chars() {
            if c.is_ascii_digit() {
                if !last_digit {
                    out.push('N');
                }
                last_digit = true;
            } else {
                out.push(c);
                last_digit = false;
            }
        }
        out.push(' ');
    }
    out.trim_end().chars().take(110).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Status {
    Match,
    DiagnosticsOnly,
    Mismatch,
    Unsupported,
    RhpCrash,
    RhpTimeout,
    Skipped,
    /// On the ignore list ([`IGNORED`]): never run.
    Ignored,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Match => "match",
            Status::DiagnosticsOnly => "diagnostics-only",
            Status::Mismatch => "mismatch",
            Status::Unsupported => "unsupported",
            Status::RhpCrash => "rhp-crash",
            Status::RhpTimeout => "rhp-timeout",
            Status::Skipped => "skipped",
            Status::Ignored => "ignored",
        }
    }
}

struct Verdict {
    status: Status,
    /// Cluster key (non-match) or skip reason.
    key: String,
}

fn verdict(status: Status, key: impl Into<String>) -> Verdict {
    Verdict {
        status,
        key: key.into(),
    }
}

/// Classify one php/rhp run pair (each a merged stdout+stderr stream).
fn classify(php: &Outcome, rhp: &Outcome) -> Verdict {
    if php.timed_out {
        return verdict(Status::Skipped, "php-timeout");
    }
    if php.signaled {
        return verdict(Status::Skipped, "php-crash");
    }
    if rhp.timed_out {
        return verdict(Status::RhpTimeout, "rhp timeout");
    }
    let (p, r) = (
        String::from_utf8_lossy(&php.out),
        String::from_utf8_lossy(&rhp.out),
    );
    if let Some(line) = r
        .lines()
        .find(|l| l.contains("panicked at") || l.contains("overflowed its stack"))
    {
        return verdict(Status::RhpCrash, cluster_key("crash: ", line));
    }
    if rhp.signaled {
        return verdict(Status::RhpCrash, "crash: killed by signal");
    }
    if normalise(&php.out) == normalise(&rhp.out) {
        return verdict(Status::Match, "");
    }
    if let Some(msg) = r.lines().find_map(|l| l.split_once("RHP_UNSUPPORTED: ")) {
        let msg = msg.1.split(" (").next().unwrap_or(msg.1);
        return verdict(Status::Unsupported, cluster_key("unsupported: ", msg));
    }
    // Only php's diagnostics may be missing: if php printed none, any extra
    // rhp diagnostic (e.g. a fatal php doesn't hit) is a real divergence.
    if let Some(first) = p.lines().find(|l| is_diagnostic(l)) {
        if strip_diagnostics(&php.out) == strip_diagnostics(&rhp.out) {
            // Name the first diagnostic php printed, to rank the missing ones.
            let first = first.split(" in ").next().unwrap_or(first).trim();
            return verdict(Status::DiagnosticsOnly, cluster_key("diagnostics: ", first));
        }
    }
    if let Some(fatal) = r.lines().find(|l| l.starts_with("PHP Fatal error:")) {
        let fatal = fatal.trim_start_matches("PHP Fatal error:").trim();
        let fatal = fatal.trim_start_matches("Uncaught ");
        return verdict(Status::Mismatch, cluster_key("rhp fatal: ", fatal));
    }
    let (mut pl, mut rl) = (p.lines(), r.lines());
    let key = loop {
        match (pl.next(), rl.next()) {
            (Some(a), Some(b)) if a == b => continue,
            (a, b) => {
                let show =
                    |s: Option<&str>| s.unwrap_or("<eof>").chars().take(48).collect::<String>();
                break format!("php «{}» rhp «{}»", show(a), show(b));
            }
        }
    };
    verdict(Status::Mismatch, cluster_key("diff: ", &key))
}

// ---------------------------------------------------------------------------
// running a test
// ---------------------------------------------------------------------------

/// The two CGI engines: the reference php-cgi and the candidate command
/// (default `<rhp> --cgi`); both get run-tests.php's CGI arguments appended.
#[derive(Clone)]
struct CgiEngines {
    php_cgi: PathBuf,
    candidate: Vec<String>,
}

#[derive(Clone)]
struct Ctx {
    rhp_bin: PathBuf,
    /// [`BASE_ENV`], for every engine run.
    base_env: Vec<(String, String)>,
    php_ini: Vec<String>,
    php_modules: BTreeSet<String>,
    timeout: Duration,
    mutate: usize,
    /// Mutate every test, not only those rhp matches. Mutants of a test rhp
    /// doesn't match yet run through php only — they hunt php bugs.
    mutate_all: bool,
    seed: u64,
    fixtures: Option<PathBuf>,
    /// Where to write repros of runs in which php itself misbehaved.
    php_findings: Option<PathBuf>,
    minimize: bool,
    /// `(php's PHP_VERSION, rhp's)`, when they differ: a test whose php output
    /// contains php's is printing the engine version, which can't match (and
    /// a CGI response's `X-Powered-By` is rewritten to rhp's).
    version_skew: Option<(String, String)>,
    /// The CGI engines, when php-cgi was found (CGI tests skip otherwise).
    cgi: Option<CgiEngines>,
    /// Pause between the php and rhp runs of a test. The `--write-known`
    /// confirmation pass sets it past a second, so any test whose output
    /// depends on the current time diverges deterministically.
    skew: Duration,
    /// Which `--INI--` settings run, and which reach rhp.
    ini: std::sync::Arc<IniRules>,
}

/// Everything needed to run one script through both engines.
struct Invocation<'a> {
    dir: &'a Path,
    script: &'a Path,
    ini: &'a [String],
    env: &'a [(String, String)],
    args: &'a [String],
    stdin: Option<&'a [u8]>,
    /// `--CLEAN--` script path + body, run after every engine run.
    clean: Option<(&'a Path, &'a [u8])>,
    /// A CGI test's request: the script then runs under `php-cgi -C` and the
    /// CGI candidate instead of the CLIs.
    cgi: Option<&'a CgiRequest>,
    /// The test directory as it was before any engine ran (plus our script).
    snapshot: Snapshot,
}

impl Ctx {
    fn php(&self, inv: &Invocation, extra_ini: &[String], script: &Path) -> Outcome {
        self.php_within(inv, extra_ini, script, self.timeout)
    }

    fn php_within(
        &self,
        inv: &Invocation,
        extra_ini: &[String],
        script: &Path,
        timeout: Duration,
    ) -> Outcome {
        let mut cmd = Command::new(php_bin());
        cmd.arg("-n")
            .current_dir(inv.dir)
            .envs(self.base_env.iter().cloned())
            .envs(inv.env.iter().cloned());
        for kv in self.php_ini.iter().chain(extra_ini) {
            cmd.arg("-d").arg(kv);
        }
        cmd.arg(script).args(inv.args);
        run_proc(cmd, inv.stdin, timeout)
    }

    /// `<program> -C -n -d opcache.enable=0 <ini> -f <script> [-- args]` with
    /// the request's CGI environment: run-tests.php's CGI command line.
    /// opcache is built into php 8.5 and on by default under CGI; the
    /// reference always runs with it off (rhp mimics "built in, disabled").
    fn cgi_cmd(&self, program: &[String], inv: &Invocation, req: &CgiRequest) -> Command {
        let mut cmd = Command::new(&program[0]);
        cmd.args(&program[1..])
            .args(["-C", "-n", "-d", "opcache.enable=0"])
            .current_dir(inv.dir)
            .envs(self.base_env.iter().cloned())
            .envs(req.env.iter());
        for kv in self.php_ini.iter().chain(inv.ini) {
            cmd.arg("-d").arg(kv);
        }
        cmd.arg("-f").arg(inv.script);
        if cgi_passes_args(req) && !inv.args.is_empty() {
            cmd.arg("--").args(inv.args);
        }
        cmd
    }

    /// A CGI test's SKIPIF runs under php-cgi with `-q` (no headers), with
    /// the test's `--ENV--` but without the request variables, as
    /// run-tests.php runs it.
    fn php_skipif(&self, inv: &Invocation, script: &Path) -> Outcome {
        let (Some(_), Some(cgi)) = (inv.cgi, &self.cgi) else {
            return self.php(inv, inv.ini, script);
        };
        let mut cmd = Command::new(&cgi.php_cgi);
        cmd.args(["-n", "-q", "-d", "opcache.enable=0"])
            .current_dir(inv.dir)
            .envs(self.base_env.iter().cloned())
            .envs(inv.env.iter().cloned());
        for kv in self.php_ini.iter().chain(inv.ini) {
            cmd.arg("-d").arg(kv);
        }
        cmd.args(["-d", "display_errors=1", "-d", "display_startup_errors=0"])
            .arg(script);
        run_proc(cmd, None, self.timeout)
    }

    fn rhp(&self, inv: &Invocation) -> Outcome {
        if let (Some(req), Some(cgi)) = (inv.cgi, &self.cgi) {
            let cmd = self.cgi_cmd(&cgi.candidate, inv, req);
            return run_proc(cmd, req.body.as_deref(), self.timeout);
        }
        let mut cmd = Command::new(&self.rhp_bin);
        cmd.current_dir(inv.dir)
            .envs(self.base_env.iter().cloned())
            .envs(inv.env.iter().cloned());
        // run-tests' ini overwrites, as php gets them, then the test's
        // settings rhp applies.
        let test_ini = inv.ini.iter().filter(|kv| self.ini.rhp_applies(kv));
        for kv in self.php_ini.iter().chain(test_ini) {
            cmd.arg("-d").arg(kv);
        }
        cmd.arg(inv.script).args(inv.args);
        run_proc(cmd, inv.stdin, self.timeout)
    }

    /// Undo an engine run: run `--CLEAN--` (for files outside the test dir),
    /// then put the test directory back to its snapshot. php and rhp — and
    /// every mutant — thus start from the same filesystem state.
    fn reset(&self, inv: &Invocation) {
        if let Some((path, body)) = inv.clean {
            if fs::write(path, body).is_ok() {
                let _ = self.php(inv, inv.ini, path);
                let _ = fs::remove_file(path);
            }
        }
        restore(inv.dir, &inv.snapshot);
    }

    fn php_script(&self, inv: &Invocation) -> Outcome {
        self.php_script_within(inv, self.timeout)
    }

    fn php_script_within(&self, inv: &Invocation, timeout: Duration) -> Outcome {
        let out = match (inv.cgi, &self.cgi) {
            (Some(req), Some(cgi)) => {
                let program = [cgi.php_cgi.to_string_lossy().into_owned()];
                let mut out = run_proc(
                    self.cgi_cmd(&program, inv, req),
                    req.body.as_deref(),
                    timeout,
                );
                if let Some((php, rhp)) = &self.version_skew {
                    normalise_powered_by(&mut out.out, php, rhp);
                }
                out
            }
            _ => self.php_within(inv, inv.ini, inv.script, timeout),
        };
        self.reset(inv);
        out
    }

    fn rhp_script(&self, inv: &Invocation) -> Outcome {
        let out = self.rhp(inv);
        self.reset(inv);
        out
    }
}

struct TestResult {
    test: String,
    status: Status,
    key: String,
}

struct MutantResult {
    test: String,
    mutation: String,
    status: Status,
    key: String,
}

#[derive(Default)]
struct MutationStats {
    generated: usize,
    discarded: usize,
    /// Run through php only (`--mutate-all`, test not matched by rhp).
    php_only: usize,
    /// Compared against rhp.
    compared: usize,
}

/// A run (a test, or one of its mutants) in which php itself misbehaved.
struct PhpFinding {
    test: String,
    mutation: Option<String>,
    key: String,
}

/// Minimised repros written per php-finding cluster: enough to report the
/// bug, without minimising a hundred mutants that hit the same one.
const PHP_REPROS_PER_CLUSTER: usize = 3;
/// Per-run timeout while checking that a php finding reproduces.
const PHP_REPRO_TIMEOUT: Duration = Duration::from_secs(5);
/// Wall-clock budget for minimising one php finding.
const PHP_REPRO_BUDGET: Duration = Duration::from_secs(180);

/// What workers report besides each test's verdict.
#[derive(Default)]
struct Sinks {
    mutants: Mutex<Vec<MutantResult>>,
    mstats: Mutex<MutationStats>,
    php: Mutex<Vec<PhpFinding>>,
    php_emitted: Mutex<HashMap<String, usize>>,
}

fn skipped(test: &str, reason: impl Into<String>) -> TestResult {
    TestResult {
        test: test.to_string(),
        status: Status::Skipped,
        key: reason.into(),
    }
}

/// Run one test (and its mutants, if it passes). Mutant divergences are
/// appended to `mutants`.
fn run_test(ctx: &Ctx, root: &Path, rel: &str, sinks: &Sinks) -> TestResult {
    let path = root.join(rel);
    let Ok(data) = fs::read(&path) else {
        return skipped(rel, "unreadable");
    };
    let phpt = Phpt::parse(&data);
    let dir = path.parent().unwrap_or(root).to_path_buf();
    if let Some(s) = UNSUPPORTED_SECTIONS.iter().find(|s| phpt.has(s)) {
        return skipped(rel, format!("section:{s}"));
    }
    let is_cgi = phpt.is_cgi();
    if is_cgi && ctx.cgi.is_none() {
        return skipped(rel, "cgi-unavailable");
    }
    let file = match (phpt.section("FILE"), phpt.section("FILE_EXTERNAL")) {
        (Some(f), _) => f.to_vec(),
        (None, Some(ext)) => {
            let name = String::from_utf8_lossy(ext).trim().to_string();
            match fs::read(dir.join(&name)) {
                Ok(f) => f,
                Err(_) => return skipped(rel, "file_external unreadable"),
            }
        }
        (None, None) => return skipped(rel, "no FILE"),
    };
    let mentions = |needle: &str| {
        let hay = |b: Option<&[u8]>| {
            b.is_some_and(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes()))
        };
        hay(Some(&file)) || hay(phpt.section("SKIPIF")) || hay(phpt.section("CLEAN"))
    };
    if let Some(m) = SUBPROCESS_MARKERS.iter().find(|m| mentions(m)) {
        return skipped(rel, format!("subprocess:{m}"));
    }
    let ini = match ctx.ini.test_ini(&phpt, &dir, is_cgi) {
        Ok(ini) => ini,
        Err(reason) => return skipped(rel, reason),
    };
    if let Some(exts) = phpt.section("EXTENSIONS") {
        for ext in String::from_utf8_lossy(exts).split_whitespace() {
            if !ctx.php_modules.contains(&ext.to_ascii_lowercase()) {
                return skipped(rel, format!("ext:{ext}"));
            }
        }
    }
    let mut env: Vec<(String, String)> = phpt
        .section("ENV")
        .map(|b| {
            String::from_utf8_lossy(b)
                .lines()
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.trim().to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    // ext/curl's server.inc starts `TEST_PHP_EXECUTABLE -S` as the HTTP peer
    // of the transfers under test: php's CLI server for both engines (rhp
    // has no -S). The server only answers; curl is the code compared.
    if rel.starts_with("ext/curl/tests/") {
        env.push(("TEST_PHP_EXECUTABLE".into(), php_bin()));
    }
    let args: Vec<String> = phpt
        .section("ARGS")
        .map(|b| {
            String::from_utf8_lossy(b)
                .split_whitespace()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let stdin = phpt.section("STDIN");
    let stem = rel.trim_end_matches(".phpt");
    let script = root.join(format!("{stem}.php"));
    let clean_script = root.join(format!("{stem}.clean.php"));
    if (phpt.has("GZIP_POST") || phpt.has("DEFLATE_POST")) && !ctx.php_modules.contains("zlib") {
        return skipped(rel, "ext:zlib");
    }
    let cgi_req = if is_cgi {
        match cgi_request(&phpt, &script, &run_tests_env(&phpt, &dir)) {
            Ok(r) => Some(r),
            Err(e) => return skipped(rel, e),
        }
    } else {
        None
    };
    let pristine = pristine_snapshot(&dir);
    let mut inv = Invocation {
        dir: &dir,
        script: &script,
        ini: &ini,
        env: &env,
        args: &args,
        stdin,
        clean: phpt.section("CLEAN").map(|c| (clean_script.as_path(), c)),
        snapshot: (*pristine).clone(),
        cgi: cgi_req.as_ref(),
    };

    if let Some(skipif) = phpt.section("SKIPIF") {
        let skip_script = root.join(format!("{stem}.skip.php"));
        if fs::write(&skip_script, skipif).is_err() {
            return skipped(rel, "unwritable work dir");
        }
        let out = ctx.php_skipif(&inv, &skip_script);
        let _ = fs::remove_file(&skip_script);
        restore(&dir, &pristine);
        let text = String::from_utf8_lossy(&out.out);
        let text = text.trim_start();
        if text.len() >= 4 && text[..4].eq_ignore_ascii_case("skip") {
            return skipped(rel, "skipif");
        }
    }

    if fs::write(&script, &file).is_err() {
        return skipped(rel, "unwritable work dir");
    }
    inv.snapshot = match script.file_name() {
        Some(name) if script.parent() == Some(dir.as_path()) => {
            pristine.with_new_file(&dir, Path::new(name))
        }
        _ => snapshot(&dir),
    };
    let php_out = ctx.php_script(&inv);
    if let Some(key) = php_anomaly(&php_out) {
        record_php_finding(ctx, &inv, sinks, rel, None, &file, key);
        let _ = fs::write(&script, &file); // the repro run may have rewritten it
    }
    // (A CGI response's X-Powered-By already carries rhp's version here.)
    if let Some((v, _)) = &ctx.version_skew {
        if php_out.out.windows(v.len()).any(|w| w == v.as_bytes()) {
            let _ = fs::remove_file(&script);
            restore(&dir, &pristine);
            return skipped(rel, "version");
        }
    }
    std::thread::sleep(ctx.skew);
    let rhp_out = ctx.rhp_script(&inv);
    let mut v = classify(&php_out, &rhp_out);
    // Some php output is unspecified (raw memcmp() magnitudes, ids, sizes) and
    // can differ run to run; php-src marks it with EXPECTF/EXPECTREGEX. When
    // the engines differ but both satisfy the test's own spec, it passes —
    // exactly run-tests.php's verdict.
    // A CGI run's spec is about the body; its headers must still be equal.
    let spec_holds = |php: &[u8], rhp: &[u8]| match (is_cgi, split_cgi(php), split_cgi(rhp)) {
        (false, ..) => {
            spec_matches(&phpt, php) == Some(true) && spec_matches(&phpt, rhp) == Some(true)
        }
        (true, Some((ph, pb)), Some((rh, rb))) => {
            normalise(ph) == normalise(rh)
                && spec_matches(&phpt, pb) == Some(true)
                && spec_matches(&phpt, rb) == Some(true)
        }
        _ => false,
    };
    if matches!(v.status, Status::Mismatch | Status::DiagnosticsOnly)
        && spec_holds(&php_out.out, &rhp_out.out)
    {
        v = verdict(Status::Match, "spec");
    }

    let mutate = ctx.mutate > 0
        && (v.status == Status::Match
            || (ctx.mutate_all && !php_out.timed_out && php_anomaly(&php_out).is_none()));
    let php_only = v.status != Status::Match;
    if mutate {
        let mut rng = Rng::new(ctx.seed ^ fnv1a(rel.as_bytes()));
        for (k, m) in mutate::mutants(&file, &mut rng, ctx.mutate)
            .into_iter()
            .enumerate()
        {
            if fs::write(&script, &m.source).is_err() {
                break;
            }
            let php_m = ctx.php_script(&inv);
            sinks.mstats.lock().unwrap().generated += 1;
            if let Some(key) = php_anomaly(&php_m) {
                let mutation = Some((k, m.description.as_str()));
                record_php_finding(ctx, &inv, sinks, rel, mutation, &m.source, key);
                sinks.mstats.lock().unwrap().discarded += 1;
                continue;
            }
            if php_m.timed_out || String::from_utf8_lossy(&php_m.out).contains("Parse error: ") {
                sinks.mstats.lock().unwrap().discarded += 1;
                continue;
            }
            if php_only {
                sinks.mstats.lock().unwrap().php_only += 1;
                continue; // rhp doesn't match the unmutated test; comparing is noise
            }
            let rhp_m = ctx.rhp_script(&inv);
            let mv = classify(&php_m, &rhp_m);
            if mv.status == Status::Skipped {
                sinks.mstats.lock().unwrap().discarded += 1;
                continue;
            }
            sinks.mstats.lock().unwrap().compared += 1;
            if mv.status != Status::Match {
                // Golden fixtures are CLI programs: a CGI mutant can't move there.
                if let (Some(dirf), Status::Mismatch | Status::RhpCrash, false) =
                    (&ctx.fixtures, mv.status, is_cgi)
                {
                    emit_fixture(ctx, &inv, dirf, rel, k, &m, mv.status);
                }
                sinks.mutants.lock().unwrap().push(MutantResult {
                    test: rel.to_string(),
                    mutation: m.description,
                    status: mv.status,
                    key: mv.key,
                });
            }
        }
    }

    let _ = fs::remove_file(&script);
    restore(&dir, &pristine);
    TestResult {
        test: rel.to_string(),
        status: v.status,
        key: v.key,
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

// ---------------------------------------------------------------------------
// fixtures: minimised, self-contained repros of mutant divergences
// ---------------------------------------------------------------------------

/// Sources that read sibling files or their own path can't be moved into the
/// golden suite verbatim.
fn is_portable(src: &[u8]) -> bool {
    let s = String::from_utf8_lossy(src).to_ascii_lowercase();
    ![
        "include",
        "require",
        "__dir__",
        "__file__",
        "fopen",
        "file_",
        "opendir",
        "glob(",
        "unlink",
        "mkdir",
        "getenv",
        "$argv",
        "stdin",
        "tempnam",
        "sys_get_temp_dir",
    ]
    .iter()
    .any(|needle| s.contains(needle))
}

/// Whether `src` still makes php and rhp diverge the same way (`want`),
/// with php accepting the program.
fn still_diverges(ctx: &Ctx, inv: &Invocation, src: &[u8], want: Status) -> Option<Outcome> {
    fs::write(inv.script, src).ok()?;
    let php = ctx.php_script(inv);
    if php.timed_out || String::from_utf8_lossy(&php.out).contains("Parse error: ") {
        return None;
    }
    let rhp = ctx.rhp_script(inv);
    (classify(&php, &rhp).status == want).then_some(php)
}

/// Greedy line-chunk reduction (ddmin-style), bounded to `budget` runs.
/// Greedy line-chunk reduction (ddmin-style) of `src` while `keep` holds,
/// bounded to `budget` evaluations.
fn minimise(src: &[u8], budget: usize, mut keep: impl FnMut(&[u8]) -> bool) -> Vec<u8> {
    let mut lines: Vec<&[u8]> = src.split_inclusive(|&b| b == b'\n').collect();
    let mut runs = 0;
    let mut chunk = lines.len().div_ceil(2).max(1);
    while chunk >= 1 && runs < budget {
        let mut i = 0;
        let mut removed_any = false;
        while i < lines.len() && runs < budget {
            let end = (i + chunk).min(lines.len());
            // Never drop the opening tag.
            if lines[i..end]
                .iter()
                .any(|l| l.windows(5).any(|w| w == b"<?php"))
            {
                i = end;
                continue;
            }
            let candidate: Vec<u8> = lines[..i]
                .iter()
                .chain(&lines[end..])
                .flat_map(|l| l.iter().copied())
                .collect();
            runs += 1;
            if keep(&candidate) {
                lines.drain(i..end);
                removed_any = true;
            } else {
                i = end;
            }
        }
        if !removed_any {
            if chunk == 1 {
                break;
            }
            chunk = chunk.div_ceil(2);
        }
    }
    lines.concat()
}

/// Record a php misbehaviour and, under `--php-findings`, write a repro
/// (minimised under `--minimize`, keeping the *same* anomaly) plus php's
/// output, up to [`PHP_REPROS_PER_CLUSTER`] per cluster.
fn record_php_finding(
    ctx: &Ctx,
    inv: &Invocation,
    sinks: &Sinks,
    rel: &str,
    mutation: Option<(usize, &str)>,
    src: &[u8],
    key: String,
) {
    sinks.php.lock().unwrap().push(PhpFinding {
        test: rel.to_string(),
        mutation: mutation.map(|(_, d)| d.to_string()),
        key: key.clone(),
    });
    let Some(dir) = &ctx.php_findings else {
        return;
    };
    // A bare signal says nothing about *which* crash it was, so without a
    // sanitizer every crashing test gets its own repro slots.
    let slot = if key.starts_with("php killed by signal") {
        format!("{key} @ {rel}")
    } else {
        key.clone()
    };
    {
        let mut emitted = sinks.php_emitted.lock().unwrap();
        let n = emitted.entry(slot.clone()).or_insert(0);
        if *n >= PHP_REPROS_PER_CLUSTER {
            return;
        }
        *n += 1;
    }
    // Minimising can delete a loop's exit, and each such candidate would burn
    // the full timeout: bound both the candidate runs and the whole repro.
    let started = Instant::now();
    let candidate_timeout = ctx.timeout.min(PHP_REPRO_TIMEOUT);
    let reproduces = |candidate: &[u8]| -> Option<Outcome> {
        fs::write(inv.script, candidate).ok()?;
        let o = ctx.php_script_within(inv, candidate_timeout);
        (!o.timed_out && php_anomaly(&o).as_deref() == Some(key.as_str())).then_some(o)
    };
    let src = if ctx.minimize {
        minimise(src, 200, |c| {
            started.elapsed() < PHP_REPRO_BUDGET && reproduces(c).is_some()
        })
    } else {
        src.to_vec()
    };
    let Some(out) = reproduces(&src) else {
        // Didn't reproduce (flaky): give the slot back to a later run.
        *sinks.php_emitted.lock().unwrap().get_mut(&slot).unwrap() -= 1;
        return;
    };
    let clean = |s: &str| s.replace("?>", "? >").replace(['\r', '\n'], " ");
    let mut program = format!(
        "<?php\n// php-finding: {}\n// from: {}\n// mutation: {}\n?>\n",
        clean(&key),
        clean(rel),
        clean(mutation.map_or("none (the test itself)", |(_, d)| d)),
    )
    .into_bytes();
    program.extend_from_slice(&src);
    let slug: String = rel
        .trim_end_matches(".phpt")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let which = mutation.map_or("test".to_string(), |(k, _)| format!("m{k}"));
    let base = dir.join(format!(
        "{slug}_{:08x}_{which}",
        fnv1a(rel.as_bytes()) as u32
    ));
    let _ = fs::write(base.with_extension("php"), program);
    let _ = fs::write(base.with_extension("txt"), &out.out);
}

fn emit_fixture(
    ctx: &Ctx,
    inv: &Invocation,
    dir: &Path,
    rel: &str,
    k: usize,
    m: &mutate::Mutant,
    status: Status,
) {
    if !is_portable(&m.source) {
        return;
    }
    let src = if ctx.minimize {
        minimise(&m.source, 200, |c| {
            still_diverges(ctx, inv, c, status).is_some()
        })
    } else {
        m.source.clone()
    };
    // Header comments must not end PHP mode or the comment early.
    let clean = |s: &str| {
        s.replace("?>", "? >")
            .replace('\r', "\\r")
            .replace('\n', "\\n")
    };
    let mut program = format!(
        "<?php\n// phpt-mutant: {}\n// mutation: {}\n?>\n",
        clean(rel),
        clean(&m.description)
    )
    .into_bytes();
    program.extend_from_slice(&src);
    // Injective name: the readable slug plus a hash of the exact test path.
    let slug: String = rel
        .trim_end_matches(".phpt")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let base = dir.join(format!("{slug}_{:08x}_m{k}", fnv1a(rel.as_bytes()) as u32));
    let php_path = base.with_extension("php");
    if fs::write(&php_path, &program).is_err() {
        return;
    }
    // The expectation is php's output for exactly this file, run where it now
    // lives — and only if that output is location-independent and rhp still
    // diverges on it, so the pair can move into tests/golden as is.
    let here = Invocation {
        dir,
        script: &php_path,
        ini: &[],
        env: &[],
        args: &[],
        stdin: None,
        clean: None,
        snapshot: Snapshot::default(),
        cgi: None,
    };
    let php = ctx.php(&here, &[], &php_path);
    let rhp = ctx.rhp(&here);
    let text = String::from_utf8_lossy(&php.out);
    let portable = !text.contains(" on line ")
        && !text.contains("Stack trace:")
        && !text.contains("Parse error: ")
        && !text.contains(&*dir.to_string_lossy());
    if php.timed_out || !portable || classify(&php, &rhp).status != status {
        let _ = fs::remove_file(&php_path);
        return;
    }
    let _ = fs::write(base.with_extension("out"), &php.out);
}

// ---------------------------------------------------------------------------
// test selection + work tree
// ---------------------------------------------------------------------------

/// All `.phpt` files under `root`, as `/`-joined paths relative to it.
fn find_phpt(root: &Path, rel: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(root.join(rel)) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let sub = rel.join(&name);
        match e.file_type() {
            Ok(t) if t.is_dir() => find_phpt(root, &sub, out),
            Ok(t) if t.is_file() && name.to_string_lossy().ends_with(".phpt") => {
                out.push(sub.to_string_lossy().replace('\\', "/"))
            }
            _ => {}
        }
    }
}

/// The directory a test's fixtures live under: its path up to and including
/// the first `tests` component (`ext/standard/tests`, `Zend/tests`, `tests`).
fn tree_root(rel: &str) -> String {
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.iter().position(|p| *p == "tests") {
        Some(i) => parts[..=i].join("/"),
        None => parts[..parts.len().saturating_sub(1)].join("/"),
    }
}

/// What a test directory held before a run: every path under it (relative,
/// symlinks not followed), plus a change stamp for each directory so
/// [`restore`] only lists the directories a run touched.
#[derive(Clone, Debug, Default)]
struct Snapshot {
    paths: BTreeSet<PathBuf>,
    /// Each directory (relative, "" for the root), its stamp, and how many
    /// entries it holds. [`restore`] re-stamps a directory it put back
    /// exactly, so later restores skip it again.
    dirs: Vec<SnapDir>,
}

#[derive(Clone, Debug)]
struct SnapDir {
    rel: PathBuf,
    stamp: std::cell::Cell<DirStamp>,
    entries: usize,
}

impl Snapshot {
    /// Whether every directory still carries its recorded stamp.
    fn is_current(&self, dir: &Path) -> bool {
        self.dirs
            .iter()
            .all(|d| DirStamp::of(&dir.join(&d.rel)) == Some(d.stamp.get()))
    }

    /// This snapshot plus `name`, just created directly in `dir`.
    fn with_new_file(&self, dir: &Path, name: &Path) -> Snapshot {
        let mut s = self.clone();
        if s.paths.insert(name.to_path_buf()) {
            if let Some(root) = s.dirs.iter_mut().find(|d| d.rel.as_os_str().is_empty()) {
                root.entries += 1;
            }
        }
        if let (Some(root), Some(now)) = (
            s.dirs.iter().find(|d| d.rel.as_os_str().is_empty()),
            DirStamp::of(dir),
        ) {
            root.stamp.set(now);
        }
        s
    }
}

thread_local! {
    /// Per worker: the pristine snapshot of each test directory it has run.
    static SNAPSHOTS: RefCell<HashMap<PathBuf, std::rc::Rc<Snapshot>>> =
        RefCell::new(HashMap::new());
}

/// `snapshot(dir)`, reused from this worker's last test in `dir` when no
/// directory under it has changed since (each run ends with a restore that
/// re-stamps what it put back).
fn pristine_snapshot(dir: &Path) -> std::rc::Rc<Snapshot> {
    if let Some(s) = SNAPSHOTS.with(|m| m.borrow().get(dir).cloned()) {
        if s.is_current(dir) {
            return s;
        }
    }
    let s = std::rc::Rc::new(snapshot(dir));
    SNAPSHOTS.with(|m| m.borrow_mut().insert(dir.to_path_buf(), s.clone()));
    s
}

/// Adding, removing or renaming an entry updates a directory's mtime and
/// ctime; ctime can't be set back from user space, so an unchanged stamp
/// means the directory's own entries are as they were.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DirStamp {
    ino: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl DirStamp {
    fn of(p: &Path) -> Option<DirStamp> {
        use std::os::unix::fs::MetadataExt;
        let m = fs::symlink_metadata(p).ok().filter(|m| m.is_dir())?;
        Some(DirStamp {
            ino: m.ino(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
        })
    }
}

fn snapshot(dir: &Path) -> Snapshot {
    fn walk(root: &Path, rel: &Path, out: &mut Snapshot) {
        let here = root.join(rel);
        // Stamp before listing: a change racing the listing then still
        // shows up as a changed stamp.
        let Some(stamp) = DirStamp::of(&here) else {
            return;
        };
        let Ok(entries) = fs::read_dir(&here) else {
            return;
        };
        let slot = out.dirs.len();
        out.dirs.push(SnapDir {
            rel: rel.to_path_buf(),
            stamp: std::cell::Cell::new(stamp),
            entries: 0,
        });
        for e in entries.flatten() {
            out.dirs[slot].entries += 1;
            let sub = rel.join(e.file_name());
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            out.paths.insert(sub.clone());
            if is_dir {
                walk(root, &sub, out);
            }
        }
    }
    let mut out = Snapshot::default();
    walk(dir, Path::new(""), &mut out);
    out
}

/// Delete everything under `dir` that isn't in `keep` (files a run created).
/// Only directories whose stamp changed are listed; anything new inside one
/// is removed whole. Tests sometimes chmod what they create, so retry once
/// after making the path (and, for a directory, its contents) writable again.
fn restore(dir: &Path, keep: &Snapshot) {
    use std::os::unix::fs::PermissionsExt;
    fn make_writable(p: &Path) {
        let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o700));
        if fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()) {
            for e in fs::read_dir(p).into_iter().flatten().flatten() {
                make_writable(&e.path());
            }
        }
    }
    fn remove(p: &Path) -> std::io::Result<()> {
        match fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(p),
            Ok(_) => fs::remove_file(p),
            Err(_) => Ok(()),
        }
    }
    for d in &keep.dirs {
        let here = dir.join(&d.rel);
        match DirStamp::of(&here) {
            Some(now) if now == d.stamp.get() => continue,
            None => continue,
            Some(_) => {}
        }
        let Ok(entries) = fs::read_dir(&here) else {
            continue;
        };
        let (mut kept, mut clean) = (0, true);
        for e in entries.flatten() {
            if keep.paths.contains(&d.rel.join(e.file_name())) {
                kept += 1;
                continue;
            }
            let p = e.path();
            if remove(&p).is_err() {
                let _ = fs::set_permissions(&here, fs::Permissions::from_mode(0o700));
                make_writable(&p);
                clean &= remove(&p).is_ok();
            }
        }
        // Back to exactly the snapshot's entries: record the new stamp. (If
        // a run deleted an original entry, leave the stamp stale so the
        // next pristine_snapshot() walks the directory again.)
        if clean && kept == d.entries {
            if let Some(now) = DirStamp::of(&here) {
                d.stamp.set(now);
            }
        }
    }
}

/// Scratch copies `w{i}` (i in `ids`) under `work` of every tree in `roots`,
/// made in parallel; returns their canonical paths.
fn make_worker_roots(
    work: &Path,
    ids: std::ops::Range<usize>,
    roots: &BTreeSet<String>,
    php_src: &Path,
) -> Result<Vec<PathBuf>, String> {
    let dirs: Vec<PathBuf> = ids.map(|i| work.join(format!("w{i}"))).collect();
    let errors: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = dirs
            .iter()
            .map(|w| {
                s.spawn(move || {
                    roots.iter().find_map(|r| {
                        copy_tree(&php_src.join(r), &w.join(r))
                            .err()
                            .map(|e| format!("copying {r}: {e}"))
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok().flatten())
            .collect()
    });
    if let Some(e) = errors.into_iter().next() {
        return Err(e);
    }
    Ok(dirs
        .into_iter()
        .map(|w| w.canonicalize().unwrap_or(w))
        .collect())
}

/// Copy the tree `from` to `to`. On macOS a fresh destination is first tried
/// as one APFS `clonefile()` (copy-on-write, near-instant for a whole test
/// tree); anything else falls back to a file-by-file copy.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    if !to.exists() {
        use std::os::unix::ffi::OsStrExt;
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).ok();
        if let (Some(src), Some(dst)) = (c(from), c(to)) {
            const CLONE_NOFOLLOW: u32 = 0x0001;
            // SAFETY: both are valid NUL-terminated paths.
            if unsafe { libc::clonefile(src.as_ptr(), dst.as_ptr(), CLONE_NOFOLLOW) } == 0 {
                return Ok(());
            }
        }
    }
    copy_tree_files(from, to)
}

fn copy_tree_files(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let t = e.file_type()?;
        if t.is_dir() {
            copy_tree_files(&src, &dst)?;
        } else if t.is_file() {
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

fn php_modules(ini: &[String]) -> BTreeSet<String> {
    let mut cmd = Command::new(php_bin());
    cmd.arg("-n");
    for kv in ini {
        cmd.arg("-d").arg(kv);
    }
    cmd.arg("-m");
    let out = run_proc(cmd, None, Duration::from_secs(20));
    // run-tests' remap_loaded_extensions_names(): `--EXTENSIONS-- opcache`
    // names Zend OPcache.
    String::from_utf8_lossy(&out.out)
        .lines()
        .map(|l| l.trim().to_ascii_lowercase())
        .filter(|l| !l.is_empty() && !l.starts_with('['))
        .map(|l| {
            if l == "zend opcache" {
                "opcache".into()
            } else {
                l
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// reporting
// ---------------------------------------------------------------------------

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_json(
    path: &Path,
    results: &[TestResult],
    mutants: &[MutantResult],
    php: &[PhpFinding],
) -> std::io::Result<()> {
    let mut s = String::from("{\n\"tests\": [\n");
    for (i, r) in results.iter().enumerate() {
        let sep = if i + 1 < results.len() { "," } else { "" };
        let _ = writeln!(
            s,
            "{{\"test\":{},\"status\":\"{}\",\"key\":{}}}{sep}",
            json_str(&r.test),
            r.status.label(),
            json_str(&r.key)
        );
    }
    s.push_str("],\n\"mutants\": [\n");
    for (i, m) in mutants.iter().enumerate() {
        let sep = if i + 1 < mutants.len() { "," } else { "" };
        let _ = writeln!(
            s,
            "{{\"test\":{},\"mutation\":{},\"status\":\"{}\",\"key\":{}}}{sep}",
            json_str(&m.test),
            json_str(&m.mutation),
            m.status.label(),
            json_str(&m.key)
        );
    }
    s.push_str("],\n\"php_findings\": [\n");
    for (i, f) in php.iter().enumerate() {
        let sep = if i + 1 < php.len() { "," } else { "" };
        let mutation = f.mutation.as_deref().map_or("null".to_string(), json_str);
        let _ = writeln!(
            s,
            "{{\"test\":{},\"mutation\":{mutation},\"key\":{}}}{sep}",
            json_str(&f.test),
            json_str(&f.key)
        );
    }
    s.push_str("]\n}\n");
    fs::write(path, s)
}

/// Print the `top` largest clusters: count, key and one example.
fn print_clusters<'a>(items: impl Iterator<Item = (&'a str, &'a str)>, top: usize) {
    let mut clusters: BTreeMap<&str, (usize, &str)> = BTreeMap::new();
    for (key, example) in items {
        clusters.entry(key).or_insert((0, example)).0 += 1;
    }
    let mut v: Vec<_> = clusters.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(b.0)));
    for (key, (n, ex)) in v.iter().take(top) {
        println!("  {n:6}  {key}\n          e.g. {ex}");
    }
    if v.len() > top {
        println!("  ... {} more clusters (see --json)", v.len() - top);
    }
}

/// `(php's PHP_VERSION, rhp's)` if they differ, else `None`.
fn version_skew(ctx: &Ctx, scratch: &Path) -> Option<(String, String)> {
    let script = scratch.join("__rhp_phpt_version.php");
    fs::write(&script, b"<?php echo PHP_VERSION;").ok()?;
    let inv = Invocation {
        dir: scratch,
        script: &script,
        ini: &[],
        env: &[],
        args: &[],
        stdin: None,
        clean: None,
        snapshot: Snapshot::default(),
        cgi: None,
    };
    let php = String::from_utf8_lossy(&ctx.php(&inv, &[], &script).out)
        .trim()
        .to_string();
    let rhp = String::from_utf8_lossy(&ctx.rhp(&inv).out)
        .trim()
        .to_string();
    let _ = fs::remove_file(&script);
    (!php.is_empty() && php != rhp).then_some((php, rhp))
}

/// The php-cgi to use as the CGI reference: `--php-cgi`, else
/// [`crate::php_cgi`] (CI's `-n` shims live side by side, so this finds the
/// php-cgi shim next to the php shim).
pub(crate) fn find_php_cgi(explicit: Option<&Path>) -> Option<PathBuf> {
    explicit.map(Path::to_path_buf).or_else(crate::php_cgi)
}

/// Whether the file at `root/rel` is a CGI test (for `--cgi-only`).
fn is_cgi_test(root: &Path, rel: &str) -> bool {
    fs::read(root.join(rel)).is_ok_and(|d| Phpt::parse(&d).is_cgi())
}

/// Run `tests` across the worker roots (one worker per root), sorted by path.
/// The fixed instant every process starts its clock at under `--faketime`.
const FAKETIME_START: &str = "@2026-01-01 12:00:00";

/// The environment that loads libfaketime (`lib`, or `auto`: the usual
/// install locations) into every process, all clocks starting at
/// [`FAKETIME_START`] and then running at normal speed.
fn faketime_env(lib: &str) -> Result<Vec<(String, String)>, String> {
    const CANDIDATES: &[&str] = &[
        "/opt/homebrew/opt/libfaketime/lib/faketime/libfaketime.1.dylib",
        "/usr/local/opt/libfaketime/lib/faketime/libfaketime.1.dylib",
        "/usr/lib/x86_64-linux-gnu/faketime/libfaketime.so.1",
        "/usr/lib/aarch64-linux-gnu/faketime/libfaketime.so.1",
        "/usr/lib/faketime/libfaketime.so.1",
        "/usr/local/lib/faketime/libfaketime.so.1",
    ];
    let path = if lib == "auto" {
        CANDIDATES
            .iter()
            .find(|p| Path::new(p).is_file())
            .map(|p| p.to_string())
            .ok_or("--faketime auto: libfaketime not found (brew install libfaketime / apt install libfaketime)")?
    } else if Path::new(lib).is_file() {
        lib.to_string()
    } else {
        return Err(format!("--faketime: no such library {lib:?}"));
    };
    let mut env = vec![("FAKETIME".to_string(), FAKETIME_START.to_string())];
    if cfg!(target_os = "macos") {
        env.push(("DYLD_INSERT_LIBRARIES".into(), path));
        env.push(("DYLD_FORCE_FLAT_NAMESPACE".into(), "1".into()));
    } else {
        env.push(("LD_PRELOAD".into(), path));
    }
    Ok(env)
}

/// Confirming runs per job slot (they mostly sleep; see `--write-known`).
const CONFIRM_OVERSUBSCRIBE: usize = 4;

fn run_parallel(ctx: &Ctx, roots: &[PathBuf], tests: &[String], sinks: &Sinks) -> Vec<TestResult> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(tests.len()));
    let started = Instant::now();
    std::thread::scope(|s| {
        for root in roots {
            let (next, done, results) = (&next, &done, &results);
            s.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(rel) = tests.get(i) else { break };
                let r = run_test(ctx, root, rel, sinks);
                results.lock().unwrap().push(r);
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                if d.is_multiple_of(500) {
                    eprintln!(
                        "  {d}/{} ({:.0}s)",
                        tests.len(),
                        started.elapsed().as_secs_f64()
                    );
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.test.cmp(&b.test));
    results
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

const USAGE: &str = "\
usage: rhp-difftest phpt --php-src <dir> [options]

Runs php-src .phpt tests through php and rhp and compares their output.
Tests on tests/phpt/ignored.txt (built in) never run.

selection:
  --php-src <dir>        php-src checkout (or env RHP_PHP_SRC)
  --filter <substr>      only tests whose path contains <substr> (repeatable)
  --limit <n>            run at most <n> tests
  --check-known <file>   run only the listed tests; exit 1 if any no longer match, if
                         the list and tests/phpt/ignored.txt share a test, or if an
                         ignored test is missing from --php-src
  --cgi-only             only CGI tests (--GET--/--POST--/--CGI--/... sections)
  --rhp-ini <directive>  run tests that set <directive> too, passing it to rhp
                         (repeatable): to measure whether rhp honours it
running:
  --rhp <bin>            rhp binary (default: `rhp` next to this binary, or env RHP_BIN)
  --php-cgi <bin>        reference for CGI tests (default: env RHP_PHP_CGI, else the
                         php-cgi next to php, else on PATH; CGI tests skip without one)
  --cgi-candidate <cmd>  CGI command compared against php-cgi, split on whitespace
                         (default: \"<rhp> --cgi\", or env RHP_CGI_CANDIDATE); both get
                         `-C -n -d opcache.enable=0 -d <ini>... -f <script> [-- args]`
                         and run-tests.php's CGI environment, body on stdin
  --jobs <n>             parallel workers (default: all cores)
  --timeout <secs>       per-process timeout (default: 10)
  --work <dir>           scratch copy of the test trees (default: a temp dir)
  --keep-work            don't delete the scratch copy
  --faketime <lib|auto>  run every php/rhp process under libfaketime, all starting
                         at the same instant (FAKETIME=\"@2026-01-01 12:00:00\"), so
                         tests that print the date/time compare equal; the
                         --write-known confirmation re-run then needs no 1.1s gap.
                         `auto` looks in the usual Homebrew/Debian locations
                         (also env RHP_FAKETIME)
mutation:
  --mutate <n>           run <n> first-order mutants of every passing test
  --mutate-all           mutate every test; for tests rhp doesn't match yet, mutants
                         run through php only (php bug hunting, with --php-findings)
  --seed <n>             mutant selection seed (default: 0xC0FFEE)
  --emit-fixtures <dir>  write self-contained repros of mutant mismatches/crashes
  --php-findings <dir>   write repros of runs where php itself crashed or tripped a
                         sanitizer (ASan/UBSan builds), up to 3 per cluster
  --minimize             minimise emitted repros (line-based ddmin)
output:
  --json <file>          full per-test / per-mutant report
  --write-known <file>   write the passing tests (the ratchet list)
  --top <n>              clusters to print (default: 25)
";

struct Opts {
    php_src: PathBuf,
    filters: Vec<String>,
    limit: Option<usize>,
    check_known: Option<PathBuf>,
    cgi_only: bool,
    rhp: Option<PathBuf>,
    php_cgi: Option<PathBuf>,
    cgi_candidate: Option<String>,
    jobs: usize,
    timeout: u64,
    work: Option<PathBuf>,
    keep_work: bool,
    faketime: Option<String>,
    mutate: usize,
    mutate_all: bool,
    seed: u64,
    fixtures: Option<PathBuf>,
    php_findings: Option<PathBuf>,
    minimize: bool,
    json: Option<PathBuf>,
    write_known: Option<PathBuf>,
    top: usize,
    rhp_ini: Vec<String>,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        php_src: std::env::var_os("RHP_PHP_SRC")
            .map(PathBuf::from)
            .unwrap_or_default(),
        filters: Vec::new(),
        limit: None,
        check_known: None,
        cgi_only: false,
        rhp: std::env::var_os("RHP_BIN").map(PathBuf::from),
        php_cgi: None,
        cgi_candidate: std::env::var("RHP_CGI_CANDIDATE").ok(),
        jobs: std::thread::available_parallelism().map_or(4, |n| n.get()),
        timeout: 10,
        work: None,
        keep_work: false,
        faketime: std::env::var("RHP_FAKETIME").ok().filter(|v| !v.is_empty()),
        mutate: 0,
        mutate_all: false,
        seed: 0xC0FFEE,
        fixtures: None,
        php_findings: None,
        minimize: false,
        json: None,
        write_known: None,
        top: 25,
        rhp_ini: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        let num = |v: String| -> Result<u64, String> {
            let v = v.trim();
            match v.strip_prefix("0x") {
                Some(h) => u64::from_str_radix(h, 16),
                None => v.parse(),
            }
            .map_err(|_| format!("bad number {v:?}"))
        };
        match a.as_str() {
            "--php-src" => o.php_src = val()?.into(),
            "--filter" => o.filters.push(val()?),
            "--limit" => o.limit = Some(num(val()?)? as usize),
            "--check-known" => o.check_known = Some(val()?.into()),
            "--rhp" => o.rhp = Some(val()?.into()),
            "--cgi-only" => o.cgi_only = true,
            "--php-cgi" => o.php_cgi = Some(val()?.into()),
            "--cgi-candidate" => o.cgi_candidate = Some(val()?),
            "--jobs" => o.jobs = (num(val()?)? as usize).max(1),
            "--timeout" => o.timeout = num(val()?)?.max(1),
            "--work" => o.work = Some(val()?.into()),
            "--keep-work" => o.keep_work = true,
            "--faketime" => o.faketime = Some(val()?),
            "--mutate" => o.mutate = num(val()?)? as usize,
            "--mutate-all" => o.mutate_all = true,
            "--seed" => o.seed = num(val()?)?,
            "--emit-fixtures" => o.fixtures = Some(val()?.into()),
            "--php-findings" => o.php_findings = Some(val()?.into()),
            "--minimize" => o.minimize = true,
            "--json" => o.json = Some(val()?.into()),
            "--write-known" => o.write_known = Some(val()?.into()),
            "--top" => o.top = num(val()?)? as usize,
            "--rhp-ini" => o.rhp_ini.push(val()?),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    if o.php_src.as_os_str().is_empty() {
        return Err("--php-src is required".into());
    }
    Ok(o)
}

/// `rhp-difftest phpt ...` entry point.
pub fn main(args: &[String]) -> ExitCode {
    let o = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("rhp-difftest phpt: {e}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let rhp_bin = o.rhp.clone().unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("rhp")))
            .unwrap_or_else(|| PathBuf::from("rhp"))
    });
    if !rhp_bin.exists() {
        eprintln!(
            "rhp-difftest phpt: rhp binary not found at {} (build rhp-cli, or pass --rhp)",
            rhp_bin.display()
        );
        return ExitCode::from(2);
    }
    let rhp_bin = rhp_bin.canonicalize().unwrap_or(rhp_bin);
    let cgi = find_php_cgi(o.php_cgi.as_deref())
        .filter(|p| p.is_file())
        .map(|php_cgi| CgiEngines {
            php_cgi: php_cgi.canonicalize().unwrap_or(php_cgi),
            candidate: match &o.cgi_candidate {
                Some(c) => c.split_whitespace().map(String::from).collect(),
                None => vec![rhp_bin.to_string_lossy().into_owned(), "--cgi".into()],
            },
        })
        .filter(|c| !c.candidate.is_empty());
    if !crate::php_available() {
        eprintln!("rhp-difftest phpt: no `php` binary (set RHP_PHP)");
        return ExitCode::from(2);
    }

    // Select tests.
    let known = match &o.check_known {
        Some(p) => match read_known(p) {
            Ok(k) => Some(k),
            Err(e) => {
                eprintln!("rhp-difftest phpt: can't read {}: {e}", p.display());
                return ExitCode::from(2);
            }
        },
        None => None,
    };
    let ignored = match parse_ignored(IGNORED) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("rhp-difftest phpt: tests/phpt/ignored.txt: {e}");
            return ExitCode::from(2);
        }
    };
    let mut tests = Vec::new();
    find_phpt(&o.php_src, Path::new(""), &mut tests);
    tests.sort();
    let mut missing_known = Vec::new();
    if let Some(k) = &known {
        let have: BTreeSet<&String> = tests.iter().collect();
        missing_known = k.iter().filter(|t| !have.contains(t)).cloned().collect();
        // The two lists must agree, and the ignore list must not go stale.
        let listed: Vec<&String> = k.iter().filter(|t| ignored.contains_key(*t)).collect();
        let stale: Vec<&String> = ignored.keys().filter(|t| !have.contains(t)).collect();
        if !listed.is_empty() || !stale.is_empty() {
            for t in &listed {
                println!("known-passing test is on tests/phpt/ignored.txt: {t}");
            }
            for t in &stale {
                println!("ignored test not found in --php-src: {t}");
            }
            return ExitCode::FAILURE;
        }
        let want: BTreeSet<&String> = k.iter().collect();
        tests.retain(|t| want.contains(t));
    }
    if !o.filters.is_empty() {
        tests.retain(|t| o.filters.iter().any(|f| t.contains(f.as_str())));
    }
    if o.cgi_only {
        tests.retain(|t| is_cgi_test(&o.php_src, t));
    }
    // Ignored tests count against --limit, as skipped ones do.
    if let Some(n) = o.limit {
        tests.truncate(n);
    }
    let mut ignored_results = Vec::new();
    tests.retain(|t| match ignored.get(t) {
        Some(reason) => {
            ignored_results.push(TestResult {
                test: t.clone(),
                status: Status::Ignored,
                key: reason.clone(),
            });
            false
        }
        None => true,
    });
    if tests.is_empty() {
        eprintln!(
            "rhp-difftest phpt: no tests selected under {}{}",
            o.php_src.display(),
            if ignored_results.is_empty() {
                String::new()
            } else {
                format!(" ({} on tests/phpt/ignored.txt)", ignored_results.len())
            }
        );
        return ExitCode::from(2);
    }

    // A private scratch copy of every tree the selected tests live in, per
    // worker: tests create and delete files next to themselves, and each run
    // is reset to a snapshot afterwards, which must not race another worker.
    let work = o
        .work
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join(format!("rhp-phpt-{}", std::process::id())));
    let roots: BTreeSet<String> = tests.iter().map(|t| tree_root(t)).collect();
    let jobs = o.jobs.min(tests.len());
    let worker_roots = match make_worker_roots(&work, 0..jobs, &roots, &o.php_src) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("rhp-difftest phpt: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(d) = &o.fixtures {
        let _ = fs::create_dir_all(d);
    }

    let php_ini: Vec<String> = PHP_DEFAULT_INI.iter().map(|s| s.to_string()).collect();
    let mut base_env: Vec<(String, String)> = BASE_ENV
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let faketime = match o.faketime.as_deref().map(faketime_env).transpose() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("rhp-difftest phpt: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(f) = &faketime {
        eprintln!("phpt: clocks faked from {FAKETIME_START} ({})", f[1].1);
        base_env.extend(f.iter().cloned());
    }
    let ctx = Ctx {
        php_modules: php_modules(&php_ini),
        php_ini: php_ini.clone(),
        base_env,
        rhp_bin,
        timeout: Duration::from_secs(o.timeout),
        mutate: o.mutate,
        mutate_all: o.mutate_all,
        seed: o.seed,
        fixtures: o.fixtures.clone(),
        php_findings: o.php_findings.clone(),
        minimize: o.minimize,
        skew: Duration::ZERO,
        version_skew: None,
        cgi,
        ini: std::sync::Arc::new(IniRules {
            probe: IniProbe::new(Some(php_bin().into()), &php_ini),
            extra: o.rhp_ini.clone(),
        }),
    };
    let ctx = Ctx {
        version_skew: version_skew(&ctx, &worker_roots[0]),
        ..ctx
    };
    if let Some((v, r)) = &ctx.version_skew {
        eprintln!(
            "phpt: php is {v} but rhp reports {r}; tests printing it are skipped \
             (a CGI response's X-Powered-By is rewritten to {r})"
        );
    }
    match &ctx.cgi {
        Some(c) => eprintln!(
            "phpt: CGI tests: {} vs `{}`",
            c.php_cgi.display(),
            c.candidate.join(" ")
        ),
        None => eprintln!("phpt: no php-cgi found (pass --php-cgi); CGI tests are skipped"),
    }
    eprintln!(
        "phpt: {} tests in {} tree(s), {} jobs{}",
        tests.len(),
        roots.len(),
        jobs,
        if o.mutate > 0 {
            format!(", {} mutants/passing test", o.mutate)
        } else {
            String::new()
        }
    );

    // Run, in parallel.
    if let Some(d) = &o.php_findings {
        let _ = fs::create_dir_all(d);
    }
    let sinks = Sinks::default();
    let started = Instant::now();
    let mut results = run_parallel(&ctx, &worker_roots, &tests, &sinks);

    // A ratchet list must not contain tests that only match by luck (output
    // with the current time, say): re-run every match and keep the steady ones.
    let mut flaky = Vec::new();
    if o.write_known.is_some() {
        let matched: Vec<String> = results
            .iter()
            .filter(|r| r.status == Status::Match)
            .map(|r| r.test.clone())
            .collect();
        // Under --faketime every process's clock starts at the same instant
        // whatever the wall clock says, so a gap between the engines proves
        // nothing: the re-run only catches other nondeterminism.
        let skew = if faketime.is_some() {
            Duration::ZERO
        } else {
            Duration::from_millis(1100)
        };
        eprintln!(
            "phpt: confirming {} matches (re-run{})",
            matched.len(),
            if skew.is_zero() {
                String::new()
            } else {
                format!(" with {:.1}s between engines", skew.as_secs_f64())
            }
        );
        let confirm_ctx = Ctx {
            mutate: 0,
            fixtures: None,
            php_findings: None,
            skew,
            ..ctx.clone()
        };
        // Each confirming run spends the gap asleep, so run several per core:
        // extra scratch copies, for the trees the matches live in.
        let mut pool = worker_roots.clone();
        if !skew.is_zero() {
            let confirm_roots: BTreeSet<String> = matched.iter().map(|t| tree_root(t)).collect();
            let extra = (jobs * CONFIRM_OVERSUBSCRIBE).min(matched.len()).max(jobs);
            match make_worker_roots(&work, jobs..extra, &confirm_roots, &o.php_src) {
                Ok(more) => pool.extend(more),
                Err(e) => eprintln!("phpt: confirming on {jobs} workers only ({e})"),
            }
        }
        let again = run_parallel(&confirm_ctx, &pool, &matched, &Sinks::default());
        for r in again.into_iter().filter(|r| r.status != Status::Match) {
            if let Some(orig) = results.iter_mut().find(|o| o.test == r.test) {
                orig.status = Status::Mismatch;
                orig.key = format!("flaky: {}", r.key);
            }
            flaky.push(r.test);
        }
    }
    let mut mutant_results = sinks.mutants.into_inner().unwrap();
    mutant_results.sort_by(|a, b| a.test.cmp(&b.test).then(a.mutation.cmp(&b.mutation)));
    let mstats = sinks.mstats.into_inner().unwrap();
    let mut php_findings = sinks.php.into_inner().unwrap();
    php_findings.sort_by(|a, b| a.test.cmp(&b.test).then(a.mutation.cmp(&b.mutation)));
    if !o.keep_work && o.work.is_none() {
        let _ = fs::remove_dir_all(&work);
    }

    results.extend(ignored_results);
    results.sort_by(|a, b| a.test.cmp(&b.test));

    // Summary.
    let count = |st: Status| results.iter().filter(|r| r.status == st).count();
    let ran = results.len() - count(Status::Skipped) - count(Status::Ignored);
    println!(
        "\nphpt: {} selected, {} ran, {} skipped, {} ignored ({:.0}s)",
        results.len(),
        ran,
        count(Status::Skipped),
        count(Status::Ignored),
        started.elapsed().as_secs_f64()
    );
    for st in [
        Status::Match,
        Status::DiagnosticsOnly,
        Status::Mismatch,
        Status::Unsupported,
        Status::RhpCrash,
        Status::RhpTimeout,
    ] {
        let n = count(st);
        println!(
            "  {:17} {n:6}  {:5.1}%",
            st.label(),
            100.0 * n as f64 / ran.max(1) as f64
        );
    }
    let via_spec = results
        .iter()
        .filter(|r| r.status == Status::Match && r.key == "spec")
        .count();
    if via_spec > 0 {
        println!("  ({via_spec} of the matches differ from php but both satisfy the test's EXPECTF/EXPECTREGEX)");
    }
    if count(Status::Skipped) > 0 {
        println!("\nskipped, by reason:");
        print_clusters(
            results
                .iter()
                .filter(|r| r.status == Status::Skipped)
                .map(|r| (r.key.split(':').next().unwrap_or(&r.key), r.test.as_str())),
            8,
        );
    }
    if count(Status::Ignored) > 0 {
        println!("\nignored (tests/phpt/ignored.txt), by reason:");
        print_clusters(
            results
                .iter()
                .filter(|r| r.status == Status::Ignored)
                .map(|r| (r.key.split(':').next().unwrap_or(&r.key), r.test.as_str())),
            8,
        );
    }
    let failing: Vec<&TestResult> = results
        .iter()
        .filter(|r| !matches!(r.status, Status::Match | Status::Skipped | Status::Ignored))
        .collect();
    if !failing.is_empty() {
        println!("\ntop clusters (everything that ran but didn't match):");
        print_clusters(
            failing.iter().map(|r| (r.key.as_str(), r.test.as_str())),
            o.top,
        );
    }
    if o.mutate > 0 {
        let compared = mstats.compared;
        let divergent = mutant_results.len();
        println!(
            "\nmutation: {} mutants; {} discarded (php parse error/timeout/crash); {} php-only; \
             {} compared with rhp, {} diverged",
            mstats.generated, mstats.discarded, mstats.php_only, compared, divergent
        );
        for st in [
            Status::DiagnosticsOnly,
            Status::Mismatch,
            Status::Unsupported,
            Status::RhpCrash,
            Status::RhpTimeout,
        ] {
            let n = mutant_results.iter().filter(|m| m.status == st).count();
            if n > 0 {
                println!("  {:17} {n:6}", st.label());
            }
        }
        if divergent > 0 {
            println!("\ntop mutant clusters:");
            let examples: Vec<(String, String)> = mutant_results
                .iter()
                .map(|m| (m.key.clone(), format!("{} [{}]", m.test, m.mutation)))
                .collect();
            print_clusters(
                examples.iter().map(|(k, e)| (k.as_str(), e.as_str())),
                o.top,
            );
        }
    }

    if !php_findings.is_empty() {
        let clusters: BTreeSet<&str> = php_findings.iter().map(|f| f.key.as_str()).collect();
        println!(
            "\nphp itself misbehaved (possible php bugs): {} run(s) in {} cluster(s){}",
            php_findings.len(),
            clusters.len(),
            match &o.php_findings {
                Some(d) => format!("; repros in {}", d.display()),
                None => "; pass --php-findings <dir> for repros".to_string(),
            }
        );
        let examples: Vec<(String, String)> = php_findings
            .iter()
            .map(|f| {
                let what = f.mutation.as_deref().unwrap_or("the test itself");
                (f.key.clone(), format!("{} [{what}]", f.test))
            })
            .collect();
        print_clusters(
            examples.iter().map(|(k, e)| (k.as_str(), e.as_str())),
            o.top,
        );
    }

    if let Some(p) = &o.json {
        if let Err(e) = write_json(p, &results, &mutant_results, &php_findings) {
            eprintln!("rhp-difftest phpt: writing {}: {e}", p.display());
        }
    }
    if !flaky.is_empty() {
        println!("\nunsteady (matched, then diverged on the time-skewed re-run; left out of --write-known):");
        for t in &flaky {
            println!("  {t}");
        }
    }
    if let Some(p) = &o.write_known {
        let mut s = String::from(
            "# php-src .phpt tests rhp matches php on (rhp-difftest phpt --write-known).\n\
             # CI fails if one stops matching: fix the regression, or remove the line.\n",
        );
        for r in results.iter().filter(|r| r.status == Status::Match) {
            s.push_str(&r.test);
            s.push('\n');
        }
        if let Err(e) = fs::write(p, s) {
            eprintln!("rhp-difftest phpt: writing {}: {e}", p.display());
        } else {
            println!(
                "\nwrote {} passing tests to {}",
                count(Status::Match),
                p.display()
            );
        }
    }

    if known.is_some() {
        // A known test may only be skipped for an environmental reason php
        // itself reports (its SKIPIF, a php timeout/crash). Any other skip —
        // a missing extension, an unreadable file, a newly unsupported
        // section — would silently drop the test from the ratchet.
        let allowed_skip = |r: &TestResult| {
            r.status == Status::Skipped
                && matches!(
                    r.key.as_str(),
                    "skipif" | "php-timeout" | "php-crash" | "version"
                )
        };
        let failures: Vec<&TestResult> = results
            .iter()
            .filter(|r| r.status != Status::Match && !allowed_skip(r))
            .collect();
        let allowed: Vec<&TestResult> = results.iter().filter(|r| allowed_skip(r)).collect();
        for r in &allowed {
            println!(
                "note: known-passing test skipped here: {} ({})",
                r.test, r.key
            );
        }
        let ran = results.len() - allowed.len();
        let mut failed = false;
        if !missing_known.is_empty() {
            println!(
                "\n{} known-passing test(s) not found in --php-src:",
                missing_known.len()
            );
            for t in &missing_known {
                println!("  {t}");
            }
            failed = true;
        }
        if !failures.is_empty() {
            println!("\n{} known-passing test(s) regressed:", failures.len());
            for r in &failures {
                println!("  {}  [{}] {}", r.test, r.status.label(), r.key);
            }
            failed = true;
        }
        if ran == 0 {
            println!("\nno known-passing test actually ran");
            failed = true;
        }
        if failed {
            return ExitCode::FAILURE;
        }
        println!("\nall {ran} runnable known-passing tests still match");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections() {
        let p = Phpt::parse(
            b"--TEST--\nname\n--SKIPIF--\n<?php ?>\n--FILE--\n<?php\necho 1;\n?>\n--EXPECT--\n1\n",
        );
        assert_eq!(p.section("TEST"), Some(&b"name\n"[..]));
        assert_eq!(p.section("FILE"), Some(&b"<?php\necho 1;\n?>\n"[..]));
        assert_eq!(p.section("EXPECT"), Some(&b"1\n"[..]));
        assert!(p.has("SKIPIF") && !p.has("INI"));
    }

    #[test]
    fn section_header_rules() {
        assert_eq!(section_header(b"--FILE--\n"), Some("FILE".into()));
        assert_eq!(
            section_header(b"--FILE_EXTERNAL--  \r\n"),
            Some("FILE_EXTERNAL".into())
        );
        assert_eq!(section_header(b"--file--\n"), None);
        assert_eq!(section_header(b"-- FILE --\n"), None);
        assert_eq!(section_header(b"----\n"), None);
        assert_eq!(section_header(b"$a--;--\n"), None);
    }

    #[test]
    fn normalises_like_run_tests() {
        assert_eq!(normalise(b"\n a\r\nb \x0b\0\n"), b"a\nb");
    }

    #[test]
    fn strips_diagnostics_and_blank_lines() {
        let php = b"1\n\nWarning: Undefined variable $x in /t.php on line 3\n2\n";
        assert_eq!(strip_diagnostics(php), strip_diagnostics(b"1\n2\n"));
    }

    #[test]
    fn ignore_list_format() {
        let l = parse_ignored("# c\n\na/b.phpt  why: this\nc.phpt\tthat\n").unwrap();
        assert_eq!(l["a/b.phpt"], "why: this");
        assert_eq!(l["c.phpt"], "that");
        assert_eq!(
            parse_ignored("a.phpt\n").unwrap_err(),
            "line 1: a.phpt has no reason"
        );
        assert_eq!(
            parse_ignored("a.phpt x\na.phpt y\n").unwrap_err(),
            "line 2: a.phpt is listed twice"
        );
    }

    /// The committed list parses, is C-sorted, and known-passing.txt lists
    /// none of it (`--check-known` checks that too).
    #[test]
    fn committed_ignore_list() {
        let ignored = parse_ignored(IGNORED).unwrap();
        assert!(ignored.keys().all(|t| t.ends_with(".phpt")));
        let order: Vec<&str> = IGNORED
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        assert!(order.is_sorted(), "tests/phpt/ignored.txt is not C-sorted");
        let known = read_known(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/phpt/known-passing.txt"),
        )
        .unwrap();
        let both: Vec<&String> = known.iter().filter(|t| ignored.contains_key(*t)).collect();
        assert!(both.is_empty(), "known-passing and ignored: {both:?}");
    }

    #[test]
    fn ini_noops_run_others_skip() {
        let dir = Path::new("/w");
        let rules = IniRules {
            probe: IniProbe::new(None, &[]),
            extra: Vec::new(),
        };
        let test_ini = |p: &Phpt| rules.test_ini(p, dir, false);
        let p = Phpt::parse(b"--INI--\nprecision=14\nopcache.enable=0\n--FILE--\n");
        assert_eq!(test_ini(&p).unwrap().len(), 2);
        let p = Phpt::parse(b"--INI--\nvariables_order=GPS\n--FILE--\n");
        assert_eq!(test_ini(&p).unwrap_err(), "ini:variables_order");
        let p = Phpt::parse(b"--INI--\noutput_buffering=0\ndisplay_errors=On\n--FILE--\n");
        assert_eq!(test_ini(&p).unwrap().len(), 2);
        let p = Phpt::parse(b"--INI--\noutput_buffering=4096\n--FILE--\n");
        assert_eq!(test_ini(&p).unwrap_err(), "ini:output_buffering");
        // Directives rhp applies run, and only those reach rhp.
        let p = Phpt::parse(
            b"--INI--\npcre.jit=0\noutput_buffering=0\nopcache.jit=tracing\n--FILE--\n",
        );
        let ini = test_ini(&p).unwrap();
        assert_eq!(ini.len(), 3);
        let to_rhp: Vec<&String> = ini.iter().filter(|kv| rules.rhp_applies(kv)).collect();
        assert_eq!(to_rhp, ["pcre.jit=0", "opcache.jit=tracing"]);
        // opcache runs, except with dumps or the file cache. Listed opcode
        // dumps never get here; an unlisted one is flagged.
        let p = Phpt::parse(b"--INI--\nopcache.enable_cli=1\n--FILE--\n");
        assert_eq!(test_ini(&p).unwrap(), ["opcache.enable_cli=1"]);
        let p = Phpt::parse(b"--INI--\nopcache.opt_debug_level=0x20000\n--FILE--\n");
        assert_eq!(
            test_ini(&p).unwrap_err(),
            "opcode dump not on tests/phpt/ignored.txt"
        );
        let p = Phpt::parse(b"--INI--\nopcache.opt_debug_level=0\n--FILE--\n");
        assert!(test_ini(&p).is_ok());
        let p = Phpt::parse(b"--INI--\n;opcache.opt_debug_level=0x20000\n--FILE--\n");
        assert!(test_ini(&p).is_ok());
        let p = Phpt::parse(b"--INI--\nopcache.jit_debug=0x1\n--FILE--\n");
        assert_eq!(test_ini(&p).unwrap_err(), "opcache:jit_debug");
        let p = Phpt::parse(b"--INI--\nopcache.file_cache={PWD}\n--FILE--\n");
        assert_eq!(test_ini(&p).unwrap_err(), "opcache:file_cache");
        // As run-tests' settings2array: trimmed, the last value kept, and a
        // line without '=' dropped.
        let p = Phpt::parse(
            b"--INI--\nerror_reporting = E_ALL\nprecision=10\nprecision = 12 \nflag\n--FILE--\n",
        );
        assert_eq!(
            test_ini(&p).unwrap(),
            ["error_reporting=E_ALL", "precision=12"]
        );
        // --rhp-ini
        let rules = IniRules {
            probe: IniProbe::new(None, &[]),
            extra: vec!["variables_order".into()],
        };
        let p = Phpt::parse(b"--INI--\nvariables_order=GPS\n--FILE--\n");
        assert_eq!(
            rules.test_ini(&p, dir, false).unwrap(),
            ["variables_order=GPS"]
        );
    }

    #[test]
    fn ini_substitutions() {
        let dir = Path::new("/w/t");
        assert_eq!(
            substitute_ini("include_path={PWD}", dir).unwrap(),
            "include_path=/w/t"
        );
        std::env::set_var("RHP_PHPT_UNIT_ENV", "u");
        assert_eq!(
            substitute_ini("a={ENV:RHP_PHPT_UNIT_ENV}!", dir).unwrap(),
            "a=u!"
        );
        assert_eq!(
            substitute_ini("a={ENV:RHP_PHPT_UNSET_ENV_X}", dir).unwrap_err(),
            "ini:env:RHP_PHPT_UNSET_ENV_X"
        );
    }

    /// Needs php: a setting equal to php's effective default is a no-op, an
    /// unknown directive never is.
    #[test]
    fn ini_probe_effective_values() {
        if !crate::php_available() {
            return;
        }
        let defaults: Vec<String> = PHP_DEFAULT_INI.iter().map(|s| s.to_string()).collect();
        let probe = IniProbe::new(Some(php_bin().into()), &defaults);
        assert!(probe.is_noop("allow_url_fopen", "allow_url_fopen=1"));
        assert!(probe.is_noop("zend.enable_gc", "zend.enable_gc=On"));
        // E_ALL is 30719 in php 8.5, run-tests sets 32767.
        assert!(!probe.is_noop("error_reporting", "error_reporting=E_ALL"));
        assert!(!probe.is_noop("precision", "precision=10"));
        assert!(!probe.is_noop("no_such.directive", "no_such.directive=1"));
    }

    #[test]
    fn tree_roots() {
        assert_eq!(
            tree_root("ext/standard/tests/array/a.phpt"),
            "ext/standard/tests"
        );
        assert_eq!(tree_root("Zend/tests/x/y.phpt"), "Zend/tests");
        assert_eq!(tree_root("tests/lang/015.phpt"), "tests");
        assert_eq!(tree_root("sapi/cli/a.phpt"), "sapi/cli");
    }

    fn out(merged: &[u8]) -> Outcome {
        Outcome {
            out: merged.to_vec(),
            timed_out: false,
            signaled: false,
            signal: None,
        }
    }

    #[test]
    fn php_anomalies() {
        let asan = b"x\n=================================================================\n\
==123==ERROR: AddressSanitizer: heap-use-after-free on address 0x6020 at pc 0x55 bp 0x7f\n\
READ of size 8 at 0x6020 thread T0\n\
    #0 0x55d5 in __interceptor_memcpy (/php+0x1)\n\
    #1 0x55d6 in zend_hash_find /src/Zend/zend_hash.c:123:5\n\
    #2 0x55d7 in zif_foo /src/ext/standard/foo.c:9:1\n";
        assert_eq!(
            php_anomaly(&out(asan)).as_deref(),
            Some("asan heap-use-after-free in zend_hash_find")
        );
        let ubsan = b"/src/Zend/zend_operators.c:2041:18: runtime error: signed integer overflow: 9223372036854775807 + 1 cannot be represented in type 'long'\n";
        assert_eq!(
            php_anomaly(&out(ubsan)).as_deref(),
            Some("ubsan zend_operators.c:2041: signed integer overflow: N + N cannot be represented in type 'long'")
        );
        assert_eq!(
            php_anomaly(&out(b"zend_mm_heap corrupted\n")).as_deref(),
            Some("zend_mm_heap corrupted")
        );
        let mut segv = out(b"partial");
        segv.signal = Some(11);
        segv.signaled = true;
        assert_eq!(
            php_anomaly(&segv).as_deref(),
            Some("php killed by signal 11")
        );
        // Ordinary php errors and timeouts are not php bugs.
        assert_eq!(
            php_anomaly(&out(b"PHP Fatal error:  Allowed memory size exhausted\n")),
            None
        );
        let mut slow = out(b"");
        slow.timed_out = true;
        assert_eq!(php_anomaly(&slow), None);
        // A program printing the words is not a sanitizer report.
        for text in [
            &b"parse.php:3: runtime error: something\n"[..],
            b"Bug #63305 (zend_mm_heap corrupted with traits)\n",
            b"echo ERROR: AddressSanitizer: heap-use-after-free\n",
        ] {
            assert_eq!(php_anomaly(&out(text)), None);
        }
        // Addresses differ per run (ASLR): the key must not carry them.
        let ub = |a: &str| {
            php_anomaly(&out(format!("/s/Zend/zend_execute.h:364:64: runtime error: member access within address {a} with insufficient space\n").as_bytes()))
        };
        assert_eq!(ub("0xaaaae4bf9320"), ub("0xaaaad7bf9320"));
        assert_eq!(
            ub("0x1").as_deref(),
            Some("ubsan zend_execute.h:364: member access within address ADDR with insufficient space")
        );
        // Where a stack overflow is caught varies run to run.
        let so = b"==9==ERROR: AddressSanitizer: stack-overflow on address 0xfff\n    #0 0x1 in zend_array_destroy /s/Zend/zend_hash.c:1815\n";
        assert_eq!(
            php_anomaly(&out(so)).as_deref(),
            Some("asan stack-overflow")
        );
        // The key names the first frame in php's sources, past interceptors.
        let hbo = b"==9==ERROR: AddressSanitizer: heap-buffer-overflow on address 0x1\n    #0 0x1 in memchr (/usr/lib/libasan.so+0x1)\n    #1 0x2 in php_memnstr /s/main/php.h:9:1\n";
        assert_eq!(
            php_anomaly(&out(hbo)).as_deref(),
            Some("asan heap-buffer-overflow in php_memnstr")
        );
    }

    #[test]
    fn classification() {
        let c = |p: &[u8], r: &[u8]| classify(&out(p), &out(r));
        assert_eq!(c(b"1\n", b"1").status, Status::Match);
        let d = c(
            b"1\n\nWarning: Undefined variable $x in /a/t.php on line 3\n",
            b"1\n",
        );
        assert_eq!(d.status, Status::DiagnosticsOnly);
        assert_eq!(d.key, "diagnostics: Warning: Undefined variable $x");
        let u = c(
            b"1\n2\n",
            b"1\nRHP_UNSUPPORTED: goto statement is not supported (phase 2)\n",
        );
        assert_eq!(u.status, Status::Unsupported);
        assert_eq!(u.key, "unsupported: goto statement is not supported");
        let m = c(b"int(3)\n", b"int(4)\n");
        assert_eq!(
            (m.status, m.key.as_str()),
            (Status::Mismatch, "diff: php «int(N)» rhp «int(N)»")
        );
        let f = c(
            b"a\n",
            b"PHP Fatal error:  Uncaught Error: Call to undefined function strtok()\n",
        );
        assert_eq!(
            f.key,
            "rhp fatal: Error: Call to undefined function strtok()"
        );
        let k = c(
            b"a",
            b"thread 'main' panicked at crates/x/src/lib.rs:12:5:\n",
        );
        assert_eq!(k.status, Status::RhpCrash);
    }

    #[test]
    fn stderr_is_compared() {
        // rhp fatals in a trailing silent statement: same stdout, but the
        // merged stream differs and php printed no diagnostic -> not a match.
        let v = classify(
            &out(b"done\n"),
            &out(b"done\nPHP Fatal error:  Uncaught Error: Call to undefined function cleanup()\n"),
        );
        assert_eq!(v.status, Status::Mismatch);
        assert_eq!(
            v.key,
            "rhp fatal: Error: Call to undefined function cleanup()"
        );
    }

    #[test]
    fn blank_line_differences_are_not_diagnostics() {
        let v = classify(
            &out(b"a\n\nb\n\nWarning: x in /t.php on line 1\n"),
            &out(b"a\nb\n"),
        );
        assert_eq!(v.status, Status::Mismatch);
    }

    #[test]
    fn cluster_key_keeps_later_numbers() {
        assert_eq!(cluster_key("", "int(3) and 42"), "int(N) and N");
    }

    #[test]
    fn run_proc_merges_streams_and_survives_background_children() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo out; echo err >&2; (sleep 30 &) ; echo end"]);
        let started = Instant::now();
        let o = run_proc(cmd, None, Duration::from_secs(5));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(String::from_utf8_lossy(&o.out), "out\nerr\nend\n");
        assert!(!o.timed_out && !o.signaled);
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        assert!(run_proc(cmd, None, Duration::from_millis(200)).timed_out);
    }

    #[test]
    fn uncaught_exception_trace_counts_as_diagnostic() {
        let php = b"start\nPHP Fatal error:  Uncaught Error: Call to undefined function f() in /t.php:3\n\
                    Stack trace:\n#0 {main}\n  thrown in /t.php on line 3\n";
        assert_eq!(strip_diagnostics(php), "start");
        let php = b"start\n\nFatal error: Uncaught Exception: a in /t.php:2\nStack trace:\n#0 {main}\n\n\
                    Next RuntimeException: b in /t.php:4\nStack trace:\n#0 {main}\n  thrown in /t.php on line 4\n";
        assert_eq!(strip_diagnostics(php), "start");
        // A `#`-line that isn't part of a trace is real output.
        assert_eq!(strip_diagnostics(b"#1 not a frame\n"), "#1 not a frame");
    }

    #[test]
    fn restore_removes_only_new_paths() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("rhp-phpt-restore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("keep_dir")).unwrap();
        fs::write(d.join("keep.txt"), b"x").unwrap();
        let snap = snapshot(&d);
        fs::write(d.join("new.txt"), b"y").unwrap();
        fs::write(d.join("keep_dir/new_inner.txt"), b"y").unwrap();
        fs::create_dir_all(d.join("new_dir/sub")).unwrap();
        fs::write(d.join("new_dir/sub/f"), b"z").unwrap();
        fs::set_permissions(d.join("new_dir"), fs::Permissions::from_mode(0o000)).unwrap();
        restore(&d, &snap);
        assert_eq!(snapshot(&d).paths, snap.paths);
        assert_eq!(fs::read(d.join("keep.txt")).unwrap(), b"x");
        // A run that puts a directory's mtime back is still caught (ctime).
        let mtime = fs::metadata(d.join("keep_dir"))
            .unwrap()
            .modified()
            .unwrap();
        fs::write(d.join("keep_dir/sneaky.txt"), b"y").unwrap();
        fs::File::open(d.join("keep_dir"))
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        restore(&d, &snap);
        assert_eq!(snapshot(&d).paths, snap.paths);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn expectf_translation_matches_run_tests() {
        assert_eq!(preg_quote("a.b/c(d)"), "a\\.b\\/c\\(d\\)");
        assert_eq!(expectf_to_regex("int(%d)"), "int\\(\\d+\\)");
        assert_eq!(expectf_to_regex("x%r[ab]+%ry."), "x([ab]+)y\\.");
        assert_eq!(
            expectf_to_regex("%s|%S|%a|%A"),
            "[^\\r\\n]+\\|[^\\r\\n]*\\|.+?\\|.*?"
        );
        assert_eq!(expectf_to_regex("100%"), "100%"); // lone % is literal
    }

    #[test]
    fn spec_fallback_accepts_unspecified_values() {
        let p = Phpt::parse(b"--FILE--\n<?php\n--EXPECTREGEX--\nint\\(-[1-9][0-9]*\\)\n");
        assert_eq!(spec_matches(&p, b"int(-1)\n"), Some(true));
        assert_eq!(spec_matches(&p, b"int(-32)"), Some(true));
        assert_eq!(spec_matches(&p, b"int(1)"), Some(false));
        let p = Phpt::parse(b"--FILE--\n<?php\n--EXPECTF--\nfloat(%f) at %s\nDone\n");
        assert_eq!(spec_matches(&p, b"float(1.5) at x.php\nDone"), Some(true));
        assert_eq!(spec_matches(&p, b"float(1.5) at x.php\n"), Some(false));
        let p = Phpt::parse(b"--FILE--\n<?php\n--EXPECT--\n1\n");
        assert_eq!(spec_matches(&p, b"1"), None);
    }

    #[test]
    fn portability_heuristic() {
        assert!(is_portable(b"<?php var_dump(1 + 2);"));
        assert!(!is_portable(b"<?php include 'x.inc';"));
        assert!(!is_portable(b"<?php echo __DIR__;"));
    }

    #[test]
    fn json_escaping() {
        assert_eq!(json_str("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
    }

    #[test]
    fn cgi_detection_follows_run_tests() {
        let cgi = |s: &[u8]| Phpt::parse(s).is_cgi();
        assert!(cgi(b"--CGI--\n--FILE--\n<?php\n"));
        assert!(cgi(b"--GET--\na=1\n--FILE--\n<?php\n"));
        assert!(cgi(b"--EXPECTHEADERS--\nStatus: 404\n--FILE--\n<?php\n"));
        assert!(!cgi(b"--GET--\n--FILE--\n<?php\n"));
        assert!(!cgi(b"--FILE--\n<?php\n"));
    }

    #[test]
    fn cgi_requests_match_run_tests() {
        let script = Path::new("/w/t.php");
        let env = |r: &CgiRequest, k: &str| r.env.get(k).cloned().unwrap_or_default();

        let p = Phpt::parse(b"--GET--\na=1&b=2\n--COOKIE--\nc=3\n--FILE--\n<?php\n");
        let r = cgi_request(&p, script, &[]).unwrap();
        assert_eq!(env(&r, "REQUEST_METHOD"), "GET");
        assert_eq!(env(&r, "QUERY_STRING"), "a=1&b=2");
        assert_eq!(env(&r, "HTTP_COOKIE"), "c=3");
        assert_eq!(env(&r, "SCRIPT_FILENAME"), "/w/t.php");
        assert_eq!(env(&r, "PATH_TRANSLATED"), "/w/t.php");
        assert_eq!(env(&r, "REDIRECT_STATUS"), "1");
        assert!(r.env.contains_key("CONTENT_LENGTH") && r.body.is_none());
        assert!(cgi_passes_args(&r), "a GET gets `-- <ARGS>`");

        let p = Phpt::parse(b"--POST--\nx=1\n\n--FILE--\n<?php\n");
        let r = cgi_request(&p, script, &[]).unwrap();
        assert!(!cgi_passes_args(&r), "run-tests drops ARGS for a POST");
        assert_eq!(env(&r, "REQUEST_METHOD"), "POST");
        assert_eq!(env(&r, "CONTENT_TYPE"), "application/x-www-form-urlencoded");
        assert_eq!(env(&r, "CONTENT_LENGTH"), "3");
        assert_eq!(r.body.as_deref(), Some(&b"x=1"[..]));

        let p = Phpt::parse(
            b"--POST_RAW--\nContent-Type: multipart/form-data; boundary=x1\r\n--x1\r\nbody\r\n--x1--\n--FILE--\n<?php\n",
        );
        let r = cgi_request(&p, script, &[("REQUEST_METHOD".into(), "PUT".into())]).unwrap();
        assert_eq!(env(&r, "CONTENT_TYPE"), "multipart/form-data; boundary=x1");
        assert_eq!(env(&r, "REQUEST_METHOD"), "PUT"); // ENV wins where run-tests only fills
        assert_eq!(r.body.as_deref(), Some(&b"--x1\r\nbody\r\n--x1--"[..]));
        assert_eq!(env(&r, "CONTENT_LENGTH"), "18");

        let p = Phpt::parse(b"--GZIP_POST--\na=b\n--FILE--\n<?php\n");
        let r = cgi_request(&p, script, &[]).unwrap();
        assert_eq!(env(&r, "HTTP_CONTENT_ENCODING"), "gzip");
        let body = r.body.unwrap();
        assert_eq!(&body[..2], b"\x1f\x8b");
        let mut plain = Vec::new();
        flate2::read::GzDecoder::new(&body[..])
            .read_to_end(&mut plain)
            .unwrap();
        assert_eq!(plain, b"a=b");
    }

    #[test]
    fn cgi_output_splits_and_version_is_rewritten() {
        let out = b"X-Powered-By: PHP/8.5.9\r\nContent-type: text/html; charset=UTF-8\r\n\r\nbody PHP/8.5.9";
        let (h, b) = split_cgi(out).unwrap();
        assert_eq!(
            h,
            &b"X-Powered-By: PHP/8.5.9\r\nContent-type: text/html; charset=UTF-8"[..]
        );
        assert_eq!(b, b"body PHP/8.5.9");
        assert_eq!(split_cgi(b"a\n\nb").unwrap(), (&b"a"[..], &b"b"[..]));
        assert!(split_cgi(b"no blank line\r\n").is_none());
        let mut v = out.to_vec();
        normalise_powered_by(&mut v, "8.5.9", "8.5.8");
        assert!(v.starts_with(b"X-Powered-By: PHP/8.5.8\r\n"));
        assert!(v.ends_with(b"body PHP/8.5.9"), "the body is left alone");
    }
}
