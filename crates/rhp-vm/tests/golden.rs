//! The VM's tests: the goldens in `tests/golden/vm` (the reference php's
//! output, see docs/testing.md), each run as a CLI request. They live in
//! this package so `cargo test -p rhp-vm`, and cargo-mutants on rhp-vm, run
//! them. `GOLDEN_ONLY=SUBSTR` runs only those whose name (`vm/NAME`)
//! contains SUBSTR; rhp-cli's diff.rs fails a filter that matches nothing.

use std::path::PathBuf;

use rhp_sapi::{CaptureSapi, Request, Script};

// The engine's memory accounting (memory_get_usage(), memory_limit).
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

#[test]
fn vm_goldens_match_php() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/vm");
    let dir = dir.canonicalize().expect("tests/golden/vm exists");
    let only = std::env::var("GOLDEN_ONLY").ok();
    let mut cases: Vec<_> = std::fs::read_dir(&dir)
        .expect("read tests/golden/vm")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "php"))
        .filter(|p| {
            let name = format!("vm/{}", p.file_stem().unwrap().to_string_lossy());
            only.as_deref().is_none_or(|o| name.contains(o))
        })
        .collect();
    cases.sort();
    if only.is_some() && cases.is_empty() {
        return;
    }
    assert!(!cases.is_empty());
    // As php runs a golden: by its absolute path, from its directory.
    std::env::set_current_dir(&dir).expect("chdir");
    let mut failures = Vec::new();
    for php in &cases {
        let want = std::fs::read(php.with_extension("out")).unwrap_or_default();
        let script = Script::File(php.to_string_lossy().into_owned());
        let (_, got) = CaptureSapi::cli().run(Request::new(script));
        if got.stdout != want {
            failures.push(format!(
                "{}:\n  expected: {:?}\n  actual:   {:?}\n  stderr:   {:?}",
                php.display(),
                String::from_utf8_lossy(&want),
                String::from_utf8_lossy(&got.stdout),
                String::from_utf8_lossy(&got.stderr),
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
