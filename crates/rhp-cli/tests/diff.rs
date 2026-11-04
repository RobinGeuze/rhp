//! Golden test: every fixture below `tests/golden` runs as a CLI request
//! (`rhp_sapi::CaptureSapi`) and must print exactly its committed `NAME.out`,
//! which the reference php printed (`rhp-difftest golden --check`, see
//! docs/testing.md); `tests/golden/vm` is rhp-vm's to run. It needs no php
//! binary, so it also runs in the php-free CI job. `GOLDEN_ONLY=SUBSTR`
//! runs only the fixtures whose name (`lang/closures`) contains SUBSTR.
//!
//! The fixtures run one after another: they chdir, and some set the umask.

use std::path::{Path, PathBuf};

use rhp_difftest::golden::{fixtures, Fixture};
use rhp_sapi::{CaptureSapi, Outcome, Request, Script};

// The engine's memory accounting (memory_get_usage(), memory_limit).
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden")
        .canonicalize()
        .expect("golden dir exists")
}

/// Run `php` as php runs a golden: by its absolute path, from its directory.
fn run(php: &Path) -> (Outcome, rhp_sapi::Captured) {
    std::env::set_current_dir(php.parent().unwrap()).expect("chdir");
    let script = Script::File(php.to_string_lossy().into_owned());
    CaptureSapi::cli().run(Request::new(script))
}

/// An engine refusal (`Thrown::Unsupported`), reported on stderr.
fn refused(stderr: &[u8]) -> bool {
    stderr.windows(16).any(|w| w == b"RHP_UNSUPPORTED:")
}

fn cases() -> Vec<Fixture> {
    let only = std::env::var("GOLDEN_ONLY").ok();
    fixtures(&golden_dir())
        .expect("read golden dir")
        .into_iter()
        // rhp-vm's tests/golden.rs runs these.
        .filter(|f| !f.name.starts_with("vm/"))
        .filter(|f| only.as_deref().is_none_or(|o| f.name.contains(o)))
        .collect()
}

/// The VM must match php on every fixture.
#[test]
fn vm_matches_php_baseline() {
    let cases = cases();
    assert!(!cases.is_empty(), "no golden fixture matches GOLDEN_ONLY");
    let cwd = std::env::current_dir().expect("cwd");
    let mut passed = 0;
    let mut failures = Vec::new();
    for Fixture { name, php } in &cases {
        let Ok(expected) = std::fs::read(php.with_extension("out")) else {
            failures.push(format!("{name}: missing .out"));
            continue;
        };
        match std::panic::catch_unwind(|| run(php)) {
            Ok((_, out)) if refused(&out.stderr) => failures.push(format!(
                "{name}: refused: {}",
                String::from_utf8_lossy(&out.stderr)
            )),
            Ok((_, out)) if out.stdout == expected => passed += 1,
            Ok((outcome, out)) => failures.push(format!(
                "{name}: MISMATCH ({outcome:?})\n  expected: {:?}\n  actual:   {:?}\n  stderr:   {:?}",
                String::from_utf8_lossy(&expected),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            )),
            Err(_) => failures.push(format!("{name}: panicked")),
        }
    }
    std::env::set_current_dir(cwd).expect("chdir back");
    eprintln!("[VM] golden: {passed}/{} passed", cases.len());
    assert!(
        failures.is_empty(),
        "[VM] {} failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
