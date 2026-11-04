//! rhp reports the reference php's build identity: `tools/capture-ref-build.php`
//! prints the same under `php -n` and `rhp -n` (the values live in
//! `crates/rhp-builtins/src/version.rs`).
//!
//! Against Homebrew php on macOS the "release" section must match, so
//! upgrading php without bumping rhp fails here; against the reference
//! bottle itself (see [`bottle`]) the "build" section must match too, which
//! catches a rebottle. Any other php is compared on the "release" section
//! only, and only when it is the same release as rhp.
//!
//! The "linked" section holds the versions of the libraries rhp links
//! itself. It must match any php 8.5 on the same machine (Homebrew's on the
//! mac, setup-php's on CI): rhp has to link what php links. The ICU and
//! OpenSSL rows are left out when that php has no intl or openssl under `-n`.

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

/// Which Homebrew bottle `bin` is: the Darwin major version and architecture
/// in its phpinfo "Build System" (`uname -a` of the build host). Homebrew
/// builds one bottle per macOS version and CPU, each with its own build
/// date, paths (Intel: /usr/local) and library build hosts, while a rebottle
/// of the same php keeps both. `None` when the row is missing or names no
/// kernel release (Ubuntu's php says just "Linux").
fn bottle(bin: &Path) -> Option<(String, String)> {
    let code = r#"ob_start(); phpinfo(INFO_GENERAL);
        foreach (explode("\n", ob_get_clean()) as $l) {
            if (str_starts_with($l, 'Build System => ')) { echo substr($l, 16); }
        }"#;
    let uname = run(bin, &["-n", "-r", code])?;
    let words: Vec<&str> = uname.split_whitespace().collect();
    // "Darwin <host> 27.0.0 Darwin Kernel Version 27.0.0: ... arm64"
    let major = words.get(2)?.split('.').next()?;
    Some((major.to_string(), words.last()?.to_string()))
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
    let (darwin, arch) = bottle(rhp).expect("rhp -n prints phpinfo's Build System on macOS");
    match bottle(&php) {
        Some((d, a)) if d == darwin && a == arch => {}
        theirs => {
            eprintln!(
                "php is another Homebrew bottle (built on {theirs:?}, the reference on \
                 Darwin {darwin} {arch}); build section not compared"
            );
            return;
        }
    }
    assert_eq!(
        section(&ours, "build"),
        section(&theirs, "build"),
        "build section differs from the reference php: update version.rs"
    );
}

#[test]
fn rhp_links_what_php_links() {
    let php = PathBuf::from(std::env::var("RHP_PHP").unwrap_or_else(|_| "php".into()));
    let rhp = Path::new(env!("CARGO_BIN_EXE_rhp"));
    let script = script();
    let script = script.to_str().unwrap();
    let ours = run(rhp, &["-n", script]).expect("rhp runs the capture script");
    assert!(!section(&ours, "linked").is_empty(), "{ours}");
    let Some(theirs) = run(&php, &["-n", script]) else {
        eprintln!("no php; skipping");
        return;
    };
    let minor = run(
        &php,
        &[
            "-n",
            "-r",
            "echo PHP_MAJOR_VERSION, '.', PHP_MINOR_VERSION;",
        ],
    );
    if minor.as_deref() != Some("8.5") {
        eprintln!("php is {minor:?}, not 8.5: no reference; skipping");
        return;
    }
    let (mut linked_ours, mut linked_theirs) =
        (section(&ours, "linked"), section(&theirs, "linked"));
    // A php without intl under -n (built without it, or a shared module that
    // -n leaves out) has no ICU version to compare.
    if linked_theirs.contains(&"INTL_ICU_VERSION = '(undefined)'") {
        eprintln!(
            "{} -n has no intl: ICU versions not compared",
            php.display()
        );
        linked_ours.retain(|l| !l.starts_with("INTL_"));
        linked_theirs.retain(|l| !l.starts_with("INTL_"));
    }
    // Likewise a php without openssl prints no OpenSSL row.
    let openssl = "phpinfo[OpenSSL Library Version]";
    if !linked_theirs.iter().any(|l| l.starts_with(openssl)) {
        eprintln!(
            "{} -n has no openssl: OpenSSL version not compared",
            php.display()
        );
        linked_ours.retain(|l| !l.starts_with(openssl));
    }
    assert_eq!(
        linked_ours,
        linked_theirs,
        "rhp links other library versions than {}: install the libraries it links \
         (docs/design/libraries.md) and rebuild",
        php.display()
    );
}

/// `rhp -v` / `rhp --cgi -v` print the reference build's banner; identical to
/// `php -n -v` / `php-cgi -n -v` when those are the reference release and
/// bottle (the banner carries the build date).
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
    // php-cgi is the same build as php (CI and Homebrew install them together).
    let same_bottle = bottle(&php).is_some_and(|b| bottle(rhp) == Some(b));
    for (bin, args, ours_args) in [
        (&php, &["-n", "-v"][..], &["-n", "-v"][..]),
        (&cgi, &["-n", "-v"], &["--cgi", "-n", "-v"]),
    ] {
        let Some(theirs) = run(bin, args) else {
            eprintln!("{} not runnable; skipping", bin.display());
            continue;
        };
        let provider = run(&php, &["-n", "-r", "echo PHP_BUILD_PROVIDER;"]).unwrap_or_default();
        if !theirs.starts_with(&format!("PHP {version} ")) || provider != "Homebrew" || !same_bottle
        {
            eprintln!("{} is not the reference build; skipping", bin.display());
            continue;
        }
        assert_eq!(run(rhp, ours_args).unwrap(), theirs, "{}", bin.display());
    }
}
