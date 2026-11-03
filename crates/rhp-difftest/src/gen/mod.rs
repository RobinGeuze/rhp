//! `rhp-difftest gen <name> [--check] …`: regenerate a committed table from
//! its source. With `--check` nothing is written; the command fails when the
//! committed file differs from what it would write.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod dumps;
mod html_tables;
mod parser;

pub fn main(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("rhp-difftest gen: {e}");
            ExitCode::from(2)
        }
    }
}

/// Runs one generator: `Ok(false)` when `--check` found a stale file.
pub fn run(args: &[String]) -> Result<bool, String> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("dumps") => dumps::main(rest),
        Some("html-tables") => html_tables::main(rest),
        Some("parser") => parser::main(rest),
        _ => Err("usage: rhp-difftest gen <dumps|html-tables|parser> [--check] …".into()),
    }
}

/// The workspace root (generated files are named relative to it).
pub(crate) fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The value of `--name <value>`.
pub(crate) fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).map(String::as_str)
}

/// Write `content` to `rel` (relative to the repo root), or with `check`
/// compare it. Returns whether the file is (now) up to date.
pub(crate) fn emit(rel: &str, content: &str, check: bool) -> Result<bool, String> {
    let path = repo_root().join(rel);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    if old == content {
        println!("up to date: {rel}");
        return Ok(true);
    }
    if check {
        println!("OUT OF DATE: {rel} (run `rhp-difftest gen` without --check)");
        return Ok(false);
    }
    std::fs::write(&path, content).map_err(|e| format!("{rel}: {e}"))?;
    println!("wrote {rel}");
    Ok(true)
}

pub(crate) fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}
