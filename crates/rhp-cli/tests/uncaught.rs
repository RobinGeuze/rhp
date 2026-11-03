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

#[test]
fn exception_handler_receives_uncaught() {
    if !php_available() {
        return;
    }
    let dir = case_dir("handler");
    write(
        &dir.join("main.php"),
        "<?php\nvar_dump(set_exception_handler(function ($e) { echo 'h: ', get_class($e), ' ', $e->getMessage(), \"\\n\"; }));\nvar_dump(set_exception_handler('first') instanceof Closure);\nrestore_exception_handler();\necho \"a\\n\";\nnope();\n",
    );
    assert_same(&dir.join("main.php"));
    // A handler that throws: the new exception is the uncaught one.
    let dir = case_dir("handler_throws");
    write(
        &dir.join("main.php"),
        "<?php\nset_exception_handler(function (Throwable $e) {\n    echo 'Caught: ', $e->getMessage(), \"\\n\";\n    throw new Exception('from handler');\n});\nthrow new Exception('main');\n",
    );
    let out = assert_same(&dir.join("main.php"));
    assert!(out.contains("[internal function]"), "{out}");
}

/// Run each `(case, script)` through both engines (stdout, stderr, rc).
fn assert_all_same(cases: &[(&str, &str)]) {
    for (name, src) in cases {
        let dir = case_dir(name);
        write(&dir.join("main.php"), src);
        assert_same(&dir.join("main.php"));
    }
}

#[test]
fn uncaught_honours_display_errors_log_errors_and_error_reporting() {
    if !php_available() {
        return;
    }
    assert_all_same(&[
        // composer's bin/composer: display_errors=stderr (no leading newline).
        (
            "de_stderr",
            "<?php\nini_set('display_errors', 'stderr');\necho \"a\\n\";\nthrow new Exception('to stderr');\n",
        ),
        ("de_two", "<?php\nini_set('display_errors', '2');\nthrow new Exception('x');\n"),
        ("de_off", "<?php\nini_set('display_errors', '0');\necho \"a\\n\";\nthrow new Exception('hidden');\n"),
        ("de_on", "<?php\nini_set('display_errors', 'On');\nthrow new Exception('x');\n"),
        ("er_zero", "<?php\nerror_reporting(0);\necho \"a\\n\";\nthrow new Exception('hidden');\n"),
        (
            "er_no_error",
            "<?php\nerror_reporting(E_ALL & ~E_ERROR);\nthrow new Exception('hidden');\n",
        ),
        (
            "log_and_display",
            "<?php\nini_set('log_errors', '1');\nob_start();\necho \"a\\n\";\nthrow new Exception('both');\n",
        ),
        (
            "log_only",
            "<?php\nini_set('log_errors', 'yes');\nini_set('display_errors', 'off');\nnull_fn();\n",
        ),
        (
            "prepend_append",
            "<?php\nini_set('error_prepend_string', '<<');\nini_set('error_append_string', '>>');\nthrow new Exception('x');\n",
        ),
    ]);
}

