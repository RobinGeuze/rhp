//! The `rhp` command: argv parsing that picks a SAPI mode and hands the
//! request to the `rhp-sapi` library.
//!
//! - `rhp [options] script.php [args...]` / `rhp [options] -r code`: php-cli.
//!   The php CLI options composer and friends pass when re-invoking
//!   `PHP_BINARY` are accepted: `-n`, `-q`, `-H`, `-e`, `-c <path>`
//!   (ignored), `-d key[=value]` (ini settings), `-f <file>`, `-r <code>`,
//!   and `--` to end option parsing.
//! - `rhp --cgi [php-cgi options] [script.php [args...]]`, or the binary
//!   invoked as `php-cgi`: php-cgi (request from the CGI environment,
//!   `Status:` + headers + body on stdout).

use std::process::ExitCode;

use rhp_sapi::Script;

fn usage() -> ExitCode {
    eprintln!("usage: rhp [options] <script.php> [args...] | rhp [options] -r <code> [args...] | rhp --cgi [options] [script.php]");
    ExitCode::from(1)
}

fn exit(code: i32) -> ExitCode {
    ExitCode::from(code.clamp(0, 255) as u8)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    // --- SAPI selection -------------------------------------------------------
    let argv0_cgi = args
        .first()
        .and_then(|a| std::path::Path::new(a).file_name())
        .is_some_and(|n| n == "php-cgi");
    if argv0_cgi {
        return exit(rhp_sapi::cgi::main(
            &args,
            rhp_sapi::cgi::CgiEnv::from_process(),
        ));
    }
    if args.get(1).map(String::as_str) == Some("--cgi") {
        let mut cgi_args = vec![args[0].clone()];
        cgi_args.extend(args[2..].iter().cloned());
        return exit(rhp_sapi::cgi::main(
            &cgi_args,
            rhp_sapi::cgi::CgiEnv::from_process(),
        ));
    }

    // --- CLI option parsing (php_cli.c's getopt subset) -----------------------
    let mut i = 1;
    let mut script: Option<Script> = None;
    // `-d key[=value]` ini settings (`-n`: no php.ini, always the case here).
    let mut ini: Vec<(String, String)> = Vec::new();
    let mut add_ini = |kv: &str| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, "1"));
        ini.push((k.trim().to_string(), rhp_sapi::ini_define_value(v)));
    };
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--" => {
                i += 1;
                break;
            }
            "-n" | "-q" | "-H" | "-e" => i += 1,
            "-d" => {
                if let Some(kv) = args.get(i + 1) {
                    add_ini(kv);
                }
                i += 2;
            }
            "-c" => i += 2,
            "-f" => {
                let Some(f) = args.get(i + 1) else {
                    return usage();
                };
                script = Some(Script::File(f.clone()));
                i += 2;
                break;
            }
            "-r" => {
                let Some(c) = args.get(i + 1) else {
                    return usage();
                };
                script = Some(Script::Code(c.clone()));
                i += 2;
                break;
            }
            _ if a.starts_with("-d") => {
                add_ini(&a[2..]);
                i += 1;
            }
            _ if a.starts_with("-c") => i += 1,
            _ if a.starts_with("-r") && a.len() > 2 => {
                script = Some(Script::Code(a[2..].to_string()));
                i += 1;
                break;
            }
            _ if a.starts_with('-') && a.len() > 1 => {
                eprintln!("rhp: unsupported option {a}");
                return usage();
            }
            _ => break,
        }
    }
    let script = match script {
        Some(s) => {
            // `--` may still separate the script's own arguments.
            if args.get(i).map(String::as_str) == Some("--") {
                i += 1;
            }
            s
        }
        None => match args.get(i) {
            Some(p) => {
                i += 1;
                Script::File(p.clone())
            }
            None => return usage(),
        },
    };
    let rest = args[i..].iter().map(|s| s.as_bytes().to_vec()).collect();
    exit(rhp_sapi::run_cli(script, ini, rest))
}
