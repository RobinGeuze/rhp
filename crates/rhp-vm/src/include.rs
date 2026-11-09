//! `include` / `require` / `eval`: resolving, compiling and installing a file's
//! program.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_builtins::{self as builtins, Host};
use rhp_bytecode::{FunctionProto, IncludeKind, Program, Reg};
use rhp_values::{Thrown, Value};

use crate::errors::UnwindResult;
use crate::{ini_bool, opcache, Frame, UnitBase, UnitFn, UnitId, Vm};

/// SAPI-injected callback that parses + compiles an included file's source into
/// a `Program`, keeping the VM compiler-agnostic (see the `includer` field).
pub(crate) type Includer = Box<dyn FnMut(&str, &[u8]) -> Result<Program, IncludeError>>;

/// An included file's source: a stream wrapper's contents, or an open
/// local file (read only when the compiled-script cache misses).
pub(crate) enum IncludeSource {
    Bytes(Vec<u8>),
    /// Read from a stream resource that already took its id.
    Streamed(Vec<u8>),
    File(std::fs::File),
    /// Not on disk, but a cached script that needn't be revalidated.
    Cached,
}

/// Why the SAPI's includer could not produce a `Program`.
#[derive(Debug)]
pub enum IncludeError {
    /// The parser refused the file: a catchable `class` (`ParseError`, or
    /// `CompileError` from a grammar action) at (file, `line`).
    Parse {
        class: &'static str,
        message: String,
        line: u32,
        /// The diagnostics the parser raised before it (level, message,
        /// line).
        diags: Vec<(i64, String, u32)>,
    },
    /// A compile error (E_COMPILE_ERROR) at `line`: a fatal error, not an
    /// exception — the request bails out (shutdown functions still run).
    /// `diags` are the diagnostics the compiler raised before it (level,
    /// message, line).
    Compile {
        message: String,
        line: u32,
        diags: Vec<(i64, String, u32)>,
    },
    /// Compiling went past php's hard memory limit at `line`: php exits
    /// there ([`Vm::compile_hard_limit`]), naming the line only when no
    /// code runs yet (the main script, preloading).
    HardLimit { line: u32 },
    /// Anything else (an unsupported construct, …).
    Thrown(Thrown),
}

impl From<Thrown> for IncludeError {
    fn from(t: Thrown) -> Self {
        IncludeError::Thrown(t)
    }
}

impl Vm {
    /// The diagnostics php's compiler raised for a file just loaded: once
    /// per compile, or on every load from opcache with
    /// `opcache.record_warnings`.
    pub fn emit_compile_diags(&mut self, prog: &Program) {
        if prog.compile_diags.is_empty()
            || (prog.compile_diags_reported.replace(true)
                && !ini_bool(&self.ini_get(b"opcache.record_warnings").unwrap_or_default()))
        {
            return;
        }
        for (level, msg, line) in &prog.compile_diags {
            let file = prog.main.debug.file.as_bytes().to_vec();
            self.raise_error_at(*level, msg.as_bytes(), file, *line as i64);
        }
    }

    /// Install the SAPI's file-inclusion callback (parse + compile a path's
    /// source to a `Program`). Without it, `include`/`require` raise an error.
    pub fn set_includer(
        &mut self,
        f: impl FnMut(&str, &[u8]) -> Result<Program, IncludeError> + 'static,
    ) {
        self.includer = Some(Box::new(f));
    }

    /// Execute one `include`/`require` at runtime: resolve the path, (compile &)
    /// run the file, and return its value (default `1`, or `false` for a missing
    /// `include`). `require` of a missing file is fatal.
    /// `eval($code)`: compile the PHP source at runtime and run it in the current
    /// variable scope (`top`). Returns the script's return value (Null if none).
    pub(crate) fn do_eval(&mut self, top: usize, src: Reg) -> Result<Value, Thrown> {
        // zval_try_get_string: __toString for an object, its exception or
        // "could not be converted" Error.
        let v = self.reg_r(top, src)?;
        let code = match &v {
            Value::Str(s) => s.as_bytes().to_vec(),
            v => self.to_string_vm(v)?,
        };
        // compile_string compiles nothing for "": eval() is false.
        if code.is_empty() {
            return Ok(Value::Bool(false));
        }
        // eval() code is already in PHP mode; prepend an open tag so the parser
        // (which starts in inline/HTML mode) enters PHP.
        let mut source = b"<?php ".to_vec();
        source.extend_from_slice(&code);
        // php names eval'd code "FILE(LINE) : eval()'d code" (its __FILE__, getFile()).
        let file = format!(
            "{}({}) : eval()'d code",
            String::from_utf8_lossy(&self.current_file(top)),
            self.current_line(top)
        );

        let mut cb = self
            .includer
            .take()
            .ok_or_else(|| Thrown::error("Error", "eval is not available"))?;
        let compiled = cb(&file, &source);
        self.includer = Some(cb);
        // compile_string records no errors: the user handler sees the
        // diagnostics even when the compile then fails.
        let compiled = compiled.map_err(|e| self.include_error(e, &file, true));
        let prog = Rc::new(compiled?);
        self.emit_compile_diags(&prog);

        // eval()'d declarations are preloaded too (recorded without a file).
        let (main, unit) = self.install_file(b"", &prog)?;
        self.run_included(main, unit, top, "eval")
    }

