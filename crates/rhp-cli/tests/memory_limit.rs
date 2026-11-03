//! memory_limit against the real php: scripts run through `php -n` and the
//! built `rhp`, comparing stdout, stderr and the exit code. Where the size
//! php failed to allocate depends on its data structures (an array's hash
//! table), "tried to allocate N bytes" is compared without N. Skips when no
//! `php` is on PATH.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn php_available() -> bool {
    Command::new("php")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn script(name: &str, code: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rhp_memory_limit_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    // php reports resolved real paths (e.g. /private/var on macOS).
    let path = dir.canonicalize().expect("canonical dir").join("main.php");
    std::fs::write(&path, code).expect("write script");
    path
}

fn run(bin: &str, main: &Path) -> Output {
    let mut c = Command::new(bin);
    if bin == "php" {
        c.arg("-n");
    }
    c.arg(main).output().expect("run engine")
}

/// "tried to allocate 123 bytes" → "tried to allocate N bytes".
fn without_sizes(s: &str) -> String {
    let marker = "tried to allocate ";
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(marker) {
        out.push_str(&rest[..i + marker.len()]);
        rest = rest[i + marker.len()..].trim_start_matches(|c: char| c.is_ascii_digit());
        out.push('N');
    }
    out.push_str(rest);
    out
}

fn assert_same(name: &str, code: &str, exact: bool) -> String {
    if !php_available() {
        return String::new();
    }
    let main = script(name, code);
    let php = run("php", &main);
    let rhp = run(env!("CARGO_BIN_EXE_rhp"), &main);
    let norm = |b: &[u8]| {
        let s = String::from_utf8_lossy(b).into_owned();
        if exact {
            s
        } else {
            without_sizes(&s)
        }
    };
    let out = norm(&php.stdout);
    assert_eq!(out, norm(&rhp.stdout), "{name}: stdout differs");
    assert_eq!(
        norm(&php.stderr),
        norm(&rhp.stderr),
        "{name}: stderr differs"
    );
    assert_eq!(
        php.status.code(),
        rhp.status.code(),
        "{name}: exit code differs"
    );
    out
}

#[test]
fn str_repeat_past_the_limit() {
    // Refused up front with php's exact size; buffered output is dropped,
    // `finally` doesn't run, the shutdown function sees the error.
    let out = assert_same(
        "repeat",
        r#"<?php
register_shutdown_function(function () {
    echo "shutdown\n";
    $e = error_get_last();
    echo $e['type'], ' ', $e['line'], ' ', $e['message'], "\n";
});
ini_set('memory_limit', '8M');
ob_start();
echo "buffered, then discarded\n";
function make($n) {
    try {
        return str_repeat('x', $n);
    } finally {
        echo "finally does not run\n";
    }
}
$s = make(16 * 1024 * 1024);
echo "not reached\n";
"#,
        true,
    );
    assert!(
        out.is_empty() || out.contains("1 12 Allowed memory size of 8388608"),
        "{out}"
    );
}

#[test]
fn array_grows_until_it_fails() {
    assert_same(
        "grow",
        r#"<?php
ini_set('memory_limit', '8M');
class D { function __destruct() { echo "destructor does not run\n"; } }
$d = new D;
register_shutdown_function(function () { echo "shutdown\n"; });
$a = [];
for ($i = 0; ; $i++) {
    $a[] = $i;
}
"#,
        false,
    );
}

#[test]
fn recursion_that_allocates() {
    assert_same(
        "recurse",
        r#"<?php
ini_set('memory_limit', '16M');
function f($n) {
    $s = str_repeat('a', 1000);
    return f($n + 1) . $s;
}
f(0);
"#,
        true,
    );
}

#[test]
fn shutdown_functions_after_the_fatal() {
    // The first shutdown function runs out again (the global is still
    // held); the second one never runs.
    assert_same(
        "shutdown",
        r#"<?php
ini_set('memory_limit', '16M');
$GLOBALS['keep'] = str_repeat('k', 6 * 1024 * 1024);
register_shutdown_function(function () {
    echo "first shutdown function\n";
    $x = str_repeat('y', 12 * 1024 * 1024);
    echo "not reached\n";
});
register_shutdown_function(function () { echo "second shutdown function does not run\n"; });
$big = str_repeat('z', 12 * 1024 * 1024);
echo "not reached\n";
"#,
        true,
    );
}

#[test]
fn delayed_until_the_next_interrupt_check() {
    // The concat crosses the limit; php reports it at the next interrupt
    // check (a conditional jump on a string), so the echo still runs.
    assert_same(
        "delayed",
        r#"<?php
ini_set('memory_limit', '8M');
$a = str_repeat('a', 2 * 1024 * 1024);
$b = $a . str_repeat('b', 2 * 1024 * 1024);
echo "after the concat\n";
if ($b) { echo "not reached\n"; }
"#,
        false,
    );
}

#[test]
fn hard_limit_exits_at_once() {
    // A string offset far past twice the limit: php-cli prints "Hard
    // memory limit" on stderr and exits 137, without shutdown functions.
    assert_same(
        "hard",
        r#"<?php
register_shutdown_function(function () { echo "not reached\n"; });
echo "start\n";
$s = '';
$s[300000000] = 'x';
"#,
        true,
    );
}

#[test]
fn limit_settings() {
    assert_same(
        "limits",
        r#"<?php
var_dump(ini_set('memory_limit', '1M'), ini_get('memory_limit'));
var_dump(ini_set('memory_limit', '-2'), ini_get('memory_limit'));
var_dump(ini_set('memory_limit', '64M'), ini_get('memory_limit'));
var_dump(ini_set('memory_limit', '-1'), ini_get('memory_limit'));
var_dump(memory_get_usage(true), memory_get_peak_usage(true));
var_dump(ini_set('memory_hard_limit_percent', '50'), ini_set('memory_hard_limit_percent', '300'));
var_dump(memory_reset_peak_usage());
var_dump(is_int(memory_get_usage()), memory_get_usage() <= memory_get_peak_usage());
"#,
        true,
    );
}
