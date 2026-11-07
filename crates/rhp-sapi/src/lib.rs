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
//! Provided SAPIs: [`CliSapi`] (php-cli: unbuffered stdout, no headers),
//! [`cgi::CgiSapi`] (php-cgi: `Status:` + headers + body on stdout) and
//! [`CaptureSapi`] (everything kept in memory, for tests and tools);
//! [`cgi::main`] is the `php-cgi` front end.
//!
//! The full contract (lifecycle, threading model, what is process-wide,
//! the road to `-S` / server embedding / FastCGI) is in
//! `docs/design/sapi.md`.

use std::io::Write;

use rhp_bytecode::Program;
use rhp_compiler::parse::Arena;
use rhp_values::{Thrown, Value};
use rhp_vm::Vm;

mod capture;
pub mod cgi;
pub mod cli;
mod getopt;

pub use capture::{CaptureSapi, Captured};

pub use rhp_compiler::Mode;
pub use rhp_vm::IncludeError;

pub use rhp_builtins::main::sapi::{RequestInfo, Response, Sapi};
/// The `-v` banner (`PHP 8.5.x (cli) (built: …)`) for a SAPI name.
pub use rhp_builtins::version::banner as version_banner;
/// The global allocator a binary that runs requests must install, for
/// `memory_get_usage()` and `memory_limit`:
///
/// ```ignore
/// #[global_allocator]
/// static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;
/// ```
///
/// It is the system allocator plus per-thread accounting of the request
/// running on that thread (see `docs/design/sapi.md`, *Memory*).
pub use rhp_values::mem::CountingAlloc;

/// What to run.
pub enum Script {
    /// A script file (its real path becomes `__FILE__`).
    File(String),
    /// A script the caller already holds, run as the file `path` (its
    /// `__FILE__` and `$argv[0]`, as given); not cached.
    Source {
        /// The script's path.
        path: String,
        /// Its source.
        bytes: Vec<u8>,
    },
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
    /// The directory the script runs in (`getcwd()`, relative paths, child
    /// processes): php-cgi's script directory, an embedder's document root.
    /// `None`: the process's. A relative script path is still opened from
    /// the process's directory, as php-cgi opens the script before its
    /// chdir. The process's own directory never changes.
    pub cwd: Option<Vec<u8>>,
    /// Which ini files to load (php_init_config); `None`: none at all.
    pub ini_files: Option<IniFiles>,
}

/// php_init_config's inputs: php-cli's `-c` and `-n`. The php.ini is the
/// `-c` file (or `php-<sapi>.ini` / `php.ini` in that directory), else
/// `$PHPRC`'s; then, unless `-n`, the `.ini` files in `$PHP_INI_SCAN_DIR`.
/// rhp has no built-in search path or scan directory of its own.
#[derive(Clone, Debug, Default)]
pub struct IniFiles {
    /// `-c <path>`.
    pub path_override: Option<String>,
    /// `-n`.
    pub ignore: bool,
}

impl Request {
    /// A CLI-style request for `script`: no settings, arguments or request
    /// input.
    pub fn new(script: Script) -> Request {
        Request {
            script,
            ini: Vec::new(),
            args: Vec::new(),
            info: None,
            php_binary: None,
            cwd: None,
            ini_files: None,
        }
    }
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

    fn exits_on_hard_limit(&self) -> bool {
        true
    }
}

/// rhp-builtins as the compiler sees it.
static BUILTINS: rhp_compiler::Builtins = rhp_compiler::Builtins {
    prelude: rhp_builtins::prelude,
    native_function: rhp_builtins::native_function,
    constant: rhp_builtins::ct_constant,
    cwd: rhp_builtins::zend::virtual_cwd::getcwd,
    assertions: rhp_builtins::zend::assertions,
};