    /// What an includer's failure throws: a syntax error becomes a
    /// ParseError object located in the included file (or eval()'d code) at
    /// the error's line. `eval` is set for eval()'d code.
    fn include_error(&mut self, e: IncludeError, file: &str, eval: bool) -> Thrown {
        match e {
            IncludeError::Thrown(t) => t,
            // php names the executing include or eval, not the file it
            // was compiling (zend_mm_hard_limit_error).
            IncludeError::HardLimit { .. } => self.compile_hard_limit(None),
            IncludeError::Compile {
                message,
                line,
                diags,
            } => {
                self.compile_failed(&message, file.as_bytes(), line, &diags, eval);
                Thrown::Exit(255)
            }
            // raised outside the file: where the include/eval runs
            IncludeError::Parse {
                class,
                message,
                line,
                diags,
            } => {
                self.emit_parse_diags(file.as_bytes(), &diags);
                if line == 0 {
                    // raised outside the file: where the include/eval runs
                    return Thrown::error(class, message);
                }
                self.materialize(
                    Thrown::error(class, message),
                    Some((file.as_bytes().to_vec(), line)),
                )
            }
        }
    }

    /// The diagnostics the parser raised in `file` before it failed.
    pub fn emit_parse_diags(&mut self, file: &[u8], diags: &[(i64, String, u32)]) {
        for (level, msg, l) in diags {
            self.raise_error_at(*level, msg.as_bytes(), file.to_vec(), *l as i64);
        }
    }

    /// A file failed to compile: the diagnostics raised before the error,
    /// then the error itself ([`Vm::compile_error`]). php's compile_file
    /// records the diagnostics, so they skip the user error handler when it
    /// bails out; eval()'d code (`user_handler`) is not recorded.
    pub fn compile_failed(
        &mut self,
        message: &str,
        file: &[u8],
        line: u32,
        diags: &[(i64, String, u32)],
        user_handler: bool,
    ) {
        for (level, msg, l) in diags {
            self.raise_error_at_ex(
                *level,
                msg.as_bytes(),
                file.to_vec(),
                *l as i64,
                user_handler,
            );
        }
        self.compile_error(message, file, line);
    }

    /// php_error_cb for an E_COMPILE_ERROR at `file:line`: recorded for
    /// error_get_last(), displayed with php 8.5's backtrace (the frames
    /// running the include/eval, `#0 {main}` for the main script), and
    /// objects are no longer destructed. The caller bails out (exit 255).
    pub fn compile_error(&mut self, message: &str, file: &[u8], line: u32) {
        let trace = self.fatal_trace();
        let line = line as i64;
        self.last_error = Some((64, message.as_bytes().to_vec(), file.to_vec(), line));
        self.display_error_trace(64, "Fatal error", message.as_bytes(), file, line, &trace);
        self.destructibles.clear();
    }

    /// The main script failed to parse: php reports the uncaught `class`
    /// (zend_exception_error) as `Parse error: …` (E_PARSE) or, for a
    /// CompileError, `Fatal error: …` (E_COMPILE_ERROR) through the usual
    /// display/log path (exit status 255 is the SAPI's job).
    pub fn report_parse_error(&mut self, class: &str, message: &str, file: &str, line: u32) {
        let (ty, label) = if class == "ParseError" {
            (4, "Parse error")
        } else {
            (64, "Fatal error")
        };
        self.display_error(ty, label, message.as_bytes(), file.as_bytes(), line as i64);
        // php_error_cb records it (error_get_last(), the cli-server's log).
        self.last_error = Some((
            ty,
            message.as_bytes().to_vec(),
            file.as_bytes().to_vec(),
            line as i64,
        ));
    }

