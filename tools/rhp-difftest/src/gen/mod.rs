//! `rhp-difftest gen <name> [--check]`: generators for the tables rhp
//! derives from php-src. Each generator writes its committed files; with
//! `--check` it fails instead when a committed file differs.

use std::process::ExitCode;

pub mod curl_consts;
pub mod dumps;
pub mod html_tables;
pub mod intl_consts;
pub mod parser;
pub mod readme;
pub mod signatures;
pub mod sockets_consts;
mod util;
pub mod zip_consts;

const USAGE: &str = "usage: rhp-difftest gen <name> [options] [--check]\n\
generators:\n  curl-consts  ext/curl constant names and version gates from php-src\n\
\x20 dumps        ini, constant and function tables from the reference php\n\
\x20 html-tables  HTML entity tables from php-src's ext/standard/html_tables\n\
\x20 intl-consts  intl's global constants from php-src's ext/intl stubs\n\
\x20 parser       token ids and Bison tables from the php release tarball\n\
\x20 readme       README.md's PHP test table from tests/phpt (--php-src: recount totals)\n\
\x20 signatures   builtin signatures from php-src's *.stub.php\n\
\x20 sockets-consts  ext/sockets constant names and guards from php-src\n\
\x20 zip-consts   ZipArchive's constant names and guards from php-src\n";

pub fn main(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("curl-consts") => curl_consts::main(&args[1..]),
        Some("dumps") => dumps::main(&args[1..]),
        Some("html-tables") => html_tables::main(&args[1..]),
        Some("intl-consts") => intl_consts::main(&args[1..]),
        Some("parser") => parser::main(&args[1..]),
        Some("readme") => readme::main(&args[1..]),
        Some("signatures") => signatures::main(&args[1..]),
        Some("sockets-consts") => sockets_consts::main(&args[1..]),
        Some("zip-consts") => zip_consts::main(&args[1..]),
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
