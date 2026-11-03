//! rhp's request layer: the library a server (or the `rhp` command) calls
//! to run one PHP request on the current thread.
//!
//! The model follows php's SAPI layer (`main/SAPI.c`): the embedder supplies
//! a [`Sapi`] (name, ini defaults, body writer, header emission, flush,
//! logging) and, for a web request, a [`RequestInfo`] (server variables,
//! method, query string, cookies, body, request time); [`run_request`]
//! builds a fresh engine for the request, runs the script and returns its
//! [`Outcome`]. One engine per request, one request at a time per thread:
//! engine values are not `Send`, so a threaded server keeps a long-lived
//! worker thread per concurrent request and calls [`run_request`] on it.
//!
//! Provided SAPIs: [`CliSapi`] (php-cli: unbuffered stdout, no headers) and
//! [`cgi::CgiSapi`] (php-cgi: `Status:` + headers + body on stdout);
//! [`cgi::main`] is the `php-cgi` front end.
//!
//! The full contract (lifecycle, threading model, what is process-wide,
//! the road to `-S` / server embedding / FastCGI) is in
//! `docs/design/sapi.md`.

use std::io::Write;

pub use rhp_builtins::zend::ParseFailure;
use rhp_compiler::{compile_include, compile_with_source};
use rhp_frontend::{Arena, Parsed};
use rhp_values::{Thrown, Value};
use rhp_vm::{IncludeError, Vm};

pub mod cgi;

pub use rhp_builtins::sapi::{RequestInfo, Response, Sapi};
/// The `-v` banner (`PHP 8.5.x (cli) (built: …)`) for a SAPI name.
pub use rhp_builtins::version::banner as version_banner;

/// What to run.
pub enum Script {
    /// A script file (its real path becomes `__FILE__`).
    File(String),
    /// Inline code (`-r`), run as "Command line code".
    Code(String),
    /// No script file and no `-r`: the script is read from standard input.
    Stdin,
}

/// One request for [`run_request`].
pub struct Request {
    /// The primary script.
    pub script: Script,
    /// `-d name=value` settings, applied over the SAPI's ini defaults.
    pub ini: Vec<(String, String)>,
    /// php-cli's `$argv` after the script name (the script name is added).
    pub args: Vec<Vec<u8>>,
    /// The web request's input; `None` for a CLI run (superglobals from
    /// the process environment).
    pub info: Option<RequestInfo>,
    /// `PHP_BINARY` (composer re-invokes it); default: unchanged.
    pub php_binary: Option<Vec<u8>>,
}

/// How a request ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The script ran (or failed to compile); its exit status.
    Exit(i32),
    /// The script file could not be read.
    CannotRead {
        /// The path as given.
        path: String,
        /// The OS error text.
        error: String,
    },
}

/// php-cli: output straight to stdout as it is produced (so it interleaves
/// with stderr and child processes), no header store, status 0.
pub struct CliSapi;

impl Sapi for CliSapi {
    fn name(&self) -> &str {
        "cli"
    }

    fn ini_defaults(&self) -> Vec<(&'static str, &'static str)> {
        // sapi_cli_ini_defaults (the built-in table already holds the CLI's
        // HARDCODED_INI values).
        vec![("display_errors", "1")]
    }

    fn stores_headers(&self) -> bool {
        false
    }

    fn initial_response_code(&self) -> i64 {
        0
    }

    fn ub_write(&mut self, data: &[u8]) {
        let mut so = std::io::stdout().lock();
        let _ = so.write_all(data);
        let _ = so.flush();
    }

    fn flush(&mut self) {
        let _ = std::io::stdout().flush();
    }
}

/// The error php raises parsing `src`: its own grammar, scanner and grammar
/// action errors (`rhp_builtins::zend`). For code php parses but mago-syntax
/// does not (mago's bugs, or its recursion limit on deeply nested code),
/// mago's error, which is not php's.
pub fn parse_error(parsed: &Parsed, src: &[u8]) -> Option<ParseFailure> {
    rhp_builtins::zend::syntax_error(src).or_else(|| {
        let (message, line) = parsed.parse_error()?;
        Some(ParseFailure {
            class: "ParseError",
            message,
            line,
        })
    })
}