    pub(crate) fn do_include(
        &mut self,
        top: usize,
        raw_path: Vec<u8>,
        kind: IncludeKind,
    ) -> Result<Value, Thrown> {
        // Taken first: user code that runs while the file loads (a stream
        // wrapper, an error handler for a compile warning, an autoloader)
        // may include files of its own.
        let primary = std::mem::take(&mut self.primary_next);
        let path_str = String::from_utf8_lossy(&raw_path).into_owned();
        builtins::main::streams::clear_errno();
        // php looks a script up in opcache by the name it was asked for
        // before opening anything ([`Vm::opcache_lookup_name`]): a cached
        // script needs no open, and no realpath or stat while its revalidate
        // window lasts; a file that is gone may still be served.
        // data: URLs (php's rfc2397 wrapper, a URL wrapper): only with
        // allow_url_fopen and allow_url_include.
        let mut reason_override = None;
        let mut quiet_open = false;
        let mut user_stream = None;
        let mut user_shown = None;
        let user = if self.user_wrappers.is_empty() {
            None
        } else {
            self.user_include(&raw_path, kind)?
        };
        let found = if let Some(u) = user {
            match u {
                crate::user_streams::UserInclude::Opened { key, data, stream } => {
                    user_stream = Some(stream);
                    Some((key, IncludeSource::Streamed(data)))
                }
                crate::user_streams::UserInclude::Failed(reason, name) => {
                    quiet_open = reason.is_none();
                    reason_override = reason;
                    user_shown = Some(name);
                    None
                }
                crate::user_streams::UserInclude::AlreadyIncluded => return Ok(Value::Bool(true)),
            }
        } else if builtins::main::streams::is_filter_url(&raw_path) {
            // php://filter: opened and read through the stream layer.
            self.include_docref = Some(kind.name());
            let data = match builtins::Host::stream_open_ctx(self, &raw_path, b"rb", &Value::Null) {
                Some(res) => {
                    let d = self.stream_read_all_for_include(&res);
                    builtins::Host::stream_close(self, &res);
                    // a failed read fails the include without a reason
                    quiet_open = d.is_none();
                    d
                }
                None => {
                    reason_override = Some(
                        builtins::main::streams::take_wrapper_error()
                            .unwrap_or_else(|| "operation failed".to_string()),
                    );
                    None
                }
            };
            self.include_docref = None;
            if let Some(t) = self.pending_throw.take() {
                return Err(t);
            }
            data.map(|d| (raw_path.clone(), IncludeSource::Streamed(d)))
        } else if builtins::main::streams::is_data_url(&raw_path) {
            let on = |vm: &Self, n: &[u8]| crate::ini_bool(&vm.ini_get(n).unwrap_or_default());
            let off = if !on(self, b"allow_url_fopen") {
                Some("allow_url_fopen")
            } else if !on(self, b"allow_url_include") {
                Some("allow_url_include")
            } else {
                None
            };
            if let Some(off) = off {
                let fname = kind.name();
                self.diag(
                    builtins::E_WARNING,
                    &format!(
                        "{fname}(): data:// wrapper is disabled in the server configuration by {off}=0"
                    ),
                )?;
                reason_override = Some("no suitable wrapper could be found".to_string());
                None
            } else {
                match builtins::main::streams::parse_data_url(&raw_path) {
                    Ok((data, _)) => Some((raw_path.clone(), IncludeSource::Bytes(data))),
                    Err(e) => {
                        reason_override = Some(e.to_string());
                        None
                    }
                }
            }
        } else if let Some(real) = self.opcache_lookup_name(top, &raw_path) {
            Some((real, IncludeSource::Cached))
        } else {
            self.resolve_include_file(top, &path_str).or_else(|| {
                self.opcache_cached_name(&raw_path)
                    .map(|real| (real, IncludeSource::Cached))
            })
        };
        // The plain-files opener's open_basedir check of the resolved path.
        let mut shown = match user_shown {
            Some(n) => String::from_utf8_lossy(&n).into_owned(),
            None => path_str.clone(),
        };
        let found = match found {
            Some((real, IncludeSource::File(_)))
                if builtins::main::fopen_wrappers::active()
                    && !builtins::main::fopen_wrappers::check_queued(&real, true) =>
            {
                self.flush_stream_diags(kind.name());
                shown = String::from_utf8_lossy(&real).into_owned();
                None
            }
            // Not found: the plain opener still checks the name as given.
            None if reason_override.is_none()
                && !path_str.is_empty()
                && builtins::main::fopen_wrappers::active()
                && builtins::main::streams::split_scheme(&raw_path).0 == b"file"
                && !builtins::main::fopen_wrappers::check_queued(
                    builtins::main::streams::file_path(&raw_path),
                    true,
                ) =>
            {
                self.flush_stream_diags(kind.name());
                None
            }
            f => f,
        };
        let (realpath, source) = match found {
            Some(found) => found,
            None => {
                // php_stream_open_for_zend_ex's warning, then the include's.
                let fname = kind.name();
                let ip =
                    String::from_utf8_lossy(&self.ini_get(b"include_path").unwrap_or_default())
                        .into_owned();
                let reason = if path_str.is_empty() || quiet_open {
                    None
                } else if let Some(r) = reason_override {
                    Some(r)
                } else {
                    Some(match builtins::main::streams::last_errno() {
                        Some(c) => builtins::main::streams::strerror(c),
                        None => builtins::main::streams::strerror(2), // ENOENT
                    })
                };
                if let Some(reason) = reason {
                    let msg = format!("{fname}({shown}): Failed to open stream: {reason}");
                    let msg = self.docref_html(msg.as_bytes()).into_owned();
                    self.diag(builtins::E_WARNING, &msg)?;
                }
                if kind.is_require() {
                    return Err(Thrown::error(
                        "Error",
                        format!("Failed opening required '{path_str}' (include_path='{ip}')"),
                    ));
                }
                self.diag(
                    builtins::E_WARNING,
                    &format!(
                        "{fname}(): Failed opening '{path_str}' for inclusion (include_path='{ip}')"
                    ),
                )?;
                return Ok(Value::Bool(false));
            }
        };

        // `_once`: skip a file already included (count it as included before
        // running the body, so recursive `_once` includes terminate).
        if kind.is_once()
            && (self.included_files.contains(&realpath)
                || self.main_included_path().as_ref() == Some(&realpath))
        {
            self.opcache_note_once(&raw_path, &realpath);
            return Ok(Value::Bool(true));
        }
        self.included_files.insert(realpath.clone());

        let prog = self.compile_included_file(&realpath, raw_path.as_slice(), source);
        // zend_destroy_file_handle: the stream closes after compiling
        if let Some(s) = user_stream.take() {
            if let Err(Thrown::Exit(c)) = &prog {
                self.pending_throw.get_or_insert(Thrown::Exit(*c));
                self.end_include_stream(s);
                self.pending_throw = None;
            } else {
                self.include_docref = Some(kind.name());
                self.include_docref_depth = self.stack.len();
                self.close_user_state(Some(s));
                self.include_docref = None;
            }
        }
        let prog = prog?;
        self.emit_compile_diags(&prog);
        if let Some(t) = self.pending_throw.take() {
            return Err(t);
        }
        let (main, unit) = self.install_file(&realpath, &prog)?;
        let ret = if primary {
            self.run_as_primary(main, unit)?
        } else {
            self.run_included(main, unit, top, kind.name())?
        };
        // A file with no `return` yields int(1).
        Ok(match ret {
            Value::Null => Value::Int(1),
            v => v,
        })
    }

