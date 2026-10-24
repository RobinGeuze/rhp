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
use rhp_difftest::{default_byte_corpus, php_available, Diff};

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
    assert!(algos.len() >= 40, "expected the full php algo set, got {}", algos.len());
    let corpus = default_byte_corpus(0xC0FFEE, 48);

    let mut failures = Vec::new();
    let mut covered = 0u32;
    for algo in &algos {
        if PENDING.contains(&algo.as_str()) {
            continue;
        }
        match hash_algo(algo, &corpus) {
            Diff::Match => covered += 1,
            Diff::Mismatch { php, rhp } => {
                let (p, q) = first_diff(&php, &rhp);
                failures.push(format!("{algo}: MISMATCH (php={p} rhp={q})"));
            }
            Diff::PhrustErr(e) => failures.push(format!("{algo}: rhp error: {}", one_line(&e))),
            Diff::PhpErr(e) => failures.push(format!("{algo}: php error: {}", one_line(&e))),
        }
    }
    eprintln!("[hash] {covered} algorithm(s) byte-identical to php");
    assert!(failures.is_empty(), "{} algorithm(s) differ:\n{}", failures.len(), failures.join("\n"));
}

fn first_diff(php: &[u8], rhp: &[u8]) -> (String, String) {
    let (ps, qs) = (String::from_utf8_lossy(php), String::from_utf8_lossy(rhp));
    for (a, b) in ps.lines().zip(qs.lines()) {
        if a != b {
            return (a.to_string(), b.to_string());
        }
    }
    (format!("<{} B>", php.len()), format!("<{} B>", rhp.len()))
}

fn one_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}
