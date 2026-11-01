//! CLI SAPI behaviours that need a real process: the script read from stdin
//! (no script file, no `-r`; phpunit's process isolation pipes code into
//! `PHP_BINARY`), and `getopt()` over the request's argv. Each case runs
//! through `php -n` and the built `rhp` and compares stdout, stderr and the
//! exit code. Skips cleanly when no `php` is on PATH.

use std::io::Write;
use std::process::{Command, Stdio};

fn php_available() -> bool {
    Command::new("php")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run `bin args...` with `stdin` piped in; (stdout, stderr, exit code).
fn run(bin: &str, args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn same(php_args: &[&str], rhp_args: &[&str], stdin: &[u8]) {
    let php = run("php", php_args, stdin);
    let rhp = run(env!("CARGO_BIN_EXE_rhp"), rhp_args, stdin);
    assert_eq!(php, rhp, "php {php_args:?} vs rhp {rhp_args:?}");
}

#[test]
fn script_from_stdin() {
    if !php_available() {
        eprintln!("[cli] no php — skipping");
        return;
    }
    let script = br#"<?php
echo __FILE__, "\n", __LINE__, "\n", basename(__DIR__) === basename(getcwd()) ? "cwd" : "other", "\n";
var_dump($argv, $argc, $_SERVER['SCRIPT_FILENAME'], $_SERVER['PHP_SELF'], get_included_files());
var_dump(fgets(STDIN));
trigger_error("note");
throw new Exception("boom");
"#;
    same(&["-n"], &[], script);
    same(
        &["-n", "-d", "x=1", "--", "a", "b"],
        &["-d", "x=1", "--", "a", "b"],
        script,
    );
    same(
        &["-n", "-d", "display_errors=0"],
        &["-d", "display_errors=0"],
        b"<?php echo 'quiet'; exit(3);",
    );
    same(&["-n"], &[], b"plain text, no php tag\n");
}

#[test]
fn getopt_over_argv() {
    if !php_available() {
        eprintln!("[cli] no php — skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rhp_getopt_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let script = dir.join("g.php");
    std::fs::write(
        &script,
        br#"<?php
var_dump(getopt('abc::d:', ['long::', 'req:', 'flag', 'no-cache', 'threads:', '1'], $rest), $rest);
$_SERVER['argv'] = ['ignored', '-a'];
var_dump(getopt('a'));
"#,
    )
    .unwrap();
    let s = script.to_str().unwrap();
    for args in [
        &["--no-cache", "--threads=1", "-c", "f", "a"][..],
        &[
            "-ab",
            "-cval",
            "-d",
            "dv",
            "--long=lv",
            "--req",
            "rv",
            "--flag",
            "--flag",
            "-a",
            "--1",
            "--unknown",
            "-z",
            "rest",
            "-a",
        ],
        &["-c", "v", "--long", "v2", "--", "-a"],
        &["-a", "-", "-b"],
        &["--req=", "-d"],
        &["--req"],
        &["-a:b", "-ba"],
        &["--long=", "--req=x=y", "-dd", "-x", "-a"],
        &[],
    ] {
        let mut php_args = vec!["-n", s];
        php_args.extend_from_slice(args);
        let mut rhp_args = vec![s];
        rhp_args.extend_from_slice(args);
        same(&php_args, &rhp_args, b"");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
