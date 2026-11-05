//! The tables `rhp-difftest gen dumps` makes from the reference php are up
//! to date. Off the reference host (Homebrew php on macOS, rhp's release) the
//! generator skips. CI checks the php-src and tarball generators in ci.yml.
//! README.md's PHP test table needs only files in the repo, so it is checked
//! everywhere.

use std::process::ExitCode;

#[test]
fn dumps_match_the_reference_php() {
    let args = ["dumps".to_string(), "--check".to_string()];
    assert_eq!(
        rhp_difftest::gen::main(&args),
        ExitCode::SUCCESS,
        "run `rhp-difftest gen dumps` on the reference host"
    );
}

#[test]
fn readme_test_table_matches_the_phpt_lists() {
    let args = ["readme".to_string(), "--check".to_string()];
    assert_eq!(
        rhp_difftest::gen::main(&args),
        ExitCode::SUCCESS,
        "run `cargo run -p rhp-difftest -- gen readme`"
    );
}
