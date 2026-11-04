//! `include` / `require` / `eval`: resolving, compiling and installing a file's
//! program.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_builtins::{self as builtins, Host};
use rhp_bytecode::{FunctionProto, IncludeKind, Program, Reg};
use rhp_values::{conv, Thrown, Value};

use crate::errors::UnwindResult;
use crate::{ini_bool, opcache, Frame, UnitBase, UnitFn, UnitId, Vm};

/// SAPI-injected callback that parses + compiles an included file's source into
/// a `Program`, keeping the VM compiler-agnostic (see the `includer` field).
pub(crate) type Includer = Box<dyn FnMut(&str, &[u8]) -> Result<Program, IncludeError>>;

/// An included file's source: a stream wrapper's contents, or an open
/// local file (read only when the compiled-script cache misses).
pub(crate) enum IncludeSource {
    Bytes(Vec<u8>),
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
    },
    /// A compile error (E_COMPILE_ERROR) at `line`: a fatal error, not an
    /// exception — the request bails out (shutdown functions still run).
    Compile { message: String, line: u32 },
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
    pub(crate) fn emit_compile_diags(&mut self, prog: &Program) {
        if prog.compile_diags.is_empty()
            || (prog.compile_diags_reported.replace(true)
                && !ini_bool(&self.ini_get(b"opcache.record_warnings").unwrap_or_default()))
        {
            return;
        }
        for (level, msg, line) in &prog.compile_diags {
            let file = prog.main.debug.file.as_bytes().to_vec();
            self.raise_error_at(*level, msg, file, *line as i64);
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
        let code = conv::to_string_bytes(&self.reg_get(top, src));
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
        let prog = Rc::new(self.included_program(compiled, &file)?);
        self.emit_compile_diags(&prog);

        // eval()'d declarations are preloaded too (recorded without a file).
        let (main, unit) = self.install_file(b"", &prog)?;
        self.run_included(main, unit, top, "eval")
    }

    /// The includer's result: a syntax error becomes a ParseError object
    /// located in the included file (or eval()'d code) at the error's line.
    fn included_program(
        &mut self,
        compiled: Result<Program, IncludeError>,
        file: &str,
    ) -> Result<Program, Thrown> {
        compiled.map_err(|e| self.include_error(e, file))
    }

    /// What an includer's failure throws.
    fn include_error(&mut self, e: IncludeError, file: &str) -> Thrown {
        match e {
            IncludeError::Thrown(t) => t,
            IncludeError::Compile { message, line } => {
                self.compile_error(&message, file.as_bytes(), line);
                Thrown::Exit(255)
            }
            // raised outside the file: where the include/eval runs
            IncludeError::Parse {
                class,
                message,
                line: 0,
            } => Thrown::error(class, message),
            IncludeError::Parse {
                class,
                message,
                line,
            } => self.materialize(
                Thrown::error(class, message),
                Some((file.as_bytes().to_vec(), line)),
            ),
        }
    }

    /// php_error_cb for an E_COMPILE_ERROR at `file:line`: recorded for
    /// error_get_last(), displayed with php 8.5's backtrace (the frames
    /// running the include/eval, `#0 {main}` for the main script), and
    /// objects are no longer destructed. The caller bails out (exit 255).
    pub fn compile_error(&mut self, message: &str, file: &[u8], line: u32) {
        let backtraces = self.ini_get(b"fatal_error_backtraces").unwrap_or_default();
        let trace = if ini_bool(&backtraces) {
            let trace = Value::Array(self.build_trace_ex(false, false));
            builtins::php_fns::trace_to_string(self, &trace)
        } else {
            Vec::new()
        };
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
    }

    pub(crate) fn do_include(
        &mut self,
        top: usize,
        raw_path: Vec<u8>,
        kind: IncludeKind,
    ) -> Result<Value, Thrown> {
        let path_str = String::from_utf8_lossy(&raw_path).into_owned();
        builtins::streams::clear_errno();
        // php looks a script up in opcache by the name it was asked for
        // before opening anything: a cached absolute path needs no open, and
        // no realpath while its revalidate window lasts; a file that is gone
        // may still be served.
        let found = if self.opcache_fast_hit(&raw_path) {
            Some((raw_path.clone(), IncludeSource::Cached))
        } else {
            self.resolve_include_file(top, &path_str).or_else(|| {
                self.opcache_cached_name(&raw_path)
                    .map(|real| (real, IncludeSource::Cached))
            })
        };
        let (realpath, source) = match found {
            Some(found) => found,
            None => {
                // php_stream_open_for_zend_ex's warning, then the include's.
                let fname = kind.name();
                let ip =
                    String::from_utf8_lossy(&self.ini_get(b"include_path").unwrap_or_default())
                        .into_owned();
                let reason = if path_str.is_empty() {
                    None
                } else {
                    Some(match builtins::streams::last_errno() {
                        Some(c) => builtins::streams::strerror(c),
                        None => builtins::streams::strerror(2), // ENOENT
                    })
                };
                if let Some(reason) = reason {
                    self.diag(
                        builtins::E_WARNING,
                        &format!("{fname}({path_str}): Failed to open stream: {reason}"),
                    )?;
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
        if kind.is_once() && self.included_files.contains(&realpath) {
            return Ok(Value::Bool(true));
        }
        self.included_files.insert(realpath.clone());

        let prog = self.compile_included_file(&realpath, raw_path.as_slice(), source)?;
        self.emit_compile_diags(&prog);
        if let Some(t) = self.pending_throw.take() {
            return Err(t);
        }
        let (main, unit) = self.install_file(&realpath, &prog)?;
        let ret = self.run_included(main, unit, top, kind.name())?;
        // A file with no `return` yields int(1).
        Ok(match ret {
            Value::Null => Value::Int(1),
            v => v,
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
            IncludeSource::Bytes(src) => {
                opened.set(true);
                compile(self, &src).map(Rc::new)
            }
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
        compiled.map_err(|e| self.include_error(e, &rp_str))
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
    /// the first candidate that opens as a file, in php_resolve_path's
    /// order: an absolute or `./` / `../` path as is (relative to the
    /// request's working directory), else each include_path entry, then
    /// the including file's directory; failing those, the plain open of
    /// the name relative to the working directory.
    pub(crate) fn resolve_include_file(
        &self,
        top: usize,
        path: &str,
    ) -> Option<(Vec<u8>, IncludeSource)> {
        use std::path::{Path, PathBuf};
        // Schemed URLs: no candidate search — the wrapper owns resolution. The
        // returned key (→ `__FILE__`/`__DIR__`/`_once`) is the normalized URL.
        let (scheme, _) = builtins::streams::split_scheme(path.as_bytes());
        if scheme != b"file" {
            let url = path.as_bytes();
            let w = self.wrapper_for(url)?;
            let src = w.get_contents(url)?;
            let key = w.canonical(url).unwrap_or_else(|| normalize_url(url));
            return Some((key, IncludeSource::Bytes(src)));
        }
        let p = Path::new(path);
        let mut candidates: Vec<PathBuf> = Vec::new();
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
        for cand in candidates {
            use std::os::unix::ffi::OsStrExt;
            let cand = builtins::zend::virtual_cwd::os_path(cand.as_os_str().as_bytes());
            let Ok(canon) = std::fs::canonicalize(&cand) else {
                continue;
            };
            let Ok(f) = std::fs::File::open(&canon) else {
                continue;
            };
            if f.metadata().is_ok_and(|m| m.is_dir()) {
                continue;
            }
            return Some((
                canon.to_string_lossy().into_owned().into_bytes(),
                IncludeSource::File(f),
            ));
        }
        None
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
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot redeclare function {}()",
                        String::from_utf8_lossy(proto.name.as_bytes())
                    ),
                ));
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
        frame.include_kind = Some(kind);
        frame.include_caller = Some(caller_top);
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
                let rc = self.promote(caller_top, caller_ci as Reg);
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

    /// An include/eval frame that is finishing shares its caller's variables
    /// through the cells bound at entry; a variable it re-bound (`$a = &$b`,
    /// `global $a`, `static $a`) or unset carries that over to the including
    /// scope, as php's shared symbol table does.
    pub(crate) fn include_writeback(&mut self, frame: &Frame) {
        let Some(caller) = frame.include_caller else {
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
                Value::Null => self.var_var_unset(caller, name.as_bytes()),
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
