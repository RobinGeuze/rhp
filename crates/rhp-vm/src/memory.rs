//! `memory_limit` enforcement, as php 8.5 delays an out-of-memory error
//! (Zend/zend_alloc.c: zend_mm_delayed_error, zend_handle_delayed_oom).
//!
//! The allocator records that the limit was crossed (`rhp_values::mem`).
//! The VM notes where at the next instruction boundary, then raises the
//! fatal error at php's next interrupt check: a jump, entering or leaving a
//! user function, a builtin returning, the end of the main script or of a
//! shutdown function. Instructions in between still run, as in php.
//! Raising unwinds like `exit` (no `finally`, no destructors afterwards).

use rhp_builtins::Host;
use rhp_bytecode::{Instr, Reg};
use rhp_values::mem::Oom;
use rhp_values::{PhpArray, Thrown, Value};

use crate::{ini_bool, Vm};

/// A recorded crossing waiting for an interrupt point.
pub(crate) struct DelayedOom {
    message: String,
    file: Vec<u8>,
    line: i64,
    /// The stack depth and next pc at the last check: the next check finds
    /// `(depth, pc + 1)` unless control flow jumped, entered or left a
    /// function.
    at: (usize, usize),
}

impl Vm {
    /// The between-instructions check, while a crossing is recorded or
    /// waiting (`self.mem.attention()`): note the crossing, or raise it if
    /// the instruction that just ran was an interrupt point. `top` / `pc`:
    /// the frame and instruction about to run.
    #[cold]
    pub(crate) fn oom_tick(&mut self, top: usize, pc: usize) -> Result<(), Thrown> {
        let depth = self.stack.len();
        if let Some(oom) = self.mem.take_oom() {
            if let Oom::Hard { .. } = oom {
                return Err(self.hard_limit_exit(oom));
            }
            let (file, line) = self.throw_site();
            self.delayed_oom = Some(Box::new(DelayedOom {
                message: oom.message(),
                file,
                line: line as i64,
                at: (depth, pc),
            }));
            return Ok(());
        }
        let Some(d) = self.delayed_oom.as_deref() else {
            return Ok(());
        };
        let (d0, p0) = d.at;
        let interrupt = depth != d0 || pc != p0 + 1 || self.cond_jump_checks(top, p0);
        if interrupt {
            return Err(self.raise_oom());
        }
        if let Some(d) = self.delayed_oom.as_deref_mut() {
            d.at = (depth, pc);
        }
        Ok(())
    }

    /// php's conditional jumps check for interrupts unless the condition is
    /// a bool (their fast path): did the one at `pc` in frame `top`?
    fn cond_jump_checks(&self, top: usize, pc: usize) -> bool {
        let f = &self.stack[top];
        match f.proto.code.get(pc) {
            Some(Instr::JmpTrue { cond, .. } | Instr::JmpFalse { cond, .. }) => {
                !matches!(self.reg_get(top, *cond), Value::Bool(_))
            }
            _ => false,
        }
    }

    /// An interrupt point outside the instruction loop (a builtin returned,
    /// the main script or a shutdown function ended): raise a recorded
    /// crossing.
    #[inline]
    pub(crate) fn oom_interrupt(&mut self) -> Result<(), Thrown> {
        if self.mem.attention() {
            return self.oom_interrupt_slow();
        }
        Ok(())
    }

    #[cold]
    fn oom_interrupt_slow(&mut self) -> Result<(), Thrown> {
        if let Some(oom) = self.mem.take_oom() {
            if let Oom::Hard { .. } = oom {
                return Err(self.hard_limit_exit(oom));
            }
            let (file, line) = self.throw_site();
            self.delayed_oom = Some(Box::new(DelayedOom {
                message: oom.message(),
                file,
                line: line as i64,
                at: (0, 0),
            }));
        }
        match self.delayed_oom {
            Some(_) => Err(self.raise_oom()),
            None => Ok(()),
        }
    }

    /// zend_handle_delayed_oom: report the E_ERROR (php_error_cb drops every
    /// output buffer first, unless a handler is running; the backtrace stops
    /// after its first frame while an OOM is handled), then bail out like a
    /// fatal error: objects are no longer destructed, exit status 255.
    fn raise_oom(&mut self) -> Thrown {
        self.mem.oom_raised();
        let Some(d) = self.delayed_oom.take() else {
            return Thrown::Exit(255);
        };
        self.output_discard_all_unlocked();
        let backtraces = self.ini_get(b"fatal_error_backtraces").unwrap_or_default();
        let trace = if ini_bool(&backtraces) {
            let mut first = PhpArray::new();
            if let Some((_, frame)) = self.build_trace_ex(false, false).iter().next() {
                first.push(frame.clone());
            }
            rhp_builtins::php_fns::trace_to_string(self, &Value::Array(first))
        } else {
            Vec::new()
        };
        let msg = d.message.into_bytes();
        self.last_error = Some((1, msg.clone(), d.file.clone(), d.line));
        self.display_error_trace(1, "Fatal error", &msg, &d.file, d.line, &trace);
        self.destructed_below = self.next_object_id;
        Thrown::Exit(255)
    }

    /// php's CONCAT on a temporary string it holds the only reference to:
    /// extend it to exactly the new length instead of copying both operands
    /// (`a . b . c` then needs one result buffer, as in php). `a` must be a
    /// temporary (dead after this instruction) or the destination itself.
    pub(crate) fn concat_in_place(
        &mut self,
        top: usize,
        num_cvs: u16,
        dst: Reg,
        a: Reg,
        b: Reg,
    ) -> bool {
        if a == b || (a != dst && a < num_cvs) {
            return false;
        }
        let regs = &mut self.stack[top].regs;
        let tail = match &regs[b as usize] {
            Value::Str(s) => s.clone(),
            _ => return false,
        };
        let Value::Str(head) = &mut regs[a as usize] else {
            return false;
        };
        let Some(buf) = head.get_mut() else {
            return false;
        };
        buf.reserve_exact(tail.len());
        buf.extend_from_slice(tail.as_bytes());
        let v = std::mem::replace(&mut regs[a as usize], Value::Null);
        self.reg_set(top, dst, v);
        true
    }

    /// zend_mm_hard_limit_error: php-cli prints the message to stderr and
    /// exits with status 137 on the spot (no shutdown functions, no output
    /// flush). A server SAPI ends just the request instead, as a fatal
    /// error, so one script can't take the whole process down.
    fn hard_limit_exit(&mut self, oom: Oom) -> Thrown {
        let (file, line) = self.throw_site();
        let (file, line) = if file.is_empty() {
            (b"Unknown".to_vec(), 0)
        } else {
            (file, line)
        };
        // (Only php-cli itself: an engine run in-process, without a SAPI,
        // must not take its host down.)
        if self.sapi.sapi.is_some() && rhp_builtins::sapi::sapi_name() == "cli" {
            let mut m = oom.message().into_bytes();
            m.extend_from_slice(b" in ");
            m.extend_from_slice(&file);
            m.extend_from_slice(format!(" on line {line}\n").as_bytes());
            self.flush_stdout();
            use std::io::Write;
            let _ = std::io::stderr().write_all(&m);
            std::process::exit(137);
        }
        self.mem.oom_raised();
        self.delayed_oom = Some(Box::new(DelayedOom {
            message: oom.message(),
            file,
            line: line as i64,
            at: (0, 0),
        }));
        self.raise_oom()
    }

    /// php_request_shutdown starts with no OOM pending: shutdown code may
    /// run out of memory again.
    pub(crate) fn oom_reset(&mut self) {
        self.delayed_oom = None;
        self.mem.reset_oom();
    }
}
