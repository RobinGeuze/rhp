//! Shared helpers for the phrust-vm unit suite.
//!
//! These tests go PHP source -> compile -> run on the register VM -> assert the
//! exact captured output. They are self-contained (no shelling out to `php`):
//! every expected string is a literal whose value matches php 8.5 semantics and
//! is cross-checked by the php-differential golden harness in phrust-cli.
//!
//! Kept out of `src/lib.rs` on purpose so the VM core can be rewritten
//! (generators, diagnostics) without colliding with these tests.

use phrust_compiler::compile;
use phrust_frontend::Arena;
use phrust_vm::Vm;

/// Compile and run `src`, returning captured stdout as a `String`.
///
/// Panics if the program fails to parse/compile or the VM throws, so a test
/// that expects an uncaught throw should use [`run_throws`] instead.
pub fn run(src: &str) -> String {
    let arena = Arena::new();
    let parsed = arena.parse(b"test", src.as_bytes());
    assert!(!parsed.has_parse_errors(), "unexpected parse error in:\n{src}");
    let program = compile(parsed.program()).expect("compile failed");
    let mut vm = Vm::new();
    vm.run(&program).expect("VM threw");
    String::from_utf8_lossy(vm.output()).into_owned()
}

/// Run `src`, returning `(captured stdout, Err class name)`. Panics if the
/// program did NOT throw (or failed to compile).
#[allow(dead_code)]
pub fn run_throws(src: &str) -> (String, String) {
    use phrust_values::Thrown;
    let arena = Arena::new();
    let parsed = arena.parse(b"test", src.as_bytes());
    assert!(!parsed.has_parse_errors(), "unexpected parse error in:\n{src}");
    let program = compile(parsed.program()).expect("compile failed");
    let mut vm = Vm::new();
    match vm.run(&program) {
        Ok(_) => panic!("expected a throw, but program completed:\n{src}"),
        Err(t) => {
            let out = String::from_utf8_lossy(vm.output()).into_owned();
            let class = match t {
                Thrown::Error { class, .. } => class,
                Thrown::Object(_) => "Object",
                Thrown::Exit(_) => "Exit",
                Thrown::Unsupported { .. } => "Unsupported",
            };
            (out, class.to_string())
        }
    }
}
