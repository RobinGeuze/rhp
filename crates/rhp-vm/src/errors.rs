//! Diagnostics, exceptions and unwinding: raising and displaying errors,
//! backtraces, uncaught throwables and `finally`.

use std::rc::Rc;

use rhp_builtins::{self as builtins, Host};
use rhp_bytecode::{FunctionProto, Instr};
use rhp_values::{conv, ArrayKey, ClassId, ObjectHandle, PhpArray, Thrown, Value};

use crate::calls::{is_engine_helper_name, is_handler_accessor};
use crate::{deref, ini_bool, Frame, Vm};

/// PG(display_errors) (main.c php_get_display_errors_mode).
pub(crate) enum DisplayErrors {
    Off,
    Stdout,
    Stderr,
}

/// Result of unwinding: a handler was installed, or the exception escaped the
/// current (sub)run and should propagate to the Rust caller.
pub(crate) enum UnwindResult {
    Caught,
    Escaped(Thrown),
}

/// What to do after a `finally` block completes.
pub(crate) enum FinallyCont {
    /// An exception was in flight (no catch matched); re-raise after finally.
    Rethrow(Thrown),
    /// A `return` crossed out of the try/catch; complete it after finally.
    Return(Value),
    /// A by-reference function's `return` (at `pc`, of `RET_REF_*` kind)
    /// crossed out of the try/catch: its type is checked again after the
    /// finally blocks, then RETURN_BY_REF's notice.
    ReturnRef(Value, u32, u8),
    /// A `break`/`continue` crossed out of the try/catch; jump after finally.
    Jump(u32),
}

pub(crate) use builtins::zend::builtin_functions::SILENCE_MASK as SILENCE_KEEPS;

/// A boolean ini directive (php's OnUpdateBool: "1", "on", "yes", "true").
fn ini_flag(name: &[u8]) -> bool {
    builtins::ini_value(name)
        .flatten()
        .is_some_and(|v| builtins::ini_parse_bool(&v))
}

/// A name as php's `%s` prints it: up to the first NUL (an anonymous class
/// `X@anonymous\0file:line$n` shows as `X@anonymous`).
pub(crate) fn c_name(name: &[u8]) -> &[u8] {
    &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())]
}

impl Vm {
    /// An `E_COMPILE_ERROR` raised while executing (e.g. a runtime class
    /// redeclaration): never seen by a user error handler, displayed with
    /// php 8.5's backtrace (when `fatal_error_backtraces` is on), then the
    /// request bails out (exit status 255).
    pub(crate) fn fatal_compile_error(&mut self, msg: &[u8]) -> Thrown {
        let (file, line) = match (0..self.stack.len())
            .rev()
            .find(|&i| !self.stack[i].proto.is_internal())
        {
            None => (Vec::new(), 0),
            Some(i) => (self.current_file(i), self.current_line(i) as i64),
        };
        self.fatal_compile_error_at(msg, file, line)
    }

    /// The backtrace a fatal error shows (`fatal_error_backtraces`).
    pub(crate) fn fatal_trace(&mut self) -> Vec<u8> {
        self.last_fatal_trace = None;
        if !ini_bool(&self.ini_get(b"fatal_error_backtraces").unwrap_or_default()) {
            return Vec::new();
        }
        let arr = self.build_trace();
        let s = builtins::zend::exceptions::trace_to_string(self, &Value::Array(arr.clone()));
        self.last_fatal_trace = Some(arr);
        s
    }

    /// [`Vm::fatal_compile_error`] reported at `file`:`line` (php's
    /// zend_error_at for errors found while linking a class).
    pub(crate) fn fatal_compile_error_at(
        &mut self,
        msg: &[u8],
        file: Vec<u8>,
        line: i64,
    ) -> Thrown {
        self.fatal_error_at(64, msg, file, line)
    }

    /// A fatal error of type `ty` (E_COMPILE_ERROR or E_ERROR) raised while
    /// executing, reported at `file`:`line`.
    pub(crate) fn fatal_error_at(
        &mut self,
        ty: i64,
        msg: &[u8],
        file: Vec<u8>,
        line: i64,
    ) -> Thrown {
        self.last_error = Some((ty, msg.to_vec(), file.clone(), line));
        let trace = self.fatal_trace();
        self.display_error_trace(ty, "Fatal error", msg, &file, line, &trace);
        self.mark_objects_destructed();
        Thrown::Exit(255)
    }

    /// A fatal error of type `ty` at the innermost user frame, with the
    /// backtrace; the request bails out.
    pub(crate) fn fatal_error_here(&mut self, ty: i64, msg: &[u8]) -> Thrown {
        let (file, line) = match (0..self.stack.len())
            .rev()
            .find(|&i| !self.stack[i].proto.is_internal())
        {
            None => (Vec::new(), 0),
            Some(i) => (self.current_file(i), self.current_line(i) as i64),
        };
        self.fatal_error_at(ty, msg, file, line)
    }

    /// A throwable as `zval_get_string` makes it (its `__toString()`).
    pub(crate) fn throwable_string(&mut self, thrown: Thrown) -> Vec<u8> {
        let thrown = self.materialize(thrown, None);
        let Thrown::Object(h) = thrown else {
            return Vec::new();
        };
        match self.call_to_string(&h) {
            Ok(Value::Str(s)) => s.as_bytes().to_vec(),
            _ => Vec::new(),
        }
    }

    /// `$throwable->__toString()`, its result dereferenced.
    fn call_to_string(&mut self, h: &ObjectHandle) -> Result<Value, Thrown> {
        let cls = h.borrow().class;
        match self.lookup_method(cls, b"__tostring") {
            Some(m) => self
                .call_sync(&m, Some(h.clone()), cls, Vec::new())
                .map(|v| deref(&v)),
            None => {
                let obj = Value::Object(h.clone());
                builtins::zend::exceptions::throwable_to_string(self, &obj).map(Value::str)
            }
        }
    }

    /// Raise a php diagnostic (`E_WARNING`, `E_NOTICE`, …): record it for
    /// `error_get_last()` and display it (CLI `display_errors=1`) unless an
    /// `@` is in effect.
    /// php's `zend_error` for a non-fatal diagnostic of type `level`: the
    /// user error handler (when its mask covers `level`; called even under
    /// `@`, where `error_reporting()` reads as the fatal mask), and unless it
    /// returns false, nothing else; otherwise php_error_cb: `error_get_last`
    /// and the display / log step (masked by `error_reporting` and `@`). An
    /// exception from the handler becomes `pending_throw`.
    pub(crate) fn raise_error(&mut self, level: i64, msg: &(impl AsRef<[u8]> + ?Sized)) {
        let msg = msg.as_ref();
        if let Some((file, line)) = self.const_expr_site() {
            return self.raise_error_at(level, msg, file, line as i64);
        }
        // The innermost user frame: prelude (built-in PHP-source) code has no
        // line map, and php reports internal functions at their caller.
        let (file, line) = match (0..self.stack.len())
            .rev()
            .find(|&i| !self.stack[i].proto.is_internal() && !self.is_const_expr_frame(i))
        {
            None if self.sapi.startup => (b"Unknown".to_vec(), 0),
            None => match self.fiber_resumer_site() {
                Some((_, _, true)) => (b"Unknown".to_vec(), 0),
                Some((file, line, false)) => (file, line as i64),
                None => (Vec::new(), 0),
            },
            // The user exception handler's call: php has no frame there.
            Some(i)
                if self.stack[i]
                    .proto
                    .flags
                    .contains(rhp_bytecode::ProtoFlags::SHUTDOWN_SCOPE) =>
            {
                (b"Unknown".to_vec(), 0)
            }
            Some(i) => (self.current_file(i), self.current_line(i) as i64),
        };
        self.raise_error_at(level, msg, file, line);
    }

