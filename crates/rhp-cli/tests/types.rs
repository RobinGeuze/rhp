//! Differential tests for `declare(strict_types=1)` enforcement and coercive-mode
//! parameter/return type coercion. Compares the built `rhp` binary against the
//! real `php`. Skips when no `php` is on PATH. (These live here, not in the golden
//! suite, because the tree-walker oracle does not enforce types.)

use std::path::Path;
use std::process::Command;

fn php_available() -> bool {
    Command::new("php")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn case(name: &str, src: &str) {
    if !php_available() {
        eprintln!("[types] no php — skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rhp_types_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.php");
    std::fs::write(&f, src).unwrap();
    let php = run("php", &f);
    let rhp = run(env!("CARGO_BIN_EXE_rhp"), &f);
    assert_eq!(
        String::from_utf8_lossy(&php),
        String::from_utf8_lossy(&rhp),
        "type-enforcement output differs from php for case {name}"
    );
}

fn run(bin: &str, file: &Path) -> Vec<u8> {
    let args: &[&str] = if bin == "php" { &["-n"] } else { &[] };
    let o = Command::new(bin)
        .args(args)
        .arg(file)
        .output()
        .expect("run");
    // Merge stderr (TypeError messages) so we compare the full observable result.
    let mut out = o.stdout;
    out.extend_from_slice(&o.stderr);
    out
}

#[test]
fn strict_params_and_returns() {
    case(
        "strict",
        r#"<?php
        declare(strict_types=1);
        function f(int $x): int { return $x; }
        function g(float $y): float { return $y; }
        echo f(5), "\n";
        echo g(3), "\n";            // int->float widening allowed in strict
        try { f("5"); } catch (\TypeError $e) { echo $e->getMessage(), "\n"; }
        try { f(true); } catch (\TypeError $e) { echo "TE-bool\n"; }
        try { (function(): int { return "x"; })(); } catch (\TypeError $e) { echo "TE-ret\n"; }
        "#,
    );
}

#[test]
fn coercive_params_and_returns() {
    case(
        "coerce",
        r#"<?php
        function f(int $x) { return $x; }
        function s(string $x) { return $x; }
        function r(): int { return "42"; }
        echo f("5"), "\n";          // "5" -> 5
        echo f(5.0), "\n";          // 5.0 -> 5
        echo s(42), "\n";           // 42 -> "42"
        echo var_export(f(true), true), "\n";
        echo r(), "\n";
        try { f([1]); } catch (\TypeError $e) { echo "TE-arr\n"; }
        try { f("abc"); } catch (\TypeError $e) { echo "TE-nan\n"; }
        "#,
    );
}

#[test]
fn class_nullable_union_types() {
    case(
        "classy",
        r#"<?php
        declare(strict_types=1);
        class A {} class B extends A {} interface I {} class C implements I {}
        function needA(A $a): string { return get_class($a); }
        function needN(?int $x): string { return $x === null ? "null" : (string)$x; }
        function needU(int|string $x): string { return gettype($x); }
        function needI(I $x): string { return "i-ok"; }
        echo needA(new B), "\n";
        echo needN(null), "\n";
        echo needU(5), " ", needU("hi"), "\n";
        echo needI(new C), "\n";
        try { needA(new C); } catch (\TypeError $e) { echo "TE-class\n"; }
        try { needN("x"); } catch (\TypeError $e) { echo "TE-null\n"; }
        try { needU([1]); } catch (\TypeError $e) { echo "TE-union\n"; }
        "#,
    );
}
