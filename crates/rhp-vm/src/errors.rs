//! Diagnostics, exceptions and unwinding: raising and displaying errors,
//! backtraces, uncaught throwables and `finally`.

use std::rc::Rc;

use rhp_builtins::{self as builtins, Host};
use rhp_bytecode::FunctionProto;
use rhp_values::{conv, ArrayKey, ClassId, ObjectHandle, PhpArray, Thrown, Value};

use crate::calls::{is_engine_helper_name, is_handler_accessor};
use crate::{deref, ini_bool, Vm};

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
    /// A `break`/`continue` crossed out of the try/catch; jump after finally.
    Jump(u32),
}

pub(crate) use builtins::php_fns::SILENCE_MASK as SILENCE_KEEPS;

impl Vm {
    /// An `E_COMPILE_ERROR` raised while executing (e.g. a runtime class
    /// redeclaration): never seen by a user error handler, displayed with
    /// php 8.5's backtrace, then the request bails out (exit status 255).
    pub(crate) fn fatal_compile_error(&mut self, msg: &str) -> Thrown {
        let (file, line) = match (0..self.stack.len())
            .rev()
            .find(|&i| !self.stack[i].proto.is_internal())
        {
            None => (Vec::new(), 0),
            Some(i) => (self.current_file(i), self.current_line(i) as i64),
        };
        self.last_error = Some((64, msg.as_bytes().to_vec(), file.clone(), line));
        let trace = Value::Array(self.build_trace_ex(false, false));
        let trace = builtins::php_fns::trace_to_string(self, &trace);
        self.display_error_trace(64, "Fatal error", msg.as_bytes(), &file, line, &trace);
        self.destructed_below = self.next_object_id;
        Thrown::Exit(255)
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
    pub(crate) fn raise_error(&mut self, level: i64, msg: &str) {
        // The innermost user frame: prelude (built-in PHP-source) code has no
        // line map, and php reports internal functions at their caller.
        let (file, line) = match (0..self.stack.len())
            .rev()
            .find(|&i| !self.stack[i].proto.is_internal())
        {
            None if self.sapi.startup => (b"Unknown".to_vec(), 0),
            None => (Vec::new(), 0),
            Some(i) => (self.current_file(i), self.current_line(i) as i64),
        };
        self.raise_error_at(level, msg, file, line);
    }

    /// [`Vm::raise_error`] reported at `file`:`line`.
    pub(crate) fn raise_error_at(&mut self, level: i64, msg: &str, file: Vec<u8>, line: i64) {
        let docref = std::mem::take(&mut self.sapi.docref_msg);
        if self.pending_throw.is_some() {
            return; // php skips diagnostics while an exception is pending
        }
        // A diagnostic prelude code silenced with `@` is engine plumbing (php's
        // C implementation raises none): it only updates error_get_last().
        // (`silence_stack` records the stack depth at each `@`.)
        let prelude_silenced = self.silence > 0
            && self.silence_stack.last().is_some_and(|&(depth, _)| {
                depth
                    .checked_sub(1)
                    .and_then(|i| self.stack.get(i))
                    .is_some_and(|f| f.proto.is_internal())
            });
        let handler = match self.error_handlers.last() {
            _ if prelude_silenced => None,
            Some((h, mask)) if !matches!(h, Value::Null | Value::Bool(false)) => {
                (mask & level != 0).then(|| h.clone())
            }
            _ => None,
        };
        if let Some(h) = handler {
            let args = vec![
                Value::Int(level),
                Value::str(msg.as_bytes().to_vec()),
                Value::str(file.clone()),
                Value::Int(line),
            ];
            // php's EG(user_error_handler) swap: while the handler runs there
            // is none (`false` marks php's UNDEF); it comes back afterwards
            // only if the handler did not install another one.
            let idx = self.error_handlers.len() - 1;
            self.error_handlers[idx].0 = Value::Bool(false);
            let r = self.call_callable_value(&h, args);
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
        self.last_error = Some((level, msg.as_bytes().to_vec(), file.clone(), line));
        if level == builtins::E_USER_ERROR {
            // E_USER_ERROR is fatal: displayed with the backtrace, then the
            // request bails out (shutdown functions still run; exit 255).
            let trace = Value::Array(self.build_trace_ex(false, false));
            let trace = builtins::php_fns::trace_to_string(self, &trace);
            self.display_error_trace(level, "Fatal error", msg.as_bytes(), &file, line, &trace);
            self.destructed_below = self.next_object_id;
            self.pending_throw = Some(Thrown::Exit(255));
            return;
        }
        if self.silence > 0 && level & SILENCE_KEEPS == 0 {
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
        self.sapi.docref_msg = docref;
        self.display_error(level, label, msg.as_bytes(), &file, line);
        self.sapi.docref_msg = false;
    }

    /// Source line of the instruction currently executing in frame `top`.
    pub(crate) fn current_line(&self, top: usize) -> u32 {
        let f = &self.stack[top];
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
        f.include_kind.is_none()
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
    fn native_active_from(&self, i: usize) -> bool {
        self.native_calls.iter().any(|n| n.caller == i)
    }

    /// Where php's `zend_get_executed_filename/lineno` point: the innermost
    /// user-code frame's current file and line.
    pub(crate) fn throw_site(&self) -> (Vec<u8>, u32) {
        let n = self.stack.len();
        let Some(i) = (0..n)
            .rev()
            .find(|&i| !self.is_internal_frame(i))
            .or(n.checked_sub(1))
        else {
            return (Vec::new(), 0);
        };
        (self.current_file(i), self.current_line(i))
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

    /// The trace `args` of frame `i`: the passed arguments' current values.
    fn frame_args(&self, i: usize) -> PhpArray {
        let f = &self.stack[i];
        let mut args = PhpArray::new();
        for (k, p) in f.proto.params.iter().enumerate() {
            if p.variadic {
                if let Value::Array(a) = deref(&f.regs[p.cv as usize]) {
                    for (key, v) in a.iter() {
                        match key {
                            ArrayKey::Int(_) => args.push(deref(v)),
                            ArrayKey::Str(_) => {
                                args.insert(key.clone(), deref(v));
                            }
                        }
                    }
                }
                break;
            }
            if k >= f.argc {
                break;
            }
            args.push(deref(&f.regs[p.cv as usize]));
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
    fn build_trace(&self) -> PhpArray {
        self.build_trace_ex(false, false)
    }

    /// [`Vm::build_trace`] with debug_backtrace's options: `$trace['object']` for
    /// frames with a `$this`, and no `args`.
    pub(crate) fn build_trace_ex(&self, provide_object: bool, ignore_args: bool) -> PhpArray {
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
        for i in (0..self.stack.len()).rev() {
            if !self.is_internal_frame(i) {
                for n in self.native_calls.iter().rev().filter(|n| n.caller == i) {
                    let mut e = PhpArray::new();
                    e.insert(k(b"file"), file_of(i));
                    e.insert(k(b"line"), Value::Int(self.current_line(i) as i64));
                    e.insert(k(b"function"), Value::str(n.name));
                    if !ignore_args {
                        let mut a = PhpArray::new();
                        // SAFETY: see `NativeCall::args` — every entry
                        // still in `native_calls` points at a live slice.
                        let call_args = unsafe { &*n.args };
                        for v in call_args {
                            a.push(deref(v));
                        }
                        e.insert(k(b"args"), Value::Array(a));
                    }
                    out.push(Value::Array(e));
                }
            }
            if i == 0 {
                break;
            }
            let f = &self.stack[i];
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
                && self.is_plumbing_frame(caller)
                && self.dispatched_method(caller).is_none()
            {
                caller -= 1;
            }
            let mut e = PhpArray::new();
            if !self.is_internal_frame(caller) && !self.native_active_from(caller) {
                e.insert(k(b"file"), file_of(caller));
                e.insert(k(b"line"), Value::Int(self.current_line(caller) as i64));
            }
            if let Some(kind) = f.include_kind {
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
            out.push(Value::Array(e));
        }
        out
    }

    /// Stamp a freshly created Throwable with php's creation-site `file`,
    /// `line` and `trace` (zend_default_exception_new).
    pub(crate) fn init_throwable(&self, obj: &ObjectHandle) {
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
        let Ok(cls) = self.resolve_class_by_name(class.as_bytes()) else {
            return thrown;
        };
        let obj = self.new_object(cls);
        {
            let mut b = obj.borrow_mut();
            b.properties.insert(
                rhp_values::PhpStr::from("message"),
                Value::str(message.clone()),
            );
            if let Some((file, line)) = at {
                b.properties
                    .insert(rhp_values::PhpStr::from("file"), Value::str(file));
                b.properties
                    .insert(rhp_values::PhpStr::from("line"), Value::Int(line as i64));
            }
        }
        Thrown::Object(obj)
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
        let r = self.call_callable_value(&handler, vec![arg]);
        self.stack.truncate(1);
        r.err()
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
        let docref = std::mem::take(&mut self.sapi.docref_msg);
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
        if builtins::php_fns::error_reporting_level(self) & ty == 0 {
            return;
        }
        let ini = |vm: &Self, n: &[u8]| vm.ini_get(n).unwrap_or_default();
        let log = ini_bool(&ini(self, b"log_errors"));
        let mut mode = display_errors_mode(&ini(self, b"display_errors"));
        if matches!(mode, DisplayErrors::Stderr)
            && !matches!(
                builtins::sapi::sapi_name().as_str(),
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
                if builtins::vcwd::append_file(&target, &line, 0o644).is_err() {
                    self.write_stderr(&m);
                }
            }
        }
        let html = ini_bool(&ini(self, b"html_errors"));
        if html && !matches!(mode, DisplayErrors::Off) {
            // php_error_cb's html branch: E_ERROR / E_PARSE escape the
            // message and drop the trace; others show it as given (a
            // builtin's docref message was escaped when it was built).
            let fatal = ty == builtins::E_ERROR || ty == builtins::E_PARSE;
            let mut m = ini(self, b"error_prepend_string");
            m.extend_from_slice(format!("<br />\n<b>{label}</b>:  ").as_bytes());
            if fatal || docref {
                let charset = ini(self, b"default_charset");
                m.extend_from_slice(&builtins::sapi::escape_html(message, &charset));
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
        let thrown = self.materialize(thrown, None);
        let h = match thrown {
            Thrown::Object(h) => h,
            Thrown::Error { class, message } => {
                let (file, line) = self.throw_site();
                let msg = format!("Uncaught {class}: {message}");
                self.fatal_error(1, "Fatal error", msg.as_bytes(), &file, line as i64);
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
        let file = conv::to_string_bytes(&prop("file"));
        let line = conv::to_int(&prop("line"));
        // An emptied `file` makes php report the location as "Unknown".
        let file: &[u8] = if file.is_empty() { b"Unknown" } else { &file };
        let cname = self.class(cls).name.as_bytes().to_ascii_lowercase();
        if cname == b"parseerror" || cname == b"compileerror" {
            let msg = conv::to_string_bytes(&prop("message"));
            let (ty, label) = if cname == b"parseerror" {
                (4, "Parse error")
            } else {
                (64, "Fatal error")
            };
            self.fatal_error(ty, label, &msg, file, line);
            return;
        }
        let obj = Value::Object(h.clone());
        let s = match self.lookup_method(cls, b"__tostring") {
            Some(m) => match self.call_sync(&m, Some(h.clone()), cls, Vec::new()) {
                Ok(Value::Str(s)) => s.as_bytes().to_vec(),
                _ => builtins::php_fns::throwable_to_string(self, &obj),
            },
            None => builtins::php_fns::throwable_to_string(self, &obj),
        };
        let mut msg = b"Uncaught ".to_vec();
        msg.extend_from_slice(&s);
        msg.extend_from_slice(b"\n  thrown");
        self.fatal_error(1, "Fatal error", &msg, file, line);
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
        while let Some(&(d, lvl)) = self.silence_stack.last() {
            if d < depth {
                break;
            }
            self.silence = lvl;
            self.silence_stack.pop();
        }
    }

    /// Unwind a thrown exception through the exception table: find a matching
    /// catch (bind + jump) or finally (re-raise after) in the current frame,
    /// else pop and continue in the caller. Returns `Ok` when a handler is
    /// installed (re-enter `run_frames`), `Err` when it escapes the top frame.
    pub(crate) fn unwind(&mut self, thrown: Thrown, floor: usize) -> Result<UnwindResult, Thrown> {
        // An engine error becomes a real object here, before any frame is popped,
        // so its file/line/trace describe the throw point.
        let mut thrown = self.materialize(thrown, None);
        loop {
            if self.stack.len() <= floor {
                return Ok(UnwindResult::Escaped(thrown)); // escaped the (sub)run
            }
            let top = self.stack.len() - 1;
            let proto = self.stack[top].proto.clone();
            // The throwing pc has already been advanced past the faulting instr.
            let throw_pc = (self.stack[top].pc as u32).saturating_sub(1);
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
            if let Some(region) = handler {
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
                            if let Some(vr) = arm.var_reg {
                                self.cv_assign(top, vr, obj);
                            }
                            self.stack[top].pc = arm.target as usize;
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
            let popped = self.stack.pop();
            if let Some(f) = &popped {
                self.include_writeback(f);
            }
            let depth = self.stack.len();
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

    /// Raise a diagnostic from inside an instruction; an exception thrown by
    /// the user error handler is returned to be thrown at this point.
    pub(crate) fn diag(&mut self, level: i64, msg: &str) -> Result<(), Thrown> {
        self.raise_error(level, msg);
        match self.pending_throw.take() {
            Some(t) => Err(t),
            None => Ok(()),
        }
    }
}

/// Whether a diagnostic text is a builtin's `php_error_docref` message:
/// it starts with `name(…): ` (a function or `Class::method`).
pub(crate) fn is_docref_message(msg: &str) -> bool {
    let b = msg.as_bytes();
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
