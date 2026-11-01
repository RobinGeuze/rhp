//! Differential-test harness for extensions reimplemented in Rust.
//!
//! The strategy is dead simple and hard to fool: take one piece of PHP source,
//! run it through the **real `php` binary** (the canonical C engine, our oracle)
//! *and* through **rhp's VM**, and compare their stdout byte-for-byte. An
//! extension reimplemented in Rust is "correct" exactly when rhp reproduces
//! the C engine's output over a broad, generated corpus of inputs.
//!
//! This module provides the primitives:
//!   * [`php_available`] / [`run_php`] — drive the system `php`,
//!   * [`run_rhp`] — drive rhp's VM in-process,
//!   * [`diff_source`] — run both and classify the result,
//!   * [`Rng`] + the `php_bytes_literal` / framing helpers — build generated
//!     corpora deterministically so a failing run is reproducible from a seed.
//!
//! Extension-specific sweeps (e.g. every `hash()` algorithm over random byte
//! strings) are built on top of these in the `rhp-difftest` binary and in
//! the crate's integration tests.

pub mod lint;
pub mod mutate;
pub mod phpt;
pub mod sweeps;

use std::io::Write;
use std::process::{Command, Stdio};

use rhp_compiler::compile_with_source;
use rhp_frontend::Arena;
use rhp_values::Thrown;
use rhp_vm::Vm;

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

/// Run PHP `source` on the real C engine and capture stdout.
///
/// Uses `php -n` (no `php.ini`) with a fixed UTC timezone so results match the
/// rhp defaults; the source is fed on stdin so no temp files are touched.
pub fn run_php(source: &[u8]) -> Result<Vec<u8>, String> {
    let mut child = Command::new(php_bin())
        .args(["-n", "-d", "date.timezone=UTC", "-d", "error_reporting=0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn php: {e}"))?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(source)
        .map_err(|e| format!("write php stdin: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("wait php: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "php exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}

/// Run PHP `source` on rhp's VM and capture its output buffer.
pub fn run_rhp(source: &[u8]) -> Result<Vec<u8>, String> {
    let arena = Arena::new();
    let parsed = arena.parse(b"difftest", source);
    if parsed.has_parse_errors() {
        return Err("rhp: parse error".to_string());
    }
    let program = compile_with_source(parsed.program(), source, "difftest")
        .map_err(|t| format!("rhp compile: {t:?}"))?;
    // A genuine rhp panic (an engine bug) is a differential failure, not a
    // harness crash — catch it and report it like any other mismatch cause.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut vm = Vm::new();
        let r = vm.run(&program).map(|code| {
            // Request shutdown (shutdown functions, destructors, buffers);
            // an uncaught throwable there fails the run like one in the script.
            let mut late: Option<Thrown> = None;
            vm.request_shutdown(code, &mut |t| {
                late.get_or_insert(t);
                255
            });
            late
        });
        let r = match r {
            Ok(None) => Ok(0),
            Ok(Some(t)) | Err(t) => Err(t),
        };
        (r, vm.output().to_vec())
    }));
    match result {
        Ok((Ok(_), out)) => Ok(out),
        Ok((Err(t), _)) => Err(format!("rhp run: {t:?}")),
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
    PhrustErr(String),
}

impl Diff {
    pub fn is_match(&self) -> bool {
        matches!(self, Diff::Match)
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
        Err(e) => return Diff::PhrustErr(e),
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
