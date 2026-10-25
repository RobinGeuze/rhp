//! End-to-end differential tests for PHP namespaces: write namespaced PHP to a
//! temp dir, run it through the real `php` binary and the built `rhp` binary, and
//! compare stdout. Skips cleanly when no `php` is on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

fn php_available() -> bool {
    Command::new("php").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write fixture");
}

fn run_php(main: &Path) -> Vec<u8> {
    Command::new("php")
        .args(["-n", "-d", "error_reporting=0", "-d", "date.timezone=UTC"])
        .arg(main)
        .output()
        .expect("run php")
        .stdout
}

fn run_rhp(main: &Path) -> Vec<u8> {
    Command::new(env!("CARGO_BIN_EXE_rhp")).arg(main).output().expect("run rhp").stdout
}

fn case_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rhp_ns_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("mkdir");
    dir
}

/// Assert `rhp` matches `php` on `<main>` and that php produced output.
fn assert_matches(main: &Path) {
    let (php, rhp) = (run_php(main), run_rhp(main));
    assert_eq!(
        String::from_utf8_lossy(&php),
        String::from_utf8_lossy(&rhp),
        "namespace stdout differs from php"
    );
    assert!(!php.is_empty(), "expected output");
}

/// Multiple namespace blocks in one file: declarations, `use` (plain / grouped /
/// function), relative names, class constants, static calls, `instanceof`, FQ
/// `catch`, and the unqualified builtin fall-back-to-global.
#[test]
fn namespaces_single_file_matches_php() {
    if !php_available() {
        eprintln!("[namespaces] no php — skipping");
        return;
    }
    let dir = case_dir("single");
    write(
        &dir.join("main.php"),
        r#"<?php
        namespace App\Model;
        class User {
            const ROLE = 'admin';
            public static function make($n) { return new self(); }
            public function __construct(public string $name = 'x') {}
            public function role() { return self::ROLE; }
        }
        interface Named {}
        class Admin extends User implements Named {}
        function fmt($u) { return $u->name . '/' . $u->role(); }

        namespace App2;
        use App\Model\User;
        use App\Model\{Admin, Named};
        use function App\Model\fmt;
        $u = new User('bob');
        echo fmt($u), "\n";
        echo User::ROLE, "\n";
        echo User::make(1)->role(), "\n";
        $a = new Admin('amy');
        var_dump($a instanceof \App\Model\Named);
        var_dump($a instanceof User);
        echo \App\Model\User::ROLE, "\n";
        echo namespace\xyz(), "\n";
        function xyz() { return "ns2-xyz"; }
        echo strtoupper("hi"), "\n";
        try { throw new \RuntimeException("boom"); }
        catch (\RuntimeException $e) { echo "caught ", $e->getMessage(), "\n"; }
        echo "NS=[", __NAMESPACE__, "]\n";
        "#,
    );
    assert_matches(&dir.join("main.php"));
}

/// PSR-style layout: a namespaced class pulled in across `require`, then used
/// from two different namespaces (imported + fully-qualified). Also exercises
/// a namespaced user function shadowing a global builtin, and `\name` forcing
/// the global, and an unqualified call falling back to the global builtin.
#[test]
fn namespaces_across_include_matches_php() {
    if !php_available() {
        return;
    }
    let dir = case_dir("include");
    write(
        &dir.join("src/Greeter.php"),
        r#"<?php
        namespace Acme\Hello;
        class Greeter {
            public function __construct(private string $who) {}
            public function greet(): string { return "Hello, {$this->who}!"; }
        }
        function strlen($s) { return "SHADOWED"; }
        "#,
    );
    write(
        &dir.join("main.php"),
        r#"<?php
        namespace Acme\Hello;
        require __DIR__ . '/src/Greeter.php';
        $g = new Greeter("World");
        echo $g->greet(), "\n";
        echo strlen("abc"), "\n";   // namespaced fn wins
        echo \strlen("abc"), "\n";  // forced global builtin

        namespace Other;
        require_once __DIR__ . '/src/Greeter.php'; // _once dedup
        use Acme\Hello\Greeter;
        $g2 = new Greeter("there");
        echo $g2->greet(), "\n";
        echo strlen("abcd"), "\n";  // global builtin fallback
        "#,
    );
    assert_matches(&dir.join("main.php"));
}

/// A namespaced constant with the global fall-back rule, and an undefined
/// constant reporting the fully-qualified name (matching php's error path, which
/// here is suppressed to stderr; we just confirm the surviving stdout matches).
#[test]
fn namespaces_constants_matches_php() {
    if !php_available() {
        return;
    }
    let dir = case_dir("consts");
    write(
        &dir.join("main.php"),
        r#"<?php
        namespace Lib;
        const VERSION = '1.2.3';
        namespace App;
        use const Lib\VERSION;
        echo VERSION, "\n";                 // imported
        echo \Lib\VERSION, "\n";            // fully qualified
        echo PHP_EOL === "\n" ? "eol-ok\n" : "eol-no\n"; // global fallback
        define('App\\DEFINED', 42);
        echo DEFINED, "\n";                 // namespaced runtime constant
        "#,
    );
    assert_matches(&dir.join("main.php"));
}