#[test]
fn uncaught_flushes_output_buffers_and_runs_shutdown_functions() {
    if !php_available() {
        return;
    }
    assert_all_same(&[
        ("ob_exc", "<?php\nob_start();\necho \"buffered\";\nthrow new Exception('x');\n"),
        ("ob_engine", "<?php\nob_start();\necho \"buffered\";\n$x = null;\n$x->m();\n"),
        (
            "sd_after_fatal",
            "<?php\nregister_shutdown_function(function () { echo \"shutdown\\n\"; });\nthrow new RuntimeException('boom');\n",
        ),
        (
            "sd_with_handler",
            "<?php\nset_exception_handler(function ($e) { echo \"H\\n\"; });\nregister_shutdown_function(function () { echo \"shutdown\\n\"; });\nthrow new RuntimeException('boom');\n",
        ),
        // A fatal in a shutdown function stops the rest.
        (
            "sd_throws",
            "<?php\nregister_shutdown_function(function ($a) { echo \"sd1 $a\\n\"; register_shutdown_function(function () { echo \"nested\\n\"; }); }, 'arg');\nregister_shutdown_function(function () { echo \"sd2\\n\"; throw new Exception('in sd'); });\nregister_shutdown_function(function () { echo \"sd3\\n\"; });\nob_start();\necho \"buffered\\n\";\nthrow new Exception('main');\n",
        ),
        // exit() in a shutdown function sets the status and stops the rest.
        (
            "sd_exit",
            "<?php\nregister_shutdown_function(function () { echo \"sd1\\n\"; exit(3); });\nregister_shutdown_function(function () { echo \"sd2\\n\"; });\nthrow new Exception('main');\n",
        ),
        // With a handler, a shutdown function's exception is handled too.
        (
            "sd_handler_continues",
            "<?php\nset_exception_handler(function ($e) { echo 'H ', $e->getMessage(), \"\\n\"; });\nregister_shutdown_function(function () { echo \"sd1\\n\"; throw new Exception('from sd'); });\nregister_shutdown_function(function () { echo \"sd2\\n\"; });\nthrow new Exception('main');\n",
        ),
        (
            "sd_handler_rethrows",
            "<?php\nset_exception_handler(function ($e) { echo 'H ', $e->getMessage(), \"\\n\"; throw new LogicException('again'); });\nregister_shutdown_function(function () { echo \"sd1\\n\"; });\nthrow new Exception('main');\n",
        ),
        (
            "sd_display_off",
            "<?php\nregister_shutdown_function(function () { echo \"sd\\n\"; });\nini_set('display_errors', '0');\nthrow new Exception('x');\n",
        ),
    ]);
}

#[test]
fn parse_errors_main_eval_and_include() {
    if !php_available() {
        return;
    }
    let dir = case_dir("parse");
    // The main script itself does not parse.
    let bad = dir.join("bad.php");
    write(&bad, "<?php\necho 1;\n$x = ;\n");
    let out = assert_same(&bad);
    assert!(out.contains("Parse error: syntax error, unexpected token \";\""));
    // Uncaught ParseError from eval() and from include.
    let ev = dir.join("ev.php");
    write(
        &ev,
        "<?php\necho \"a\\n\";\neval(\"echo 1;\\n\\nfunction (\");\n",
    );
    let out = assert_same(&ev);
    assert!(out.contains("Unclosed '('"));
    let inc = dir.join("inc.php");
    write(
        &inc,
        "<?php\nfunction f() { include __DIR__ . '/bad.php'; }\nf();\n",
    );
    assert_same(&inc);
    // A grammar action's CompileError: a fatal error for the main script,
    // an uncaught CompileError from include.
    let mods = dir.join("mods.php");
    write(&mods, "<?php\nclass A {\n    public\n    public $x;\n}\n");
    let out = assert_same(&mods);
    assert!(out.contains("Fatal error: Multiple access type modifiers are not allowed"));
    let inc_mods = dir.join("inc_mods.php");
    write(
        &inc_mods,
        "<?php\necho 1;\ninclude __DIR__ . '/mods.php';\n",
    );
    assert_same(&inc_mods);
}

/// An arity error of a prelude-implemented builtin is reported at the
/// caller's file and line, like php's internal functions.
#[test]
fn uncaught_prelude_arity_error_at_caller() {
    if !php_available() {
        return;
    }
    let dir = case_dir("prelude_arity");
    let main = dir.join("main.php");
    write(
        &main,
        "<?php\nfunction f() {\n    return new ArrayObject([], 0, 'ArrayIterator', 5);\n}\ntry {\n    iterator_to_array();\n} catch (ArgumentCountError $e) {\n    echo basename($e->getFile()), ':', $e->getLine(), \"\\n\";\n}\nf();\n",
    );
    let out = assert_same(&main);
    assert!(out.contains("main.php:6"), "{out}");
    assert!(out.contains("main.php on line 3"), "{out}");
}
