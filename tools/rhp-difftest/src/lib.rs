//! Differential-test harness for extensions reimplemented in Rust.
//!
//! The strategy is dead simple and hard to fool: take one piece of PHP source,
//! run it through the **real `php` binary** (the canonical C engine, our oracle)
//! *and* through **rhp's VM**, and compare their stdout byte-for-byte. An
//! extension reimplemented in Rust is "correct" exactly when rhp reproduces
//! the C engine's output over a broad, generated corpus of inputs.
//!
//! This module provides the primitives, for the `rhp-difftest` binary and
//! for every test crate that compares with php:
//!   * [`php_bin`] / [`php_available`] / [`php_cgi`] — find php and php-cgi,
//!   * [`spawn`] / [`Ran`] — run a process and compare what it printed,
//!   * [`read_known`] — read a ratchet list,
//!   * [`run_php`] / [`run_rhp`] — run a snippet on php / on rhp in-process,
//!   * [`diff_source`] / [`Diff`] — run both and classify the result,
//!   * [`Rng`] + the `php_bytes_literal` / framing helpers — build generated
//!     corpora deterministically so a failing run is reproducible from a seed.
//!
//! Extension-specific sweeps (e.g. every `hash()` algorithm over random byte
//! strings) are built on top of these in the `rhp-difftest` binary and in
//! the crate's integration tests.

pub mod app;
pub mod gen;
pub mod golden;
pub mod lint;
pub mod mutate;
pub mod phpt;
pub mod sweeps;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rhp_sapi::{CaptureSapi, Outcome, Request, Script};

// ---------------------------------------------------------------------------
// the two engines
// ---------------------------------------------------------------------------

/// The `php` binary to shell out to (override with `RHP_PHP`).
pub fn php_bin() -> String {
    std::env::var("RHP_PHP").unwrap_or_else(|_| "php".to_string())
}

/// Whether a usable `php` binary is on `PATH`. Sweeps and differential tests
/// skip (rather than fail) when this is false, so the workspace still builds
/// and tests where no PHP is installed.
pub fn php_available() -> bool {
    Command::new(php_bin())
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The php-cgi to compare `rhp --cgi` with: `RHP_PHP_CGI`, else the
/// `php-cgi` next to the php [`php_bin`] names (setup-php and Homebrew
/// install them side by side), else the first on `PATH`, else Homebrew's.
/// `None` when there is none.
///
/// On CI the php next to `php` is the `-n` shim (the setup-php action),
/// which the phpt and app harnesses want; the CGI fixtures need the real
/// binary and get it through `RHP_PHP_CGI`.
pub fn php_cgi() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("RHP_PHP_CGI") {
        return Some(PathBuf::from(p));
    }
    let on_path = |name: &str| -> Option<PathBuf> {
        if name.contains('/') {
            return Some(PathBuf::from(name));
        }
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    };
    let beside = on_path(&php_bin()).and_then(|php| Some(php.parent()?.join("php-cgi")));
    beside
        .into_iter()
        .chain(on_path("php-cgi"))
        .chain([PathBuf::from("/opt/homebrew/bin/php-cgi")])
        .find(|p| p.is_file())
}

/// What a finished process printed, and its exit code (`None`: it was
/// killed by a signal).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ran {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: Option<i32>,
}

impl Ran {
    /// The exit code as text, `signal` for a process a signal killed.
    pub fn status_text(&self) -> String {
        self.status.map_or("signal".into(), |c| c.to_string())
    }

    /// How `got` (labelled `got_label`) differs from `self` (`want_label`):
    /// one report per stream that differs (stdout, stderr, exit status);
    /// empty when they agree.
    pub fn differences(&self, want_label: &str, got: &Ran, got_label: &str) -> Vec<String> {
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        let mut d = Vec::new();
        for (what, w, g) in [
            ("stdout", &self.stdout, &got.stdout),
            ("stderr", &self.stderr, &got.stderr),
        ] {
            if w != g {
                d.push(format!(
                    "{what}:\n--- {want_label}\n{}\n--- {got_label}\n{}",
                    text(w),
                    text(g)
                ));
            }
        }
        if self.status != got.status {
            d.push(format!(
                "exit: {want_label} {}, {got_label} {}",
                self.status_text(),
                got.status_text()
            ));
        }
        d
    }
}