/// Parse and compile the source `src` of `file` as rhp runs a file: php's
/// own grammar decides what parses (see [`parse_error`]), then the
/// compiler. Every script, include, eval and preload goes through here, and
/// `rhp-difftest lint` too.
pub fn compile_file(file: &str, src: &[u8], mode: Mode) -> Result<Program, IncludeError> {
    // zend.multibyte: the scanner reads the script converted.
    let filtered = rhp_builtins::zend::multibyte::filter(src);
    let src = filtered
        .as_ref()
        .and_then(|f| f.source.as_deref())
        .unwrap_or(src);
    let mut r = compile_source(file, src, mode);
    if let (Ok(p), Some(out)) = (&mut r, filtered.and_then(|f| f.output)) {
        output_filter(p, &out);
    }
    r
}

/// The scanner's output filter (zend.multibyte) over a compiled program:
/// its string constants converted to the internal encoding.
fn output_filter(p: &mut Program, f: &rhp_builtins::zend::multibyte::OutputFilter) {
    let proto = |rc: &mut std::rc::Rc<rhp_bytecode::FunctionProto>| {
        if let Some(proto) = std::rc::Rc::get_mut(rc) {
            for c in &mut proto.consts {
                if let Value::Str(s) = c {
                    *c = Value::str(f.apply(s.as_bytes()));
                }
            }
        }
    };
    proto(&mut p.main);
    p.functions.iter_mut().for_each(proto);
    p.closures.iter_mut().for_each(proto);
    for class in p.classes.iter_mut().chain(p.cond_classes.iter_mut()) {
        if let Some(class) = std::rc::Rc::get_mut(class) {
            for m in &mut class.methods {
                proto(&mut m.proto);
            }
        }
    }
}

/// [`compile_source_checked`] as php compiles: past the hard memory limit
/// it stops ([`IncludeError::HardLimit`]).
fn compile_source(file: &str, src: &[u8], mode: Mode) -> Result<Program, IncludeError> {
    rhp_values::mem::compiling(|| compile_source_checked(file, src, mode))
}

/// Went past the hard memory limit while scanning or parsing: php stops
/// in the scanner, somewhere in the file; rhp parses it whole first and
/// names its last line.
fn parse_hard_limit(src: &[u8]) -> Result<(), IncludeError> {
    if rhp_values::mem::hard_oom_pending() {
        let line = 1 + src.iter().filter(|&&c| c == b'\n').count() as u32;
        return Err(IncludeError::HardLimit { line });
    }
    Ok(())
}

fn compile_source_checked(file: &str, src: &[u8], mode: Mode) -> Result<Program, IncludeError> {
    let short_tags = rhp_builtins::short_open_tag();
    // php's own grammar decides what parses (its scanner, grammar and
    // grammar action errors), with the warnings its parser raised. It runs
    // before the parse arena exists, so the two never add up in the peak.
    let php_failure = rhp_builtins::zend::syntax_error_ex(src, short_tags);
    let parse_diags: Vec<_> = rhp_builtins::zend::take_parse_warnings()
        .into_iter()
        .map(|(m, l)| (rhp_builtins::E_COMPILE_WARNING, m, l))
        .collect();
    parse_hard_limit(src)?;
    if let Some(f) = php_failure {
        return Err(IncludeError::Parse {
            class: f.class,
            message: f.message,
            line: f.line,
            diags: parse_diags,
        });
    }
    let arena = Arena::new();
    let parsed = arena.parse_with_short_tags(file.as_bytes(), src, short_tags);
    parse_hard_limit(src)?;
    if parsed.has_parse_errors() {
        // Code php's grammar accepts but mago does not, that php then
        // refuses to compile.
        if let Some((message, line)) = rhp_compiler::parse::mago_gap_error(src) {
            return Err(IncludeError::Compile {
                message,
                line,
                diags: parse_diags,
            });
        }
        // Code php parses but mago-syntax does not (mago's bugs, or its
        // recursion limit on deeply nested code): mago's error, which is
        // not php's.
        if let Some((message, line)) = parsed.parse_error() {
            return Err(IncludeError::Parse {
                class: "ParseError",
                message,
                line,
                diags: parse_diags,
            });
        }
    }
    let r = rhp_compiler::compile(&BUILTINS, parsed.program(), src, file, mode);
    if rhp_values::mem::hard_oom_pending() {
        // The compiler stopped at a line, or went past the limit finishing.
        let line = match r {
            Err(Thrown::Exit(_)) => rhp_compiler::compile_error_line(),
            _ => 1 + src.iter().filter(|&&c| c == b'\n').count() as u32,
        };
        return Err(IncludeError::HardLimit { line });
    }
    if parse_diags.is_empty() {
        return r.map_err(include_error);
    }
    // The parser's warnings come before the compiler's diagnostics.
    match r.map_err(include_error) {
        Ok(mut p) => {
            p.compile_diags.splice(0..0, parse_diags);
            Ok(p)
        }
        Err(IncludeError::Compile {
            message,
            line,
            mut diags,
        }) => {
            diags.splice(0..0, parse_diags);
            Err(IncludeError::Compile {
                message,
                line,
                diags,
            })
        }
        Err(e) => Err(e),
    }
}