    /// [`Vm::raise_error`] reported at `file`:`line`.
    pub(crate) fn raise_error_at(&mut self, level: i64, msg: &[u8], file: Vec<u8>, line: i64) {
        self.raise_error_at_ex(level, msg, file, line, true);
    }

    /// [`Vm::raise_error_at`]; without `user_handler` the user error
    /// handler is skipped (php_error_cb only).
    pub(crate) fn raise_error_at_ex(
        &mut self,
        level: i64,
        msg: &[u8],
        file: Vec<u8>,
        line: i64,
        user_handler: bool,
    ) {
        if self.pending_throw.is_some() {
            return; // php skips diagnostics while an exception is pending
        }
        self.last_fatal_trace = None;
        // A diagnostic prelude code silenced with `@` is engine plumbing (php's
        // C implementation raises none): it only updates error_get_last().
        // (`silence_stack` records the stack depth at each `@`.)
        let prelude_silenced = self.silence > 0
            && self.silence_stack.last().is_some_and(|&(depth, _, _)| {
                depth
                    .checked_sub(1)
                    .and_then(|i| self.stack.get(i))
                    .is_some_and(|f| f.proto.is_internal())
            });
        // zend_error: these may not be handled in user space.
        let unsafe_level = level
            & (builtins::E_ERROR
                | builtins::E_PARSE
                | builtins::E_CORE_ERROR
                | builtins::E_CORE_WARNING
                | builtins::E_COMPILE_ERROR
                | builtins::E_COMPILE_WARNING)
            != 0;
        let handler = match self.error_handlers.last() {
            _ if prelude_silenced || unsafe_level || !user_handler => None,
            Some((h, mask)) if !matches!(h, Value::Null | Value::Bool(false)) => {
                (mask & level != 0).then(|| h.clone())
            }
            _ => None,
        };
        if let Some(h) = handler {
            let args = vec![
                Value::Int(level),
                Value::str(msg.to_vec()),
                Value::str(file.clone()),
                Value::Int(line),
            ];
            // php's EG(user_error_handler) swap: while the handler runs there
            // is none (`false` marks php's UNDEF); it comes back afterwards
            // only if the handler did not install another one.
            let idx = self.error_handlers.len() - 1;
            self.error_handlers[idx].0 = Value::Bool(false);
            // The diagnostic may come from the middle of an lvalue write:
            // the handler's own writes must not disturb that cursor.
            let r = self.with_saved_cursor(|vm| vm.call_callable_value(&h, args));
            if self.error_handlers.len() == idx + 1
                && matches!(self.error_handlers[idx].0, Value::Bool(false))
            {
                self.error_handlers[idx].0 = h;
            }
            match r {
                Ok(Value::Bool(false)) => {} // fall through to php_error_cb
                Ok(_) => return,
                Err(t) => {
                    self.pending_throw = Some(t);
                    return;
                }
            }
        }
        // php_error_cb: ignore_repeated_errors drops a message equal to the
        // last one (from the same place, unless ignore_repeated_source).
        let repeated = ini_flag(b"ignore_repeated_errors")
            && self.last_error.as_ref().is_some_and(|(_, m, f, l)| {
                m == msg && (ini_flag(b"ignore_repeated_source") || (*l == line && *f == file))
            });
        if !repeated {
            self.last_error = Some((level, msg.to_vec(), file.clone(), line));
        } else if level != builtins::E_USER_ERROR {
            return;
        }
        if level == builtins::E_USER_ERROR {
            // E_USER_ERROR is fatal: displayed with the backtrace, then the
            // request bails out (shutdown functions still run; exit 255).
            let trace = self.fatal_trace();
            self.display_error_trace(level, "Fatal error", msg, &file, line, &trace);
            self.mark_objects_destructed();
            self.pending_throw = Some(Thrown::Exit(255));
            return;
        }
        if self.quiet > 0 && level & SILENCE_KEEPS == 0 {
            return;
        }
        use builtins::*;
        let label = match level {
            E_ERROR | E_CORE_ERROR | E_COMPILE_ERROR | E_USER_ERROR => "Fatal error",
            E_WARNING | E_CORE_WARNING | E_COMPILE_WARNING | E_USER_WARNING => "Warning",
            E_NOTICE | E_USER_NOTICE => "Notice",
            E_DEPRECATED | E_USER_DEPRECATED => "Deprecated",
            E_RECOVERABLE_ERROR => "Recoverable fatal error",
            _ => "Unknown error",
        };
        self.display_error(level, label, msg, &file, line);
    }

    /// Source line of the instruction currently executing in frame `top`.
    pub(crate) fn current_line(&self, top: usize) -> u32 {
        frame_line(&self.stack[top])
    }

    /// The source file name of frame `top` (for exception getFile()).
    pub(crate) fn current_file(&self, top: usize) -> Vec<u8> {
        self.stack[top].proto.debug.file.as_bytes().to_vec()
    }

    /// Stamp an unlocated refusal with the innermost user frame's file and
    /// line (engine-internal prelude frames have no source to point at).
    pub(crate) fn locate_unsupported(&self, t: Thrown) -> Thrown {
        if !matches!(t, Thrown::Unsupported { location: None, .. }) {
            return t;
        }
        match (0..self.stack.len())
            .rev()
            .find(|&i| !self.is_internal_frame(i))
        {
            Some(i) => t.with_location(
                || String::from_utf8_lossy(&self.current_file(i)).into_owned(),
                self.current_line(i),
            ),
            None => t,
        }
    }

    /// Frame `i` runs no user code: built-in (prelude) code, or the
    /// finished script's `{main}` that holds the globals while shutdown
    /// callbacks run (php has no frame there).
    pub(crate) fn is_internal_frame(&self, i: usize) -> bool {
        use rhp_bytecode::ProtoFlags as F;
        let flags = self.stack[i].proto.flags;
        flags.intersects(F::INTERNAL.union(F::SHUTDOWN_SCOPE))
    }

    /// Frame `i` is engine plumbing with no php counterpart: a prelude
    /// class's `__`-prefixed helper, or a magic method standing in for
    /// php's property, dimension or `get_method` handlers. Traces leave it
    /// out (see [`Self::dispatched_method`]).
    fn is_plumbing_frame(&self, i: usize) -> bool {
        let f = &self.stack[i];
        f.include_kind().is_none()
            && f.proto.is_internal()
            && (is_engine_helper_name(f.proto.name.as_bytes())
                || f.defining_class.is_some_and(|c| {
                    is_handler_accessor(self.class(c).name.as_bytes(), f.proto.name.as_bytes())
                }))
    }

    /// For a stand-in `__call`/`__callStatic` frame (XMLReader's `open` and
    /// `XML`, static methods that php's `get_method` also runs on an
    /// instance): the stub method it runs and that call's arguments, as a
    /// trace shows them. None when the name is no such method: php fails
    /// before any call.
    fn dispatched_method(&self, i: usize) -> Option<(&'static str, PhpArray)> {
        let f = &self.stack[i];
        let lc = f.proto.name.as_bytes().to_ascii_lowercase();
        if lc != b"__call" && lc != b"__callstatic" {
            return None;
        }
        let class = self.class(f.defining_class?).name.clone();
        let args = self.frame_args(i);
        let mut it = args.iter().map(|(_, v)| v);
        let Some(Value::Str(name)) = it.next() else {
            return None;
        };
        let m = rhp_bytecode::signatures::method(class.as_bytes(), name.as_bytes())?;
        let mut shown = PhpArray::new();
        if let Some(Value::Array(a)) = it.next() {
            for (_, v) in a.iter() {
                shown.push(deref(v));
            }
        }
        Some((m.name, shown))
    }

