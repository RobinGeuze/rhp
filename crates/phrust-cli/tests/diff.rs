//! Differential golden test. The **VM** is held strictly to the committed
//! `NAME.out` (captured from php 8.5.8) on every fixture. The tree-walking
//! **oracle** is a secondary, php-free cross-check: it is held to the same
//! output on every fixture it supports, and simply *skips* fixtures that use
//! features it doesn't implement (it reports them as `Thrown::Unsupported`).
//! This lets the VM lead on advanced features (references, …) while the oracle
//! keeps guarding the shared core.

use std::path::PathBuf;

use phrust_compiler::compile_with_source;
use phrust_frontend::Arena;
use phrust_oracle::Interp;
use phrust_values::Thrown;
use phrust_vm::Vm;

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
    vm.run(&program)?;
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
        match std::panic::catch_unwind(|| run_vm(&source, &name)) {
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
    assert!(failures.is_empty(), "[VM] {} failure(s):\n{}", failures.len(), failures.join("\n"));
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
        let Ok(expected) = std::fs::read(php.with_extension("out")) else { continue };
        let source = std::fs::read(php).expect("read php");
        match std::panic::catch_unwind(|| run_oracle(&source, &name)) {
            Ok(Ok(actual)) if actual == expected => passed += 1,
            Ok(Ok(actual)) => failures.push(format!(
                "{name}: MISMATCH\n  expected: {:?}\n  actual:   {:?}",
                String::from_utf8_lossy(&expected),
                String::from_utf8_lossy(&actual),
            )),
            // Features the oracle doesn't implement are skipped, not failed.
            Ok(Err(Thrown::Unsupported { .. })) => skipped += 1,
            Ok(Err(e)) => failures.push(format!("{name}: oracle error: {e:?}")),
            Err(_) => failures.push(format!("{name}: panicked")),
        }
    }
    eprintln!("[oracle] golden: {passed}/{} passed, {skipped} skipped (unsupported)", cases.len());
    assert!(failures.is_empty(), "[oracle] {} failure(s):\n{}", failures.len(), failures.join("\n"));
}