/// The includer every SAPI installs: compile an included or eval'd file.
fn install_includer(vm: &mut Vm) {
    vm.set_includer(|file: &str, src: &[u8]| compile_file(file, src, Mode::Include));
}

/// A compile entry point's failure as the includer reports it: a compile
/// error (E_COMPILE_ERROR) is a located fatal, anything else is thrown.
fn include_error(t: Thrown) -> IncludeError {
    match t {
        Thrown::Error {
            class: "CompileError",
            message,
        } => IncludeError::Compile {
            message,
            line: rhp_compiler::compile_error_line(),
            diags: rhp_compiler::take_compile_error_diags(),
        },
        t => IncludeError::Thrown(t),
    }
}

/// Run one request on a fresh engine: compile the script, run it with the
/// request's superglobals, run the shutdown sequence, send the headers if
/// nothing did, and flush. Parse errors are displayed per `display_errors`
/// (exit 255); uncaught throwables are reported like php does.
pub fn run_request(sapi: Box<dyn Sapi>, req: Request) -> Outcome {
    // php refuses to start when preloading fails.
    if !preload(&*sapi, &req.ini) {
        return Outcome::Exit(1);
    }
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
        Script::Source { path, bytes } => (path.clone(), bytes.clone(), path.clone()),
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

    let mut vm = Vm::new(sapi);
    if let Some(dir) = &req.cwd {
        vm.set_cwd(dir);
    }
    if let Some(files) = &req.ini_files {
        load_ini_files(&mut vm, files);
    }
    for (k, v) in &req.ini {
        vm.set_ini(k.as_bytes(), v.as_bytes());
    }
    if let Some(info) = req.info {
        vm.set_request(info);
    }
    if let Script::File(_) = req.script {
        // php-cli resolves the script (VCWD_REALPATH) before the request
        // starts; under open_basedir module startup already turned the
        // cache off.
        let _ = rhp_builtins::zend::virtual_cwd::realpath(file.as_bytes());
    }
    vm.start_request();
    if vm.startup_failed() {
        vm.finish_request();
        return Outcome::Exit(1);
    }

    // Compile to owned bytecode (the arena is dropped after this). A script
    // file goes through the compiled-script cache; `-r` code and standard
    // input never do, as in php.
    let compile = |source: &[u8]| compile_file(&file, source, Mode::Main);
    let compiled = match req.script {
        Script::File(_) => vm.compile_main(&file, &argv0, source, compile),
        _ => compile(&source).map(std::rc::Rc::new),
    };
    let program = match compiled {
        Ok(p) => p,
        Err(IncludeError::Parse {
            class,
            message,
            line,
            diags,
        }) => {
            vm.emit_parse_diags(file.as_bytes(), &diags);
            // line 0: php raised it outside any file
            let at = if line == 0 { "[no active file]" } else { &file };
            vm.report_parse_error(class, &message, at, line);
            vm.finish_request();
            return Outcome::Exit(255);
        }
        Err(IncludeError::Compile {
            message,
            line,
            diags,
        }) => {
            vm.compile_failed(&message, file.as_bytes(), line, &diags, false);
            vm.finish_request();
            return Outcome::Exit(255);
        }
        Err(IncludeError::HardLimit { line }) => {
            let t = vm.compile_hard_limit(Some((file.as_bytes(), line)));
            let code = fatal(&mut vm, t);
            vm.finish_request();
            return Outcome::Exit(code);
        }
        Err(IncludeError::Thrown(t)) => {
            let code = fatal(&mut vm, t);
            vm.finish_request();
            return Outcome::Exit(code);
        }
    };

    if let Some(bin) = req.php_binary {
        vm.set_constant("PHP_BINARY", Value::str(bin));
    }
    if let Script::File(_) | Script::Source { .. } = req.script {
        // The top-level script's path, so `Phar::mapPhar` can parse it.
        vm.set_main_script_path(file.as_bytes().to_vec());
    }
    // $argv / $_SERVER['argv'] (CLI): the script name as given, then its
    // arguments.
    let mut argv = vec![argv0.into_bytes()];
    argv.extend(req.args);
    vm.set_argv(argv);
    if let Script::Code(_) = req.script {
        vm.disable_auto_prepend();
    }
    install_includer(&mut vm);
    let code = match vm.run_main(&program) {
        Ok(code) => code,
        Err(t) => fatal(&mut vm, t),
    };
    vm.finish_request();
    Outcome::Exit(code)
}

