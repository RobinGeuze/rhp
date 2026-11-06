//! The `rhp` command: picks the SAPI and hands the command line to
//! `rhp-sapi`.
//!
//! - `rhp [options] [-f] script.php [args...]`, `rhp [options] -r code`:
//!   php-cli ([`rhp_sapi::cli::main`] parses the options as php-cli does).
//! - `rhp --cgi [php-cgi options] [script.php [args...]]`, or the binary
//!   invoked as `php-cgi`: php-cgi (request from the CGI environment,
//!   `Status:` + headers + body on stdout).

use std::process::ExitCode;

#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

fn exit(code: i32) -> ExitCode {
    ExitCode::from(code.clamp(0, 255) as u8)
}

fn main() -> ExitCode {
    // Arguments need not be UTF-8: php's $argv holds their bytes.
    let raw: Vec<Vec<u8>> = std::env::args_os().map(os_bytes).collect();
    let args: Vec<String> = raw
        .iter()
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
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
    exit(rhp_sapi::cli::main_bytes(&raw))
}

#[cfg(unix)]
fn os_bytes(a: std::ffi::OsString) -> Vec<u8> {
    use std::os::unix::ffi::OsStringExt;
    a.into_vec()
}

#[cfg(not(unix))]
fn os_bytes(a: std::ffi::OsString) -> Vec<u8> {
    a.to_string_lossy().into_owned().into_bytes()
}
