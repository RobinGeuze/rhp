//! Output control: a port of php's main/output.c — the output handler stack
//! behind `ob_start` & co.
//!
//! Every byte the script produces (`echo`, `print`, inline HTML, displayed
//! diagnostics, `printf`, …) enters through [`Vm::output_write`] and passes
//! down the handler stack (top first; a handler buffers it, and on a flush /
//! clean / final op or a full chunk runs its callback and hands what that
//! returned to the handler below). Whatever falls out of the bottom goes to
//! the SAPI through [`Vm::emit_stdout`] — the single boundary to the
//! response body.
//!
//! Handlers are shared (`Rc<RefCell<…>>`) like php's `php_output_handler *`:
//! a user callback runs with no borrow held, and may look at (but not
//! change: that is php's "Cannot use output buffering in output buffering
//! display handlers" fatal error) the stack while it runs.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use rhp_builtins::{Host, ObCall};
use rhp_values::{conv, ArrayKey, PhpArray, Thrown, Value};

use crate::Vm;

// php_output.h: handler op bits (the `$phase` passed to a user callback).
const OP_WRITE: i64 = 0x00;
const OP_START: i64 = 0x01;
const OP_CLEAN: i64 = 0x02;
const OP_FLUSH: i64 = 0x04;
const OP_FINAL: i64 = 0x08;

// Handler type / ability / status flags.
const H_USER: i64 = 0x0001;
const H_CLEANABLE: i64 = 0x0010;
const H_FLUSHABLE: i64 = 0x0020;
const H_REMOVABLE: i64 = 0x0040;
const H_STDFLAGS: i64 = 0x0070;
const H_STARTED: i64 = 0x1000;
const H_DISABLED: i64 = 0x2000;
const H_PROCESSED: i64 = 0x4000;
const H_PRODUCED_OUTPUT: i64 = 0x8000;

/// PHP_OUTPUT_HANDLER_ABILITY_FLAGS: what a caller may pass in `$flags`.
fn ability_flags(f: i64) -> i64 {
    f & !0xf00f
}

// php_output_stack_pop flags.
const POP_TRY: u32 = 0x000;
const POP_FORCE: u32 = 0x001;
const POP_DISCARD: u32 = 0x010;

// Output layer status flags (OG(flags)).
const O_IMPLICITFLUSH: u32 = 0x01;
const O_DISABLED: u32 = 0x02;
const O_WRITTEN: u32 = 0x04;
const O_SENT: u32 = 0x08;
const O_ACTIVATED: u32 = 0x10_0000;

const E_WARNING: i64 = 2;
const E_NOTICE: i64 = 8;
const E_DEPRECATED: i64 = 8192;

const DEFAULT_HANDLER_NAME: &[u8] = b"default output handler";

/// PHP_OUTPUT_HANDLER_INITBUF_SIZE: the (emulated) buffer capacity php
/// reports as `buffer_size` in `ob_get_status()`.
fn initbuf_size(s: usize) -> usize {
    if s > 0 {
        s.saturating_add(0xfff) & !0xfff
    } else {
        0x4000
    }
}

enum Func {
    /// php_output_handler_default_func: pass the buffer through.
    Default,
    /// A user callback, resolved (in `ob_start`'s scope) when started.
    User(Value),
}

pub(crate) struct Handler {
    name: Vec<u8>,
    func: Func,
    /// Chunk size (0: unlimited).
    size: usize,
    flags: i64,
    /// Position in the stack (0: bottom).
    level: usize,
    buf: Vec<u8>,
    /// php's allocated buffer size (reported, never enforced).
    buf_size: usize,
}

type HRef = Rc<RefCell<Handler>>;

/// php's `zend_output_globals`.
pub(crate) struct Output {
    handlers: Vec<HRef>,
    active: Option<HRef>,
    running: Option<HRef>,
    flags: u32,
    /// Opening the ini `output_handler` before the script runs (diagnostics
    /// say `PHP Request Startup` / `Unknown`).
    starting: bool,
}

