//! End-to-end tests for the phar / stream / resource / superglobal machinery
//! (milestone M1 — running a phar like composer). Builds a phar with the real
//! `php -d phar.readonly=0`, runs it through `php` and the built `rhp`, and
//! compares. Skips without php (`RHP_PHP`).

use std::path::{Path, PathBuf};
use std::process::Command;

use rhp_difftest::{php_available, php_bin};

fn can_build_phar() -> bool {
    // phar.readonly must be togglable; check php accepts -d phar.readonly=0.
    Command::new(php_bin())
        .args([
            "-d",
            "phar.readonly=0",
            "-r",
            "echo (int)ini_get('phar.readonly');",
        ])
        .output()
        .map(|o| o.stdout == b"0")
        .unwrap_or(false)
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write");
}

fn case_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rhp_phar_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// Build `<dir>/<name>.phar` from a map of entry→source plus a stub, using php.
fn build_phar(dir: &Path, name: &str, maker_php: &str) {
    let maker = dir.join("make.php");
    write(&maker, maker_php);
    let out = Command::new(php_bin())
        .args(["-d", "phar.readonly=0"])
        .arg(&maker)
        .output()
        .expect("run php make");
    assert!(
        out.status.success(),
        "phar build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        dir.join(format!("{name}.phar")).exists(),
        "phar not created"
    );
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
        let o = Command::new(php_bin())
            .args(["-n"])
            .arg(&main)
            .arg("hello")
            .output()
            .expect("php");
        o.stdout
    };
    let rhp = {
        let o = Command::new(env!("CARGO_BIN_EXE_rhp"))
            .arg(&main)
            .arg("hello")
            .output()
            .expect("rhp");
        o.stdout
    };
    assert_eq!(
        String::from_utf8_lossy(&php),
        String::from_utf8_lossy(&rhp),
        "phar stdout differs"
    );
    assert!(!php.is_empty());
}

/// `Phar::running()` reports only a phar that is actually executing (not the
/// main script), `Phar::mapPhar` maps the file that calls it (a phar required
/// from a wrapper script), and an explicit alias that contradicts the stored
/// one is refused with php's message.
#[test]
fn phar_running_and_map_from_wrapper() {
    if !php_available() || !can_build_phar() {
        eprintln!("[phar] no php / phar.readonly locked — skipping");
        return;
    }
    let dir = case_dir("wrap");
    build_phar(
        &dir,
        "s",
        r#"<?php
        $p = new Phar(__DIR__.'/s.phar');
        $p->addFromString('a.php', '<?php echo "in a: ", basename(Phar::running()), " ", basename(Phar::running(false)), "\n"; require "phar://s.phar/b.php";');
        $p->addFromString('b.php', '<?php echo "in b: ", basename(__FILE__), "\n";');
        $p->setStub('<?php Phar::mapPhar("s.phar"); require "phar://s.phar/a.php"; __HALT_COMPILER();');
        $q = new Phar(__DIR__.'/al.phar', 0, 'implicit.phar');
        $q->addFromString('x.php', '<?php echo "x\n";');
        $q->setStub('<?php __HALT_COMPILER();');
        "#,
    );
    write(
        &dir.join("wrap.php"),
        r#"<?php
namespace A;
var_dump(\Phar::running(), \Phar::running(false));
require __DIR__ . '/s.phar';
var_dump(\Phar::running());
try { \Phar::loadPhar(__DIR__ . '/al.phar', 'other.phar'); } catch (\PharException $e) { echo str_replace(__DIR__, 'DIR', $e->getMessage()), "\n"; }
var_dump(\Phar::loadPhar(__DIR__ . '/al.phar', 'implicit.phar'));
include 'phar://implicit.phar/x.php';
"#,
    );
    for target in ["wrap.php", "s.phar"] {
        let path = dir.join(target);
        let php = Command::new(php_bin())
            .arg("-n")
            .arg(&path)
            .output()
            .expect("php");
        let rhp = Command::new(env!("CARGO_BIN_EXE_rhp"))
            .arg(&path)
            .output()
            .expect("rhp");
        assert_eq!(
            String::from_utf8_lossy(&php.stdout),
            String::from_utf8_lossy(&rhp.stdout),
            "{target}: stdout differs"
        );
        assert_eq!(php.status.code(), rhp.status.code(), "{target}: exit code");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
