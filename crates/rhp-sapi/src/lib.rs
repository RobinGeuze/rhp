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

use std::io::Write;

use rhp_compiler::{compile_include, compile_with_source};
use rhp_frontend::Arena;
use rhp_values::{Thrown, Value};
use rhp_vm::{IncludeError, Vm};

pub mod cgi;

pub use rhp_builtins::sapi::{
    set_multipart_handler, MultipartHandler, RequestInfo, Response, Sapi,
};

/// What to run.
pub enum Script {
    /// A script file (its real path becomes `__FILE__`).
    File(String),
    /// Inline code (`-r`), run as "Command line code".
    Code(String),
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

/// The includer every SAPI installs: parse + compile an included file.
fn install_includer(vm: &mut Vm) {
    vm.set_includer(|file: &str, src: &[u8]| {
        let arena = Arena::new();
        let parsed = arena.parse(file.as_bytes(), src);
        if let Some((message, line)) = parsed.parse_error() {
            return Err(IncludeError::Parse { message, line });
        }
        Ok(compile_include(parsed.program(), src, file)?)
    });
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
    if let Some((message, line)) = parsed.parse_error() {
        vm.report_parse_error(&message, &file, line);
        vm.finish_request();
        return Outcome::Exit(255);
    }
    // Compile to owned bytecode; the arena can be dropped after this.
    let program = match compile_with_source(parsed.program(), &source, &file) {
        Ok(p) => p,
        Err(t) => {
            let code = fatal(t);
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
        Err(t) => fatal(t),
    };
    vm.finish_request();
    Outcome::Exit(code)
}

/// Report an uncaught throwable (or honour `exit`); returns the exit code.
fn fatal(t: Thrown) -> i32 {
    match t {
        Thrown::Exit(code) => code,
        Thrown::Error { class, message } => {
            eprintln!("PHP Fatal error:  Uncaught {class}: {message}");
            255
        }
        Thrown::Object(_) => {
            // Uncaught user exception (details rendered to stderr only).
            eprintln!("PHP Fatal error:  Uncaught exception");
            255
        }
        t @ Thrown::Unsupported { .. } => {
            eprintln!("{}", t.unsupported_message().unwrap_or_default());
            255
        }
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
        Outcome::CannotRead { path, error } => {
            eprintln!("rhp: cannot read {path}: {error}");
            1
        }
    }
}