impl Default for Output {
    fn default() -> Self {
        Output {
            handlers: Vec::new(),
            active: None,
            running: None,
            flags: O_ACTIVATED,
            starting: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Failure,
    NoData,
    Success,
}

/// php_output_context: the op plus the bytes going in and coming out of a
/// handler.
struct Ctx<'a> {
    op: i64,
    inp: Cow<'a, [u8]>,
    out: Vec<u8>,
}

impl<'a> Ctx<'a> {
    fn new(op: i64) -> Self {
        Ctx {
            op,
            inp: Cow::Borrowed(&[]),
            out: Vec::new(),
        }
    }
    /// php_output_context_reset.
    fn reset(&mut self) {
        self.inp = Cow::Borrowed(&[]);
        self.out = Vec::new();
    }
    /// php_output_context_swap: output becomes the next handler's input.
    fn swap(&mut self) {
        self.inp = Cow::Owned(std::mem::take(&mut self.out));
    }
    /// php_output_context_pass: input passes through unchanged.
    fn pass(&mut self) {
        self.out = std::mem::replace(&mut self.inp, Cow::Borrowed(&[])).into_owned();
    }
}

fn same(a: &HRef, b: &HRef) -> bool {
    Rc::ptr_eq(a, b)
}

impl Vm {
    /// php_output_write: everything the script outputs enters here.
    pub(crate) fn output_write(&mut self, bytes: &[u8]) {
        let f = self.output.flags;
        if f & O_ACTIVATED != 0 {
            if self.output.active.is_none() {
                // Fast path of php_output_op with no handler.
                if !bytes.is_empty() && f & O_DISABLED == 0 {
                    self.output_sapi_write(bytes);
                }
                return;
            }
            self.output_op(OP_WRITE, bytes);
        } else if f & O_DISABLED == 0 {
            self.emit_stdout(bytes);
        }
    }

    /// The bottom of php_output_op: hand bytes to the SAPI.
    fn output_sapi_write(&mut self, bytes: &[u8]) {
        self.emit_stdout(bytes);
        if self.output.flags & O_IMPLICITFLUSH != 0 {
            self.sapi_flush();
        }
        self.output.flags |= O_SENT;
    }

