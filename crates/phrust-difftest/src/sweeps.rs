//! Extension-specific differential sweeps built on the generic harness.
//!
//! A *sweep* generates a broad corpus of inputs for one extension's functions,
//! runs it through both engines, and reports a per-feature verdict. Today this
//! covers the `hash` extension; the same shape (generate → `diff_source` →
//! classify) is how any future reimplemented extension is driven.

use crate::{diff_source, php_bytes_array, run_php, Diff};

/// Ask the real `php` which `hash()` algorithms it supports.
pub fn php_hash_algos() -> Result<Vec<String>, String> {
    let out = run_php(b"<?php foreach (hash_algos() as $a) echo $a, \"\\n\";")?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// PHP source that hashes every element of `corpus` with `algo` (raw output,
/// hex-encoded one per line). Identical text runs on both engines.
pub fn hash_algo_snippet(algo: &str, corpus: &[Vec<u8>]) -> String {
    // algo is a hash-algorithm name (ascii, may contain ',' or '/'): safe inside
    // a double-quoted PHP string with no escaping needed.
    format!(
        "<?php\n$in = {};\nforeach ($in as $d) echo bin2hex(hash(\"{}\", $d, true)), \"\\n\";\n",
        php_bytes_array(corpus),
        algo
    )
}

/// Run the `hash()` differential sweep for one algorithm over a corpus.
pub fn hash_algo(algo: &str, corpus: &[Vec<u8>]) -> Diff {
    diff_source(hash_algo_snippet(algo, corpus).as_bytes())
}