    /// A builtin called from frame `i` is still running (so a frame above `i`
    /// was invoked by it, i.e. from `[internal function]`).
    pub(crate) fn native_active_from(&self, i: usize) -> bool {
        self.native_calls.iter().any(|n| n.caller == i)
    }

    /// Frame `i` runs a constant-expression thunk (a class constant,
    /// property default or attribute argument): php evaluates those without
    /// a frame of its own.
    pub(crate) fn is_const_expr_frame(&self, i: usize) -> bool {
        self.stack[i]
            .proto
            .flags
            .contains(rhp_bytecode::ProtoFlags::CONST_EXPR)
    }

    /// EG(filename_override)/EG(lineno_override) while a constant expression
    /// is evaluated (zend_ast_evaluate_ex): the innermost thunk's file and
    /// current line (a plain constant name sets none).
    pub(crate) fn const_expr_site(&self) -> Option<(Vec<u8>, u32)> {
        use rhp_bytecode::ProtoFlags as F;
        let i = (0..self.stack.len()).rev().find(|&i| {
            let flags = self.stack[i].proto.flags;
            flags.contains(F::CONST_EXPR) && !flags.contains(F::CONST_EXPR_PLAIN)
        })?;
        if self.autoload_depths.last().is_some_and(|&d| d > i) {
            return None;
        }
        Some((self.current_file(i), self.current_line(i)))
    }

    /// Where php's `zend_get_executed_filename/lineno` point: the innermost
    /// user-code frame's current file and line.
    pub(crate) fn throw_site(&self) -> (Vec<u8>, u32) {
        if let Some(site) = self.const_expr_site() {
            return site;
        }
        // zend_get_executed_filename: "[no active file]" with no user frame.
        match (0..self.stack.len())
            .rev()
            .find(|&i| !self.is_internal_frame(i) && !self.is_const_expr_frame(i))
        {
            Some(i) => (self.current_file(i), self.current_line(i)),
            None => match self.fiber_resumer_site() {
                Some((file, line, false)) => (file, line),
                _ => (b"[no active file]".to_vec(), 0),
            },
        }
    }

    /// The call site of a frame about to be pushed by the current top frame,
    /// when that caller is user code (php appends ", called in %s on line %d"
    /// / "passed in %s on line %d" only then).
    pub(crate) fn user_call_site(&self) -> Option<(Vec<u8>, u32)> {
        let top = self.stack.len().checked_sub(1)?;
        if self.is_internal_frame(top) || self.native_active_from(top) {
            return None;
        }
        Some((self.current_file(top), self.current_line(top)))
    }

    /// php's display name for a function in messages: `Class::fn` when scoped.
    pub(crate) fn func_display_name(
        &self,
        proto: &FunctionProto,
        scope: Option<ClassId>,
    ) -> String {
        let f = String::from_utf8_lossy(proto.name.as_bytes());
        match scope {
            Some(c) => format!(
                "{}::{}",
                String::from_utf8_lossy(self.class(c).name.as_bytes()),
                f
            ),
            None => f.into_owned(),
        }
    }

    /// php's get_function_or_method_name printed with `%s`: the joined
    /// `Class::fn` cut at a NUL, so an anonymous class's method shows as
    /// just `class@anonymous`.
    pub(crate) fn func_c_name(&self, proto: &FunctionProto, scope: Option<ClassId>) -> String {
        let mut s = self.func_display_name(proto, scope);
        if let Some(i) = s.find('\0') {
            s.truncate(i);
        }
        s
    }

    /// The trace `args` of frame `i`: the passed arguments' current values.
    fn frame_args(&self, i: usize) -> PhpArray {
        let f = &self.stack[i];
        let mut args = PhpArray::new();
        let shown = |k: usize, v: Value| {
            if crate::attributes::param_is_sensitive(&f.proto, k) {
                self.sensitive_value(v)
            } else {
                v
            }
        };
        for (k, p) in f.proto.params.iter().enumerate() {
            if p.variadic {
                if let Value::Array(a) = deref(&f.regs[p.cv as usize]) {
                    for (key, v) in a.iter() {
                        match key {
                            ArrayKey::Int(_) => args.push(shown(k, deref(v))),
                            ArrayKey::Str(_) => {
                                args.insert(key.clone(), shown(k, deref(v)));
                            }
                        }
                    }
                }
                break;
            }
            if k >= f.argc {
                break;
            }
            args.push(shown(k, deref(&f.regs[p.cv as usize])));
        }
        for v in &f.extra_args {
            args.push(deref(v));
        }
        args
    }

    /// php's `zend_fetch_debug_backtrace` as stored in a Throwable's `trace`:
    /// innermost first; each user frame paired with its call site (absent for a
    /// call made by a builtin: `[internal function]`), each running builtin as
    /// its own frame, `include`/`require`/`eval` as pseudo-frames.
    fn build_trace(&mut self) -> PhpArray {
        self.build_trace_ex(false, builtins::zend::exceptions::ignore_args())
    }

    /// [`Vm::build_trace`] with debug_backtrace's options: `$trace['object']` for
    /// frames with a `$this`, and no `args`.
    pub(crate) fn build_trace_ex(&mut self, provide_object: bool, ignore_args: bool) -> PhpArray {
        self.build_trace_skip(provide_object, ignore_args, false)
    }

    /// [`Vm::build_trace_ex`]; `skip_last` leaves out the innermost running
    /// builtin (debug_backtrace() itself, zend_fetch_debug_backtrace's
    /// `skip_last`).
    pub(crate) fn build_trace_skip(
        &mut self,
        provide_object: bool,
        ignore_args: bool,
        skip_last: bool,
    ) -> PhpArray {
        let mut out = self.build_trace_part(provide_object, ignore_args, skip_last, true);
        if self.active_fiber.is_some() {
            self.fiber_trace_tail(&mut out, provide_object, ignore_args);
        }
        out
    }

