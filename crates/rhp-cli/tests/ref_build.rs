//! rhp reports the reference php's build identity: `tools/capture-ref-build.php`
//! prints the same under `php -n` and `rhp -n` (the values live in
//! `crates/rhp-builtins/src/version.rs`).
//!
//! Against the reference build itself (Homebrew php on macOS) both sections
//! must match, so upgrading php without bumping rhp fails here. Any other php
//! is compared on the "release" section only, and only when it is the same
//! release as rhp.

use std::path::{Path, PathBuf};
use std::process::Command;

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/capture-ref-build.php")
}

fn run(bin: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The lines of one `# name` section.
fn section<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("# {name}");
    text.lines()
        .skip_while(|l| *l != header)
        .skip(1)
        .take_while(|l| !l.starts_with("# "))
        .collect()
}

#[test]
fn rhp_reports_the_reference_build() {
    let php = PathBuf::from(std::env::var("RHP_PHP").unwrap_or_else(|_| "php".into()));
    let rhp = Path::new(env!("CARGO_BIN_EXE_rhp"));
    let script = script();
    let script = script.to_str().unwrap();
    let ours = run(rhp, &["-n", script]).expect("rhp runs the capture script");
    assert!(
        section(&ours, "release")
            .first()
            .is_some_and(|l| l.starts_with("PHP_VERSION = '8.5.")),
        "{ours}"
    );
    let Some(theirs) = run(&php, &["-n", script]) else {
        eprintln!("no php; skipping");
        return;
    };
    let (rel_ours, rel_theirs) = (section(&ours, "release"), section(&theirs, "release"));
    // On the reference host (Homebrew php on macOS) a release mismatch means
    // php was upgraded and rhp was not: fail and point at version.rs. Any
    // other php is only compared when it happens to be the same release.
    let provider = run(&php, &["-n", "-r", "echo PHP_BUILD_PROVIDER;"]).unwrap_or_default();
    let reference_host = cfg!(target_os = "macos") && provider == "Homebrew";
    if !reference_host && rel_ours.first() != rel_theirs.first() {
        eprintln!(
            "php is {:?}, rhp is {:?}: not the reference release; skipping",
            rel_theirs.first(),
            rel_ours.first()
        );
        return;
    }
    assert_eq!(
        rel_ours, rel_theirs,
        "release section differs from php: bump crates/rhp-builtins/src/version.rs \
         (see its header for the steps)"
    );
    if !reference_host {
        eprintln!("php is not the Homebrew reference build; build section not compared");
        return;
    }
    assert_eq!(
        section(&ours, "build"),
        section(&theirs, "build"),
        "build section differs from the reference php: update version.rs"
    );
}

/// `rhp -v` / `rhp --cgi -v` print the reference build's banner; identical to
/// `php -n -v` / `php-cgi -n -v` when those are the reference release.
#[test]
fn version_banner() {
    let rhp = Path::new(env!("CARGO_BIN_EXE_rhp"));
    let version = run(rhp, &["-n", "-r", "echo PHP_VERSION;"]).expect("rhp -r");
    for (args, sapi) in [
        (&["-n", "-v"][..], "cli"),
        (&["-n", "--version"], "cli"),
        (&["--cgi", "-n", "-v"], "cgi-fcgi"),
    ] {
        let ours = run(rhp, args).expect("rhp -v");
        assert!(
            ours.starts_with(&format!("PHP {version} ({sapi}) (built: ")),
            "{ours}"
        );
        assert!(ours.contains("\nZend Engine v4."), "{ours}");
    }
    // php -v exits before running anything, even with a script or -r.
    let ours = run(rhp, &["-n", "-v", "-r", "echo 1;"]).expect("rhp -v -r");
    assert!(!ours.contains("\n1"), "{ours}");

    let php = PathBuf::from(std::env::var("RHP_PHP").unwrap_or_else(|_| "php".into()));
    let cgi = PathBuf::from(std::env::var("PHP_CGI").unwrap_or_else(|_| "php-cgi".into()));
    for (bin, args, ours_args) in [
        (&php, &["-n", "-v"][..], &["-n", "-v"][..]),
        (&cgi, &["-n", "-v"], &["--cgi", "-n", "-v"]),
    ] {
        let Some(theirs) = run(bin, args) else {
            eprintln!("{} not runnable; skipping", bin.display());
            continue;
        };
        let provider = run(&php, &["-n", "-r", "echo PHP_BUILD_PROVIDER;"]).unwrap_or_default();
        if !theirs.starts_with(&format!("PHP {version} ")) || provider != "Homebrew" {
            eprintln!("{} is not the reference build; skipping", bin.display());
            continue;
        }
        assert_eq!(run(rhp, ours_args).unwrap(), theirs, "{}", bin.display());
    }
}
