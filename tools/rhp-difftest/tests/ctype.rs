//! Differential regression test for the `ctype` extension — every `ctype_*`
//! over every byte, the integer char-code quirk, and strings.
//! Skips cleanly when no `php` binary is on PATH.

use rhp_difftest::php_available;
use rhp_difftest::sweeps::ctype;

#[test]
fn ctype_matches_php() {
    if !php_available() {
        eprintln!("[ctype] no `php` on PATH — skipping differential sweep");
        return;
    }
    if let Some(why) = ctype().failure() {
        panic!("[ctype] {why}");
    }
    eprintln!("[ctype] all ctype_* byte-identical to php");
}
