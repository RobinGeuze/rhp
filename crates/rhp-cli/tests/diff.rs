//! Differential golden test. The **VM** is held strictly to the committed
//! `NAME.out` (captured from the reference php, see `rhp_builtins::version`)
//! on every fixture. The tree-walking
//! **oracle** is a secondary, php-free cross-check: it is held to the same
//! output on every fixture it supports, and simply *skips* fixtures that use
//! features it doesn't implement (it reports them as `Thrown::Unsupported`).
//! This lets the VM lead on advanced features (references, …) while the oracle
//! keeps guarding the shared core.

use std::path::PathBuf;

use rhp_compiler::{compile_include, compile_with_source};
use rhp_frontend::Arena;
use rhp_oracle::Interp;
use rhp_values::Thrown;
use rhp_vm::{IncludeError, Vm};

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden")
        .canonicalize()
        .expect("golden dir exists")
}

fn run_vm(source: &[u8], name: &str) -> Result<Vec<u8>, Thrown> {
    let arena = Arena::new();
    let parsed = arena.parse(name.as_bytes(), source);
    assert!(!parsed.has_parse_errors(), "{name}: unexpected parse error");
    let program = compile_with_source(parsed.program(), source, name)?;
    let mut vm = Vm::new();
    // include/require/eval compile like the CLI SAPI's includer.
    vm.set_includer(|file: &str, src: &[u8]| {
        let arena = Arena::new();
        let parsed = arena.parse(file.as_bytes(), src);
        if let Some((message, line)) = parsed.parse_error() {
            return Err(IncludeError::Parse { message, line });
        }
        compile_include(parsed.program(), src, file).map_err(rhp_sapi::include_error)
    });
    // The whole CLI request: uncaught-throwable report, shutdown functions,
    // output-buffer flush.
    vm.run_main(&program)?;
    Ok(vm.output().to_vec())
}

fn run_oracle(source: &[u8], name: &str) -> Result<Vec<u8>, Thrown> {
    let arena = Arena::new();
    let parsed = arena.parse(name.as_bytes(), source);
    assert!(!parsed.has_parse_errors(), "{name}: unexpected parse error");
    let mut interp = Interp::new();
    interp.run(parsed.program())?;
    Ok(interp.output().to_vec())
}

fn cases() -> Vec<PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(golden_dir())
        .expect("read golden dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "php"))
        .collect();
    v.sort();
    v
}

/// The VM must match php on every fixture.
#[test]
fn vm_matches_php_baseline() {
    let cases = cases();
    assert!(!cases.is_empty());
    let mut passed = 0;
    let mut failures = Vec::new();
    for php in &cases {
        let name = php.file_stem().unwrap().to_string_lossy().into_owned();
        let Ok(expected) = std::fs::read(php.with_extension("out")) else {
            failures.push(format!("{name}: missing .out"));
            continue;
        };
        let source = std::fs::read(php).expect("read php");
        // Compile under the fixture's real path, as php reports it (getFile(),
        // traces, __FILE__); fixtures print only its basename to stay portable.
        let file = php.to_string_lossy().into_owned();
        match std::panic::catch_unwind(|| run_vm(&source, &file)) {
            Ok(Ok(actual)) if actual == expected => passed += 1,
            Ok(Ok(actual)) => failures.push(format!(
                "{name}: MISMATCH\n  expected: {:?}\n  actual:   {:?}",
                String::from_utf8_lossy(&expected),
                String::from_utf8_lossy(&actual),
            )),
            Ok(Err(e)) => failures.push(format!("{name}: VM error: {e:?}")),
            Err(_) => failures.push(format!("{name}: panicked")),
        }
    }
    eprintln!("[VM] golden: {passed}/{} passed", cases.len());
    assert!(
        failures.is_empty(),
        "[VM] {} failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The oracle must match php on every fixture it supports; unsupported ones skip.
#[test]
fn oracle_matches_php_on_supported() {
    let cases = cases();
    let mut passed = 0;
    let mut skipped = 0;
    let mut failures = Vec::new();
    for php in &cases {
        let name = php.file_stem().unwrap().to_string_lossy().into_owned();
        let Ok(expected) = std::fs::read(php.with_extension("out")) else {
            continue;
        };
        let source = std::fs::read(php).expect("read php");
        let file = php.to_string_lossy().into_owned();
        match std::panic::catch_unwind(|| run_oracle(&source, &file)) {
            Ok(Ok(actual)) if actual == expected => passed += 1,
            Ok(Ok(actual)) => failures.push(format!(
                "{name}: MISMATCH\n  expected: {:?}\n  actual:   {:?}",
                String::from_utf8_lossy(&expected),
                String::from_utf8_lossy(&actual),
            )),
            // Features the oracle doesn't implement are skipped, not failed.
            Ok(Err(Thrown::Unsupported { .. })) => skipped += 1,
            // The oracle has no uncaught-throwable renderer (the VM's
            // `run_main` does that): fixtures ending in one are VM-only.
            Ok(Err(Thrown::Error { .. } | Thrown::Object(_)))
                if expected
                    .windows(22)
                    .any(|w| w == b"\nFatal error: Uncaught") =>
            {
                skipped += 1
            }
            Ok(Err(e)) => failures.push(format!("{name}: oracle error: {e:?}")),
            Err(_) => failures.push(format!("{name}: panicked")),
        }
    }
    eprintln!(
        "[oracle] golden: {passed}/{} passed, {skipped} skipped (unsupported)",
        cases.len()
    );
    assert!(
        failures.is_empty(),
        "[oracle] {} failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