    /// The trace frames of the running stack; `innermost`: it is the
    /// running context's (not a fiber resumer's below it).
    pub(crate) fn build_trace_part(
        &self,
        provide_object: bool,
        ignore_args: bool,
        skip_last: bool,
        innermost: bool,
    ) -> PhpArray {
        let mut skip_native = skip_last;
        // Keys and file names are shared (Rc) rather than re-allocated per
        // frame: a trace is built at every Throwable creation.
        let key = |s: &str| ArrayKey::Str(rhp_values::PhpStr::from(s));
        let keys = [
            key("file"),
            key("line"),
            key("function"),
            key("class"),
            key("object"),
            key("type"),
            key("args"),
        ];
        let k = |s: &[u8]| -> ArrayKey {
            let i = match s {
                b"file" => 0,
                b"line" => 1,
                b"function" => 2,
                b"class" => 3,
                b"object" => 4,
                b"type" => 5,
                _ => 6,
            };
            keys[i].clone()
        };
        let file_of = |i: usize| Value::Str(self.stack[i].proto.debug.file.clone());
        let mut out = PhpArray::new();
        // zend_fetch_debug_backtrace under a filename override: the real
        // execution point, when it differs, as a `[constant expression]`
        // frame.
        if let Some((ofile, oline)) = self.const_expr_site().filter(|_| innermost) {
            // (None when an internal function evaluates it.)
            let user = (0..self.stack.len())
                .rev()
                .find(|&i| !self.is_const_expr_frame(i))
                .filter(|&i| !self.is_internal_frame(i));
            if let Some(i) = user {
                let (file, line) = (self.current_file(i), self.current_line(i));
                if file != ofile || line != oline {
                    let mut e = PhpArray::new();
                    e.insert(k(b"file"), Value::str(file));
                    e.insert(k(b"line"), Value::Int(line as i64));
                    e.insert(k(b"function"), Value::str("[constant expression]"));
                    out.push(Value::Array(e));
                }
            }
        }
        // php's `include_filename`: the call-site file of the last frame
        // added, the argument an include frame shows.
        let mut include_file: Option<Value> = None;
        for i in (0..self.stack.len()).rev() {
            // A builtin the engine called with no frame running (a shutdown
            // function) shows as `[internal function]`.
            let no_frame = self.stack[i]
                .proto
                .flags
                .contains(rhp_bytecode::ProtoFlags::SHUTDOWN_SCOPE);
            if !self.is_internal_frame(i) || no_frame {
                for n in self.native_calls.iter().rev().filter(|n| n.caller == i) {
                    if std::mem::take(&mut skip_native) {
                        continue;
                    }
                    let mut e = PhpArray::new();
                    if no_frame {
                        include_file = None;
                    } else {
                        include_file = Some(file_of(i));
                        e.insert(k(b"file"), file_of(i));
                        e.insert(k(b"line"), Value::Int(self.current_line(i) as i64));
                    }
                    e.insert(k(b"function"), Value::str(n.name));
                    if !ignore_args {
                        let mut a = PhpArray::new();
                        // SAFETY: see `NativeCall::args` — every entry
                        // still in `native_calls` points at a live slice.
                        let call_args = unsafe { &*n.args };
                        let sig = rhp_bytecode::signatures::function(n.name.as_bytes());
                        for (j, v) in call_args.iter().enumerate() {
                            use rhp_bytecode::signatures::Signature;
                            if sig.and_then(|s| s.param(j)).is_some_and(|p| p.sensitive()) {
                                a.push(self.sensitive_value(deref(v)));
                            } else if j < 64 && n.shown_str >> j & 1 != 0 {
                                a.push(Value::str(rhp_values::conv::to_string_bytes(&deref(v))));
                            } else {
                                a.push(deref(v));
                            }
                        }
                        // SAFETY: as for `args`.
                        for (name, v) in unsafe { &*n.named } {
                            a.insert(ArrayKey::Str(name.clone()), deref(v));
                        }
                        e.insert(k(b"args"), Value::Array(a));
                    }
                    out.push(Value::Array(e));
                }
            }
            skip_native = false;
            if i == 0 {
                break;
            }
            let f = &self.stack[i];
            if self.is_const_expr_frame(i)
                || f.proto.flags.contains(rhp_bytecode::ProtoFlags::CALL_SITE)
            {
                continue;
            }
            let shown_as = if self.is_plumbing_frame(i) {
                match self.dispatched_method(i) {
                    Some(m) => Some(m),
                    None => continue,
                }
            } else {
                None
            };
            // The call site is the nearest frame below that a trace shows.
            let mut caller = i - 1;
            while caller > 0
                && (self.is_const_expr_frame(caller)
                    || self.is_plumbing_frame(caller) && self.dispatched_method(caller).is_none())
            {
                caller -= 1;
            }
            let mut e = PhpArray::new();
            if !self.is_internal_frame(caller) && !self.native_active_from(caller) {
                e.insert(k(b"file"), file_of(caller));
                e.insert(k(b"line"), Value::Int(self.current_line(caller) as i64));
            }
            if let Some(kind) = f.include_kind() {
                if kind != "eval" {
                    if let Some(file) = &include_file {
                        let mut a = PhpArray::new();
                        a.push(file.clone());
                        e.insert(k(b"args"), Value::Array(a));
                    }
                }
                e.insert(k(b"function"), Value::str(kind));
            } else if let Some((name, args)) = shown_as {
                let c = f.defining_class.expect("a method frame");
                e.insert(k(b"function"), Value::str(name));
                e.insert(k(b"class"), Value::Str(self.class(c).name.clone()));
                let this = self.current_this(i);
                let ty = if this.is_some() { "->" } else { "::" };
                if let (true, Some(o)) = (provide_object, this) {
                    e.insert(k(b"object"), Value::Object(o));
                }
                e.insert(k(b"type"), Value::str(ty));
                if !ignore_args {
                    e.insert(k(b"args"), Value::Array(args));
                }
            } else {
                e.insert(k(b"function"), Value::Str(f.proto.name.clone()));
                if let Some(c) = f.defining_class {
                    e.insert(k(b"class"), Value::Str(self.class(c).name.clone()));
                    let this = self.current_this(i);
                    let ty = if this.is_some() { "->" } else { "::" };
                    if let (true, Some(o)) = (provide_object, this) {
                        e.insert(k(b"object"), Value::Object(o));
                    }
                    e.insert(k(b"type"), Value::str(ty));
                }
                if !ignore_args {
                    e.insert(k(b"args"), Value::Array(self.frame_args(i)));
                }
            }
            let file = e.get(&k(b"file")).cloned();
            out.push(Value::Array(e));
            // A frame the engine called (an error handler, an autoloader)
            // while its caller runs an include that has not started a frame
            // yet: php adds the include as a frame of its own.
            if f.include_kind().is_none() && !self.is_internal_frame(caller) {
                let cf = &self.stack[caller];
                let at = cf.pc.checked_sub(1).and_then(|pc| cf.proto.code.get(pc));
                if let Some(rhp_bytecode::Instr::Include { kind, .. }) = at {
                    let mut e = PhpArray::new();
                    e.insert(k(b"file"), file_of(caller));
                    e.insert(k(b"line"), Value::Int(self.current_line(caller) as i64));
                    if let Some(file) = &include_file {
                        let mut a = PhpArray::new();
                        a.push(file.clone());
                        e.insert(k(b"args"), Value::Array(a));
                    }
                    e.insert(k(b"function"), Value::str(kind.name()));
                    out.push(Value::Array(e));
                }
            }
            include_file = file;
        }
        out
    }

    /// Stamp a freshly created Throwable with php's creation-site `file`,
    /// `line` and `trace` (zend_default_exception_new).
    pub(crate) fn init_throwable(&mut self, obj: &ObjectHandle) {
        let (file, line) = self.throw_site();
        let trace = self.build_trace();
        let (kf, kl, kt) = (
            self.declared_prop_key(obj, b"file"),
            self.declared_prop_key(obj, b"line"),
            self.declared_prop_key(obj, b"trace"),
        );
        let mut b = obj.borrow_mut();
        b.properties.insert(kf, Value::str(file));
        b.properties.insert(kl, Value::Int(line as i64));
        b.properties.insert(kt, Value::Array(trace));
    }

    /// Turn an engine-raised `Thrown::Error` into a real Throwable object, created
    /// *now* (so its file/line/trace are the throw point). `at` overrides the
    /// file/line (argument errors are raised at the callee's parameter line).
    pub(crate) fn materialize(&mut self, thrown: Thrown, at: Option<(Vec<u8>, u32)>) -> Thrown {
        let Thrown::Error { class, message } = &thrown else {
            return thrown;
        };
        if crate::fibers::is_graceful_exit(&thrown) {
            return thrown;
        }
        let Ok(cls) = self.resolve_class_by_name(class.as_bytes()) else {
            return thrown;
        };
        let message = self.cut_anon_names(message.as_bytes());
        let obj = self.new_object(cls);
        {
            let mut b = obj.borrow_mut();
            b.properties
                .insert(rhp_values::PhpStr::from("message"), Value::str(message));
            if let Some((file, line)) = at {
                b.properties
                    .insert(rhp_values::PhpStr::from("file"), Value::str(file));
                b.properties
                    .insert(rhp_values::PhpStr::from("line"), Value::Int(line as i64));
            }
        }
        Thrown::Object(obj)
    }

