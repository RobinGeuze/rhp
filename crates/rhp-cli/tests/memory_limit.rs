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

fn run(bin: &str, args: &[&str], main: &Path) -> Output {
    let mut c = Command::new(bin);
    if bin == "php" {
        c.arg("-n");
    }
    c.args(args).arg(main).output().expect("run engine")
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
    assert_same_with(name, &[], code, exact)
}

/// [`assert_same`] with `-d` options for both engines.
fn assert_same_with(name: &str, args: &[&str], code: &str, exact: bool) -> String {
    if !php_available() {
        return String::new();
    }
    let main = script(name, code);
    let php = run("php", args, &main);
    let rhp = run(env!("CARGO_BIN_EXE_rhp"), args, &main);
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
    // Where the limit is crossed depends on each engine's frames: no N.
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
        false,
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

#[test]
fn limit_above_max_memory_limit_keeps_the_old_one() {
    // ini_set stores max_memory_limit's value but leaves the allocator's
    // limit where it was (OnChangeMemoryLimit returns before setting it).
    let code = r#"<?php
var_dump(ini_set('memory_limit', '512M'), ini_get('memory_limit'));
var_dump(ini_set('memory_limit', '-1'), ini_get('memory_limit'));
$x = str_repeat('a', 40 << 20);
echo "40M\n";
$y = str_repeat('a', 40 << 20);
echo "80M\n";
$z = str_repeat('a', 100 << 20);
echo "180M\n";
"#;
    assert_same_with("max_runtime", &["-d", "max_memory_limit=256M"], code, true);
    assert_same_with(
        "max_runtime64",
        &["-d", "max_memory_limit=256M", "-d", "memory_limit=64M"],
        code,
        true,
    );
}

#[test]
fn clamped_startup_value_is_restored_quietly() {
    // The startup warning is printed once: the entry holds the clamped value.
    assert_same_with(
        "max_startup",
        &["-d", "memory_limit=64M", "-d", "max_memory_limit=32M"],
        r#"<?php
echo "x\n";
ini_set('memory_limit', '16M');
echo ini_get('memory_limit'), "\n";
"#,
        true,
    );
}

#[test]
fn concat_reports_php_sizes_and_stops_at_the_hard_limit() {
    // The result's zend_string size; growing a huge string in place checks
    // only the growth; past twice the limit php-cli exits 137. (Each
    // crossing is on the line of the last call, which php reports.)
    //
    // php grows a huge block in place only when the address range after it
    // is free (a fixed mmap on macOS, mremap on Linux). macOS's bottom-up
    // layout leaves room; Linux's top-down one usually doesn't, so php
    // copies instead and checks the full size, which can cross the hard
    // limit earlier. rhp always grows in place, so the sizes, and for
    // concat_size the outcome, are compared on macOS only.
    let in_place = cfg!(target_os = "macos");
    if in_place {
        assert_same(
            "concat_size",
            r#"<?php
ini_set('memory_limit', '8M');
$a = str_repeat('a', 3 << 20); $b = $a . $a . $a;
echo strlen($b), "\n";
if ($b) { echo "not reached\n"; }
"#,
            true,
        );
    }
    assert_same(
        "concat_double",
        r#"<?php
ini_set('memory_limit', '32M');
$s = str_repeat('x', 1 << 20);
while (true) $s .= $s;
"#,
        in_place,
    );
    assert_same(
        "concat_hard",
        r#"<?php
register_shutdown_function(function () { echo "not reached\n"; });
ini_set('memory_limit', '32M');
$a = str_repeat('x', 12 << 20); $b = $a . $a . $a . $a . $a . $a;
echo strlen($b), "\n";
"#,
        in_place,
    );
}

#[test]
fn file_get_contents_refuses_a_file_past_the_limit() {
    if !php_available() {
        return;
    }
    let dir = std::env::temp_dir().join("rhp_memory_limit_fgc_data");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let big = dir.canonicalize().expect("canonical dir").join("big.bin");
    std::fs::write(&big, vec![b'z'; 20 << 20]).expect("write data");
    assert_same(
        "fgc",
        &format!(
            r#"<?php
ini_set('memory_limit', '8M');
var_dump(strlen(file_get_contents('{big}', false, null, 0, 100000)));
var_dump(file_get_contents('{big}', false, null, PHP_INT_MIN));
$x = file_get_contents('{big}');
echo "not reached\n";
"#,
            big = big.display()
        ),
        true,
    );
}

#[test]
fn pending_error_is_raised_before_a_loop_starts() {
    // php's while/for jump to their condition first: an interrupt check.
    for (name, lp) in [
        ("loop_for", "for ($i = 0; $i < 3; $i++) { echo $i; }"),
        ("loop_while", "$i = 0; while ($i < 3) { echo $i++; }"),
    ] {
        assert_same(
            name,
            &format!(
                "<?php\nini_set('memory_limit', '16M');\n$s = '';\n$s[20000000] = 'x';\necho strlen($s), \"\\n\";\n{lp}\n"
            ),
            true,
        );
    }
}

#[test]
fn iconv_does_not_count_its_copy_of_the_argument() {
    // php converts the caller's string in place; the NUL-terminated copy rhp
    // hands the library is not php memory. Peaks are rounded to MiB.
    assert_same(
        "iconv_peak",
        r#"<?php
ini_set('memory_limit', '32M');
function peak(string $what, callable $f) {
    memory_reset_peak_usage();
    $base = memory_get_usage();
    $r = $f();
    printf("%s: %s, peak +%d MiB\n", $what, is_string($r) ? strlen($r) : var_export($r, true),
        round((memory_get_peak_usage() - $base) / 1048576));
}
$s = str_repeat('a', 1 << 20);
peak('iconv', fn() => iconv('UTF-8', 'ISO-8859-1', $s));
peak('iconv_strlen', fn() => iconv_strlen($s));
peak('iconv_substr', fn() => iconv_substr($s, 1, 10));
peak('iconv_strpos', fn() => iconv_strpos($s, 'b'));
peak('iconv_strrpos', fn() => iconv_strrpos($s, 'b'));
peak('iconv_mime_decode', fn() => iconv_mime_decode($s));
peak('iconv_mime_decode_headers', fn() => count(iconv_mime_decode_headers("X: $s")));
peak('iconv_mime_encode', fn() => iconv_mime_encode('X', $s, ['scheme' => 'Q']));
// The input and the output (12 MiB each) fit under the limit.
$big = str_repeat('a', 12 << 20);
echo strlen(iconv('UTF-8', 'ISO-8859-1', $big)), "\n";
"#,
        true,
    );
}

#[test]
fn hash_pbkdf2_past_the_hard_limit() {
    // Exits like php instead of aborting on the failed allocation.
    assert_same(
        "pbkdf2",
        r#"<?php
ini_set('memory_limit', '8M');
hash_pbkdf2('md5', 'password', 'salt', 1, PHP_INT_MAX);
"#,
        true,
    );
}
