//! Engine refusals (`RHP_UNSUPPORTED: …`) name the file and line of the
//! refused construct — in the main script, in included files, and for
//! builtins refused at run time. (Goldens can't express these: php supports
//! the constructs, so there is no php output to compare against.)
//!
//! The tests use constructs rhp refuses (a catch of a non-class type such as
//! `int`, create_function); a construct that lands needs another one here.
//!
//! The interactive shell (`-a`) is here too: php's answer depends on its
//! build (the reference php has readline, CI's does not), so the CLI
//! fixtures can't hold it.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Case(PathBuf);

impl Case {
    fn new(name: &str) -> Case {
        // Per-process, so concurrent test runs (other checkouts) don't collide.
        let dir =
            std::env::temp_dir().join(format!("rhp_unsupported_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        // Canonical, so the paths rhp reports (it canonicalizes includes)
        // match on systems where the temp dir is behind a symlink.
        Case(std::fs::canonicalize(&dir).expect("canonicalize"))
    }
    fn file(&self, name: &str, src: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, src).expect("write fixture");
        p
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// (stdout, stderr, exit code) of the built `rhp` on `script`.
fn run(script: &Path) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_rhp"))
        .arg(script)
        .output()
        .expect("run rhp");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn refusal(stderr: &str) -> &str {
    stderr
        .lines()
        .find(|l| l.starts_with("RHP_UNSUPPORTED: "))
        .unwrap_or_else(|| panic!("no RHP_UNSUPPORTED line in stderr: {stderr:?}"))
}

#[test]
fn main_script_refusal_names_file_and_line() {
    let c = Case::new("main");
    let main = c.file(
        "main.php",
        "<?php\n$a = [];\n\n$o = new stdClass;\ntry {} catch (int $e) {}\n",
    );
    let (out, err, code) = run(&main);
    assert_eq!(out, "", "a compile-time refusal runs nothing");
    assert_eq!(code, Some(255));
    assert_eq!(
        refusal(&err),
        format!(
            "RHP_UNSUPPORTED: catch type is not supported \
             (only class-name catch types) in {} on line 5",
            main.display()
        )
    );
}

#[test]
fn included_file_refusal_names_included_file() {
    let c = Case::new("include");
    let inc = c.file(
        "inc.php",
        "<?php\nfunction f() {\n    $a = [];\n    try {} catch (int $e) {}\n}\n",
    );
    let main = c.file(
        "main.php",
        "<?php\necho \"before\\n\";\nrequire __DIR__ . '/inc.php';\necho \"after\\n\";\n",
    );
    let (out, err, code) = run(&main);
    assert_eq!(out, "before\n");
    assert_eq!(code, Some(255));
    assert_eq!(
        refusal(&err),
        format!(
            "RHP_UNSUPPORTED: catch type is not supported \
             (only class-name catch types) in {} on line 4",
            inc.display()
        )
    );
}

#[test]
fn method_refusal_names_expression_line() {
    // (A property default with `new` is php's constant-expression error.)
    let c = Case::new("member");
    let main = c.file(
        "main.php",
        "<?php\nclass C {\n    public $x = 1;\n\n    public function y() { $a = [1,\n        2]; try {} catch (int $e) {} }\n}\n",
    );
    let err = run(&main).1;
    assert_eq!(
        refusal(&err),
        format!(
            "RHP_UNSUPPORTED: catch type is not supported \
             (only class-name catch types) in {} on line 6",
            main.display()
        )
    );
}

#[test]
fn builtin_refusal_names_calling_line() {
    let c = Case::new("builtin");
    let main = c.file(
        "main.php",
        "<?php\nfunction f() {\n    return create_function('', '');\n}\necho \"a\\n\";\nf();\n",
    );
    let (out, err, _) = run(&main);
    assert_eq!(out, "a\n");
    let line = refusal(&err);
    assert!(
        line.starts_with("RHP_UNSUPPORTED: create_function()"),
        "{line}"
    );
    assert!(
        line.ends_with(&format!(" in {} on line 3", main.display())),
        "{line}"
    );
}

/// rhp has no readline, so `-a` is php's error for a build without it, and
/// a later error replaces it.
#[test]
fn interactive_shell_needs_readline() {
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_rhp"))
            .arg("-n")
            .args(args)
            .output()
            .expect("run rhp");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code(),
        )
    };
    let needs = "Interactive shell (-a) requires the readline extension.\n";
    for args in [
        &["-a"][..],
        &["-r", "echo 1;", "-a"],
        &["-a", "-r", "echo 1;"],
    ] {
        assert_eq!(run(args), (needs.to_string(), Some(1)), "{args:?}");
    }
    assert_eq!(
        run(&["-a", "-f", "x.php", "-f", "y.php"]),
        ("You can use -f only once.\n".to_string(), Some(1))
    );
}