/// php_init_config: the php.ini, then the scanned `.ini` files, into the
/// configuration hash (the `-d` settings come after).
fn load_ini_files(vm: &mut Vm, files: &IniFiles) {
    use std::path::{Path, PathBuf};
    let sapi_name = rhp_builtins::main::sapi::sapi_name();
    let (file_name, search) = match &files.path_override {
        Some(p) => (Some(p.clone()), Some(p.clone())),
        None if !files.ignore => match std::env::var("PHPRC") {
            Ok(p) if !p.is_empty() => (Some(p.clone()), Some(p)),
            _ => (None, None),
        },
        None => (None, None),
    };
    let mut opened: Option<(PathBuf, Vec<u8>)> = None;
    if let Some(f) = file_name.filter(|f| Path::new(f).is_file()) {
        if let Ok(src) = std::fs::read(&f) {
            let abs = std::path::absolute(&f).unwrap_or_else(|_| PathBuf::from(&f));
            opened = Some((abs, src));
        }
    }
    if opened.is_none() {
        let dirs: Vec<String> = search
            .map(|s| s.split(':').map(str::to_string).collect())
            .unwrap_or_default();
        'search: for name in [format!("php-{sapi_name}.ini"), "php.ini".to_string()] {
            for d in dirs.iter().filter(|d| !d.is_empty()) {
                let p = Path::new(d).join(&name);
                if p.is_file() {
                    if let Ok(src) = std::fs::read(&p) {
                        opened = Some((p, src));
                        break 'search;
                    }
                }
            }
        }
    }
    if let Some((path, src)) = opened {
        vm.load_ini_file(&src);
        use std::os::unix::ffi::OsStrExt;
        rhp_builtins::config_set_opened_path(path.as_os_str().as_bytes());
    }
    if files.ignore {
        return;
    }
    let Ok(scan) = std::env::var("PHP_INI_SCAN_DIR") else {
        return;
    };
    let mut scanned = Vec::new();
    for dir in scan.split(':').filter(|d| !d.is_empty()) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut names: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
        names.sort();
        for name in names {
            use std::os::unix::ffi::OsStrExt;
            if !name.as_bytes().ends_with(b".ini") {
                continue;
            }
            let path = if dir.ends_with('/') {
                format!("{dir}{}", name.to_string_lossy())
            } else {
                format!("{dir}/{}", name.to_string_lossy())
            };
            if !Path::new(&path).is_file() {
                continue;
            }
            if let Ok(src) = std::fs::read(&path) {
                if vm.load_ini_file(&src) {
                    scanned.push(path.into_bytes());
                }
            }
        }
    }
    rhp_builtins::config_set_scanned_files(&scanned);
}

