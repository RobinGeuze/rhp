//! Differential golden test: run every `tests/golden/*.php` through BOTH the
//! bytecode VM and the tree-walking oracle, and assert each matches the
//! committed `NAME.out` (captured from php 8.5.8) byte-for-byte.
//!
//! Because operators/builtins are shared code, a VM≠oracle divergence is always
//! a compiler- or VM-loop bug — the oracle is an independent check of exactly
//! the new pipeline, catchable without the `php` binary.

use std::path::PathBuf;

use phrust_compiler::compile;
use phrust_frontend::Arena;
use phrust_oracle::Interp;
use phrust_vm::Vm;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden")
        .canonicalize()
        .expect("golden dir exists")
}

/// Run a source through the VM (parse → compile → execute).
fn run_vm(source: &[u8], name: &str) -> Vec<u8> {
    let arena = Arena::new();
    let parsed = arena.parse(name.as_bytes(), source);
    assert!(!parsed.has_parse_errors(), "{name}: unexpected parse error");
    let program = compile(parsed.program())
        .unwrap_or_else(|e| panic!("{name}: compile error: {e:?}"));
    let mut vm = Vm::new();
    vm.run(&program).unwrap_or_else(|e| panic!("{name}: VM error: {e:?}"));
    vm.output().to_vec()
}

/// Run a source through the tree-walking oracle.
fn run_oracle(source: &[u8], name: &str) -> Vec<u8> {
    let arena = Arena::new();
    let parsed = arena.parse(name.as_bytes(), source);
    assert!(!parsed.has_parse_errors(), "{name}: unexpected parse error");
    let mut interp = Interp::new();
    interp.run(parsed.program()).unwrap_or_else(|e| panic!("{name}: oracle error: {e:?}"));
    interp.output().to_vec()
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

#[test]
fn vm_matches_php_baseline() {
    check("VM", run_vm);
}

#[test]
fn oracle_matches_php_baseline() {
    check("oracle", run_oracle);
}

fn check(engine: &str, run: fn(&[u8], &str) -> Vec<u8>) {
    let cases = cases();
    assert!(!cases.is_empty(), "no golden fixtures found");
    let mut passed = 0;
    let mut failures = Vec::new();

    for php in &cases {
        let name = php.file_stem().unwrap().to_string_lossy().into_owned();
        let Ok(expected) = std::fs::read(php.with_extension("out")) else {
            failures.push(format!("{name}: missing .out"));
            continue;
        };
        let source = std::fs::read(php).expect("read php");
        match std::panic::catch_unwind(|| run(&source, &name)) {
            Ok(actual) if actual == expected => passed += 1,
            Ok(actual) => failures.push(format!(
                "{name}: MISMATCH\n  expected: {:?}\n  actual:   {:?}",
                String::from_utf8_lossy(&expected),
                String::from_utf8_lossy(&actual),
            )),
            Err(_) => failures.push(format!("{name}: panicked")),
        }
    }

    eprintln!("[{engine}] golden: {passed}/{} passed", cases.len());
    if !failures.is_empty() {
        panic!("[{engine}] {} failure(s):\n{}", failures.len(), failures.join("\n"));
    }
}
