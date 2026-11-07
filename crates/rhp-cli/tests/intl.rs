//! intl against php. rhp links the ICU php links, so results match the php
//! on the same machine, but ICU releases disagree (the mac's 79.1 and CI's
//! 78.2 differ on grapheme boundaries and Unicode data), so these cases
//! cannot be goldens. Both engines run the script as files with `LC_ALL=C`:
//! ICU takes its default locale from the environment. Skips without a php
//! that has intl.

use std::path::Path;
use std::process::Command;

use rhp_difftest::php_bin;

fn run(bin: &str, args: &[&str], file: &Path) -> Vec<u8> {
    let o = Command::new(bin)
        .args(args)
        .arg(file)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .expect("run");
    let mut out = o.stdout;
    out.extend_from_slice(&o.stderr);
    out
}

fn php_has_intl() -> bool {
    Command::new(php_bin())
        .args(["-n", "-r", "echo extension_loaded('intl') ? 1 : 0;"])
        .output()
        .is_ok_and(|o| o.stdout == b"1")
}

fn check(name: &str, src: &str) {
    if !php_has_intl() {
        eprintln!("[intl] no php with intl — skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rhp_intl_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.php");
    std::fs::write(&f, src).unwrap();
    let theirs = run(&php_bin(), &["-n"], &f);
    let ours = run(env!("CARGO_BIN_EXE_rhp"), &["-n"], &f);
    let _ = std::fs::remove_dir_all(&dir);
    if theirs != ours {
        let (t, o) = (
            String::from_utf8_lossy(&theirs),
            String::from_utf8_lossy(&ours),
        );
        let line = t.lines().zip(o.lines()).position(|(a, b)| a != b);
        panic!(
            "[{name}] differs from php at line {line:?}\n--- php\n{}\n--- rhp\n{}",
            line.and_then(|l| t.lines().nth(l)).unwrap_or(""),
            line.and_then(|l| o.lines().nth(l)).unwrap_or(""),
        );
    }
}

#[test]
fn icu_data_matches_php() {
    check("icu_data", include_str!("intl/icu_data.php"));
}

#[test]
fn grapheme_and_normalizer_match_php() {
    check("fuzz_grapheme", include_str!("intl/fuzz_grapheme.php"));
}

#[test]
fn idn_matches_php() {
    check("fuzz_idn", include_str!("intl/fuzz_idn.php"));
}

#[test]
fn version_constants_match_php() {
    check(
        "versions",
        "<?php var_dump(INTL_ICU_VERSION, INTL_ICU_DATA_VERSION, IDNA_DEFAULT, U_ERROR_LIMIT,\n\
         intl_error_name(31), intl_error_name(U_ERROR_LIMIT - 1));\n",
    );
}

#[test]
fn number_formatter_matches_php() {
    check("numfmt", include_str!("intl/numfmt.php"));
}

#[test]
fn collator_matches_php() {
    check("collator", include_str!("intl/collator.php"));
}

#[test]
fn locale_matches_php() {
    check("locale", include_str!("intl/locale.php"));
}
