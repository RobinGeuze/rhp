//! Golden test: the VM is held strictly to the committed `NAME.out`
//! (captured from the reference php, see `rhp_builtins::version`) on every
//! fixture in `tests/golden`. It needs no php binary, so it also runs in the
//! php-free CI job.

use std::path::PathBuf;

use rhp_compiler::{compile_include, compile_with_source};
use rhp_frontend::Arena;
use rhp_values::Thrown;
use rhp_vm::{IncludeError, Vm};

// The engine's memory accounting (memory_get_usage(), memory_limit).
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

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
        if let Some(f) = rhp_sapi::parse_error(&parsed, src) {
            return Err(IncludeError::Parse {
                class: f.class,
                message: f.message,
                line: f.line,
            });
        }
        compile_include(parsed.program(), src, file).map_err(rhp_sapi::include_error)
    });
    // The whole CLI request: uncaught-throwable report, shutdown functions,
    // output-buffer flush.
    vm.run_main(&program)?;
    Ok(vm.output().to_vec())
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
