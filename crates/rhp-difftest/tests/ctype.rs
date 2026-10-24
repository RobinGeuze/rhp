//! Differential regression test for the `ctype` extension — every `ctype_*`
//! over every byte, the integer char-code quirk, and strings.
//! Skips cleanly when no `php` binary is on PATH.

use rhp_difftest::sweeps::ctype;
use rhp_difftest::{php_available, Diff};

#[test]
fn ctype_matches_php() {
    if !php_available() {
        eprintln!("[ctype] no `php` on PATH — skipping differential sweep");
        return;
    }
    match ctype() {
        Diff::Match => eprintln!("[ctype] all ctype_* byte-identical to php"),
        Diff::Mismatch { php, rhp } => {
            let (p, q) = (String::from_utf8_lossy(&php), String::from_utf8_lossy(&rhp));
            let first = p
                .lines()
                .zip(q.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .map(|(i, _)| format!("row {i}"))
                .unwrap_or_else(|| "length differs".to_string());
            panic!("[ctype] MISMATCH at {first}");
        }
        Diff::PhrustErr(e) => panic!("[ctype] rhp error: {e}"),
        Diff::PhpErr(e) => panic!("[ctype] php error: {e}"),
    }
}
