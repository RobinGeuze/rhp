//! Differential regression gates for the gmp, mbstring and password extensions.
//! Each skips cleanly when no `php` binary is on PATH.

use phrust_difftest::sweeps;
use phrust_difftest::{php_available, Diff};

fn expect_match(name: &str, diff: Diff) {
    match diff {
        Diff::Match => {}
        Diff::Mismatch { php, phrust } => {
            let (p, q) = (String::from_utf8_lossy(&php), String::from_utf8_lossy(&phrust));
            let first = p
                .lines()
                .zip(q.lines())
                .find(|(a, b)| a != b)
                .map(|(a, b)| format!("php={a} phrust={b}"))
                .unwrap_or_else(|| "length differs".to_string());
            panic!("[{name}] MISMATCH: {first}");
        }
        Diff::PhrustErr(e) => panic!("[{name}] phrust error: {e}"),
        Diff::PhpErr(e) => panic!("[{name}] php error: {e}"),
    }
}

#[test]
fn gmp_matches_php() {
    if !php_available() {
        eprintln!("[gmp] no php — skipping");
        return;
    }
    // value-level only (object ids in var_dump differ: phrust doesn't recycle handles)
    for seed in [0xC0FFEE_u64, 7, 42] {
        expect_match("gmp", sweeps::gmp(seed, 80));
    }
}

#[test]
fn mbstring_matches_php() {
    if !php_available() {
        eprintln!("[mbstring] no php — skipping");
        return;
    }
    for seed in [0xC0FFEE_u64, 7, 42] {
        expect_match("mbstring", sweeps::mbstring(seed, 100));
    }
}

#[test]
fn password_deterministic_surface_matches_php() {
    if !php_available() {
        eprintln!("[password] no php — skipping");
        return;
    }
    // password_algos / get_info / verify-known / needs_rehash are deterministic.
    expect_match("password", sweeps::password());
}

/// password_hash is random-salted, so prove the round-trip instead of byte
/// output: phrust's hash verifies in php, and php's hash verifies in phrust.
#[test]
fn password_cross_verifies_with_php() {
    if !php_available() {
        eprintln!("[password] no php — skipping");
        return;
    }
    use phrust_difftest::{run_php, run_phrust};

    // NB: hashes contain `$`, so they MUST sit in single-quoted PHP strings.
    for (pw, algo) in [("hunter2", "PASSWORD_BCRYPT"), ("sukru-key", "PASSWORD_BCRYPT")] {
        // phrust hashes -> php verifies
        let h = run_phrust(format!("<?php echo password_hash('{pw}', {algo});").as_bytes())
            .expect("phrust hash");
        let h = String::from_utf8_lossy(&h);
        let out = run_php(
            format!("<?php var_dump(password_verify('{pw}','{h}'), password_verify('nope','{h}'));")
                .as_bytes(),
        )
        .expect("php verify");
        assert_eq!(out, b"bool(true)\nbool(false)\n", "php could not verify phrust's hash for {pw:?}");

        // php hashes -> phrust verifies
        let h = run_php(format!("<?php echo password_hash('{pw}', {algo});").as_bytes())
            .expect("php hash");
        let h = String::from_utf8_lossy(&h);
        let out = run_phrust(
            format!("<?php var_dump(password_verify('{pw}','{h}'), password_verify('nope','{h}'));")
                .as_bytes(),
        )
        .expect("phrust verify");
        assert_eq!(out, b"bool(true)\nbool(false)\n", "phrust could not verify php's hash for {pw:?}");
    }
}
