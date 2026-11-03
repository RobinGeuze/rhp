//! `rhp-difftest gen <name> [--check]`: generators for the tables rhp
//! derives from php-src. Each generator writes one committed file; with
//! `--check` it fails instead when the committed file differs.

use std::process::ExitCode;

pub mod signatures;

const USAGE: &str = "usage: rhp-difftest gen <name> [options] [--check]\n\
generators:\n  signatures   builtin signatures from php-src's *.stub.php\n";

pub fn main(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("signatures") => signatures::main(&args[1..]),
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Write `content` to `path`, or with `check` compare it with the file
/// there. Returns the exit code.
pub fn write_or_check(path: &std::path::Path, content: &str, check: bool, name: &str) -> ExitCode {
    if check {
        let old = std::fs::read_to_string(path).unwrap_or_default();
        if old == content {
            eprintln!("gen {name}: {} is up to date", path.display());
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "gen {name}: {} differs from the generated output; run `rhp-difftest gen {name}`",
            path.display()
        );
        return ExitCode::FAILURE;
    }
    match std::fs::write(path, content) {
        Ok(()) => {
            eprintln!("gen {name}: wrote {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gen {name}: cannot write {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}