/// The includer every SAPI installs: parse + compile an included file.
fn install_includer(vm: &mut Vm) {
    vm.set_includer(|file: &str, src: &[u8]| {
        let arena = Arena::new();
        let parsed = arena.parse(file.as_bytes(), src);
        if let Some(f) = parse_error(&parsed, src) {
            return Err(IncludeError::Parse {
                class: f.class,
                message: f.message,
                line: f.line,
            });
        }
        compile_include(parsed.program(), src, file).map_err(include_error)
    });
}

/// A compile entry point's failure as the includer reports it: a compile
/// error (E_COMPILE_ERROR) is a located fatal, anything else is thrown.
pub fn include_error(t: Thrown) -> IncludeError {
    match t {
        Thrown::Error {
            class: "CompileError",
            message,
        } => IncludeError::Compile {
            message,
            line: rhp_compiler::compile_error_line(),
        },
        t => IncludeError::Thrown(t),
    }
}

/// Run one request on a fresh engine: compile the script, run it with the
/// request's superglobals, run the shutdown sequence, send the headers if
/// nothing did, and flush. Parse errors are displayed per `display_errors`
/// (exit 255); uncaught throwables are reported like php does.
pub fn run_request(sapi: Box<dyn Sapi>, req: Request) -> Outcome {
    let (file, source, argv0) = match &req.script {
        Script::File(path) => {
            let source = match std::fs::read(path) {
                Ok(bytes) => bytes,
                // php opens a directory fine and reads it as an empty script.
                Err(_) if std::path::Path::new(path).is_dir() => Vec::new(),
                Err(e) => {
                    return Outcome::CannotRead {
                        path: path.clone(),
                        error: e.to_string(),
                    }
                }
            };
            // php reports the resolved real path in __FILE__ / __DIR__.
            let file = std::fs::canonicalize(path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.clone());
            (file, source, path.clone())
        }
        Script::Stdin => {
            let mut src = Vec::new();
            if let Err(e) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut src) {
                return Outcome::CannotRead {
                    path: "standard input".to_string(),
                    error: e.to_string(),
                };
            }
            (
                "Standard input code".to_string(),
                src,
                "Standard input code".to_string(),
            )
        }
        Script::Code(code) => {
            let mut src = b"<?php ".to_vec();
            src.extend_from_slice(code.as_bytes());
            (
                "Command line code".to_string(),
                src,
                "Standard input code".to_string(),
            )
        }
    };

    let mut vm = Vm::new();
    vm.set_sapi(sapi);
    for (k, v) in &req.ini {
        vm.set_ini(k.as_bytes(), v.as_bytes());
    }
    if let Some(info) = req.info {
        vm.set_request(info);
    }
    vm.start_request();

    let arena = Arena::new();
    let parsed = arena.parse(file.as_bytes(), &source);
    if let Some(f) = parse_error(&parsed, &source) {
        // line 0: php raised it outside any file
        let at = if f.line == 0 {
            "[no active file]"
        } else {
            &file
        };
        vm.report_parse_error(f.class, &f.message, at, f.line);
        vm.finish_request();
        return Outcome::Exit(255);
    }
    // Compile to owned bytecode; the arena can be dropped after this.
    let program = match compile_with_source(parsed.program(), &source, &file) {
        Ok(p) => p,
        Err(t) => {
            let t = match include_error(t) {
                IncludeError::Compile { message, line } => {
                    vm.compile_error(&message, file.as_bytes(), line);
                    vm.finish_request();
                    return Outcome::Exit(255);
                }
                IncludeError::Thrown(t) => t,
                IncludeError::Parse { .. } => unreachable!("not built by include_error"),
            };
            let code = fatal(&mut vm, t);
            vm.finish_request();
            return Outcome::Exit(code);
        }
    };

    if let Some(bin) = req.php_binary {
        vm.set_constant("PHP_BINARY", Value::str(bin));
    }
    if let Script::File(_) = req.script {
        // The top-level script's path, so `Phar::mapPhar` can parse it.
        vm.set_main_script_path(file.as_bytes().to_vec());
    }
    // $argv / $_SERVER['argv'] (CLI): the script name as given, then its
    // arguments.
    let mut argv = vec![argv0.into_bytes()];
    argv.extend(req.args);
    vm.set_argv(argv);
    install_includer(&mut vm);
    let code = match vm.run_main(&program) {
        Ok(code) => code,
        Err(t) => fatal(&mut vm, t),
    };
    vm.finish_request();
    Outcome::Exit(code)
}

