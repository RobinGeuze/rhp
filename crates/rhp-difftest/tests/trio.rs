//! Differential regression gates for the gmp, mbstring and password extensions.
//! Each skips cleanly when no `php` binary is on PATH.

use rhp_difftest::sweeps;
use rhp_difftest::{php_available, Diff};

fn expect_match(name: &str, diff: Diff) {
    if let Some(why) = diff.failure() {
        panic!("[{name}] {why}");
    }
}

#[test]
fn gmp_matches_php() {
    if !php_available() {
        eprintln!("[gmp] no php — skipping");
        return;
    }
    // value-level only (object ids in var_dump differ: rhp doesn't recycle handles)
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
/// output: rhp's hash verifies in php, and php's hash verifies in rhp.
#[test]
fn password_cross_verifies_with_php() {
    if !php_available() {
        eprintln!("[password] no php — skipping");
        return;
    }
    use rhp_difftest::{run_php, run_rhp};

    // NB: hashes contain `$`, so they MUST sit in single-quoted PHP strings.
    for (pw, algo) in [
        ("hunter2", "PASSWORD_BCRYPT"),
        ("sukru-key", "PASSWORD_BCRYPT"),
    ] {
        // rhp hashes -> php verifies
        let h = run_rhp(format!("<?php echo password_hash('{pw}', {algo});").as_bytes())
            .expect("rhp hash");
        let h = String::from_utf8_lossy(&h);
        let out = run_php(
            format!(
                "<?php var_dump(password_verify('{pw}','{h}'), password_verify('nope','{h}'));"
            )
            .as_bytes(),
        )
        .expect("php verify");
        assert_eq!(
            out, b"bool(true)\nbool(false)\n",
            "php could not verify rhp's hash for {pw:?}"
        );

        // php hashes -> rhp verifies
        let h = run_php(format!("<?php echo password_hash('{pw}', {algo});").as_bytes())
            .expect("php hash");
        let h = String::from_utf8_lossy(&h);
        let out = run_rhp(
            format!(
                "<?php var_dump(password_verify('{pw}','{h}'), password_verify('nope','{h}'));"
            )
            .as_bytes(),
        )
        .expect("rhp verify");
        assert_eq!(
            out, b"bool(true)\nbool(false)\n",
            "rhp could not verify php's hash for {pw:?}"
        );
    }
}
