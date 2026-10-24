//! Differential regression test for the reimplemented `bcmath` extension.
//! Skips cleanly when no `php` binary is on PATH.

use phrust_difftest::sweeps::bcmath;
use phrust_difftest::{php_available, Diff};

#[test]
fn bcmath_matches_php() {
    if !php_available() {
        eprintln!("[bcmath] no `php` on PATH — skipping differential sweep");
        return;
    }
    // several seeds so a lucky single draw can't hide an operand-shape bug
    for seed in [0xC0FFEE_u64, 1, 42, 2024] {
        match bcmath(seed, 100) {
            Diff::Match => {}
            Diff::Mismatch { php, phrust } => {
                let (p, q) = (String::from_utf8_lossy(&php), String::from_utf8_lossy(&phrust));
                let first = p
                    .lines()
                    .zip(q.lines())
                    .find(|(a, b)| a != b)
                    .map(|(a, b)| format!("php={a} phrust={b}"))
                    .unwrap_or_else(|| "length differs".to_string());
                panic!("[bcmath] seed {seed:#x} MISMATCH: {first}");
            }
            Diff::PhrustErr(e) => panic!("[bcmath] seed {seed:#x} phrust error: {e}"),
            Diff::PhpErr(e) => panic!("[bcmath] seed {seed:#x} php error: {e}"),
        }
    }
    eprintln!("[bcmath] byte-identical to php across all seeds");
}