/// The SAPI `opcache.preload` runs under: the request's SAPI name and ini
/// defaults, output straight to the process's stdout, no headers (php's
/// `preload_ub_write` and friends).
struct PreloadSapi {
    name: String,
    defaults: Vec<(&'static str, &'static str)>,
    exits_on_hard_limit: bool,
}

impl Sapi for PreloadSapi {
    fn name(&self) -> &str {
        &self.name
    }

    fn ini_defaults(&self) -> Vec<(&'static str, &'static str)> {
        self.defaults.clone()
    }

    fn stores_headers(&self) -> bool {
        false
    }

    fn ub_write(&mut self, data: &[u8]) {
        let mut so = std::io::stdout().lock();
        let _ = so.write_all(data);
        let _ = so.flush();
    }

    fn flush(&mut self) {
        let _ = std::io::stdout().flush();
    }

    fn exits_on_hard_limit(&self) -> bool {
        self.exits_on_hard_limit
    }
}

/// Run `opcache.preload` once per thread, before the first request that
/// has opcache on (php runs it at startup). Its functions and classes are
/// then declared in every request on this thread. False: preloading failed,
/// now or in an earlier request on this thread (php doesn't start).
fn preload(sapi: &dyn Sapi, ini: &[(String, String)]) -> bool {
    let set = |k: &str, v: &str| k == "opcache.preload" && !v.is_empty();
    if !ini.iter().any(|(k, v)| set(k, v)) && !sapi.ini_defaults().iter().any(|(k, v)| set(k, v)) {
        return true;
    }
    if let Some(ok) = Vm::preload_outcome() {
        return ok;
    }
    let mut vm = Vm::new(Box::new(PreloadSapi {
        name: sapi.name().to_string(),
        defaults: sapi.ini_defaults(),
        exits_on_hard_limit: sapi.exits_on_hard_limit(),
    }));
    for (k, v) in ini {
        vm.set_ini(k.as_bytes(), v.as_bytes());
    }
    let Some(path) = vm.preload_pending() else {
        return true;
    };
    let file = std::fs::canonicalize(&path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.clone());
    install_includer(&mut vm);
    let ok = vm.run_preload(&path, |source| compile_file(&file, source, Mode::Main));
    vm.finish_request();
    ok
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

/// A `-d key[=value]` setting (php_ini_builder_define, shared by php-cli
/// and php-cgi): `key` alone is `key=1`.
pub(crate) fn ini_define(kv: &str) -> (String, String) {
    let (k, v) = kv.split_once('=').unwrap_or((kv, "1"));
    (k.trim().to_string(), ini_define_value(v))
}

/// The value php gives a `-d key=value` setting (php_ini_builder_define,
/// shared by php-cli and php-cgi): one starting with a character other than
/// an alphanumeric or a quote is taken literally; otherwise it goes through
/// the ini scanner, which unquotes a quoted string, stops a bare value at a
/// `;` comment, maps the bareword booleans (`On`/`Yes`/`True` → "1",
/// `Off`/`No`/`False`/`None`/`Null` → ""), replaces a constant's name by
/// its value and evaluates `| & ^ ~ !` expressions (`E_ALL & ~E_NOTICE`).
pub fn ini_define_value(raw: &str) -> String {
    let Some(first) = raw.chars().next() else {
        return String::new();
    };
    if !first.is_ascii_alphanumeric() && first != '"' && first != '\'' {
        // Quoted by php_ini_builder_define, leading whitespace included.
        return raw.to_string();
    }
    let v = raw.trim_end();
    if first == '\'' {
        let inner = &v[1..];
        return match inner.find(first) {
            Some(end) => inner[..end].to_string(),
            None => inner.to_string(),
        };
    }
    if first == '"' {
        // Up to the first unescaped quote, then zend_ini_escape_string:
        // `\"`, `\\` and `\$` lose their backslash.
        let b = &v.as_bytes()[1..];
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() && b[i] != b'"' {
            if b[i] == b'\\' && i + 1 < b.len() {
                if !matches!(b[i + 1], b'"' | b'\\' | b'$') {
                    out.push(b'\\');
                }
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        return String::from_utf8_lossy(&out).into_owned();
    }
    let v = v.split(';').next().unwrap_or("").trim_end();
    match v.to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" => "1".to_string(),
        "off" | "no" | "false" | "none" | "null" => String::new(),
        _ => ini_expr(v),
    }
}

/// The ini parser's value grammar (zend_ini_parser.y): runs of value
/// characters, a constant name standing for its value, and the integer
/// operators `| & ^` (one precedence, left to right) and prefix `~ !`,
/// computed as C `int`s.
fn ini_expr(v: &str) -> String {
    const OPS: &[u8] = b"&|^~()!";
    let b = v.as_bytes();
    // Operands are the text between operators, constants substituted.
    let operand = |s: &str| -> String {
        let mut out = String::new();
        let mut word = String::new();
        let flush = |w: &mut String, out: &mut String| {
            let ident = w
                .bytes()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
                && w.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
            match ident.then(|| ini_constant(w)).flatten() {
                Some(c) => out.push_str(&c),
                None => out.push_str(w),
            }
            w.clear();
        };
        for ch in s.chars() {
            if ch == ' ' || ch == '\t' {
                flush(&mut word, &mut out);
                out.push(ch);
            } else {
                word.push(ch);
            }
        }
        flush(&mut word, &mut out);
        out
    };
    if !b.iter().any(|c| OPS.contains(c)) {
        return operand(v);
    }
    // Recursive descent over the token stream.
    enum Tok {
        Op(u8),
        Val(String),
    }
    let mut toks = Vec::new();
    let mut start = 0;
    for (i, c) in b.iter().enumerate() {
        if OPS.contains(c) {
            let text = v[start..i].trim();
            if !text.is_empty() {
                toks.push(Tok::Val(operand(text)));
            }
            toks.push(Tok::Op(*c));
            start = i + 1;
        }
    }
    let text = v[start..].trim();
    if !text.is_empty() {
        toks.push(Tok::Val(operand(text)));
    }
    fn unary(t: &[Tok], i: &mut usize) -> i32 {
        match t.get(*i) {
            Some(Tok::Op(b'~')) => {
                *i += 1;
                !unary(t, i)
            }
            Some(Tok::Op(b'!')) => {
                *i += 1;
                (unary(t, i) == 0) as i32
            }
            Some(Tok::Op(b'(')) => {
                *i += 1;
                let v = binary(t, i);
                if matches!(t.get(*i), Some(Tok::Op(b')'))) {
                    *i += 1;
                }
                v
            }
            Some(Tok::Val(s)) => {
                *i += 1;
                rhp_builtins::c_atoi(s.as_bytes())
            }
            _ => 0,
        }
    }
    fn binary(t: &[Tok], i: &mut usize) -> i32 {
        let mut acc = unary(t, i);
        while let Some(Tok::Op(op @ (b'|' | b'&' | b'^'))) = t.get(*i) {
            *i += 1;
            let rhs = unary(t, i);
            acc = match op {
                b'|' => acc | rhs,
                b'&' => acc & rhs,
                _ => acc ^ rhs,
            };
        }
        acc
    }
    binary(&toks, &mut 0).to_string()
}

/// The constants that exist when php parses `-d` settings (php_init_config):
/// Zend's standard constants and main's. Extensions register theirs later,
/// so `SORT_DESC` or `M_PI` stay literal.
const INI_CONSTANTS: &[&str] = &[
    // Zend/zend_constants.stub.php
    "E_ERROR",
    "E_WARNING",
    "E_PARSE",
    "E_NOTICE",
    "E_CORE_ERROR",
    "E_CORE_WARNING",
    "E_COMPILE_ERROR",
    "E_COMPILE_WARNING",
    "E_USER_ERROR",
    "E_USER_WARNING",
    "E_USER_NOTICE",
    "E_STRICT",
    "E_RECOVERABLE_ERROR",
    "E_DEPRECATED",
    "E_USER_DEPRECATED",
    "E_ALL",
    "DEBUG_BACKTRACE_PROVIDE_OBJECT",
    "DEBUG_BACKTRACE_IGNORE_ARGS",
    "ZEND_THREAD_SAFE",
    "ZEND_DEBUG_BUILD",
    "ZEND_VM_KIND",
    // main/main.stub.php
    "PHP_VERSION",
    "PHP_MAJOR_VERSION",
    "PHP_MINOR_VERSION",
    "PHP_RELEASE_VERSION",
    "PHP_EXTRA_VERSION",
    "PHP_VERSION_ID",
    "PHP_BUILD_DATE",
    "PHP_BUILD_PROVIDER",
    "PHP_ZTS",
    "PHP_DEBUG",
    "PHP_OS",
    "PHP_OS_FAMILY",
    "DEFAULT_INCLUDE_PATH",
    "PEAR_INSTALL_DIR",
    "PEAR_EXTENSION_DIR",
    "PHP_EXTENSION_DIR",
    "PHP_PREFIX",
    "PHP_BINDIR",
    "PHP_SBINDIR",
    "PHP_MANDIR",
    "PHP_LIBDIR",
    "PHP_DATADIR",
    "PHP_SYSCONFDIR",
    "PHP_LOCALSTATEDIR",
    "PHP_CONFIG_FILE_PATH",
    "PHP_CONFIG_FILE_SCAN_DIR",
    "PHP_SHLIB_SUFFIX",
    "PHP_EOL",
    "PHP_MAXPATHLEN",
    "PHP_INT_MAX",
    "PHP_INT_MIN",
    "PHP_INT_SIZE",
    "PHP_FD_SETSIZE",
    "PHP_FLOAT_DIG",
    "PHP_FLOAT_EPSILON",
    "PHP_FLOAT_MAX",
    "PHP_FLOAT_MIN",
    "PHP_OUTPUT_HANDLER_START",
    "PHP_OUTPUT_HANDLER_WRITE",
    "PHP_OUTPUT_HANDLER_FLUSH",
    "PHP_OUTPUT_HANDLER_CLEAN",
    "PHP_OUTPUT_HANDLER_FINAL",
    "PHP_OUTPUT_HANDLER_CONT",
    "PHP_OUTPUT_HANDLER_END",
    "PHP_OUTPUT_HANDLER_CLEANABLE",
    "PHP_OUTPUT_HANDLER_FLUSHABLE",
    "PHP_OUTPUT_HANDLER_REMOVABLE",
    "PHP_OUTPUT_HANDLER_STDFLAGS",
    "PHP_OUTPUT_HANDLER_STARTED",
    "PHP_OUTPUT_HANDLER_DISABLED",
    "PHP_OUTPUT_HANDLER_PROCESSED",
    "UPLOAD_ERR_OK",
    "UPLOAD_ERR_INI_SIZE",
    "UPLOAD_ERR_FORM_SIZE",
    "UPLOAD_ERR_PARTIAL",
    "UPLOAD_ERR_NO_FILE",
    "UPLOAD_ERR_NO_TMP_DIR",
    "UPLOAD_ERR_CANT_WRITE",
    "UPLOAD_ERR_EXTENSION",
    "PHP_SAPI",
    "PHP_BINARY",
];

/// A constant's value as the ini scanner substitutes it.
fn ini_constant(name: &str) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static CONSTANTS: OnceLock<HashMap<&'static str, Vec<u8>>> = OnceLock::new();
    let m = CONSTANTS.get_or_init(|| {
        rhp_builtins::constants()
            .into_iter()
            .filter(|(n, _)| INI_CONSTANTS.contains(n))
            .filter(|(_, v)| !matches!(v, Value::Array(_) | Value::Object(_)))
            .map(|(n, v)| (n, rhp_values::conv::to_string_bytes(&v)))
            .collect()
    });
    m.get(name).map(|v| String::from_utf8_lossy(v).into_owned())
}

/// The CLI front end's run: `rhp [options] script.php args...` semantics.
pub fn run_cli(
    script: Script,
    ini: Vec<(String, String)>,
    args: Vec<Vec<u8>>,
    ini_files: IniFiles,
) -> i32 {
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
        cwd: None,
        ini_files: Some(ini_files),
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
mod tests;