    /// An engine message with each anonymous class name cut at its NUL, as
    /// php's `%s` prints `ZSTR_VAL(ce->name)`.
    pub(crate) fn cut_anon_names(&self, msg: &[u8]) -> Vec<u8> {
        const MARK: &[u8] = b"@anonymous\0";
        if !msg.windows(MARK.len()).any(|w| w == MARK) {
            return msg.to_vec();
        }
        // The `\0file:line$n` tails of the anonymous classes declared so far.
        let tails: Vec<&[u8]> = self
            .classes
            .iter()
            .filter_map(|c| {
                let n = c.name.as_bytes();
                let at = n.windows(MARK.len()).position(|w| w == MARK)?;
                Some(&n[at + MARK.len() - 1..])
            })
            .collect();
        let mut out = Vec::with_capacity(msg.len());
        let mut i = 0;
        while i < msg.len() {
            if msg[i..].starts_with(MARK) {
                out.extend_from_slice(&MARK[..MARK.len() - 1]);
                i += MARK.len() - 1;
                if let Some(t) = tails
                    .iter()
                    .filter(|t| msg[i..].starts_with(t))
                    .max_by_key(|t| t.len())
                {
                    i += t.len();
                }
                continue;
            }
            out.push(msg[i]);
            i += 1;
        }
        out
    }

    /// Give an uncaught throwable to the user exception handler, if one is set
    /// (zend_user_exception_handler). `None`: handled; `Some(t)`: still
    /// uncaught (no handler, or the handler itself threw `t`).
    pub fn handle_uncaught(&mut self, thrown: Thrown) -> Option<Thrown> {
        if !thrown.is_catchable() {
            return Some(thrown);
        }
        let handler = match self.exception_handlers.last() {
            Some(h) if !matches!(h, Value::Null) => h.clone(),
            _ => return Some(thrown),
        };
        let thrown = self.materialize(thrown, None);
        let Thrown::Object(obj) = &thrown else {
            return Some(thrown);
        };
        let arg = Value::Object(obj.clone());
        // The handler runs on the shutdown scope: {main}'s frame stays alive
        // (its globals are still reachable via `global`, and its objects are
        // destructed only after the shutdown functions).
        self.enter_shutdown_scope();
        let before = self.exception_handlers.len();
        let r = self.call_callable_value(&handler, vec![arg]);
        self.stack.truncate(1);
        match r {
            Ok(_) => None,
            // zend_throw_exception_internal with no frame running: a handler
            // the failed one installed gets the new throwable.
            Err(t) if self.exception_handlers.len() > before => self.handle_uncaught(t),
            Err(t) => Some(t),
        }
    }

    /// Write to the SAPI's error stream.
    pub(crate) fn write_stderr(&mut self, data: &[u8]) {
        self.sapi.sapi.write_stderr(data);
    }

    /// php_error_cb's log + display step for an error of type `ty` (an E_*
    /// bit): skipped when masked by error_reporting; logged to stderr as
    /// `PHP <label>:  …` under log_errors; displayed per display_errors
    /// (stdout through the output buffers, or stderr without the leading
    /// newline for "stderr").
    pub(crate) fn display_error(
        &mut self,
        ty: i64,
        label: &str,
        message: &[u8],
        file: &[u8],
        line: i64,
    ) {
        self.display_error_trace(ty, label, message, file, line, b"");
    }

    /// [`Vm::display_error`] with php 8.5's `fatal_error_backtraces` block
    /// (`Stack trace:\n#0 …`) after the location.
    pub(crate) fn display_error_trace(
        &mut self,
        ty: i64,
        label: &str,
        message: &[u8],
        file: &[u8],
        line: i64,
        trace: &[u8],
    ) {
        use builtins::{E_COMPILE_ERROR, E_CORE_ERROR, E_ERROR, E_PARSE, E_RECOVERABLE_ERROR};
        let fatal = E_ERROR
            | E_PARSE
            | E_CORE_ERROR
            | E_COMPILE_ERROR
            | builtins::E_USER_ERROR
            | E_RECOVERABLE_ERROR;
        if ty & fatal != 0 {
            // php_error_cb's bailout: a 200 becomes a 500 when nothing shows.
            self.fatal_status();
        }
        if builtins::zend::builtin_functions::error_reporting_level(self) & ty == 0 {
            return;
        }
        let ini = |vm: &Self, n: &[u8]| vm.ini_get(n).unwrap_or_default();
        let log = ini_bool(&ini(self, b"log_errors"));
        let mut mode = display_errors_mode(&ini(self, b"display_errors"));
        if matches!(mode, DisplayErrors::Stderr)
            && !matches!(
                builtins::main::sapi::sapi_name().as_str(),
                "cli" | "cgi" | "phpdbg"
            )
        {
            // Only the cli / cgi / phpdbg SAPIs honour display_errors=stderr.
            mode = DisplayErrors::Stdout;
        }
        let mut tail = b" in ".to_vec();
        tail.extend_from_slice(file);
        tail.extend_from_slice(format!(" on line {line}\n").as_bytes());
        if !trace.is_empty() {
            tail.extend_from_slice(b"Stack trace:\n");
            tail.extend_from_slice(trace);
            tail.push(b'\n');
        }
        if log {
            let mut m = format!("PHP {label}:  ").into_bytes();
            m.extend_from_slice(message);
            m.extend_from_slice(&tail);
            let target = ini(self, b"error_log");
            if target.is_empty() || target == b"syslog" {
                // error_log unset: the SAPI's log_message (the CLI's writes
                // to stderr).
                let line = m.strip_suffix(b"\n").unwrap_or(&m).to_vec();
                builtins::Host::log_message(self, &line);
            } else {
                // php_log_err: append "[d-M-Y H:i:s e] message" to the file.
                let stamp = self
                    .invoke_builtin("date", vec![Value::str("d-M-Y H:i:s e")], false)
                    .map(|v| conv::to_string_bytes(&v))
                    .unwrap_or_default();
                let mut line = b"[".to_vec();
                line.extend_from_slice(&stamp);
                line.extend_from_slice(b"] ");
                line.extend_from_slice(&m);
                let mode = builtins::main::ini::error_log_mode();
                if builtins::zend::virtual_cwd::append_file(&target, &line, mode).is_err() {
                    self.write_stderr(&m);
                }
            }
        }
        let html = ini_bool(&ini(self, b"html_errors"));
        if html && !matches!(mode, DisplayErrors::Off) {
            // php_error_cb's html branch: E_ERROR / E_PARSE escape the
            // message and drop the trace; others show it as given (a
            // docref message was escaped when it was made, `docref_html`).
            let fatal = ty == builtins::E_ERROR || ty == builtins::E_PARSE;
            let mut m = ini(self, b"error_prepend_string");
            m.extend_from_slice(format!("<br />\n<b>{label}</b>:  ").as_bytes());
            if fatal {
                m.extend_from_slice(&builtins::main::sapi::escape_html(self, message));
            } else {
                m.extend_from_slice(message);
            }
            m.extend_from_slice(b" in <b>");
            m.extend_from_slice(file);
            m.extend_from_slice(format!("</b> on line <b>{line}</b><br />").as_bytes());
            if !fatal && !trace.is_empty() {
                m.extend_from_slice(b"\nStack trace:\n");
                m.extend_from_slice(trace);
            }
            m.push(b'\n');
            m.extend_from_slice(&ini(self, b"error_append_string"));
            self.write_out(&m);
            return;
        }
        match mode {
            DisplayErrors::Stderr => {
                let mut m = format!("{label}: ").into_bytes();
                m.extend_from_slice(message);
                m.extend_from_slice(&tail);
                self.write_stderr(&m);
            }
            DisplayErrors::Stdout => {
                let mut m = ini(self, b"error_prepend_string");
                m.extend_from_slice(format!("\n{label}: ").as_bytes());
                m.extend_from_slice(message);
                m.extend_from_slice(&tail);
                m.extend_from_slice(&ini(self, b"error_append_string"));
                self.write_out(&m);
            }
            DisplayErrors::Off => {}
        }
    }

