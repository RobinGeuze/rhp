//! Engine refusals (`RHP_UNSUPPORTED: …`) name the file and line of the
//! refused construct — in the main script, in included files, and for
//! refusals deferred to run time. (Goldens can't express these: php supports
//! the constructs, so there is no php output to compare against.)
//!
//! These use constructs rhp refuses today; when one becomes supported, swap
//! in another refused construct.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Case(PathBuf);

impl Case {
    fn new(name: &str) -> Case {
        let dir = std::env::temp_dir().join(format!("rhp_unsupported_{name}"));
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
        "<?php\n$a = [];\n\n$o = new stdClass;\n$o->p['k'] ??= 1;\n",
    );
    let (out, err, code) = run(&main);
    assert_eq!(out, "", "a compile-time refusal runs nothing");
    assert_eq!(code, Some(255));
    assert_eq!(
        refusal(&err),
        format!(
            "RHP_UNSUPPORTED: ??= on an array element is not supported \
             (use a full assignment for now) in {} on line 5",
            main.display()
        )
    );
}

#[test]
fn compound_coalesce_names_the_actual_target() {
    let c = Case::new("target");
    let prop = c.file("prop.php", "<?php\n$o = new stdClass;\n$o->p ??= 1;\n");
    let err = run(&prop).1;
    assert!(
        refusal(&err).starts_with("RHP_UNSUPPORTED: ??= on a property is not supported"),
        "{err}"
    );
    let sprop = c.file(
        "sprop.php",
        "<?php\nclass K { public static $s; }\n\nK::$s ??= 1;\n",
    );
    let err = run(&sprop).1;
    assert_eq!(
        refusal(&err),
        format!(
            "RHP_UNSUPPORTED: ??= on a static property is not supported \
             (use a full assignment for now) in {} on line 4",
            sprop.display()
        )
    );
}

#[test]
fn included_file_refusal_names_included_file() {
    let c = Case::new("include");
    let inc = c.file(
        "inc.php",
        "<?php\nfunction f() {\n    $a = [];\n    $a['k']['j'] ??= 2;\n}\n",
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
            "RHP_UNSUPPORTED: ??= on an array element is not supported \
             (use a full assignment for now) in {} on line 4",
            inc.display()
        )
    );
}

#[test]
fn class_member_refusal_names_member_line() {
    let c = Case::new("member");
    let main = c.file(
        "main.php",
        "<?php\nclass C {\n    public $x = 1;\n\n    public int $y { get => 1; }\n}\n",
    );
    let err = run(&main).1;
    assert_eq!(
        refusal(&err),
        format!(
            "RHP_UNSUPPORTED: property hooks is not supported (deferred) in {} on line 5",
            main.display()
        )
    );
}

#[test]
fn deferred_refusal_names_file_and_line() {
    let c = Case::new("deferred");
    let main = c.file(
        "main.php",
        "<?php\necho \"a\\n\";\n\ngoto end;\nend:\necho \"b\\n\";\n",
    );
    let (out, err, _) = run(&main);
    assert_eq!(out, "a\n", "a deferred refusal throws only when reached");
    let line = refusal(&err);
    assert!(line.starts_with("RHP_UNSUPPORTED: goto"), "{line}");
    assert!(
        line.ends_with(&format!(" in {} on line 4", main.display())),
        "{line}"
    );
}