    /// sapi_flush: push buffered response bytes to the client (the
    /// installed SAPI's `flush`).
    fn sapi_flush(&mut self) {
        match self.sapi.sapi.as_mut() {
            Some(s) => s.flush(),
            None => {
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        }
    }

    /// php_request_startup's output setup: the `output_handler` /
    /// `output_buffering` ini (a buffer opened before the script runs), else
    /// `implicit_flush`.
    pub(crate) fn output_startup(&mut self) {
        let ini = |vm: &Self, n: &[u8]| vm.ini_get(n).unwrap_or_default();
        let handler = ini(self, b"output_handler");
        // OnUpdateLong: "On" etc. read as 0 (atol), so php ignores them.
        let buffering = rhp_builtins::php_fns::atol(&ini(self, b"output_buffering"));
        if !handler.is_empty() {
            self.output.starting = true;
            // No autoloader is registered yet, so resolving cannot throw.
            let _ = self.ob_start_user(&Value::str(handler), 0, H_STDFLAGS);
            self.output.starting = false;
        } else if buffering != 0 {
            let chunk = if buffering > 1 { buffering as usize } else { 0 };
            self.handler_start(Handler::new(
                DEFAULT_HANDLER_NAME.to_vec(),
                Func::Default,
                chunk,
                H_STDFLAGS,
            ));
        } else if crate::ini_bool(&ini(self, b"implicit_flush")) {
            self.output.flags |= O_IMPLICITFLUSH;
        }
    }

    /// php_request_shutdown step 3 (php_output_end_all): finalize every
    /// buffer, top first, each into the one below.
    pub(crate) fn output_end_all(&mut self) {
        while self.output.active.is_some() && self.stack_pop(POP_FORCE) {}
    }

    /// php_output_discard_all, as php_error_cb runs it for an out-of-memory
    /// error: skipped while a handler runs (that would free it under itself).
    pub(crate) fn output_discard_all_unlocked(&mut self) {
        if self.output.running.is_some() {
            return;
        }
        while self.output.active.is_some() && self.stack_pop(POP_DISCARD | POP_FORCE) {}
    }

    /// The `func(): ` origin php_error_docref puts before a message: the
    /// running builtin, else the innermost user function (`main` at top
    /// level), else `PHP Request Shutdown` / `Unknown` (no code running).
    fn docref_prefix(&self) -> String {
        if self.output.starting {
            return "PHP Request Startup: ".to_string();
        }
        if let Some(nc) = self.native_calls.last() {
            if nc.caller + 1 >= self.stack.len() {
                return format!("{}(): ", nc.name);
            }
        }
        match self.stack.last() {
            _ if self.no_code_running() => "PHP Request Shutdown: ".to_string(),
            Some(f) => {
                let name = f.proto.name.as_bytes();
                let name = if name == b"{main}" {
                    "main".to_string()
                } else {
                    String::from_utf8_lossy(name).into_owned()
                };
                match f.defining_class {
                    Some(c) => format!(
                        "{}::{}(): ",
                        String::from_utf8_lossy(self.class(c).name.as_bytes()),
                        name
                    ),
                    None => format!("{name}(): "),
                }
            }
            None => "Unknown: ".to_string(),
        }
    }

    fn ob_diag(&mut self, level: i64, msg: &str) {
        let m = format!("{}{}", self.docref_prefix(), msg);
        if self.no_code_running() || self.output.starting {
            self.raise_error_at(level, &m, b"Unknown".to_vec(), 0);
        } else {
            self.raise_error(level, &m);
        }
    }

    /// Request shutdown with no php code on the stack (php_output_end_all).
    pub(crate) fn no_code_running(&self) -> bool {
        self.in_shutdown_scope && self.stack.len() <= 1 && self.native_calls.is_empty()
    }

    /// An E_ERROR from the output layer: displayed with php 8.5's backtrace,
    /// objects are no longer destructed, and the request bails out.
    fn ob_fatal(&mut self, msg: &str) {
        let msg = format!("{}{}", self.docref_prefix(), msg);
        let (file, line) = match (0..self.stack.len())
            .rev()
            .find(|&i| !crate::is_prelude_proto(&self.stack[i].proto))
        {
            None => (Vec::new(), 0),
            Some(i) => (self.current_file(i), self.current_line(i) as i64),
        };
        let backtraces = self.ini_get(b"fatal_error_backtraces").unwrap_or_default();
        let trace = if crate::ini_bool(&backtraces) {
            let trace = Value::Array(self.build_trace_ex(false, false));
            rhp_builtins::php_fns::trace_to_string(self, &trace)
        } else {
            Vec::new()
        };
        self.last_error = Some((1, msg.as_bytes().to_vec(), file.clone(), line));
        self.display_error_trace(1, "Fatal error", msg.as_bytes(), &file, line, &trace);
        self.destructibles.clear();
        self.pending_throw = Some(Thrown::Exit(255));
    }

    /// A handler threw (or exited) while finalizing at request shutdown,
    /// with no php frame to catch it: php reports it (into the running
    /// handler's buffer, so it is lost) and bails out of php_output_end_all,
    /// dropping every buffer unflushed.
    fn ob_shutdown_bailout(&mut self, t: Thrown) {
        let code = match t {
            Thrown::Exit(c) => c,
            t => {
                if t.is_catchable() {
                    self.report_uncaught(t);
                }
                255
            }
        };
        self.output_deactivate();
        self.pending_throw = Some(Thrown::Exit(code));
    }

    /// Keep the first exception a handler threw for the running statement.
    fn ob_pending(&mut self, t: Thrown) {
        if self.pending_throw.is_none() {
            // Build the Error object now, while the builtin that ran the
            // handler (`ob_end_flush()` …) is still on the trace.
            let t = self.materialize(t, None);
            self.pending_throw = Some(t);
        }
    }

    fn output_activated(&self) -> bool {
        self.output.flags & O_ACTIVATED != 0
    }

    /// php_output_deactivate: drop every handler (unflushed). Objects only
    /// a handler kept alive are destructed right away.
    fn output_deactivate(&mut self) {
        if self.output_activated() {
            self.output.flags ^= O_ACTIVATED;
            self.output.active = None;
            self.output.running = None;
            self.output.handlers.clear();
            self.dtor_pending_checks = usize::MAX / 2;
            if let Err(t) = self.collect_released() {
                self.ob_pending(t);
            }
        }
    }

    /// php_output_lock_error: a stack operation from inside a running handler
    /// is fatal.
    fn lock_error(&mut self, op: i64) -> bool {
        if op != 0 && self.output.active.is_some() && self.output.running.is_some() {
            self.output_deactivate();
            self.ob_fatal("Cannot use output buffering in output buffering display handlers");
            return true;
        }
        false
    }

    /// php_output_op: pass `data` (or just `op`) down the handler stack and
    /// write what comes out of the bottom to the SAPI.
    fn output_op(&mut self, op: i64, data: &[u8]) {
        if self.lock_error(op) {
            return;
        }
        let mut ctx = Ctx::new(op);
        let cnt = self.output.handlers.len();
        if self.output.active.is_some() && cnt > 0 {
            ctx.inp = Cow::Borrowed(data);
            if cnt > 1 {
                self.stack_apply_op(&mut ctx);
            } else {
                let top = self.output.handlers[cnt - 1].clone();
                if top.borrow().flags & H_DISABLED == 0 {
                    self.handler_op(&top, &mut ctx);
                } else {
                    ctx.pass();
                }
            }
            if !self.output_activated() {
                return; // bailed out (fatal error)
            }
        } else {
            ctx.out = data.to_vec();
        }
        if !ctx.out.is_empty() && self.output.flags & O_DISABLED == 0 {
            self.output_sapi_write(&ctx.out);
        }
    }

    /// php_output_stack_apply_op over the whole stack, top first.
    fn stack_apply_op(&mut self, ctx: &mut Ctx) {
        let hs: Vec<HRef> = self.output.handlers.iter().rev().cloned().collect();
        for h in hs {
            let was_disabled = h.borrow().flags & H_DISABLED != 0;
            let status = if was_disabled {
                Status::Failure
            } else {
                self.handler_op(&h, ctx)
            };
            if !self.output_activated() {
                return;
            }
            let level = h.borrow().level;
            match status {
                Status::NoData => return,
                Status::Success => {
                    if level != 0 {
                        ctx.swap();
                    }
                }
                Status::Failure => {
                    if was_disabled {
                        if level == 0 {
                            ctx.pass();
                        }
                    } else if level != 0 {
                        ctx.swap();
                    }
                }
            }
        }
    }

    /// php_output_handler_append: buffer the input; true when the handler
    /// need not run yet.
    fn handler_append(&mut self, h: &HRef, data: &[u8]) -> bool {
        if !data.is_empty() {
            self.output.flags |= O_WRITTEN;
            let mut hb = h.borrow_mut();
            let avail = hb.buf_size.saturating_sub(hb.buf.len());
            if avail <= data.len() {
                let grow_int = initbuf_size(hb.size);
                let grow_buf = initbuf_size(data.len() - avail);
                hb.buf_size = hb.buf_size.saturating_add(grow_int.max(grow_buf));
            }
            hb.buf.extend_from_slice(data);
            if hb.size != 0 && hb.buf.len() >= hb.size {
                // A full chunk: process it, unless output is being produced
                // by a running handler (stored away).
                return self.output.running.is_some();
            }
        }
        true
    }

    /// php_output_handler_op: apply `ctx.op` to handler `h`.
    fn handler_op(&mut self, h: &HRef, ctx: &mut Ctx) -> Status {
        let original_op = ctx.op;
        if h.borrow().flags & H_DISABLED != 0 {
            return Status::Failure;
        }
        if self.lock_error(ctx.op) {
            return Status::Failure;
        }
        if ctx.op == OP_WRITE && self.output.active.is_some() && self.output.running.is_some() {
            h.borrow_mut().flags |= H_PRODUCED_OUTPUT;
        }
        let inp = std::mem::replace(&mut ctx.inp, Cow::Borrowed(&[]));
        if self.handler_append(h, &inp) && ctx.op == OP_WRITE {
            return Status::NoData;
        }
        drop(inp);
        if h.borrow().flags & H_STARTED == 0 {
            ctx.op |= OP_START;
        }
        self.output.running = Some(h.clone());
        let user = match &h.borrow().func {
            Func::User(cb) => Some(cb.clone()),
            Func::Default => None,
        };
        let mut still_have_handler = true;
        let status = match user {
            // zend_call_function refuses while an out-of-memory error is
            // handled (php_error_cb discards the buffers then).
            Some(_) if self.mem.oom_handled() => Status::Failure,
            Some(cb) => {
                let data = h.borrow().buf.clone();
                let at_shutdown = self.no_code_running();
                let depth = self.stack.len();
                let r = self.call_callable_value(&cb, vec![Value::str(data), Value::Int(ctx.op)]);
                if r.is_err() && self.stack.len() > depth {
                    // `exit` leaves the handler's frames behind.
                    self.stack.truncate(depth);
                }
                // php destructs the handler's locals as its frame ends, while
                // it still counts as running (their output is the handler's).
                let r = match r {
                    Ok(v) if self.dtor_check && !self.destructibles.is_empty() => {
                        self.dtor_check = false;
                        self.collect_released().map(|()| v)
                    }
                    r => r,
                };
                if !self.output_activated() {
                    // A fatal error inside the handler tore the stack down.
                    if let Err(t) = r {
                        self.ob_pending(t);
                    }
                    return Status::Failure;
                }
                match r {
                    Ok(ret) => {
                        if h.borrow().flags & H_PRODUCED_OUTPUT != 0 {
                            // Writes from the handler went to its own buffer:
                            // let this deprecation pass through it.
                            {
                                let mut hb = h.borrow_mut();
                                hb.flags |= H_DISABLED;
                                hb.flags &= !H_PRODUCED_OUTPUT;
                            }
                            let name = String::from_utf8_lossy(&h.borrow().name).into_owned();
                            self.ob_diag(
                                E_DEPRECATED,
                                &format!(
                                    "Producing output from user output handler {name} is deprecated"
                                ),
                            );
                            still_have_handler = self.output.handlers.iter().any(|x| same(x, h));
                            if still_have_handler {
                                h.borrow_mut().flags &= !H_DISABLED;
                            }
                        }
                        match ret {
                            Value::Bool(false) => Status::Failure,
                            Value::Bool(true) => Status::NoData,
                            v => match self.to_string_vm(&v) {
                                Ok(s) if !s.is_empty() => {
                                    ctx.out = s;
                                    Status::Success
                                }
                                Ok(_) => Status::NoData,
                                Err(t) => {
                                    self.ob_pending(t);
                                    Status::NoData
                                }
                            },
                        }
                    }
                    Err(t) => {
                        if at_shutdown {
                            // zend_throw_exception_internal with no frame: the
                            // user exception handler runs right here (its
                            // output lands in this handler's buffer, which the
                            // failure passes on); without one, bail out.
                            if let Some(t) = self.handle_uncaught(t) {
                                self.ob_shutdown_bailout(t);
                                return Status::Failure;
                            }
                        } else {
                            self.ob_pending(t);
                        }
                        Status::Failure
                    }
                }
            }
            None => {
                ctx.out = h.borrow().buf.clone();
                if ctx.out.is_empty() {
                    Status::NoData
                } else {
                    Status::Success
                }
            }
        };
        if still_have_handler {
            h.borrow_mut().flags |= H_STARTED;
        }
        self.output.running = None;
        if !still_have_handler {
            return status;
        }
        let mut hb = h.borrow_mut();
        match status {
            Status::Failure => {
                // Disable the handler and pass its buffer along unchanged.
                hb.flags |= H_DISABLED;
                ctx.out = std::mem::take(&mut hb.buf);
                hb.buf_size = 0;
            }
            Status::NoData | Status::Success => {
                if status == Status::NoData {
                    ctx.reset();
                }
                hb.buf.clear();
                hb.flags |= H_PROCESSED;
            }
        }
        ctx.op = original_op;
        status
    }

    /// php_output_stack_pop: finalize the active handler and pop it, passing
    /// its output to the next one (unless discarding).
    fn stack_pop(&mut self, flags: u32) -> bool {
        let discard = flags & POP_DISCARD != 0;
        let what = if discard { "discard" } else { "send" };
        let Some(orphan) = self.output.active.clone() else {
            self.ob_diag(
                E_NOTICE,
                &format!("Failed to {what} buffer. No buffer to {what}"),
            );
            return false;
        };
        if flags & POP_FORCE == 0 && orphan.borrow().flags & H_REMOVABLE == 0 {
            let (name, level) = {
                let o = orphan.borrow();
                (String::from_utf8_lossy(&o.name).into_owned(), o.level)
            };
            self.ob_diag(
                E_NOTICE,
                &format!("Failed to {what} buffer of {name} ({level})"),
            );
            return false;
        }
        let mut ctx = Ctx::new(OP_FINAL);
        let oflags = orphan.borrow().flags;
        if oflags & H_DISABLED == 0 {
            if oflags & H_STARTED == 0 {
                ctx.op |= OP_START;
            }
            if discard {
                ctx.op |= OP_CLEAN;
            }
            self.handler_op(&orphan, &mut ctx);
            if !self.output_activated() {
                return false;
            }
        }
        self.output.handlers.pop();
        self.output.active = self.output.handlers.last().cloned();
        if !ctx.out.is_empty() && !discard {
            let out = std::mem::take(&mut ctx.out);
            self.output_write(&out);
        }
        true
    }

    /// php_output_flush: run the active handler's flush op and pass what it
    /// returned to the handler below.
    fn output_flush(&mut self) -> bool {
        let Some(a) = self.output.active.clone() else {
            return false;
        };
        if a.borrow().flags & H_FLUSHABLE == 0 {
            return false;
        }
        let mut ctx = Ctx::new(OP_FLUSH);
        self.handler_op(&a, &mut ctx);
        if !self.output_activated() {
            return true;
        }
        if !ctx.out.is_empty() {
            let top = self.output.handlers.pop();
            self.output_write(&ctx.out);
            if let Some(t) = top {
                self.output.handlers.push(t);
            }
        }
        true
    }

    /// php_output_clean: run the active handler's clean op, dropping its
    /// buffer.
    fn output_clean(&mut self) -> bool {
        let Some(a) = self.output.active.clone() else {
            return false;
        };
        if a.borrow().flags & H_CLEANABLE == 0 {
            return false;
        }
        let mut ctx = Ctx::new(OP_CLEAN);
        self.handler_op(&a, &mut ctx);
        true
    }

    /// php_output_handler_start: push a handler.
    fn handler_start(&mut self, mut h: Handler) -> bool {
        if !self.output_activated() {
            return false;
        }
        if self.lock_error(OP_START) {
            return false;
        }
        h.level = self.output.handlers.len();
        let r = Rc::new(RefCell::new(h));
        self.output.handlers.push(r.clone());
        self.output.active = Some(r);
        true
    }

    /// php_output_start_user: `ob_start($callback, $chunk_size, $flags)`.
    fn ob_start_user(
        &mut self,
        callback: &Value,
        chunk: usize,
        flags: i64,
    ) -> Result<bool, Thrown> {
        let h = match crate::deref(callback) {
            Value::Null => Some(Handler::new(
                DEFAULT_HANDLER_NAME.to_vec(),
                Func::Default,
                chunk,
                ability_flags(flags),
            )),
            cb => match self.check_callable(&cb)? {
                Some(err) => {
                    self.ob_diag(E_WARNING, &err);
                    None
                }
                None => {
                    let name = self.callable_name(&cb);
                    // Resolve it in this scope (zend_fcall_info_init), like
                    // a first-class callable, unless it is a closure already.
                    let call = match cb {
                        Value::Closure(_) | Value::Object(_) => cb,
                        other => self.wrap_callable(other),
                    };
                    Some(Handler::new(
                        name,
                        Func::User(call),
                        chunk,
                        ability_flags(flags) | H_USER,
                    ))
                }
            },
        };
        // php_output_handler_start checks for a running handler before it
        // looks at whether one could be created.
        Ok(match h {
            Some(h) => self.handler_start(h),
            None => {
                if self.output_activated() {
                    self.lock_error(OP_START);
                }
                false
            }
        })
    }

    /// zend_get_callable_name for a (valid) callable.
    fn callable_name(&self, cb: &Value) -> Vec<u8> {
        let obj_class =
            |h: &rhp_values::ObjectHandle| self.class(h.borrow().class).name.as_bytes().to_vec();
        match cb {
            Value::Str(s) => s.as_bytes().to_vec(),
            Value::Closure(cd) => match &cd.wrapped {
                // A first-class callable is named after what it wraps.
                Some(w) => self.callable_name(w),
                None => match self.closures.get(cd.fnidx as usize) {
                    Some(p) => p.name.as_bytes().to_vec(),
                    None => b"Closure::__invoke".to_vec(),
                },
            },
            Value::Object(h) => {
                let mut n = obj_class(h);
                n.extend_from_slice(b"::__invoke");
                n
            }
            Value::Array(a) => {
                let first = a.get(&ArrayKey::Int(0)).map(crate::deref);
                let second = a.get(&ArrayKey::Int(1)).map(crate::deref);
                let mut n = match first {
                    Some(Value::Object(h)) => obj_class(&h),
                    Some(Value::Closure(_)) => b"Closure".to_vec(),
                    Some(v) => conv::to_string_bytes(&v),
                    None => Vec::new(),
                };
                n.extend_from_slice(b"::");
                if let Some(m) = second {
                    n.extend_from_slice(&conv::to_string_bytes(&m));
                }
                n
            }
            v => conv::to_string_bytes(v),
        }
    }

    fn active_name_level(&self) -> (String, usize) {
        match &self.output.active {
            Some(a) => {
                let a = a.borrow();
                (String::from_utf8_lossy(&a.name).into_owned(), a.level)
            }
            None => (String::new(), 0),
        }
    }

    fn handler_status(h: &Handler) -> Value {
        let mut a = PhpArray::new();
        let k = |s: &str| ArrayKey::Str(s.into());
        a.insert(k("name"), Value::str(h.name.clone()));
        a.insert(k("type"), Value::Int(h.flags & 0xf));
        a.insert(k("flags"), Value::Int(h.flags));
        a.insert(k("level"), Value::Int(h.level as i64));
        a.insert(
            k("chunk_size"),
            Value::Int(i64::try_from(h.size).unwrap_or(i64::MAX)),
        );
        a.insert(
            k("buffer_size"),
            Value::Int(i64::try_from(h.buf_size).unwrap_or(i64::MAX)),
        );
        a.insert(k("buffer_used"), Value::Int(h.buf.len() as i64));
        Value::Array(a)
    }

    /// The userland functions (output.c's PHP_FUNCTIONs).
    pub(crate) fn output_control_call(&mut self, call: ObCall) -> Result<Value, Thrown> {
        Ok(match call {
            ObCall::Start {
                callback,
                chunk_size,
                flags,
            } => {
                let chunk = chunk_size.max(0) as usize;
                if self.ob_start_user(&callback, chunk, flags)? {
                    Value::Bool(true)
                } else {
                    self.ob_diag(E_NOTICE, "Failed to create buffer");
                    Value::Bool(false)
                }
            }
            ObCall::Flush => {
                if self.output.active.is_none() {
                    self.ob_diag(E_NOTICE, "Failed to flush buffer. No buffer to flush");
                    return Ok(Value::Bool(false));
                }
                if !self.output_flush() {
                    let (n, l) = self.active_name_level();
                    self.ob_diag(E_NOTICE, &format!("Failed to flush buffer of {n} ({l})"));
                    return Ok(Value::Bool(false));
                }
                Value::Bool(true)
            }
            ObCall::Clean => {
                if self.output.active.is_none() {
                    self.ob_diag(E_NOTICE, "Failed to delete buffer. No buffer to delete");
                    return Ok(Value::Bool(false));
                }
                if !self.output_clean() {
                    let (n, l) = self.active_name_level();
                    self.ob_diag(E_NOTICE, &format!("Failed to delete buffer of {n} ({l})"));
                    return Ok(Value::Bool(false));
                }
                Value::Bool(true)
            }
            ObCall::EndFlush => {
                if self.output.active.is_none() {
                    self.ob_diag(
                        E_NOTICE,
                        "Failed to delete and flush buffer. No buffer to delete or flush",
                    );
                    return Ok(Value::Bool(false));
                }
                Value::Bool(self.stack_pop(POP_TRY))
            }
            ObCall::EndClean => {
                if self.output.active.is_none() {
                    self.ob_diag(E_NOTICE, "Failed to delete buffer. No buffer to delete");
                    return Ok(Value::Bool(false));
                }
                Value::Bool(self.stack_pop(POP_DISCARD | POP_TRY))
            }
            ObCall::GetFlush => {
                let Some(contents) = self.ob_contents() else {
                    self.ob_diag(
                        E_NOTICE,
                        "Failed to delete and flush buffer. No buffer to delete or flush",
                    );
                    return Ok(Value::Bool(false));
                };
                if !self.stack_pop(POP_TRY) {
                    let (n, l) = self.active_name_level();
                    self.ob_diag(E_NOTICE, &format!("Failed to delete buffer of {n} ({l})"));
                }
                Value::str(contents)
            }
            ObCall::GetClean => {
                let Some(contents) = self.ob_contents() else {
                    return Ok(Value::Bool(false));
                };
                if !self.stack_pop(POP_DISCARD | POP_TRY) {
                    let (n, l) = self.active_name_level();
                    self.ob_diag(E_NOTICE, &format!("Failed to delete buffer of {n} ({l})"));
                }
                Value::str(contents)
            }
            ObCall::GetContents => match self.ob_contents() {
                Some(c) => Value::str(c),
                None => Value::Bool(false),
            },
            ObCall::GetLength => match &self.output.active {
                Some(a) => Value::Int(a.borrow().buf.len() as i64),
                None => Value::Bool(false),
            },
            ObCall::GetLevel => Value::Int(self.ob_level() as i64),
            ObCall::ListHandlers => {
                let mut a = PhpArray::new();
                if self.output.active.is_some() {
                    for h in &self.output.handlers {
                        a.push(Value::str(h.borrow().name.clone()));
                    }
                }
                Value::Array(a)
            }
            ObCall::GetStatus { full } => match &self.output.active {
                None => Value::Array(PhpArray::new()),
                Some(a) if !full => Self::handler_status(&a.borrow()),
                Some(_) => {
                    let mut arr = PhpArray::new();
                    for h in &self.output.handlers {
                        arr.push(Self::handler_status(&h.borrow()));
                    }
                    Value::Array(arr)
                }
            },
            ObCall::ImplicitFlush(on) => {
                if on {
                    self.output.flags |= O_IMPLICITFLUSH;
                } else {
                    self.output.flags &= !O_IMPLICITFLUSH;
                }
                Value::Null
            }
            ObCall::SapiFlush => {
                self.sapi_flush();
                Value::Null
            }
        })
    }

    /// php_output_get_contents.
    pub(crate) fn ob_contents(&self) -> Option<Vec<u8>> {
        self.output.active.as_ref().map(|a| a.borrow().buf.clone())
    }

    /// php_output_get_level.
    pub(crate) fn ob_level(&self) -> usize {
        if self.output.active.is_some() {
            self.output.handlers.len()
        } else {
            0
        }
    }
}

impl Handler {
    fn new(name: Vec<u8>, func: Func, size: usize, flags: i64) -> Self {
        Handler {
            name,
            func,
            size,
            flags,
            level: 0,
            buf: Vec::new(),
            buf_size: initbuf_size(size),
        }
    }
}
