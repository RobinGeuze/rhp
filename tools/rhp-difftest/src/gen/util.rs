//! Helpers the generators share.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The workspace root (generated files are named relative to it).
pub(crate) fn repo_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().unwrap_or(root)
}

/// The value of `--name <value>`.
pub(crate) fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).map(String::as_str)
}

pub(crate) fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes (with `check`, compares) generator `name`'s files, each a path
/// relative to the repo root and its content. Exit code 2 when the files
/// could not be made.
pub(crate) fn finish(
    name: &str,
    check: bool,
    files: Result<Vec<(&str, String)>, String>,
) -> ExitCode {
    let files = match files {
        Ok(files) => files,
        Err(e) => {
            eprintln!("gen {name}: {e}");
            return ExitCode::from(2);
        }
    };
    let mut code = ExitCode::SUCCESS;
    for (rel, content) in files {
        let c = super::write_or_check(&repo_root().join(rel), &content, check, name);
        if c != ExitCode::SUCCESS {
            code = c;
        }
    }
    code
}