    /// do_bind_function_error's message for redeclaring user function
    /// `lc` (lowercase) as `name`.
    pub(crate) fn redeclare_function_msg(&self, lc: &[u8], name: &rhp_values::PhpStr) -> String {
        let name = String::from_utf8_lossy(name.as_bytes());
        match self.fn_names.get(lc).map(|&i| &self.functions[i].proto) {
            Some(old) => format!(
                "Cannot redeclare function {name}() (previously declared in {}:{})",
                String::from_utf8_lossy(old.debug.file.as_bytes()),
                old.meta.start_line
            ),
            None => format!("Cannot redeclare function {name}()"),
        }
    }

    /// The main script as an included file (`get_included_files()`, the
    /// `_once` check): when it is a real file, not `-r` code or a script
    /// read from stdin.
    pub(crate) fn main_included_path(&self) -> Option<Vec<u8>> {
        self.main_script_path.clone().or_else(|| {
            let f = self.stack.first()?.proto.debug.file.as_bytes();
            builtins::zend::virtual_cwd::os_path(f)
                .is_file()
                .then(|| f.to_vec())
        })
    }

    /// Compile an included file through the compiled-script cache (a
    /// local file) or directly (a stream wrapper's contents).
    pub(crate) fn compile_included_file(
        &mut self,
        realpath: &[u8],
        given: &[u8],
        source: IncludeSource,
    ) -> Result<Rc<Program>, Thrown> {
        let rp_str = String::from_utf8_lossy(realpath).into_owned();
        let mut compile = |vm: &mut Vm, src: &[u8]| -> Result<Program, IncludeError> {
            // Compile via the SAPI-provided callback (keeps the VM compiler-agnostic).
            let mut cb = vm
                .includer
                .take()
                .ok_or_else(|| Thrown::error("Error", "file inclusion is not available"))?;
            let r = cb(&rp_str, src);
            vm.includer = Some(cb);
            r
        };
        // php opens an included file as a stream resource (closed again
        // after compiling), which uses up a resource id — unless opcache
        // finds the script by the name it was asked for.
        let opened = std::cell::Cell::new(false);
        let compiled = match source {
            // php's opcache caches phar:// scripts too (is_cacheable_stream_path),
            // so their compiled code stays out of the request's memory.
            IncludeSource::Bytes(src) if opcache::is_cacheable_stream_path(realpath) => {
                opened.set(!self.opcache_has_name(given, realpath));
                let mut src = Some(src);
                let mut read = || {
                    opened.set(true);
                    src.take().unwrap_or_default()
                };
                self.opcache_compile(
                    realpath,
                    given,
                    opcache::Kind::Include,
                    false,
                    &mut read,
                    &mut compile,
                )
            }
            IncludeSource::Bytes(src) => {
                opened.set(true);
                compile(self, &src).map(Rc::new)
            }
            IncludeSource::Streamed(src) => compile(self, &src).map(Rc::new),
            IncludeSource::Cached => self.opcache_compile(
                realpath,
                given,
                opcache::Kind::Include,
                true,
                &mut Vec::new,
                &mut compile,
            ),
            IncludeSource::File(mut f) => {
                opened.set(!self.opcache_has_name(given, realpath));
                let mut read = || {
                    opened.set(true);
                    let mut src = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut f, &mut src);
                    src
                };
                self.opcache_compile(
                    realpath,
                    given,
                    opcache::Kind::Include,
                    false,
                    &mut read,
                    &mut compile,
                )
            }
        };
        if opened.get() {
            self.next_resource_id += 1;
        }
        compiled.map_err(|e| self.include_error(e, &rp_str, false))
    }

    /// Compile a request's main script (`path` resolved, `given` as named,
    /// `source` read) through the compiled-script cache: `compile` runs
    /// only on a miss.
    pub fn compile_main(
        &mut self,
        path: &str,
        given: &str,
        source: Vec<u8>,
        compile: impl FnOnce(&[u8]) -> Result<Program, IncludeError>,
    ) -> Result<Rc<Program>, IncludeError> {
        let mut source = Some(source);
        let mut compile = Some(compile);
        self.opcache_compile(
            path.as_bytes(),
            given.as_bytes(),
            opcache::Kind::Main,
            false,
            &mut || source.take().unwrap_or_default(),
            &mut |_, src| (compile.take().expect("one compile per load"))(src),
        )
    }

    /// Resolve an include path to `(key, source)`: for a schemed URL
    /// (`phar://`, …) the wrapper's contents; otherwise the real path of
    /// the first candidate ([`Vm::include_candidates`]) that opens as a
    /// file, resolved through the realpath cache.
    pub(crate) fn resolve_include_file(
        &self,
        top: usize,
        path: &str,
    ) -> Option<(Vec<u8>, IncludeSource)> {
        // Schemed URLs: no candidate search — the wrapper owns resolution. The
        // returned key (→ `__FILE__`/`__DIR__`/`_once`) is the normalized URL.
        let (scheme, _) = builtins::main::streams::split_scheme(path.as_bytes());
        if scheme != b"file" {
            let url = path.as_bytes();
            let w = self.wrapper_for(url)?;
            let src = w.get_contents(url)?;
            let key = w.canonical(url).unwrap_or_else(|| normalize_url(url));
            return Some((key, IncludeSource::Bytes(src)));
        }
        for cand in self.include_candidates(top, path) {
            use std::os::unix::ffi::OsStrExt;
            // tsrm_realpath, through the realpath cache.
            let Some(real) = builtins::zend::virtual_cwd::realpath(cand.as_os_str().as_bytes())
            else {
                continue;
            };
            let real_path = std::ffi::OsStr::from_bytes(&real);
            let Ok(f) = std::fs::File::open(real_path) else {
                continue;
            };
            if f.metadata().is_ok_and(|m| m.is_dir()) {
                continue;
            }
            return Some((real, IncludeSource::File(f)));
        }
        None
    }

    /// The paths an include of `path` (no scheme) tries, in
    /// php_resolve_path's order: an absolute or `./` / `../` path as is
    /// (relative to the request's working directory), else each
    /// include_path entry, then the including file's directory, then the
    /// name relative to the working directory.
    fn include_candidates(&self, top: usize, path: &str) -> Vec<std::path::PathBuf> {
        use std::path::Path;
        let p = Path::new(path);
        let mut candidates = Vec::new();
        let include_path = self.ini_get(b"include_path").unwrap_or_default();
        let dotted = path.starts_with("./") || path.starts_with("../");
        if p.is_absolute() || dotted || include_path.is_empty() {
            candidates.push(p.to_path_buf());
        } else {
            for dir in include_path.split(|&c| c == b':') {
                // Entries that are stream wrappers (`phar://…`) aren't searched.
                if dir.is_empty() || dir.windows(3).any(|w| w == b"://") {
                    continue;
                }
                candidates.push(Path::new(&*String::from_utf8_lossy(dir)).join(p));
            }
            let cur = self.current_file(top);
            let cur = String::from_utf8_lossy(&cur).into_owned();
            if let Some(dir) = Path::new(&cur).parent().filter(|d| d.is_absolute()) {
                candidates.push(dir.join(p));
            }
            candidates.push(p.to_path_buf());
        }
        candidates
    }

    /// php_resolve_path for a local path (zend_resolve_path without
    /// opcache): the real path of the first candidate that exists.
    pub(crate) fn resolve_include_path(&self, top: usize, path: &[u8]) -> Option<Vec<u8>> {
        use std::os::unix::ffi::OsStrExt;
        let path = String::from_utf8_lossy(path);
        self.include_candidates(top, &path)
            .iter()
            .find_map(|c| builtins::zend::virtual_cwd::realpath(c.as_os_str().as_bytes()))
    }

    /// [`Vm::install_included`] for a file: recorded while preloading, and
    /// without redeclaring what preloading already declared from it.
    pub(crate) fn install_file(
        &mut self,
        path: &[u8],
        prog: &Rc<Program>,
    ) -> Result<(Rc<FunctionProto>, UnitId), Thrown> {
        let at = (
            self.closures.len(),
            self.decl_classes.len(),
            self.functions.len(),
        );
        let preloaded = self.preloaded.as_ref().is_some_and(|p| p.has_file(path));
        let main = self.install_included(prog, preloaded)?;
        if let Some(rec) = self.preload_rec.as_mut() {
            rec.push(opcache::PreloadRec::new(
                path,
                prog.clone(),
                false,
                at,
                self.functions.len() - at.2,
            ));
        }
        Ok(main)
    }

    /// Start a unit for `prog`: record where its file-local indices start
    /// and append its closures. When the last unit added no closures and
    /// no class declarations, its base is this one's and `prog` shares it,
    /// so a file included in a loop does not grow the table.
    pub(crate) fn add_unit(&mut self, prog: &Program) -> UnitId {
        let base = UnitBase {
            closures: self.closures.len() as u32,
            classes: self.decl_classes.len() as u32,
        };
        if self.units.last() != Some(&base) {
            self.units.push(base);
        }
        let unit = (self.units.len() - 1) as UnitId;
        let closures = prog.closures.iter().map(|c| UnitFn {
            proto: c.clone(),
            unit,
        });
        self.closures.extend(closures);
        unit
    }

    /// Merge an included program's declarations into the running VM as a new
    /// unit. Returns its `{main}` to execute, and the unit.
    /// `preloaded`: the file was preloaded, and php removed its declarations
    /// of what preloading declared: those are skipped, not redeclared.
    fn install_included(
        &mut self,
        prog: &Program,
        preloaded: bool,
    ) -> Result<(Rc<FunctionProto>, UnitId), Thrown> {
        let cbase = self.decl_classes.len();
        let unit = self.add_unit(prog);
        // functions: skip the re-emitted prelude helpers (the first N); a user
        // function is fatal to redeclare (php semantics).
        for proto in prog.functions.iter().skip(prog.num_prelude_functions) {
            let lc = proto.name.as_bytes().to_ascii_lowercase();
            if self.fn_names.contains_key(&lc) {
                if preloaded {
                    continue;
                }
                // do_bind_function_error at compile time: E_COMPILE_ERROR at
                // the new declaration.
                let msg = self.redeclare_function_msg(&lc, &proto.name);
                let line = proto.meta.start_line as i64;
                let file = proto.debug.file.as_bytes().to_vec();
                return Err(self.fatal_compile_error_at(msg.as_bytes(), file, line));
            }
            let idx = self.functions.len();
            self.functions.push(UnitFn {
                proto: proto.clone(),
                unit,
            });
            self.add_fn_name(lc, idx);
        }
        // classes: a name already in use is not early-bound; redeclaring it is
        // fatal only when its declaration statement runs (`DeclareClass`).
        let top = self.register_decl_classes(prog, unit);
        if preloaded {
            self.skip_preloaded_decls(cbase);
        }
        self.hoist_decl_classes(top)?;
        Ok((prog.main.clone(), unit))
    }

    /// Run an included / eval'd script's `{main}` **in the calling scope**: its
    /// CVs are bound by name to the caller frame's variables (shared cells), so
    /// writes are visible to the caller and reads see the caller's current
    /// values — php's include/eval scope semantics. `$this` / class scope are
    /// forwarded too (an include inside a method can use `$this`).
    fn run_included(
        &mut self,
        main: Rc<FunctionProto>,
        unit: UnitId,
        caller_top: usize,
        kind: &'static str,
    ) -> Result<Value, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let mut frame = Frame::new(main.clone(), unit, 0);
        frame.capture = true;
        frame.include = Some((kind, caller_top));
        frame.defining_class = self.stack[caller_top].defining_class;
        frame.called_class = self.stack[caller_top].called_class;

        // Bind each CV to the caller's same-named variable (shared via a Ref).
        // From the global scope, the included file's variables are the global
        // symbol table's (including the ones it creates).
        let caller_proto = self.stack[caller_top].proto.clone();
        if self.stack[caller_top].global_scope {
            self.bind_global_scope(&mut frame);
        }
        for ci in 0..main.cv_var_names.len() {
            if frame.global_scope {
                break;
            }
            let name = &main.cv_var_names[ci];
            if let Some(caller_ci) = caller_proto.cv_var_names.iter().position(|n| n == name) {
                let rc = self.promote_slot(caller_top, caller_ci as Reg);
                frame.regs[ci] = Value::Ref(rc);
            } else if name.as_bytes() != b"this"
                && !main.superglobals.iter().any(|(r, _)| *r as usize == ci)
            {
                // A variable the caller has no slot for still lives in the
                // caller's scope: the global symbol table at top level (so an
                // included script sees `$argv`), else its by-name table.
                let Err(rc) = self.var_var_slot(caller_top, name.as_bytes()) else {
                    unreachable!("no slot named {name:?} in the caller")
                };
                frame.regs[ci] = Value::Ref(rc);
            }
        }
        self.bind_superglobals(&mut frame);
        let base = self.stack.len();
        self.stack.push(frame);
        let ret = loop {
            match self.run_frames() {
                Ok(v) => break Ok(v),
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        break Err(thrown);
                    }
                    match self.unwind(thrown, base) {
                        Ok(UnwindResult::Caught) => {}
                        Ok(UnwindResult::Escaped(t)) | Err(t) => break Err(t),
                    }
                }
            }
        };
        // php starts an include's statics fresh each time.
        if !self.include_statics.is_empty() {
            self.include_statics.retain(|k, _| k.0 < base);
        }
        ret
    }

    /// php -a's `zend_eval_stringl(code, NULL, name)` under `zend_try`:
    /// the code compiled as `name` and run in the global scope as a
    /// top-level script. A compile error is displayed, an uncaught
    /// throwable reported as a warning, and a fatal error or `exit` only
    /// sets the exit status.
    pub(crate) fn shell_eval_code(&mut self, code: &[u8], name: &str) {
        self.eval_string_ex(code, name, (builtins::E_WARNING, "Warning"));
    }

    /// zend_eval_stringl_ex(code, NULL, name, true) under zend_try: as
    /// [`Vm::shell_eval_code`], an uncaught throwable reported at `level`
    /// (`(type, label)`). At E_ERROR (php-cli's -B/-R/-E) it and a parse
    /// error leave exit status 255. True: a fatal error bailed out.
    pub(crate) fn eval_string_ex(&mut self, code: &[u8], name: &str, level: (i64, &str)) -> bool {
        let fatal = level.0 == builtins::E_ERROR;
        let mark = self.last_error.clone();
        let mut source = b"<?php ".to_vec();
        source.extend_from_slice(code);
        let Some(mut cb) = self.includer.take() else {
            return false;
        };
        let compiled = cb(name, &source);
        self.includer = Some(cb);
        let prog = match compiled {
            Ok(p) => Rc::new(p),
            Err(IncludeError::Parse {
                class,
                message,
                line,
                diags,
            }) => {
                self.emit_parse_diags(name.as_bytes(), &diags);
                self.report_parse_error(class, &message, name, line);
                if fatal {
                    self.sapi.exit_status = 255;
                }
                return false;
            }
            Err(e) => {
                if let Thrown::Exit(code) = self.include_error(e, name, true) {
                    self.sapi.exit_status = code;
                    return true;
                }
                return false;
            }
        };
        self.emit_compile_diags(&prog);
        let r = self
            .install_file(b"", &prog)
            .and_then(|(main, unit)| self.run_as_primary(main, unit));
        let mut bailed = false;
        if r.is_err() && !self.stack.is_empty() {
            // The global scope survives the unwound frames.
            self.stack.truncate(1);
            self.finished_main = self.stack.pop();
        }
        match r {
            Ok(_) => {}
            Err(Thrown::Exit(code)) => {
                self.sapi.exit_status = code;
                bailed = self.fatal_since(&mark);
            }
            Err(t) => {
                self.report_uncaught_as(t, level.0, level.1);
                if fatal {
                    self.sapi.exit_status = 255;
                }
            }
        }
        if let Some(Thrown::Exit(code)) = self.pending_throw.take() {
            self.sapi.exit_status = code;
        }
        bailed
    }

    /// Whether a fatal error other than `mark` is now the last error: an
    /// `exit` that unwound was php's bailout.
    pub(crate) fn fatal_since(&self, mark: &Option<(i64, Vec<u8>, Vec<u8>, i64)>) -> bool {
        const FATAL: i64 = builtins::E_ERROR
            | builtins::E_CORE_ERROR
            | builtins::E_COMPILE_ERROR
            | builtins::E_USER_ERROR;
        match &self.last_error {
            Some(e) if e.0 & FATAL != 0 => mark.as_ref() != Some(e),
            _ => false,
        }
    }

    /// A loaded file run as the request's next primary script: its `{main}`
    /// replaces the finished one at the bottom of the stack, in the global
    /// scope.
    fn run_as_primary(&mut self, main: Rc<FunctionProto>, unit: UnitId) -> Result<Value, Thrown> {
        let mut frame = Frame::new(main, unit, 0);
        self.bind_superglobals(&mut frame);
        self.bind_global_scope(&mut frame);
        self.stack.clear();
        self.stack.push(frame);
        self.run_loop()
    }

    /// An include/eval frame that is finishing shares its caller's variables
    /// through the cells bound at entry; a variable it re-bound (`$a = &$b`,
    /// `global $a`, `static $a`) or unset carries that over to the including
    /// scope, as php's shared symbol table does.
    pub(crate) fn include_writeback(&mut self, frame: &Frame) {
        let Some(caller) = frame.include_caller() else {
            return;
        };
        // In the global scope every variable already is a global symbol-table
        // cell, kept in step as it is re-bound or unset.
        if caller >= self.stack.len() || frame.global_scope {
            return;
        }
        let proto = frame.proto.clone();
        for (ci, name) in proto.cv_var_names.iter().enumerate() {
            if name.as_bytes() == b"this"
                || proto.superglobals.iter().any(|(r, _)| *r as usize == ci)
            {
                continue;
            }
            let Some(slot) = frame.regs.get(ci) else {
                continue;
            };
            let current = match self.var_var_lookup(caller, name.as_bytes()) {
                Some(Ok(cv)) => match &self.stack[caller].regs[cv as usize] {
                    Value::Ref(rc) => Some(rc.clone()),
                    _ => None,
                },
                Some(Err(rc)) => Some(rc),
                None => None,
            };
            match slot {
                Value::Ref(rc) => {
                    if !current.is_some_and(|c| Rc::ptr_eq(&c, rc)) {
                        self.var_var_bind(caller, name.as_bytes(), rc.clone());
                    }
                }
                Value::Undef => self.var_var_unset(caller, name.as_bytes()),
                v => {
                    let cell = Rc::new(RefCell::new(v.clone()));
                    self.var_var_bind(caller, name.as_bytes(), cell);
                }
            }
        }
    }
}

/// Normalize a schemed URL (`scheme://…`) by collapsing `.`/`..`/empty path
/// segments after the scheme, preserving the scheme and any leading `/` of the
/// remainder. Used as the canonical `__FILE__`/`_once` key for phar entries.
fn normalize_url(url: &[u8]) -> Vec<u8> {
    let Some(pos) = url.windows(3).position(|w| w == b"://") else {
        return url.to_vec();
    };
    let (pre, rest) = url.split_at(pos + 3);
    let leading_slash = rest.first() == Some(&b'/');
    let mut segs: Vec<&[u8]> = Vec::new();
    for s in rest.split(|&b| b == b'/') {
        match s {
            b"" | b"." => {}
            b".." => {
                segs.pop();
            }
            x => segs.push(x),
        }
    }
    let mut out = pre.to_vec();
    if leading_slash {
        out.push(b'/');
    }
    out.extend_from_slice(&segs.join(&b"/"[..]));
    out
}