    /// zend_exception_error(ex, E_ERROR) for an uncaught throwable: a
    /// ParseError reports as `Parse error: <message>`, anything else as
    /// `Fatal error: Uncaught <__toString()>\n  thrown`.
    /// php_error_cb for a fatal error: recorded for `error_get_last()` (even
    /// when not displayed), then logged / displayed.
    fn fatal_error(&mut self, ty: i64, label: &str, message: &[u8], file: &[u8], line: i64) {
        self.last_error = Some((ty, message.to_vec(), file.to_vec(), line));
        self.display_error(ty, label, message, file, line);
    }

    pub(crate) fn report_uncaught(&mut self, thrown: Thrown) {
        self.report_uncaught_as(thrown, 1, "Fatal error")
    }

    /// zend_exception_error(ex, severity): [`Vm::report_uncaught`] at
    /// level `ty` (assert.bail reports a callback's exception as a warning).
    pub(crate) fn report_uncaught_as(&mut self, thrown: Thrown, ty: i64, label: &str) {
        self.last_fatal_trace = None;
        let thrown = self.materialize(thrown, None);
        let h = match thrown {
            Thrown::Object(h) => h,
            Thrown::Error { class, message } => {
                let (file, line) = self.throw_site();
                let msg = format!("Uncaught {class}: {message}");
                self.fatal_error(ty, label, msg.as_bytes(), &file, line as i64);
                return;
            }
            _ => return,
        };
        let cls = h.borrow().class;
        let prop = |n: &str| {
            h.borrow()
                .properties
                .get(&rhp_values::PhpStr::from(n))
                .map(deref)
                .unwrap_or(Value::Null)
        };
        // An emptied `file` makes php report the location as "Unknown".
        let location = || {
            let file = conv::to_string_bytes(&prop("file"));
            let file = if file.is_empty() {
                b"Unknown".to_vec()
            } else {
                file
            };
            (file, conv::to_int(&prop("line")))
        };
        let cname = self.class(cls).name.as_bytes().to_ascii_lowercase();
        if cname == b"parseerror" || cname == b"compileerror" {
            let (file, line) = location();
            let file = &file[..];
            let msg = conv::to_string_bytes(&prop("message"));
            let (ty, label) = if cname == b"parseerror" {
                (4, "Parse error")
            } else {
                (64, "Fatal error")
            };
            self.fatal_error(ty, label, &msg, file, line);
            return;
        }
        // zend_exception_error: `__toString()` caches its result in the
        // private `string`, which is what the message shows. A throwable
        // it throws is reported instead (php's zend_throw_exception_internal
        // with no frame running).
        match self.call_to_string(&h) {
            Ok(Value::Str(s)) => {
                let key = self.declared_prop_key(&h, b"string");
                h.borrow_mut().properties.insert(key, Value::Str(s));
            }
            Ok(_) => {
                let name = self.class(cls).name.clone();
                let mut m = String::from_utf8_lossy(c_name(name.as_bytes())).into_owned();
                m.push_str("::__toString() must return a string");
                self.raise_error(builtins::E_WARNING, &m);
            }
            Err(t) if t.is_catchable() => {
                self.report_uncaught_as(t, ty, label);
                return;
            }
            Err(_) => return,
        }
        let key = self.declared_prop_key(&h, b"string");
        let s = h
            .borrow()
            .properties
            .get(&key)
            .map(|v| conv::to_string_bytes(&deref(v)))
            .unwrap_or_default();
        let mut msg = b"Uncaught ".to_vec();
        msg.extend_from_slice(&s);
        msg.extend_from_slice(b"\n  thrown");
        let (file, line) = location();
        self.fatal_error(ty, label, &msg, &file, line);
    }

    /// An exception that escaped to the top level: the user handler gets it
    /// first, else it is reported. `Ok(None)`: handled, carry on; `Ok(Some(c))`:
    /// execution ends with status `c` (255 after a fatal, or an `exit(c)`);
    /// `Err`: an engine refusal the SAPI reports itself.
    pub(crate) fn top_level_throw(&mut self, thrown: Thrown) -> Result<Option<i32>, Thrown> {
        match thrown {
            Thrown::Exit(c) => Ok(Some(c)),
            t if t.is_catchable() => match self.handle_uncaught(t) {
                None => Ok(None),
                Some(Thrown::Exit(c)) => Ok(Some(c)),
                Some(t) if t.is_catchable() => {
                    self.report_uncaught(t);
                    Ok(Some(255))
                }
                Some(t) => Err(t),
            },
            t => Err(t),
        }
    }

    /// An exception was caught at frame depth `depth`: end every `@` region
    /// opened at or above it (php's live-range cleanup of ZEND_END_SILENCE).
    fn unsilence_frames(&mut self, depth: usize) {
        while let Some(&(d, _, _)) = self.silence_stack.last() {
            if d < depth {
                break;
            }
            self.end_silence();
        }
    }

    /// ZEND_BEGIN_SILENCE: error_reporting drops to the fatal types (php
    /// keeps the old value to put back).
    pub(crate) fn begin_silence(&mut self) {
        let saved = builtins::zend::builtin_functions::error_reporting_level(self);
        self.silence_stack
            .push((self.stack.len(), self.silence, saved));
        self.silence += 1;
        if saved & !SILENCE_KEEPS != 0 {
            self.set_error_reporting(saved & SILENCE_KEEPS);
        }
    }

    /// ZEND_END_SILENCE (also its live-range cleanup when an exception
    /// leaves the region): the old error_reporting comes back unless the
    /// code under `@` set one that shows more than fatal errors.
    pub(crate) fn end_silence(&mut self) {
        let Some((_, lvl, saved)) = self.silence_stack.pop() else {
            return;
        };
        self.silence = lvl;
        let cur = builtins::zend::builtin_functions::error_reporting_level(self);
        if cur & !SILENCE_KEEPS == 0 && saved & !SILENCE_KEEPS != 0 {
            self.set_error_reporting(saved);
        }
    }

    pub(crate) fn set_error_reporting(&mut self, v: i64) {
        builtins::Host::ini_set(self, b"error_reporting", v.to_string().into_bytes());
    }

