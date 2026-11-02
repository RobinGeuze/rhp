//! rhp reports the reference php's build identity: `tools/capture-ref-build.php`
//! prints the same under `php -n` and `rhp -n` (the values live in
//! `crates/rhp-builtins/src/version.rs`).
//!
//! The "release" section is compared whenever the local php is the same
//! release as rhp; the "build" section (paths, build date, library versions)
//! only against the reference build itself, Homebrew php on macOS.

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
    if rel_ours.first() != rel_theirs.first() {
        eprintln!(
            "php is {:?}, rhp is {:?}: not the reference release; skipping",
            rel_theirs.first(),
            rel_ours.first()
        );
        return;
    }
    assert_eq!(rel_ours, rel_theirs, "release section differs from php");
    let provider = run(&php, &["-n", "-r", "echo PHP_BUILD_PROVIDER;"]).unwrap_or_default();
    if !cfg!(target_os = "macos") || provider != "Homebrew" {
        eprintln!("php is not the Homebrew reference build; build section not compared");
        return;
    }
    assert_eq!(
        section(&ours, "build"),
        section(&theirs, "build"),
        "build section differs from the reference php: update version.rs"
    );
}
