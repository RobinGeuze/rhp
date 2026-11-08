//! The internal classes' surface against php's (`rhp-difftest api`): no
//! difference beyond tests/api/known-differences.txt. A row of the list
//! that no longer differs is printed, to take off the list.
//!
//! Skips unless `php` is the release rhp targets.

use std::path::Path;

use rhp_difftest::{api, php_available, read_known, run_php};

#[test]
fn surface_matches_php() {
    if !php_available() {
        eprintln!("[api] no php — skipping");
        return;
    }
    let v = run_php(b"<?php echo PHP_VERSION;").unwrap_or_default();
    if v != rhp_builtins::PHP_VERSION.as_bytes() {
        eprintln!("[api] php is not {} — skipping", rhp_builtins::PHP_VERSION);
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let known = read_known(&root.join(api::KNOWN)).expect("known-differences list");
    let report = api::compare().expect("api dump");
    assert!(
        report.compared.len() > 200,
        "too few classes compared: {}",
        report.compared.len()
    );
    let (new, fixed) = api::check(&report, &known);
    for f in &fixed {
        eprintln!("[api] fixed, take off {}: {f}", api::KNOWN);
    }
    assert!(
        new.is_empty(),
        "{} surface differences not in {}:\n{}",
        new.len(),
        api::KNOWN,
        new.join("\n")
    );
}