    /// Unwind a thrown exception through the exception table: find a matching
    /// catch (bind + jump) or finally (re-raise after) in the current frame,
    /// else pop and continue in the caller. Returns `Ok` when a handler is
    /// installed (re-enter `run_frames`), `Err` when it escapes the top frame.
    pub(crate) fn unwind(&mut self, thrown: Thrown, floor: usize) -> Result<UnwindResult, Thrown> {
        // An engine error becomes a real object here, before any frame is popped,
        // so its file/line/trace describe the throw point.
        let mut thrown = self.materialize(thrown, None);
        'unwind: loop {
            if self.stack.len() <= floor {
                return Ok(UnwindResult::Escaped(thrown)); // escaped the (sub)run
            }
            let top = self.stack.len() - 1;
            let proto = self.stack[top].proto.clone();
            // The throwing pc has already been advanced past the faulting instr.
            let throw_pc = (self.stack[top].pc as u32).saturating_sub(1);
            // The live range of NEW's result: an object whose constructor
            // threw is released before the handler runs.
            if let Some(
                Instr::New { dst, .. }
                | Instr::NewV { dst, .. }
                | Instr::NewCall { dst, .. }
                | Instr::NewCallV { dst, .. },
            ) = proto.code.get(throw_pc as usize)
            {
                self.reg_set(top, *dst, Value::Null);
            }
            // Thrown while its arguments ran: the object is released
            // without its destructor (zend_cleanup_unfinished_calls).
            for &(start, end, dst) in &proto.new_ranges {
                if throw_pc > start && throw_pc < end {
                    // zend_vm_stack_free_args first: the arguments evaluated
                    // so far (the temporaries above the object) go before it.
                    let n = self.stack[top].regs.len();
                    for r in dst as usize + 1..n {
                        if !matches!(self.stack[top].regs[r], Value::Undef | Value::Null) {
                            self.reg_set(top, r as rhp_bytecode::Reg, Value::Null);
                        }
                    }
                    if let Value::Object(h) = self.reg_get(top, dst) {
                        self.destructibles.retain(|d| !Rc::ptr_eq(d, &h));
                    }
                    self.reg_set(top, dst, Value::Null);
                }
            }
            let in_try = |r: &&rhp_bytecode::ExRegion| throw_pc >= r.start && throw_pc < r.end;
            let handler = proto.exception_table.iter().find(in_try).cloned();
            // Thrown from inside a running finally block and not handled
            // within it: the exception (or return) that block was finishing
            // is discarded, and a pending exception becomes the innermost
            // `previous` of the new one (zend_exception_set_previous).
            let in_finally = proto.exception_table.iter().filter(|r| {
                r.finally
                    .is_some_and(|f| throw_pc >= f && throw_pc <= r.finally_end)
            });
            for f in in_finally {
                let fin = f.finally.unwrap_or(0);
                let handled_inside = handler
                    .as_ref()
                    .is_some_and(|h| h.start >= fin && h.end <= f.finally_end);
                if !handled_inside {
                    let pend = &mut self.stack[top].pending;
                    if let Some(i) = pend.iter().position(|(k, _)| *k == fin) {
                        if let (_, FinallyCont::Rethrow(prev)) = pend.remove(i) {
                            self.chain_previous(&thrown, prev);
                        }
                    }
                }
            }
            // The innermost try first; one whose catches do not match and
            // that has no finally passes the exception to the next one out.
            let regions: Vec<rhp_bytecode::ExRegion> = proto
                .exception_table
                .iter()
                .filter(in_try)
                .cloned()
                .collect();
            for region in regions {
                // Destructed as they are freed, before the handler: the
                // try's temporaries, and a failed builtin's arguments.
                self.free_try_temps(top, region.temps);
                if !self.destructibles.is_empty() {
                    match self.collect_released() {
                        Ok(()) => {}
                        Err(e) if !e.is_catchable() => return Err(e),
                        Err(e) => {
                            let e = self.materialize(e, None);
                            self.chain_previous(&e, thrown);
                            thrown = e;
                        }
                    }
                }
                if let Some(cls) = self.thrown_class_id(&thrown) {
                    for arm in &region.catches {
                        if arm
                            .classes
                            .iter()
                            .any(|cn| self.class_matches(cls, cn.as_bytes()))
                        {
                            let obj = self.thrown_to_object(&thrown)?;
                            // php assigns the catch variable like `$e = ...`:
                            // through a reference it holds (a global's cell).
                            // At the catch line: a typed reference may refuse it, and
                            // the old value's destructor runs there (ZEND_CATCH),
                            // outside the try.
                            if let Some(vr) = arm.var_reg {
                                self.stack[top].pc = arm.target as usize + 1;
                                if let Err(e) = self.cv_assign_as(top, vr, obj, true) {
                                    thrown = self.materialize(e, None);
                                    continue 'unwind;
                                }
                            } else {
                                self.stack[top].pc = arm.target as usize;
                            }
                            self.unsilence_frames(top + 1);
                            return Ok(UnwindResult::Caught);
                        }
                    }
                }
                if let Some(fin) = region.finally {
                    self.push_pending(top, fin, FinallyCont::Rethrow(thrown));
                    self.stack[top].pc = fin as usize;
                    self.unsilence_frames(top + 1);
                    return Ok(UnwindResult::Caught);
                }
            }
            // cleanup_live_vars: the frame's temporaries go before it is
            // left, their destructors running at the throwing line.
            if self.free_try_temps(top, proto.num_cvs) && !self.destructibles.is_empty() {
                match self.collect_released() {
                    Ok(()) => {}
                    Err(e) if !e.is_catchable() => return Err(e),
                    Err(e) => {
                        let e = self.materialize(e, None);
                        self.chain_previous(&e, thrown);
                        thrown = e;
                    }
                }
            }
            let popped = self.stack.pop();
            if let Some(f) = &popped {
                self.include_writeback(f);
            }
            let depth = self.stack.len();
            if !self.readonly_reinit.is_empty() {
                self.close_reinit(depth);
            }
            if let Some((_, ptr)) = self.ctor_frames.pop_if(|c| c.0 == depth) {
                // The constructor threw: its object will never be destructed.
                self.destructibles.retain(|d| Rc::as_ptr(d) as usize != ptr);
            }
            if self.stack.is_empty() && floor == 0 {
                // An uncaught exception left {main}: keep its globals.
                self.finished_main = popped;
                continue;
            }
            // zend_leave_helper: the frame's variables are freed before the
            // caller handles the exception, so its locals are destructed
            // ahead of the caller's catch / finally.
            drop(popped);
            if let Err(e) = self.collect_released() {
                if !e.is_catchable() {
                    // `exit` in such a destructor discards the exception.
                    return Err(e);
                }
                // A destructor threw: the new exception carries on, with the
                // unwinding one as its innermost previous — unless no php
                // frame is left below (a destructor or callback run from
                // shutdown): php frees a top call's variables after
                // restoring the NULL execute_data, so the new exception is
                // reported on its own and the unwinding one is dropped.
                let e = self.materialize(e, None);
                if !self.stack.is_empty() && !self.no_code_running() {
                    self.chain_previous(&e, thrown);
                }
                thrown = e;
            }
        }
    }

    /// The temporaries of an unwound try body (from `from` up), freed in
    /// order before its handler runs.
    fn free_try_temps(&mut self, top: usize, from: rhp_bytecode::Reg) -> bool {
        // The loops inside the try give up their iterators.
        let f = &mut self.stack[top];
        if let Some(live) = f.fe_live.as_deref_mut() {
            for (_, arr, by_ref, id) in live.extract_if(.., |l| l.0 >= from) {
                crate::dispatch::release_fe_iter(&mut f.regs, arr, by_ref, id);
            }
        }
        let n = self.stack[top].regs.len();
        let mut freed = false;
        for r in from as usize..n {
            if matches!(self.stack[top].regs[r], Value::Object(_) | Value::Array(_)) {
                freed = true;
            }
            if !matches!(self.stack[top].regs[r], Value::Undef | Value::Null) {
                self.reg_set(top, r as rhp_bytecode::Reg, Value::Null);
            }
        }
        freed
    }

    /// Record what the finally block starting at `fin` does when it ends.
    pub(crate) fn push_pending(&mut self, top: usize, fin: u32, cont: FinallyCont) {
        let pend = &mut self.stack[top].pending;
        pend.retain(|(k, _)| *k != fin);
        pend.push((fin, cont));
    }

    /// A `return`/`break` leaving the finally blocks that contain `pc`
    /// discards whatever those blocks were finishing.
    pub(crate) fn drop_left_finallies(&mut self, top: usize, proto: &FunctionProto, pc: u32) {
        if self.stack[top].pending.is_empty() {
            return;
        }
        for r in &proto.exception_table {
            if let Some(f) = r.finally {
                if pc >= f && pc <= r.finally_end {
                    self.stack[top].pending.retain(|(k, _)| *k != f);
                }
            }
        }
    }

    /// Jump to `target`, first running the innermost enclosing finally that
    /// `target` is outside of (the jump resumes when that block ends).
    pub(crate) fn jump_through_finally(
        &mut self,
        top: usize,
        proto: &FunctionProto,
        pc: u32,
        target: u32,
    ) {
        if let Some(f) = enclosing_finally(proto, pc, Some(target)) {
            self.push_pending(top, f, FinallyCont::Jump(target));
            self.stack[top].pc = f as usize;
        } else {
            self.stack[top].pc = target as usize;
        }
    }

    /// zend_exception_set_previous: append `prev` at the end of `thrown`'s
    /// `previous` chain, unless it is already part of it (no cycles).
    pub(crate) fn chain_previous(&self, thrown: &Thrown, prev: Thrown) {
        let (Thrown::Object(ex), Thrown::Object(add)) = (thrown, &prev) else {
            return;
        };
        if Rc::ptr_eq(ex, add) {
            return;
        }
        let previous_of = |o: &ObjectHandle| match o
            .borrow()
            .properties
            .get(&self.declared_prop_key(o, b"previous"))
            .map(deref)
        {
            Some(Value::Object(p)) => Some(p),
            _ => None,
        };
        // `add`'s own chain must not already contain `ex`.
        let mut anc = previous_of(add);
        while let Some(a) = anc {
            if Rc::ptr_eq(&a, ex) {
                return;
            }
            anc = previous_of(&a);
        }
        let mut cur = ex.clone();
        loop {
            match previous_of(&cur) {
                None => {
                    let key = self.declared_prop_key(&cur, b"previous");
                    cur.borrow_mut()
                        .properties
                        .insert(key, Value::Object(add.clone()));
                    return;
                }
                Some(p) if Rc::ptr_eq(&p, add) => return,
                Some(p) => cur = p,
            }
        }
    }

    fn thrown_class_id(&self, thrown: &Thrown) -> Option<ClassId> {
        match thrown {
            Thrown::Object(h) => Some(h.borrow().class),
            Thrown::Error { .. } if crate::fibers::is_graceful_exit(thrown) => None,
            Thrown::Error { class, .. } => self.resolve_class_by_name(class.as_bytes()).ok(),
            _ => None,
        }
    }

    fn class_matches(&self, cls: ClassId, catch_name: &[u8]) -> bool {
        match self.resolve_class_by_name(catch_name) {
            Ok(target) => self.class(cls).ancestors.contains(&target),
            Err(_) => false,
        }
    }

    /// Materialize the thrown value as a `Throwable` object (instantiating the
    /// named class for an engine `Error`).
    fn thrown_to_object(&mut self, thrown: &Thrown) -> Result<Value, Thrown> {
        match thrown {
            Thrown::Object(h) => Ok(Value::Object(h.clone())),
            Thrown::Error { class, message } => {
                let cls = self.resolve_class_by_name(class.as_bytes())?;
                self.update_class_constants(cls)?;
                let obj = self.new_object(cls);
                obj.borrow_mut().properties.insert(
                    rhp_values::PhpStr::from("message"),
                    Value::str(message.clone()),
                );
                Ok(Value::Object(obj))
            }
            _ => Err(thrown.clone()),
        }
    }

    /// php_verror's html_errors step: a `php_error_docref` message is
    /// escaped as it is made, so the user error handler and
    /// `error_get_last()` see it escaped too.
    pub(crate) fn docref_html<'a>(&mut self, msg: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        // ASCII without `&<>"` escapes to itself in every charset html.c
        // knows, so most messages need no ini lookup.
        let plain = |b: &u8| b.is_ascii() && !matches!(b, b'&' | b'<' | b'>' | b'"');
        let escaped = if !msg.iter().all(plain)
            && ini_bool(&self.ini_get(b"html_errors").unwrap_or_default())
        {
            std::borrow::Cow::Owned(builtins::main::sapi::escape_html(self, msg))
        } else {
            std::borrow::Cow::Borrowed(msg)
        };
        match self.docref_link(&escaped) {
            Some(linked) => std::borrow::Cow::Owned(linked),
            None => escaped,
        }
    }

    /// php_verror's manual link: with html_errors and docref_root set, the
    /// origin `func(…)` is followed by ` [<a href='…'>function.func</a>]`.
    fn docref_link(&self, msg: &[u8]) -> Option<Vec<u8>> {
        let root = self.ini_get(b"docref_root").filter(|r| !r.is_empty())?;
        if !ini_bool(&self.ini_get(b"html_errors").unwrap_or_default()) {
            return None;
        }
        let n = msg
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'\\'))
            .count();
        let close = n + msg[n..].iter().position(|&c| c == b')')?;
        if !msg[close + 1..].starts_with(b": ") {
            return None;
        }
        let name = &msg[..n];
        let mut docref = match name.windows(2).position(|w| w == b"::") {
            Some(p) => [&name[..p], b".", &name[p + 2..]].concat(),
            None => {
                let f = name
                    .iter()
                    .position(|&c| c != b'_')
                    .map_or(&name[..0], |i| &name[i..]);
                [&b"function."[..], f].concat()
            }
        };
        for c in docref.iter_mut() {
            if *c == b'_' {
                *c = b'-';
            }
            c.make_ascii_lowercase();
        }
        docref.extend_from_slice(&self.ini_get(b"docref_ext").unwrap_or_default());
        let mut out = msg[..=close].to_vec();
        out.extend_from_slice(b" [<a href='");
        out.extend_from_slice(&root);
        out.extend_from_slice(&docref);
        out.extend_from_slice(b"'>");
        out.extend_from_slice(&docref);
        out.extend_from_slice(b"</a>]");
        out.extend_from_slice(&msg[close + 1..]);
        Some(out)
    }

    /// Raise a diagnostic from inside an instruction; an exception thrown by
    /// the user error handler is returned to be thrown at this point.
    pub(crate) fn diag(
        &mut self,
        level: i64,
        msg: &(impl AsRef<[u8]> + ?Sized),
    ) -> Result<(), Thrown> {
        self.raise_error(level, msg);
        match self.pending_throw.take() {
            Some(t) => Err(t),
            None => Ok(()),
        }
    }
}