/// Run `cmd` to the end with `stdin` as its standard input (`None`: none,
/// `/dev/null`) and capture its stdout and stderr. The input is written
/// from another thread, so a body larger than a pipe buffer can't deadlock
/// against output nobody reads yet.
pub fn spawn(cmd: &mut Command, stdin: Option<&[u8]>) -> std::io::Result<Ran> {
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let writer = match (child.stdin.take(), stdin) {
        (Some(mut pipe), Some(body)) => {
            let body = body.to_vec();
            // A child that exits without reading closes the pipe: not an error.
            Some(std::thread::spawn(move || {
                let _ = pipe.write_all(&body);
            }))
        }
        _ => None,
    };
    let out = child.wait_with_output()?;
    if let Some(w) = writer {
        let _ = w.join();
    }
    Ok(Ran {
        stdout: out.stdout,
        stderr: out.stderr,
        status: out.status.code(),
    })
}

/// `hay` with every `from` replaced by `to` (`hay` itself for an empty
/// `from`).
pub fn replace_bytes(hay: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    if from.is_empty() {
        return hay.to_vec();
    }
    let mut out = Vec::with_capacity(hay.len());
    let mut i = 0;
    while i < hay.len() {
        if hay[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

/// A ratchet or known list: one entry per line, the line's first
/// tab-separated field (what follows a tab is a note), trimmed; blank lines
/// and lines starting with `#` are skipped.
pub fn read_known(path: &Path) -> std::io::Result<Vec<String>> {
    Ok(std::fs::read_to_string(path)?
        .lines()
        .map(|l| l.split('\t').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect())
}

/// Run PHP `source` on the real C engine and capture stdout.
///
/// Uses `php -n` (no `php.ini`) with a fixed UTC timezone so results match the
/// rhp defaults; the source is fed on stdin so no temp files are touched.
pub fn run_php(source: &[u8]) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new(php_bin());
    cmd.args(["-n", "-d", "date.timezone=UTC", "-d", "error_reporting=0"]);
    let out = spawn(&mut cmd, Some(source)).map_err(|e| format!("spawn php: {e}"))?;
    if out.status != Some(0) {
        return Err(format!(
            "php exited {}: {}",
            out.status_text(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}

/// Run PHP `source` on rhp in-process, as a CLI request, and capture its
/// stdout. As with [`run_php`], a non-zero exit status (an uncaught
/// throwable, a fatal or parse error) is an error. Unlike php's side, rhp's
/// warnings stay on: a snippet that warns shows up as a mismatch.
pub fn run_rhp(source: &[u8]) -> Result<Vec<u8>, String> {
    let script = Script::Source {
        path: "difftest".to_string(),
        bytes: source.to_vec(),
    };
    // A genuine rhp panic (an engine bug) is a differential failure, not a
    // harness crash — catch it and report it like any other mismatch cause.
    let result = std::panic::catch_unwind(|| CaptureSapi::cli().run(Request::new(script)));
    match result {
        Ok((Outcome::Exit(0), out)) => Ok(out.stdout),
        Ok((outcome, out)) => {
            // The fatal error, not a warning logged before it.
            let why = out
                .log
                .iter()
                .find(|l| l.starts_with(b"PHP Fatal error") || l.starts_with(b"PHP Parse error"))
                .or(out.log.last())
                .map(|l| String::from_utf8_lossy(l).into_owned())
                .unwrap_or_else(|| String::from_utf8_lossy(&out.stderr).into_owned());
            Err(format!("rhp: {outcome:?}: {why}"))
        }
        Err(_) => Err("rhp: panicked".to_string()),
    }
}

/// Outcome of running one source through both engines.
#[derive(Debug)]
pub enum Diff {
    /// Both engines produced identical bytes.
    Match,
    /// Both ran but produced different bytes.
    Mismatch { php: Vec<u8>, rhp: Vec<u8> },
    /// The C engine failed to run this source (bad corpus, not a rhp bug).
    PhpErr(String),
    /// rhp refused/threw/panicked (e.g. an unsupported feature or a real bug).
    RhpErr(String),
}

/// Where a [`Diff::Mismatch`]'s two outputs first part ways. The sweeps print
/// one row per generated case, so the row index names the failing input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstDifference {
    /// The first row (0-based line) on which the outputs differ.
    Row {
        index: usize,
        php: String,
        rhp: String,
    },
    /// Every row both outputs have agrees; they differ in length (bytes).
    Length { php: usize, rhp: usize },
}

impl std::fmt::Display for FirstDifference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FirstDifference::Row { index, php, rhp } => {
                write!(f, "row {index}: php={php} rhp={rhp}")
            }
            FirstDifference::Length { php, rhp } => {
                write!(f, "length differs (php {php} B, rhp {rhp} B)")
            }
        }
    }
}

impl Diff {
    pub fn is_match(&self) -> bool {
        matches!(self, Diff::Match)
    }

    /// For a mismatch, the first row where php and rhp differ; `None` for
    /// any other outcome.
    pub fn first_difference(&self) -> Option<FirstDifference> {
        let Diff::Mismatch { php, rhp } = self else {
            return None;
        };
        let (p, q) = (String::from_utf8_lossy(php), String::from_utf8_lossy(rhp));
        let row = p
            .lines()
            .zip(q.lines())
            .enumerate()
            .find(|(_, (a, b))| a != b);
        Some(match row {
            Some((index, (a, b))) => FirstDifference::Row {
                index,
                php: a.to_string(),
                rhp: b.to_string(),
            },
            None => FirstDifference::Length {
                php: php.len(),
                rhp: rhp.len(),
            },
        })
    }

    /// What went wrong, for a test failure or report line: `None` on a
    /// match, else `MISMATCH: <first difference>`, `rhp error: …` or
    /// `php error: …`.
    pub fn failure(&self) -> Option<String> {
        match self {
            Diff::Match => None,
            Diff::Mismatch { .. } => self.first_difference().map(|d| format!("MISMATCH: {d}")),
            Diff::RhpErr(e) => Some(format!("rhp error: {e}")),
            Diff::PhpErr(e) => Some(format!("php error: {e}")),
        }
    }
}

/// Run one PHP source through both engines and classify the result.
pub fn diff_source(source: &[u8]) -> Diff {
    let php = match run_php(source) {
        Ok(o) => o,
        Err(e) => return Diff::PhpErr(e),
    };
    let rhp = match run_rhp(source) {
        Ok(o) => o,
        Err(e) => return Diff::RhpErr(e),
    };
    if php == rhp {
        Diff::Match
    } else {
        Diff::Mismatch { php, rhp }
    }
}

// ---------------------------------------------------------------------------
// deterministic input generation
// ---------------------------------------------------------------------------

/// A tiny splitmix64 PRNG. Deterministic and dependency-free so that any
/// failing differential run reproduces exactly from its seed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// `len` random bytes (full 0..=255 range, so binary safety is exercised).
    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next_u64() & 0xff) as u8).collect()
    }
}

