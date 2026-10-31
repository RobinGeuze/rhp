//! End-to-end differential test for `include`/`require`: write a multi-file PHP
//! project to a temp dir, run it through the real `php` binary and the built
//! `rhp` binary, and compare stdout. Skips cleanly when no `php` is on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

fn php_available() -> bool {
    Command::new("php")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write fixture");
}

/// stdout of `php -n -d error_reporting=0 <main>` (errors go to stderr, which php
/// suppresses — matching rhp, whose fatals also go to stderr).
fn run_php(main: &Path) -> Vec<u8> {
    let out = Command::new("php")
        .args(["-n", "-d", "error_reporting=0", "-d", "date.timezone=UTC"])
        .arg(main)
        .output()
        .expect("run php");
    out.stdout
}

/// stdout of the built `rhp` binary on `<main>`.
fn run_rhp(main: &Path) -> Vec<u8> {
    let out = Command::new(env!("CARGO_BIN_EXE_rhp"))
        .arg(main)
        .output()
        .expect("run rhp");
    out.stdout
}

fn case_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rhp_inc_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).expect("mkdir");
    dir
}

#[test]
fn include_require_matches_php() {
    if !php_available() {
        eprintln!("[include] no php — skipping");
        return;
    }
    let dir = case_dir("core");
    write(
        &dir.join("lib.php"),
        "<?php
        function greet($n) { return \"hi $n\"; }
        class W { function go() { $f = function () { return 'cl'; }; return $f() . '/' . (fn() => 'arrow')(); } }
        const V = '1.2';
        ",
    );
    write(
        &dir.join("config.php"),
        "<?php return ['n' => 'cfg', 's' => array_sum([1, 2, 3]), 'f' => function ($x) { return $x * 10; }];",
    );
    write(
        &dir.join("sub/relay.php"),
        "<?php require_once __DIR__ . '/../lib.php'; return greet('sub') . '|' . (new W)->go();",
    );
    write(
        &dir.join("main.php"),
        "<?php
        require_once __DIR__ . '/lib.php';
        echo greet('x'), '|', (new W)->go(), '|', V, \"\\n\";
        $c = require __DIR__ . '/config.php';
        echo $c['n'], $c['s'], ($c['f'])(2), \"\\n\";
        require_once __DIR__ . '/lib.php'; echo \"dedup-ok\\n\";
        $r = require __DIR__ . '/sub/relay.php'; echo $r, \"\\n\";
        $m = @include __DIR__ . '/nope.php'; var_dump($m);
        echo 'file-ok=', (str_contains(__FILE__, 'main.php') ? 'yes' : 'no'), \"\\n\";
        ",
    );
    let main = dir.join("main.php");
    let (php, rhp) = (run_php(&main), run_rhp(&main));
    assert_eq!(
        String::from_utf8_lossy(&php),
        String::from_utf8_lossy(&rhp),
        "include/require stdout differs from php"
    );
    assert!(!php.is_empty(), "expected output");
}

#[test]
fn spl_autoload_matches_php() {
    if !php_available() {
        return;
    }
    let dir = case_dir("autoload");
    std::fs::create_dir_all(dir.join("classes")).unwrap();
    write(
        &dir.join("classes/Mailer.php"),
        "<?php class Mailer { function send($t){ return \"to:$t\"; } }",
    );
    write(
        &dir.join("classes/Logger.php"),
        "<?php class Logger { static function log($m){ return \"L:$m\"; } }",
    );
    write(
        &dir.join("main.php"),
        "<?php
        spl_autoload_register(function ($class) {
            $f = __DIR__ . '/classes/' . $class . '.php';
            if (file_exists($f)) require $f;
        });
        echo (new Mailer())->send('x'), \"\\n\";
        echo Logger::log('y'), \"\\n\";
        var_dump(class_exists('Mailer'));
        echo count(spl_autoload_functions()), \"\\n\";
        ",
    );
    let main = dir.join("main.php");
    let (php, rhp) = (run_php(&main), run_rhp(&main));
    assert_eq!(String::from_utf8_lossy(&php), String::from_utf8_lossy(&rhp));
    assert_eq!(php, b"to:x\nL:y\nbool(true)\n1\n");
}

#[test]
fn eval_matches_php() {
    if !php_available() {
        return;
    }
    let dir = case_dir("eval");
    write(
        &dir.join("main.php"),
        "<?php
        echo eval('return 1 + 2;'), \"\\n\";
        $fn = eval('return function($v){ return $v * 2; };');
        echo $fn(21), \"\\n\";
        // composer FilesystemRepository pattern: prepend a close tag then a php file
        $content = \"<?php return array('a' => 1, 'b' => 2);\";
        $d = eval('?>' . $content);
        echo json_encode($d), \"\\n\";
        // eval executes in the calling scope.
        function g() { $x = 5; eval('$x = $x * 10; $y = 99;'); return \"$x,$y\"; }
        echo g(), \"\\n\";
        var_dump(eval('$z = 1;'));
        ",
    );
    let main = dir.join("main.php");
    let (php, rhp) = (run_php(&main), run_rhp(&main));
    assert_eq!(
        String::from_utf8_lossy(&php),
        String::from_utf8_lossy(&rhp),
        "eval stdout differs"
    );
    assert!(!php.is_empty());
}

#[test]
fn include_shares_caller_scope_like_php() {
    if !php_available() {
        return;
    }
    let dir = case_dir("incscope");
    // The included file reads a caller variable and defines new ones; php's
    // include runs in the caller's scope, so both directions are visible.
    write(
        &dir.join("vars.php"),
        "<?php $vendorDir = $base . '/vendor'; $extra = 'E'; return $vendorDir;",
    );
    write(
        &dir.join("main.php"),
        "<?php
        function load($p) {
            $base = '/root';
            $ret = require $p;         // vars.php sets $vendorDir/$extra from $base
            return \"$ret|$vendorDir|$extra\";
        }
        echo load(__DIR__ . '/vars.php'), \"\\n\";
        ",
    );
    let main = dir.join("main.php");
    let (php, rhp) = (run_php(&main), run_rhp(&main));
    assert_eq!(
        String::from_utf8_lossy(&php),
        String::from_utf8_lossy(&rhp),
        "include scope differs"
    );
    assert_eq!(php, b"/root/vendor|/root/vendor|E\n");
}

#[test]
fn require_missing_is_fatal_like_php() {
    if !php_available() {
        return;
    }
    let dir = case_dir("fatal");
    write(
        &dir.join("main.php"),
        "<?php echo \"before\\n\"; require __DIR__ . '/nope.php'; echo \"after\\n\";",
    );
    let main = dir.join("main.php");
    let (php, rhp) = (run_php(&main), run_rhp(&main));
    // Both stop after "before". php runs with error_reporting=0 here, which
    // hides its "Fatal error: Uncaught Error: Failed opening required …";
    // rhp (no ini) always displays it, as `php -n` does.
    assert_eq!(php, b"before\n");
    let rhp = String::from_utf8_lossy(&rhp);
    let (out, fatal) = rhp.split_at(php.len());
    assert_eq!(out, "before\n");
    assert!(
        fatal.starts_with("\nFatal error: Uncaught Error: Failed opening required '"),
        "unexpected fatal: {fatal:?}"
    );
}