/// Whether a diagnostic text is a builtin's `php_error_docref` message:
/// it starts with `name(…): ` (a function or `Class::method`).
pub(crate) fn is_docref_message(b: &[u8]) -> bool {
    let n = b
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'\\'))
        .count();
    if n == 0 || b.get(n) != Some(&b'(') {
        return false;
    }
    match b[n..].iter().position(|&c| c == b')') {
        Some(close) => b[n + close + 1..].starts_with(b": "),
        None => false,
    }
}

pub(crate) fn display_errors_mode(v: &[u8]) -> DisplayErrors {
    let lc = v.to_ascii_lowercase();
    match lc.as_slice() {
        b"on" | b"yes" | b"true" | b"stdout" => DisplayErrors::Stdout,
        b"stderr" => DisplayErrors::Stderr,
        _ => match builtins::c_atol(v) as u8 {
            0 => DisplayErrors::Off,
            2 => DisplayErrors::Stderr,
            _ => DisplayErrors::Stdout,
        },
    }
}

/// The innermost try/catch with a `finally` whose protected range holds `pc`
/// (and, for a jump, does not hold `target`): its finally block's start pc.
pub(crate) fn enclosing_finally(
    proto: &FunctionProto,
    pc: u32,
    target: Option<u32>,
) -> Option<u32> {
    proto.exception_table.iter().find_map(|r| {
        let f = r.finally?;
        let holds = |p: u32| p >= r.start && p < r.end;
        (holds(pc) && !target.is_some_and(holds)).then_some(f)
    })
}

/// The current line of frame `f`.
pub(crate) fn frame_line(f: &Frame) -> u32 {
    let pc = f.pc.saturating_sub(1) as u32;
    let lines = &f.proto.debug.lines;
    // Largest (pc,line) entry whose pc <= the current pc.
    let idx = lines.partition_point(|(p, _)| *p <= pc);
    if idx == 0 {
        0
    } else {
        lines[idx - 1].1
    }
}