/// Report an uncaught throwable (or honour `exit`); returns the exit code.
/// The report goes to the SAPI's error stream, not the process's stderr.
fn fatal(vm: &mut Vm, t: Thrown) -> i32 {
    let msg = match t {
        Thrown::Exit(code) => return code,
        Thrown::Error { class, message } => {
            format!("PHP Fatal error:  Uncaught {class}: {message}")
        }
        // Uncaught user exception (details rendered to stderr only).
        Thrown::Object(_) => "PHP Fatal error:  Uncaught exception".to_string(),
        t @ Thrown::Unsupported { .. } => t.unsupported_message().unwrap_or_default(),
    };
    vm.write_error_stream(format!("{msg}\n").as_bytes());
    255
}

/// The value php gives a `-d key=value` setting (php_ini_builder_define,
/// shared by php-cli and php-cgi): one starting with a character other than
/// an alphanumeric or a quote is taken literally; otherwise it goes through
/// the ini scanner, which unquotes a quoted string, stops a bare value at a
/// `;` comment, and maps the bareword booleans (`On`/`Yes`/`True` → "1",
/// `Off`/`No`/`False`/`None`/`Null` → ""). Constant expressions
/// (`E_ALL & ~E_NOTICE`) are not evaluated.
pub fn ini_define_value(raw: &str) -> String {
    let v = raw.trim();
    let Some(first) = v.chars().next() else {
        return String::new();
    };
    if first == '"' || first == '\'' {
        let inner = &v[1..];
        return match inner.find(first) {
            Some(end) => inner[..end].to_string(),
            None => inner.to_string(),
        };
    }
    if !first.is_ascii_alphanumeric() {
        return v.to_string();
    }
    let v = v.split(';').next().unwrap_or("").trim_end();
    match v.to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" => "1".to_string(),
        "off" | "no" | "false" | "none" | "null" => String::new(),
        _ => v.to_string(),
    }
}

/// The CLI front end's run: `rhp [options] script.php args...` semantics.
pub fn run_cli(script: Script, ini: Vec<(String, String)>, args: Vec<Vec<u8>>) -> i32 {
    // PHP_BINARY is this engine's own executable (composer re-invokes it for
    // `@php` scripts).
    let php_binary = std::env::current_exe().ok().map(|exe| {
        std::fs::canonicalize(&exe)
            .unwrap_or(exe)
            .to_string_lossy()
            .into_owned()
            .into_bytes()
    });
    let req = Request {
        script,
        ini,
        args,
        info: None,
        php_binary,
    };
    match run_request(Box::new(CliSapi), req) {
        Outcome::Exit(code) => code,
        Outcome::CannotRead { path, .. } => {
            eprintln!("Could not open input file: {path}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ini_define_value as v;

    #[test]
    fn ini_define_values_match_php() {
        assert_eq!(v("On"), "1");
        assert_eq!(v("on "), "1");
        assert_eq!(v("TRUE"), "1");
        assert_eq!(v("true;x"), "1");
        assert_eq!(v("off"), "");
        assert_eq!(v("NULL"), "");
        assert_eq!(v("none"), "");
        assert_eq!(v("\"On\""), "On");
        assert_eq!(v("'On'"), "On");
        assert_eq!(v(""), "");
        assert_eq!(v("-1"), "-1");
        assert_eq!(v("128M"), "128M");
        assert_eq!(v("/a;b"), "/a;b");
    }
}
