//! Differential regression test for the reimplemented `bcmath` extension.
//! Skips cleanly when no `php` binary is on PATH.

use rhp_difftest::php_available;
use rhp_difftest::sweeps::bcmath;

#[test]
fn bcmath_matches_php() {
    if !php_available() {
        eprintln!("[bcmath] no `php` on PATH — skipping differential sweep");
        return;
    }
    // several seeds so a lucky single draw can't hide an operand-shape bug
    for seed in [0xC0FFEE_u64, 1, 42, 2024] {
        if let Some(why) = bcmath(seed, 100).failure() {
            panic!("[bcmath] seed {seed:#x} {why}");
        }
    }
    eprintln!("[bcmath] byte-identical to php across all seeds");
}
