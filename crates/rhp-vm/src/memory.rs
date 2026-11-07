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
use rhp_values::mem::{self, Oom};
use rhp_values::{PhpArray, PhpStr, Thrown, Value};

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
    /// waiting (`self.mem.attention()`) or a cycle collection is due
    /// (`self.mem.tick()` covers both): run the collection; note the
    /// crossing, or raise it if the instruction that just ran was an
    /// interrupt point. `top` / `pc`: the frame and instruction about to
    /// run.
    #[cold]
    pub(crate) fn oom_tick(&mut self, top: usize, pc: usize) -> Result<(), Thrown> {
        if self.mem.gc_is_due() {
            // The automatic cycle collection (php runs it where the root
            // buffer filled up).
            self.gc_auto()?;
            if !self.mem.attention() {
                return Ok(());
            }
        }
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
        let interrupt = depth != d0 || pc != p0 + 1 || self.jump_checks(top, p0);
        if interrupt {
            return Err(self.raise_oom());
        }
        if let Some(d) = self.delayed_oom.as_deref_mut() {
            d.at = (depth, pc);
        }
        Ok(())
    }

    /// Did the instruction at `pc` in frame `top`, which went on to the next
    /// one, check for interrupts? A jump always does (the one to a loop's
    /// condition, see the compiler); a conditional jump not taken does
    /// unless its condition is a bool (php's fast path).
    fn jump_checks(&self, top: usize, pc: usize) -> bool {
        let f = &self.stack[top];
        match f.proto.code.get(pc) {
            Some(Instr::Jmp { .. }) => true,
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
            rhp_builtins::zend::exceptions::trace_to_string(self, &Value::Array(first))
        } else {
            Vec::new()
        };
        let msg = d.message.into_bytes();
        self.last_error = Some((1, msg.clone(), d.file.clone(), d.line));
        self.display_error_trace(1, "Fatal error", &msg, &d.file, d.line, &trace);
        self.mark_objects_destructed();
        Thrown::Exit(255)
    }

    /// php's CONCAT / ASSIGN_OP on a string it holds the only reference
    /// to: extend it instead of copying both operands (`a . b . c` needs one
    /// result buffer, and `$s .= $x` appends, as in php). `a` is a temporary
    /// (dead after this instruction), the destination itself, or a variable
    /// that the next instruction (`code[pc + 1]`) assigns the result to.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn concat_in_place(
        &mut self,
        top: usize,
        code: &[Instr],
        pc: usize,
        num_cvs: u16,
        dst: Reg,
        a: Reg,
        b: Reg,
    ) -> bool {
        if a != dst && a < num_cvs {
            let assigns_back = matches!(code.get(pc + 1),
                Some(Instr::Assign { cv, src }) if *cv == a && *src == dst);
            return assigns_back && dst != b && self.concat_assign_in_place(top, dst, a, b);
        }
        if a == b {
            return false;
        }
        let regs = &mut self.stack[top].regs;
        let Some(tail) = str_operand(&regs[b as usize]) else {
            return false;
        };
        let Value::Str(head) = &mut regs[a as usize] else {
            return false;
        };
        let Some(buf) = head.get_mut() else {
            return false;
        };
        append(buf, Some(tail.as_bytes()));
        let v = std::mem::replace(&mut regs[a as usize], Value::Null);
        self.reg_set(top, dst, v);
        true
    }

    /// `$cv .= $x`: append to the variable's string (through a reference
    /// too), then leave the result in `dst` for the Assign that follows.
    fn concat_assign_in_place(&mut self, top: usize, dst: Reg, a: Reg, b: Reg) -> bool {
        // `dst` may still hold the previous `.=`'s result (the same string).
        self.reg_set(top, dst, Value::Null);
        let regs = &mut self.stack[top].regs;
        let tail = if a == b {
            None
        } else {
            match str_operand(&regs[b as usize]) {
                Some(s) => Some(s),
                None => return false,
            }
        };
        let tail = tail.as_ref().map(|t| t.as_bytes());
        let result = match &mut regs[a as usize] {
            Value::Ref(rc) => {
                let rc = rc.clone();
                let mut cell = rc.borrow_mut();
                append_unique(&mut cell, tail)
            }
            v => append_unique(v, tail),
        };
        match result {
            Some(s) => {
                regs[dst as usize] = Value::Str(s);
                true
            }
            None => false,
        }
    }

    /// zend_mm_hard_limit_error: php prints the message to stderr and exits
    /// with status 137 on the spot (no shutdown functions, no output flush).
    /// Only a SAPI that says so does that here (php-cli); under any other the
    /// request ends as a fatal error, so one script can't take the whole
    /// process down.
    fn hard_limit_exit(&mut self, oom: Oom) -> Thrown {
        let (file, line) = self.throw_site();
        let (file, line) = if file.is_empty() {
            (b"Unknown".to_vec(), 0)
        } else {
            (file, line)
        };
        if self.sapi.sapi.exits_on_hard_limit() {
            let mut m = oom.message().into_bytes();
            m.extend_from_slice(b" in ");
            m.extend_from_slice(&file);
            m.extend_from_slice(format!(" on line {line}\n").as_bytes());
            self.sapi.sapi.flush();
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

/// A register's string, also through a reference (a global variable).
fn str_operand(v: &Value) -> Option<PhpStr> {
    match v {
        Value::Str(s) => Some(s.clone()),
        Value::Ref(rc) => match &*rc.borrow() {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Append `tail` (`None`: the string itself) to a string only `v` holds;
/// the new value, or `None` when `v` is not such a string.
fn append_unique(v: &mut Value, tail: Option<&[u8]>) -> Option<PhpStr> {
    let Value::Str(s) = v else {
        return None;
    };
    append(s.get_mut()?, tail);
    Some(s.clone())
}

/// php's zend_string_extend: grow to a block of the new length (it is
/// counted as such anyway), checking the limit with php's string sizes.
/// Past the hard limit nothing is appended: the VM stops at its next check.
fn append(buf: &mut Vec<u8>, tail: Option<&[u8]>) {
    let old = buf.len();
    let new = old + tail.map_or(old, <[u8]>::len);
    if new > buf.capacity() {
        if !mem::realloc_check(mem::zstr_size(old), mem::zstr_size(new)) {
            return;
        }
        buf.reserve_exact(mem::block_size(new) - old);
    }
    match tail {
        Some(t) => buf.extend_from_slice(t),
        None => buf.extend_from_within(..),
    }
}