/// A broad default corpus of byte strings for block-based algorithms: every
/// empty/short case, the exact block-boundary lengths where padding bugs hide
/// (55/56/63/64/65/119/120/127/128), and a batch of random binary strings.
pub fn default_byte_corpus(seed: u64, random_count: usize) -> Vec<Vec<u8>> {
    let mut rng = Rng::new(seed);
    let mut corpus: Vec<Vec<u8>> = Vec::new();
    let fixed_lens = [
        0usize, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 55, 56, 63, 64, 65, 119, 120, 127, 128, 256, 1000,
    ];
    for &l in &fixed_lens {
        corpus.push(rng.bytes(l));
    }
    // a few human-readable vectors too, to make failures easy to eyeball
    for s in [
        "",
        "a",
        "abc",
        "message digest",
        "The quick brown fox jumps over the lazy dog",
    ] {
        corpus.push(s.as_bytes().to_vec());
    }
    for _ in 0..random_count {
        let len = rng.below(300);
        corpus.push(rng.bytes(len));
    }
    corpus
}

/// Render bytes as a PHP double-quoted string of `\xNN` escapes — safe for any
/// byte including NUL, quotes and backslashes. Both engines parse it identically.
pub fn php_bytes_literal(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 4 + 2);
    s.push('"');
    for &b in bytes {
        s.push_str(&format!("\\x{b:02x}"));
    }
    s.push('"');
    s
}

/// Render a PHP array literal of byte-string elements.
pub fn php_bytes_array(inputs: &[Vec<u8>]) -> String {
    let mut s = String::from("[");
    for inp in inputs {
        s.push_str(&php_bytes_literal(inp));
        s.push(',');
    }
    s.push(']');
    s
}
