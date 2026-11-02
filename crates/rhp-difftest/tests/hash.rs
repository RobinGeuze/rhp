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
use rhp_difftest::{default_byte_corpus, php_available};

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
