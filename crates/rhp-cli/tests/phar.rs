//! End-to-end tests for the phar / stream / resource / superglobal machinery
//! (milestone M1 — running a phar like composer). Builds a phar with the real
//! `php -d phar.readonly=0`, runs it through `php` and the built `rhp`, and
//! compares. Skips cleanly when no `php` is on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

fn php_available() -> bool {
    Command::new("php").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn can_build_phar() -> bool {
    // phar.readonly must be togglable; check php accepts -d phar.readonly=0.
    Command::new("php")
        .args(["-d", "phar.readonly=0", "-r", "echo (int)ini_get('phar.readonly');"])
        .output()
        .map(|o| o.stdout == b"0")
        .unwrap_or(false)
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write");
}

fn case_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rhp_phar_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// Build `<dir>/<name>.phar` from a map of entry→source plus a stub, using php.
fn build_phar(dir: &Path, name: &str, maker_php: &str) {
    let maker = dir.join("make.php");
    write(&maker, maker_php);
    let out = Command::new("php")
        .args(["-d", "phar.readonly=0"])
        .arg(&maker)
        .output()
        .expect("run php make");
    assert!(out.status.success(), "phar build failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.join(format!("{name}.phar")).exists(), "phar not created");
}

/// A phar that: executes its stub, maps itself, requires an internal file across
/// directories, autoloads a namespaced class through `phar://`, writes to
/// `php://stdout` via `fopen`/`fwrite` (resources), and reads `$_SERVER`.
#[test]
fn phar_runs_like_php() {
    if !php_available() || !can_build_phar() {
        eprintln!("[phar] no php / phar.readonly locked — skipping");
        return;
    }
    let dir = case_dir("run");
    build_phar(
        &dir,
        "app",
        r#"<?php
        $p = new Phar(__DIR__.'/app.phar', 0, 'app.phar');
        $p->addFromString('src/App/Greeter.php',
            '<?php namespace App; class Greeter { public function hi($n){ return "hi $n"; } }');
        $p->addFromString('autoload.php',
            '<?php spl_autoload_register(function($c){ $f="phar://app.phar/src/".str_replace("\\\\","/",$c).".php"; if(file_exists($f)) require $f; });');
        $p->addFromString('bin/run',
            '<?php require "phar://app.phar/autoload.php";
             $g = new \App\Greeter();
             $out = fopen("php://stdout","w");
             fwrite($out, $g->hi("world")."\n");
             fwrite($out, "argv1=".($_SERVER["argv"][1] ?? "none")."\n");
             fwrite($out, "file=".(str_contains(__FILE__,"app.phar") ? "ok" : "no")."\n");
             fclose($out);');
        $p->setStub('<?php Phar::mapPhar("app.phar"); require "phar://app.phar/bin/run"; __HALT_COMPILER(); ?>');
        echo "built\n";
        "#,
    );
    let main = dir.join("app.phar");
    let php = {
        let o = Command::new("php").args(["-n"]).arg(&main).arg("hello").output().expect("php");
        o.stdout
    };
    let rhp = {
        let o =
            Command::new(env!("CARGO_BIN_EXE_rhp")).arg(&main).arg("hello").output().expect("rhp");
        o.stdout
    };
    assert_eq!(String::from_utf8_lossy(&php), String::from_utf8_lossy(&rhp), "phar stdout differs");
    assert!(!php.is_empty());
}
