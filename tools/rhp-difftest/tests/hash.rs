//! Differential regression test for the reimplemented `hash` extension.
//!
//! When a `php` binary is available, every algorithm `php` reports from
//! `hash_algos()` is swept over a broad generated corpus and required to match
//! rhp byte-for-byte. Without `php` the test logs a skip (so the workspace
//! still tests where no PHP is installed).
//!
//! `PENDING` lists algorithms not yet reimplemented; they are allowed to differ
//! so the suite stays green during buildout. It must be empty for full coverage.

use rhp_difftest::sweeps::{hash_algo, php_hash_algos};
use rhp_difftest::{default_byte_corpus, diff_source, php_available, php_bytes_array};

/// Algorithms still being implemented (table-heavy, verified via the sweep).
/// Empty — rhp covers the full php `hash_algos()` set.
const PENDING: &[&str] = &[];

#[test]
fn hash_matches_php_over_corpus() {
    if !php_available() {
        eprintln!("[hash] no `php` on PATH — skipping differential sweep");
        return;
    }
    let algos = php_hash_algos().expect("query php hash_algos()");
    assert!(
        algos.len() >= 40,
        "expected the full php algo set, got {}",
        algos.len()
    );
    let corpus = default_byte_corpus(0xC0FFEE, 48);

    let mut failures = Vec::new();
    let mut covered = 0u32;
    for algo in &algos {
        if PENDING.contains(&algo.as_str()) {
            continue;
        }
        match hash_algo(algo, &corpus).failure() {
            None => covered += 1,
            Some(why) => failures.push(format!("{algo}: {why}")),
        }
    }
    eprintln!("[hash] {covered} algorithm(s) byte-identical to php");
    assert!(
        failures.is_empty(),
        "{} algorithm(s) differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The standalone md5(), sha1() and crc32() functions, hex and raw.
#[test]
fn md5_sha1_crc32_functions_match_php() {
    if !php_available() {
        eprintln!("[hash] no `php` on PATH — skipping md5/sha1/crc32 sweep");
        return;
    }
    let corpus = default_byte_corpus(0xC0FFEE, 48);
    let src = format!(
        "<?php\nforeach ({} as $d) echo md5($d), ' ', bin2hex(md5($d, true)), ' ', sha1($d), ' ', bin2hex(sha1($d, true)), ' ', crc32($d), \"\\n\";\n",
        php_bytes_array(&corpus)
    );
    if let Some(why) = diff_source(src.as_bytes()).failure() {
        panic!("[hash] md5/sha1/crc32: {why}");
    }
}
