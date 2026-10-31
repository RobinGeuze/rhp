//! End-to-end differential test for uncaught throwables and multi-file traces:
//! run scripts through `php -n` and the built `rhp`, comparing stdout, stderr
//! and the exit code. php -n displays `Fatal error: Uncaught …` on stdout.
//! Skips cleanly when no `php` is on PATH.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn php_available() -> bool {
    Command::new("php")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn case_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rhp_uncaught_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    // php reports resolved real paths (e.g. /private/var on macOS).
    dir.canonicalize().expect("canonical dir")
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write fixture");
}

fn run(bin: &str, main: &Path) -> Output {
    let mut c = Command::new(bin);
    if bin == "php" {
        c.arg("-n");
    }
    c.arg(main).output().expect("run engine")
}

fn assert_same(main: &Path) -> String {
    let php = run("php", main);
    let rhp = run(env!("CARGO_BIN_EXE_rhp"), main);
    let (ps, rs) = (
        String::from_utf8_lossy(&php.stdout).into_owned(),
        String::from_utf8_lossy(&rhp.stdout).into_owned(),
    );
    assert_eq!(ps, rs, "stdout differs");
    assert_eq!(
        String::from_utf8_lossy(&php.stderr),
        String::from_utf8_lossy(&rhp.stderr),
        "stderr differs"
    );
    assert_eq!(php.status.code(), rhp.status.code(), "exit code differs");
    ps
}

#[test]
fn uncaught_chain_with_args_and_previous() {
    if !php_available() {
        return;
    }
    let dir = case_dir("chain");
    write(
        &dir.join("main.php"),
        "<?php\necho \"before\\n\";\nfunction f($x, $s) {\n    throw new RuntimeException(\"boom\", 3, new LogicException(\"inner\"));\n}\nf([1], \"a long string argument\");\necho \"after\\n\";\n",
    );
    let out = assert_same(&dir.join("main.php"));
    assert!(out.contains("Next RuntimeException: boom"), "{out}");
}

#[test]
fn uncaught_engine_error_from_included_method() {
    if !php_available() {
        return;
    }
    let dir = case_dir("included");
    write(
        &dir.join("lib.php"),
        "<?php\n\nclass Lib {\n    public function run() {\n        return undefined_thing(1);\n    }\n}\n",
    );
    write(
        &dir.join("main.php"),
        "<?php\nrequire __DIR__ . '/lib.php';\nfunction go() { return require __DIR__ . '/inc.php'; }\n$e = go();\necho basename($e->getFile()), ':', $e->getLine(), \"\\n\";\necho str_replace(__DIR__, '', $e->getTraceAsString()), \"\\n\";\n$ev = eval('return new Exception(\"x\");');\necho basename($ev->getFile()), ':', $ev->getLine(), \"\\n\";\necho str_replace(__DIR__, '', $ev->getTraceAsString()), \"\\n\";\n(new Lib())->run();\n",
    );
    write(
        &dir.join("inc.php"),
        "<?php\n\nreturn new Exception('inc');\n",
    );
    let out = assert_same(&dir.join("main.php"));
    assert!(out.contains("require()"), "{out}");
    assert!(out.contains("Uncaught Error: Call to undefined function undefined_thing()"));
}

#[test]
fn uncaught_user_tostring_override_and_messageless() {
    if !php_available() {
        return;
    }
    let dir = case_dir("override");
    write(
        &dir.join("main.php"),
        "<?php\nclass E extends Exception { public function __toString(): string { return 'custom!'; } }\nfunction g() { throw new E(''); }\ng();\n",
    );
    let out = assert_same(&dir.join("main.php"));
    assert!(out.contains("Uncaught custom!"), "{out}");
    let dir = case_dir("messageless");
    write(&dir.join("main.php"), "<?php\n\nthrow new Error();\n");
    assert_same(&dir.join("main.php"));
}
