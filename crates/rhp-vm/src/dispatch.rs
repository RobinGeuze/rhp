//! The instruction loop (one `match` over `Instr`), register access and the
//! operator helpers it calls.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rhp_builtins as builtins;
use rhp_builtins::zend::object_handlers::{CastTarget, Op};
use rhp_bytecode::{CallFlags, ClassRef, CmpKind, FunctionProto, Instr, NIdx, Pc, Reg};
use rhp_values::ops::{self, cast_to_array, inc_dec_value};
use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, PhpStr, Thrown, Value};

use crate::calls::{dyn_method_name, param_is_ref, undefined_function, Args, CallTarget};
use crate::errors::{enclosing_finally, FinallyCont, UnwindResult};
use crate::generators::GenStatus;
use crate::lvalue::{
    aa_key, foreach_promote, CursorRoot, CursorStep, DimFetch, DimMode, MagicSplit,
};
use crate::{classes, deref, StaticKey, UnitBase, Vm};

impl Vm {
    /// zend_match_unhandled_error's text for the subject: the value as
    /// smart_str_append_zval prints it, else `of type T`.
    #[cold]
    fn match_case_desc(&self, v: &Value) -> String {
        use rhp_builtins::zend::exceptions as ex;
        let mut out = Vec::new();
        if ex::ignore_args()
            || (matches!(deref(v), Value::Str(_)) && ex::string_param_max_len() == 0)
            || !ex::append_zval(self, v, &mut out)
        {
            return format!("of type {}", rhp_builtins::zval_type_name(self, v));
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Key of a function-static slot: the proto plus, for a method, the class
    /// it runs as. A method declared in a class always runs with that class,
    /// but a trait method is copied into every using class and each copy has
    /// its own statics (php duplicates the op_array per using class).
    /// An include's or eval's `{main}` starts its statics fresh every time
    /// it runs, as php does: they are keyed by its stack index (see
    /// [`Vm::static_table`]).
    fn static_key(&self, top: usize, proto: &Rc<FunctionProto>, sname: Vec<u8>) -> StaticKey {
        let f = &self.stack[top];
        if f.include_kind.is_some() {
            return (top, 0, None, sname);
        }
        let cls = if proto.name.as_bytes().starts_with(b"{closure") {
            None
        } else {
            f.defining_class
        };
        (Rc::as_ptr(proto) as usize, f.unit, cls, sname)
    }

    /// The table frame `top`'s statics live in: an include's or eval's
    /// `{main}` keeps them in [`Vm::include_statics`], dropped when it
    /// returns (see [`Vm::run_included`]).
    #[inline]
    fn static_table(&mut self, top: usize) -> &mut HashMap<StaticKey, Rc<RefCell<Value>>> {
        if self.stack[top].include_kind.is_some() {
            &mut self.include_statics
        } else {
            &mut self.statics
        }
    }

    /// Where the file-local indices of frame `top`'s unit start.
    fn unit_base(&self, top: usize) -> UnitBase {
        self.units[self.stack[top].unit as usize]
    }

    pub(crate) fn reg_get(&self, top: usize, r: Reg) -> Value {
        match &self.stack[top].regs[r as usize] {
            Value::Ref(rc) => rc.borrow().clone(),
            v => v.clone(),
        }
    }

    /// A read of register `r` as an operand (php's BP_VAR_R fetch): an
    /// undefined CV raises "Undefined variable $x" and reads as null.
    #[inline(always)]
    pub(crate) fn reg_r(&mut self, top: usize, r: Reg) -> Result<Value, Thrown> {
        match &self.stack[top].regs[r as usize] {
            Value::Undef => self.undef_cv(top, r),
            Value::Ref(rc) => {
                let v = rc.borrow().copy_slot();
                if let Value::Undef = v {
                    return self.undef_cv(top, r);
                }
                Ok(v)
            }
            v => Ok(v.clone()),
        }
    }

    /// Define CV `cv` as null when it is undefined (php's BP_VAR_W fetch),
    /// writing through the symbol-table cell of a global-scope variable.
    #[inline]
    pub(crate) fn cv_define(&mut self, top: usize, cv: Reg) {
        match &mut self.stack[top].regs[cv as usize] {
            slot @ Value::Undef => *slot = Value::Null,
            Value::Ref(rc) => cell_define(rc),
            _ => {}
        }
    }

    /// [`Vm::reg_r`], inline for a scalar.
    #[inline(always)]
    fn reg_r_fast(&mut self, top: usize, r: Reg) -> Result<Value, Thrown> {
        let slot = &self.stack[top].regs[r as usize];
        if let Value::Ref(rc) = slot {
            if let Ok(v) = rc.try_borrow() {
                if let Some(s) = scalar_copy(&v) {
                    return Ok(s);
                }
            }
        } else if let Some(s) = scalar_copy(slot) {
            return Ok(s);
        }
        self.reg_r(top, r)
    }

    /// php's zval_undefined_cv: the warning for a read of undefined CV `r`
    /// (null after it).
    #[cold]
    #[inline(never)]
    pub(crate) fn undef_cv(&mut self, top: usize, r: Reg) -> Result<Value, Thrown> {
        let proto = &self.stack[top].proto;
        // A temporary holding a copied undefined slot reads as null.
        if r >= proto.num_cvs {
            return Ok(Value::Null);
        }
        let name = proto
            .cv_var_names
            .get(r as usize)
            .map(|n| n.as_bytes().to_vec())
            .unwrap_or_default();
        let msg = [b"Undefined variable $".as_slice(), &name].concat();
        self.diag(builtins::E_WARNING, &msg)?;
        Ok(Value::Null)
    }

    /// `LvalVarVar` / `LvalVarVarRw`: a cursor rooted at the variable named
    /// by `name` (created; a read-modify-write warns first when undefined).
    fn lval_var_var(&mut self, top: usize, name: Reg, rw: bool) -> Result<(), Thrown> {
        let vname = conv::to_string_bytes(&self.reg_r(top, name)?);
        if vname == b"this" {
            return Err(Thrown::error("Error", "Cannot re-assign $this"));
        }
        if rw {
            let undef = match self.var_var_lookup(top, &vname) {
                Some(Ok(cv)) => self.stack[top].regs[cv as usize].is_undef(),
                Some(Err(cell)) => cell.borrow().is_undef(),
                None => true,
            };
            if undef {
                let msg = [b"Undefined variable $".as_slice(), &vname].concat();
                self.diag(builtins::E_WARNING, &msg)?;
            }
        }
        self.cursor_root = match self.var_var_slot(top, &vname) {
            Ok(cv) => {
                self.cv_define(top, cv);
                CursorRoot::Cv(cv)
            }
            Err(cell) => {
                cell_define(&cell);
                CursorRoot::Cell(cell)
            }
        };
        self.cursor_global = None;
        self.cursor_steps.clear();
        self.rw_null_keys = 0;
        self.cursor_null_key = false;
        Ok(())
    }

    /// `LvalGlobal` / `LvalGlobalRw`: a cursor rooted at global `name`
    /// (`$GLOBALS[...]` as an lvalue; a read-modify-write warns first when
    /// it is undefined).
    fn lval_global(&mut self, top: usize, name: Reg, rw: bool) -> Result<(), Thrown> {
        let n = conv::to_string_bytes(&self.reg_r(top, name)?);
        if rw
            && self
                .globals
                .get(&n[..])
                .is_none_or(|c| c.borrow().is_undef())
        {
            let msg = [b"Undefined global variable $".as_slice(), &n].concat();
            self.diag(builtins::E_WARNING, &msg)?;
        }
        let cell = self.global_cell(&n);
        cell_define(&cell);
        self.cursor_root = CursorRoot::Cell(cell);
        self.cursor_global = Some(n);
        self.cursor_steps.clear();
        self.rw_null_keys = 0;
        self.cursor_null_key = false;
        Ok(())
    }

    /// An undefined variable as a key of the open cursor: the warning may
    /// run an error handler, which must not disturb the cursor. php has
    /// fetched the container already (a null one made an array): when the
    /// handler replaces the variable, the write still goes to that
    /// container (an object's offsetSet) and is lost for the variable.
    #[cold]
    fn lval_undef_key(&mut self, top: usize, key: Reg) -> Result<Value, Thrown> {
        let target = match self.cursor_root {
            CursorRoot::Cv(cv) => match self.reg_get(top, cv) {
                Value::Null => {
                    let a = Value::Array(PhpArray::new());
                    self.cv_assign(top, cv, a.clone());
                    Some((cv, a))
                }
                v @ (Value::Array(_) | Value::Object(_)) => Some((cv, v)),
                _ => None,
            },
            CursorRoot::Cell(_) => None,
        };
        let r = self.undef_cv(top, key);
        if let Some((cv, t)) = target {
            let same = match (&t, &self.reg_get(top, cv)) {
                (Value::Array(a), Value::Array(b)) => a.data_ptr() == b.data_ptr(),
                (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
                _ => false,
            };
            if !same {
                self.cursor_root = CursorRoot::Cell(Rc::new(RefCell::new(t)));
            }
        }
        r
    }

    /// Run `f` (which may call user code) with the open cursor set aside.
    pub(crate) fn with_saved_cursor<T>(&mut self, f: impl FnOnce(&mut Vm) -> T) -> T {
        let root = self.cursor_root.clone();
        let steps = std::mem::take(&mut self.cursor_steps);
        let global = self.cursor_global.take();
        let nulls = self.rw_null_keys;
        let null_key = self.cursor_null_key;
        let r = f(self);
        self.cursor_root = root;
        self.cursor_steps = steps;
        self.cursor_global = global;
        self.rw_null_keys = nulls;
        self.cursor_null_key = null_key;
        r
    }

    /// `unset($x)`: CV `cv` of frame `top` becomes undefined. In the global
    /// scope the symbol-table entry is dropped (a reference to it keeps the
    /// old cell).
    pub(crate) fn unset_cv(&mut self, top: usize, cv: Reg) {
        let f = &self.stack[top];
        if f.global_scope
            && (cv as usize) < f.proto.cv_var_names.len()
            && matches!(f.regs[cv as usize], Value::Ref(_))
        {
            self.stack[top].regs[cv as usize] = Value::Undef;
            self.sync_global_cv(top, cv);
            self.dtor_check = true;
        } else {
            if matches!(self.stack[top].regs[cv as usize], Value::Ref(_)) {
                self.dtor_check = true;
            }
            self.reg_set(top, cv, Value::Undef);
        }
    }

    /// Plain register write (temporaries / instruction destinations): overwrites
    /// the slot. Never writes through a stale `Ref` left in a reused temp.
    pub(crate) fn reg_set(&mut self, top: usize, r: Reg, v: Value) {
        let slot = &mut self.stack[top].regs[r as usize];
        if !self.destructibles.is_empty() && matches!(slot, Value::Object(_) | Value::Array(_)) {
            self.dtor_check = true;
        }
        *slot = v;
    }

    /// [`Vm::reg_set`], inline when the old value is a scalar: nothing to
    /// drop or destruct.
    #[inline(always)]
    fn reg_set_fast(&mut self, top: usize, r: Reg, v: Value) {
        let slot = &mut self.stack[top].regs[r as usize];
        if is_scalar(slot) {
            std::mem::forget(std::mem::replace(slot, v));
        } else {
            self.reg_set(top, r, v);
        }
    }

    /// The integer fast path of a binary instruction: when both operands
    /// are ints and `f` gives a result, write it to `dst` and say so.
    #[inline(always)]
    fn int_binop(
        &mut self,
        top: usize,
        dst: Reg,
        a: Reg,
        b: Reg,
        f: impl FnOnce(i64, i64) -> Option<Value>,
    ) -> bool {
        let regs = &mut self.stack[top].regs;
        let (Some(x), Some(y)) = (int_of(&regs[a as usize]), int_of(&regs[b as usize])) else {
            return false;
        };
        let Some(v) = f(x, y) else {
            return false;
        };
        let slot = &mut regs[dst as usize];
        if is_scalar(slot) {
            std::mem::forget(std::mem::replace(slot, v));
        } else {
            self.reg_set(top, dst, v);
        }
        true
    }

    /// Write `v` to CV `cv` as [`Vm::cv_assign`] does, inline when the old
    /// value (behind a reference too) is a scalar.
    #[inline(always)]
    fn cv_assign_fast(&mut self, top: usize, cv: Reg, v: Value) {
        let slot = &mut self.stack[top].regs[cv as usize];
        if let Value::Ref(rc) = slot {
            if let Ok(mut cell) = rc.try_borrow_mut() {
                if is_scalar(&cell) {
                    std::mem::forget(std::mem::replace(&mut *cell, v));
                    return;
                }
            }
        } else if is_scalar(slot) {
            std::mem::forget(std::mem::replace(slot, v));
            return;
        }
        self.cv_assign(top, cv, v);
    }

    /// Assign to a PHP *variable* (a CV): if the slot is a reference, write
    /// through it (so by-ref aliases see the change); otherwise overwrite.
    pub(crate) fn cv_assign(&mut self, top: usize, cv: Reg, v: Value) {
        let slot = &mut self.stack[top].regs[cv as usize];
        let old = if let Value::Ref(rc) = slot {
            std::mem::replace(&mut *rc.borrow_mut(), v)
        } else {
            std::mem::replace(slot, v)
        };
        if !self.destructibles.is_empty() && matches!(old, Value::Object(_) | Value::Array(_)) {
            self.dtor_check = true;
        }
    }

    /// Drive execution, catching thrown exceptions and routing them through the
    /// exception table (per-frame handlers / finally), unwinding frames until a
    /// handler is found or the program's top frame is reached.
    pub(crate) fn run_loop(&mut self) -> Result<Value, Thrown> {
        loop {
            match self.run_frames() {
                Ok(v) => return Ok(v),
                Err(thrown) => {
                    // An `exit` in a destructor run while unwinding ends the
                    // request like any other exit.
                    let thrown = if thrown.is_catchable() {
                        match self.unwind(thrown, 0) {
                            Ok(UnwindResult::Caught) => {
                                // Unwinding may have released objects (a builtin's
                                // temporaries): their destructors run before the handler.
                                if !self.destructibles.is_empty() {
                                    self.dtor_check = true;
                                }
                                continue; // handler installed; re-enter
                            }
                            Ok(UnwindResult::Escaped(t)) => return Err(t), // uncaught
                            Err(t) if t.is_catchable() => return Err(t),
                            Err(t) => t,
                        }
                    } else {
                        thrown
                    };
                    // Exit / Unsupported: straight to the SAPI (keeping
                    // {main}'s globals for the shutdown sequence). A
                    // refusal raised by a builtin is located first.
                    let thrown = self.locate_unsupported(thrown);
                    self.stack.truncate(1);
                    if let Some(main) = self.stack.pop() {
                        self.finished_main = Some(main);
                    }
                    return Err(thrown);
                }
            }
        }
    }

    pub(crate) fn run_frames(&mut self) -> Result<Value, Thrown> {
        // The running frame and its proto, taken again only when an
        // instruction pushed or popped a frame (a call, a return): within
        // one instruction the top frame changes only that way, since
        // nested runs leave the stack as they found it.
        let mut top = self.stack.len() - 1;
        let mut proto = self.stack[top].proto.clone();
        loop {
            if self.stack.len() != top + 1 {
                top = self.stack.len() - 1;
                proto = self.stack[top].proto.clone();
            }
            debug_assert!(Rc::ptr_eq(&proto, &self.stack[top].proto));
            let frame = &mut self.stack[top];
            let pc = frame.pc;
            if pc < proto.code.len() && !self.mem.attention() {
                frame.pc = pc + 1;
            } else {
                // The frame's pc is still `pc` here (oom_tick reports at it).
                if self.mem.attention() {
                    self.oom_tick(top, pc)?;
                }
                if pc >= proto.code.len() {
                    // fell off the end == RetNull
                    if let Some(done) = self.do_return(Value::Null) {
                        return Ok(done);
                    }
                    continue;
                }
                self.stack[top].pc = pc + 1;
            }
            let instr = proto.code[pc];

            // Run destructors of objects the program has just released.
            if self.dtor_check {
                self.dtor_check = false;
                if !self.destructibles.is_empty() {
                    // Destructors belong to the releasing instruction: they
                    // report its line and its try catches their exceptions.
                    if pc > 0 {
                        self.stack[top].pc = pc;
                    }
                    self.collect_released()?;
                    self.stack[top].pc = pc;
                    continue;
                }
            }

            // pcntl async signals (php's `EG(vm_interrupt)` check): run queued
            // PHP signal handlers before the next instruction.
            if builtins::pcntl_interrupt_pending() {
                // max_execution_time ran out (zend_timeout): the fatal error
                // comes where a jump lands or a user function starts.
                if builtins::zend::timeout::timed_out() {
                    if !proto.is_internal() {
                        let landed = match self.timeout_prev {
                            Some((d, p)) if d == top => pc != p + 1,
                            Some((d, _)) => d < top && pc == 0,
                            None => false,
                        };
                        let mut at_check = self.timeout_wait == Some((top, pc));
                        if landed && !at_check {
                            match loop_body_start(&proto.code, pc) {
                                Some(body) if body != pc => {
                                    self.timeout_wait = Some((top, body));
                                }
                                _ => at_check = true,
                            }
                        }
                        if at_check {
                            self.timeout_wait = None;
                            if let Some(msg) = builtins::zend::timeout::take_timed_out() {
                                self.timeout_prev = None;
                                // Located at this instruction (pc + 1, as
                                // when it runs).
                                self.stack[top].pc = pc + 1;
                                return Err(self.fatal_error_here(1, msg.as_bytes()));
                            }
                        }
                        self.timeout_prev = Some((top, pc));
                    }
                } else {
                    // A throwing handler raises at this (not yet run)
                    // instruction; otherwise the instruction then runs as
                    // usual.
                    builtins::pcntl_dispatch(self)?;
                    self.stack[top].pc = pc;
                    continue;
                }
            }

            match instr {
                Instr::Nop => {}
                Instr::Ticks { n } => {
                    self.ticks_count += 1;
                    if self.ticks_count >= n {
                        self.ticks_count = 0;
                        builtins::run_tick_functions(self)?;
                    }
                }
                Instr::Mov { dst, src } => {
                    let v = self.reg_r_fast(top, src)?;
                    self.reg_set_fast(top, dst, v);
                }
                Instr::MovTake { dst, src } => {
                    let v = self.take_temp(top, src);
                    self.reg_set_fast(top, dst, v);
                }
                Instr::LoadConst { dst, k } => {
                    let v = proto.consts[k as usize].clone();
                    self.reg_set_fast(top, dst, v);
                }
                Instr::LoadNull { dst } => self.reg_set(top, dst, Value::Null),
                Instr::UnsetCv { cv } => self.unset_cv(top, cv),
                Instr::CheckCv { cv } => {
                    if self.stack[top].regs[cv as usize].is_undef() {
                        self.undef_cv(top, cv)?;
                    }
                }
                Instr::MovIs { dst, src } => {
                    let slot = &self.stack[top].regs[src as usize];
                    if let Some(s) = scalar_copy(slot) {
                        self.reg_set_fast(top, dst, s);
                        continue;
                    }
                    let v = match &self.stack[top].regs[src as usize] {
                        Value::Ref(rc) => rc.borrow().copy_slot(),
                        v => v.copy_slot(),
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::LoadBool { dst, b } => self.reg_set_fast(top, dst, Value::Bool(b)),
                Instr::LoadInt { dst, n } => self.reg_set_fast(top, dst, Value::Int(n)),

                Instr::Assign { cv, src } => {
                    let v = self.reg_r_fast(top, src)?;
                    self.cv_assign_fast(top, cv, v);
                }

                // Int operands that don't overflow: what ops::add/sub/mul
                // return for them.
                Instr::Add { dst, a, b } => {
                    if !self.int_binop(top, dst, a, b, |x, y| x.checked_add(y).map(Value::Int)) {
                        self.bin(top, dst, a, b, Op::Add, ops::add)?
                    }
                }
                Instr::Sub { dst, a, b } => {
                    if !self.int_binop(top, dst, a, b, |x, y| x.checked_sub(y).map(Value::Int)) {
                        self.bin(top, dst, a, b, Op::Sub, ops::sub)?
                    }
                }
                Instr::Mul { dst, a, b } => {
                    if !self.int_binop(top, dst, a, b, |x, y| x.checked_mul(y).map(Value::Int)) {
                        self.bin(top, dst, a, b, Op::Mul, ops::mul)?
                    }
                }
                Instr::Div { dst, a, b } => self.bin(top, dst, a, b, Op::Div, ops::div)?,
                Instr::Mod { dst, a, b } => {
                    let m = |x: i64, y: i64| (y != 0 && y != -1).then(|| Value::Int(x % y));
                    if !self.int_binop(top, dst, a, b, m) {
                        self.bin_inf(top, dst, a, b, Op::Mod)?
                    }
                }
                Instr::Pow { dst, a, b } => {
                    let (x, y) = (self.reg_r(top, a)?, self.reg_r(top, b)?);
                    let v = self.pow_op(&x, &y)?;
                    self.reg_set(top, dst, v);
                }
                // Int operands (shift counts 0..63): what ops::bitwise
                // returns for them.
                Instr::BitAnd { dst, a, b } => {
                    if !self.int_binop(top, dst, a, b, |x, y| Some(Value::Int(x & y))) {
                        self.bin_inf(top, dst, a, b, Op::BitAnd)?
                    }
                }
                Instr::BitOr { dst, a, b } => {
                    if !self.int_binop(top, dst, a, b, |x, y| Some(Value::Int(x | y))) {
                        self.bin_inf(top, dst, a, b, Op::BitOr)?
                    }
                }
                Instr::BitXor { dst, a, b } => {
                    if !self.int_binop(top, dst, a, b, |x, y| Some(Value::Int(x ^ y))) {
                        self.bin_inf(top, dst, a, b, Op::BitXor)?
                    }
                }
                Instr::Shl { dst, a, b } => {
                    let shl = |x: i64, y: i64| {
                        (0..64)
                            .contains(&y)
                            .then(|| Value::Int(x.wrapping_shl(y as u32)))
                    };
                    if !self.int_binop(top, dst, a, b, shl) {
                        self.bin_inf(top, dst, a, b, Op::Shl)?
                    }
                }
                Instr::Shr { dst, a, b } => {
                    let shr = |x: i64, y: i64| (0..64).contains(&y).then(|| Value::Int(x >> y));
                    if !self.int_binop(top, dst, a, b, shr) {
                        self.bin_inf(top, dst, a, b, Op::Shr)?
                    }
                }
                Instr::Concat { dst, a, b } => {
                    if self.concat_in_place(top, &proto.code, pc, proto.num_cvs, dst, a, b) {
                        continue;
                    }
                    let (x, y) = (self.reg_r(top, a)?, self.reg_r(top, b)?);
                    let slow = |v: &Value| match v {
                        Value::Object(_) | Value::Array(_) | Value::Closure(_) => true,
                        Value::Float(f) => f.is_nan(),
                        _ => false,
                    };
                    let v = if slow(&x) || slow(&y) {
                        self.concat_object(&x, &y)?
                    } else {
                        ops::concat(&x, &y)
                    };
                    self.reg_set(top, dst, v);
                }

                Instr::Neg { dst, a } => {
                    let v = self.reg_r(top, a)?;
                    let r = match v {
                        Value::Int(n) if n != i64::MIN => Value::Int(-n),
                        Value::Float(f) => Value::Float(-f),
                        // php compiles `-$x` to `$x * -1`.
                        _ => self.bin_slow(Op::Mul, ops::mul, &v, &Value::Int(-1))?,
                    };
                    self.reg_set(top, dst, r);
                }
                Instr::BitNot { dst, a } => {
                    let v = self.reg_r(top, a)?;
                    let r = match v {
                        Value::Int(n) => Value::Int(!n),
                        _ => self.bitwise_not_slow(&v)?,
                    };
                    self.reg_set(top, dst, r);
                }
                Instr::Not { dst, a } => {
                    let v = self.reg_r(top, a)?;
                    let b = self.truthy(&v)?;
                    self.reg_set(top, dst, Value::Bool(!b));
                }
                Instr::IncDec {
                    dst,
                    cv,
                    pre,
                    delta,
                } => {
                    // An int that doesn't overflow: inc_dec_value's result,
                    // written in place (through a reference too).
                    let regs = &mut self.stack[top].regs;
                    if let Some(old) = int_in_place(&mut regs[cv as usize], delta as i64) {
                        let r = old + if pre { delta as i64 } else { 0 };
                        self.reg_set_fast(top, dst, Value::Int(r));
                        continue;
                    }
                    // An undefined variable warns, then counts as null
                    // even when the handler threw (zend_pre_inc_helper).
                    let undef = if self.stack[top].regs[cv as usize].is_undef() {
                        let r = self.undef_cv(top, cv).err();
                        self.cv_assign(top, cv, Value::Null);
                        r
                    } else {
                        None
                    };
                    let cur = self.reg_get(top, cv);
                    let new = self.inc_dec_vm(&cur, delta as i64)?;
                    self.cv_assign(top, cv, new.clone());
                    self.reg_set(top, dst, if pre { new } else { cur });
                    if let Some(e) = undef {
                        return Err(e);
                    }
                }

                Instr::CastInt { dst, a } => {
                    let v = self.reg_r(top, a)?;
                    // zval_get_long: an object's cast_object (curl handles:
                    // the id), else a warning.
                    let n = match v {
                        Value::Object(_) | Value::Closure(_) => {
                            match builtins::zend::object_handlers::object_to_number(
                                self,
                                &v,
                                CastTarget::Int,
                            )? {
                                Some(n) => conv::to_int(&n),
                                None => conv::to_int(&v),
                            }
                        }
                        Value::Float(f) => self.dval_to_lval(f)?,
                        _ => conv::to_int(&v),
                    };
                    self.reg_set(top, dst, Value::Int(n));
                }
                Instr::CastFloat { dst, a } => {
                    let v = self.reg_r(top, a)?;
                    let f = match v {
                        Value::Object(_) | Value::Closure(_) => {
                            match builtins::zend::object_handlers::object_to_number(
                                self,
                                &v,
                                CastTarget::Float,
                            )? {
                                Some(f) => conv::to_float(&f),
                                None => conv::to_float(&v),
                            }
                        }
                        _ => conv::to_float(&v),
                    };
                    self.reg_set(top, dst, Value::Float(f));
                }
                Instr::CastString { dst, a } => {
                    let v = self.reg_r(top, a)?;
                    let s = self.to_string_vm(&v)?;
                    self.reg_set(top, dst, Value::str(s));
                }
                Instr::CastBool { dst, a } => {
                    let v = self.reg_r(top, a)?;
                    let b = self.truthy(&v)?;
                    self.reg_set(top, dst, Value::Bool(b));
                }
                Instr::CastArray { dst, a } => {
                    let v = self.reg_r(top, a)?;
                    let out = match &v {
                        // get_properties_for(ARRAY_CAST): an internal class's
                        // view (an ArrayObject's storage).
                        Value::Object(h) => {
                            let purpose = builtins::zend::object_handlers::PropPurpose::ArrayCast;
                            match builtins::zend::object_handlers::properties_for(self, h, purpose)?
                            {
                                Some(a) => Value::Array(a),
                                None => self.object_cast_array(h),
                            }
                        }
                        Value::Float(f) if f.is_nan() => {
                            self.nan_warning("array")?;
                            cast_to_array(v)
                        }
                        _ => cast_to_array(v),
                    };
                    self.reg_set(top, dst, out);
                }
                Instr::CastObject { dst, a } => {
                    let v = self.reg_r(top, a)?;
                    if let Value::Float(f) = v {
                        if f.is_nan() {
                            self.nan_warning("object")?;
                        }
                    }
                    let obj = self.cast_to_object(v);
                    self.reg_set(top, dst, obj);
                }

                Instr::Cmp { dst, a, b, kind } => {
                    // Two ints: what eval_cmp returns (compare_values
                    // raises a waiting pending_throw, so not then).
                    let int_cmp = |x: i64, y: i64| {
                        Some(match kind {
                            CmpKind::Eq | CmpKind::Identical => Value::Bool(x == y),
                            CmpKind::Ne | CmpKind::NotIdentical => Value::Bool(x != y),
                            CmpKind::Lt => Value::Bool(x < y),
                            CmpKind::Le => Value::Bool(x <= y),
                            CmpKind::Gt => Value::Bool(x > y),
                            CmpKind::Ge => Value::Bool(x >= y),
                            CmpKind::Spaceship => Value::Int(x.cmp(&y) as i64),
                        })
                    };
                    if self.pending_throw.is_none() && self.int_binop(top, dst, a, b, int_cmp) {
                        continue;
                    }
                    let (x, y) = (self.reg_r(top, a)?, self.reg_r(top, b)?);
                    let r = self.eval_cmp(kind, &x, &y)?;
                    self.reg_set(top, dst, r);
                }
                Instr::BeginSilence => self.begin_silence(),
                Instr::EndSilence => self.end_silence(),

                Instr::Echo { src } => {
                    let v = self.reg_r(top, src)?;
                    let bytes = self.to_string_vm(&v)?;
                    self.echo_out(&bytes)?;
                }
                Instr::EchoConst { k } => {
                    let bytes = conv::to_string_bytes(&proto.consts[k as usize]);
                    self.echo_out(&bytes)?;
                }
                Instr::Print { dst, src } => {
                    let v = self.reg_r(top, src)?;
                    let bytes = self.to_string_vm(&v)?;
                    self.echo_out(&bytes)?;
                    self.reg_set(top, dst, Value::Int(1));
                }

                Instr::Jmp { target } => self.stack[top].pc = target as usize,
                Instr::JmpIfArg { index, target } => {
                    let f = &self.stack[top];
                    if f.argc > index as usize && (index >= 64 || f.holes & (1u64 << index) == 0) {
                        self.stack[top].pc = target as usize;
                    }
                }
                Instr::RecvInitCheck { index } => self.recv_init_check(top, index)?,
                Instr::JmpTrue { cond, target } => {
                    let f = &mut self.stack[top];
                    if let Value::Bool(b) = f.regs[cond as usize] {
                        if b {
                            f.pc = target as usize;
                        }
                    } else {
                        let c = self.reg_r(top, cond)?;
                        if self.truthy(&c)? {
                            self.stack[top].pc = target as usize;
                        }
                    }
                }
                Instr::JmpFalse { cond, target } => {
                    let f = &mut self.stack[top];
                    if let Value::Bool(b) = f.regs[cond as usize] {
                        if !b {
                            f.pc = target as usize;
                        }
                    } else {
                        let c = self.reg_r(top, cond)?;
                        if !self.truthy(&c)? {
                            self.stack[top].pc = target as usize;
                        }
                    }
                }
                Instr::JmpNull { src, target } => {
                    if matches!(self.reg_get(top, src), Value::Null) {
                        self.stack[top].pc = target as usize;
                    }
                }
                Instr::JmpNotNull { src, target } => {
                    if !matches!(self.reg_get(top, src), Value::Null) {
                        self.stack[top].pc = target as usize;
                    }
                }

                Instr::NewArray { dst, .. } => {
                    self.reg_set(top, dst, Value::Array(rhp_values::PhpArray::new()));
                }
                Instr::ArrPush { arr, val } => {
                    let v = self.elem_operand(top, &proto, val)?;
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        if !a.try_push(v) {
                            return Err(crate::lvalue::cannot_add_element());
                        }
                    }
                }
                // ZEND_ADD_ARRAY_ELEMENT reads the value, then the key (an
                // undefined variable is "" without the null deprecation).
                Instr::ArrSet { arr, key, val } => {
                    let v = self.elem_operand(top, &proto, val)?;
                    let key = if self.stack[top].regs[key as usize].is_undef() {
                        self.reg_r(top, key)?;
                        ArrayKey::from_bytes(b"")
                    } else {
                        let k = self.reg_get(top, key);
                        self.init_array_key(&k)?
                    };
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        a.insert(key, v);
                    }
                }
                Instr::ArrPushRef { arr, val } => {
                    let v = self.stack[top].regs[val as usize].clone();
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        if !a.try_push(v) {
                            return Err(crate::lvalue::cannot_add_element());
                        }
                    }
                }
                Instr::ArrSetRef { arr, key, val } => {
                    let k = self.reg_r(top, key)?;
                    let v = self.stack[top].regs[val as usize].clone();
                    let key = self.init_array_key(&k)?;
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        a.insert(key, v);
                    }
                }
                Instr::ArrSpread { arr, src } => {
                    let entries = self.unpack_entries(top, src, false)?;
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        for (k, iv) in entries {
                            match k {
                                ArrayKey::Int(_) => {
                                    if !a.try_push(iv) {
                                        return Err(crate::lvalue::cannot_add_element());
                                    }
                                }
                                ArrayKey::Str(_) => a.insert(k, iv),
                            }
                        }
                    }
                }
                Instr::ArgSpread { arr, src } => {
                    let entries = self.unpack_entries(top, src, true)?;
                    let Value::Array(mut a) =
                        std::mem::replace(&mut self.stack[top].regs[arr as usize], Value::Null)
                    else {
                        unreachable!("ArgSpread target is the argument array")
                    };
                    for (k, iv) in entries {
                        match k {
                            ArrayKey::Int(_) => a.push(iv),
                            ArrayKey::Str(s) => {
                                let key = ArrayKey::Str(s.clone());
                                if a.get(&key).is_some() {
                                    return Err(Thrown::error(
                                        "Error",
                                        format!(
                                            "Named parameter ${} overwrites previous argument",
                                            String::from_utf8_lossy(s.as_bytes())
                                        ),
                                    ));
                                }
                                a.insert(key, iv);
                            }
                        }
                    }
                    self.stack[top].regs[arr as usize] = Value::Array(a);
                }

                Instr::Call {
                    dst,
                    name,
                    base,
                    argc,
                    fallback,
                    flags,
                } => match self.call_target(top, &proto, pc, name, fallback)? {
                    CallTarget::User(i) => {
                        // User function: push a frame (args bound to its CVs).
                        let callee = self.functions[i].clone();
                        self.enter_user_call(top, callee, dst, base, argc)?;
                    }
                    CallTarget::Builtin(builtin, sig) => {
                        let inline = flags.contains(CallFlags::INLINE);
                        let icall = flags.contains(CallFlags::ICALL);
                        self.cuf_caller = flags
                            .contains(CallFlags::USER_CALL)
                            .then(|| self.caller_strict());
                        if flags.contains(CallFlags::RESULT_UNUSED) {
                            self.unused_call_depth = self.stack.len();
                        }
                        let out = self.call_builtin(top, builtin, sig, base, argc, inline, icall);
                        self.cuf_caller = None;
                        self.unused_call_depth = usize::MAX;
                        let out = out?;
                        self.reg_set(top, dst, out);
                    }
                    CallTarget::Missing => {
                        return Err(undefined_function(&proto.names[name as usize]));
                    }
                },

                Instr::MakeClosure {
                    dst,
                    fnidx,
                    base,
                    ncaptures,
                } => {
                    self.make_closure(top, dst, fnidx, base, ncaptures)?;
                }
                Instr::BindGlobal { cv, name } => {
                    let gname = proto.names[name as usize].clone();
                    let cell = self.global_cell(gname.as_bytes());
                    cell_define(&cell);
                    self.ref_globals.insert(gname.as_bytes().to_vec());
                    self.stack[top].regs[cv as usize] = Value::Ref(cell);
                }
                Instr::BindGlobalDyn { name } => {
                    let gname = conv::to_string_bytes(&self.reg_r(top, name)?);
                    let cell = self.global_cell(&gname);
                    cell_define(&cell);
                    self.ref_globals.insert(gname.clone());
                    self.var_var_bind(top, &gname, cell);
                }
                Instr::BindStatic { cv, name, init } => {
                    self.bind_static(top, &proto, cv, name, init);
                }
                Instr::BindStaticOrJmp { cv, name, target } => {
                    if self.stack[top].closure.is_some() {
                        if self.bind_closure_static(top, cv, &proto.names[name as usize], None) {
                            self.stack[top].pc = target as usize;
                        }
                        continue;
                    }
                    let sname = proto.names[name as usize].as_bytes().to_vec();
                    let key = self.static_key(top, &proto, sname);
                    if let Some(c) = self.static_table(top).get(&key) {
                        let cell = c.clone();
                        self.stack[top].regs[cv as usize] = Value::Ref(cell);
                        self.sync_global_cv(top, cv);
                        self.stack[top].pc = target as usize;
                    }
                }
                Instr::BindStaticInit { cv, name, src } => {
                    let v = self.reg_get(top, src);
                    if self.stack[top].closure.is_some() {
                        self.bind_closure_static(top, cv, &proto.names[name as usize], Some(v));
                        continue;
                    }
                    let sname = proto.names[name as usize].as_bytes().to_vec();
                    let key = self.static_key(top, &proto, sname);
                    let cell = self
                        .static_table(top)
                        .entry(key)
                        .or_insert_with(|| Rc::new(RefCell::new(v)))
                        .clone();
                    self.stack[top].regs[cv as usize] = Value::Ref(cell);
                    self.sync_global_cv(top, cv);
                }
                Instr::DeclareConst { name, src } => {
                    let cname = proto.names[name as usize].as_bytes();
                    let key = builtins::zend::builtin_functions::const_key(cname);
                    let v = self.reg_get(top, src);
                    if self.const_taken(&key) {
                        let msg = builtins::zend::builtin_functions::const_redefined_message(cname);
                        self.diag(builtins::E_WARNING, &msg)?;
                    } else {
                        self.constants.insert(key.into_owned(), v);
                        self.user_const_order.push(cname.to_vec());
                    }
                }
                Instr::DeclareAttrConst { name, src, attrs } => {
                    let cname = proto.names[name as usize].as_bytes();
                    let key = builtins::zend::builtin_functions::const_key(cname);
                    let v = self.reg_get(top, src);
                    if self.const_taken(&key) {
                        let msg = builtins::zend::builtin_functions::const_redefined_message(cname);
                        self.diag(builtins::E_WARNING, &msg)?;
                    } else {
                        let unit = self.stack[top].unit;
                        self.const_attrs.insert(
                            key.to_vec(),
                            crate::attributes::ConstAttrs {
                                attrs: proto.const_attrs[attrs as usize].clone(),
                                unit,
                            },
                        );
                        self.constants.insert(key.into_owned(), v);
                        self.user_const_order.push(cname.to_vec());
                    }
                }
                Instr::MakeFcc {
                    dst,
                    callee,
                    ns_name,
                } => {
                    self.make_fcc(top, &proto, dst, callee, ns_name)?;
                }
                Instr::CallDyn {
                    dst,
                    callee,
                    base,
                    argc,
                } => {
                    self.call_dyn(top, dst, callee, base, argc)?;
                }
                Instr::CallDynV { dst, callee, args } => {
                    let c = self.reg_r(top, callee)?;
                    let arr = self.reg_r(top, args)?;
                    let (argvec, named) = self.split_call_args(&arr)?;
                    let strict = self.caller_strict();
                    self.dyn_call = true;
                    let out = self.call_named(&c, argvec, named, Some(strict));
                    self.dyn_call = false;
                    let out = out?;
                    self.reg_set(top, dst, out);
                }

                // -- array element read --
                Instr::DimFetch { dst, arr, key } => {
                    let a = self.reg_r(top, arr)?;
                    let k = self.reg_r(top, key)?;
                    let r = self.dim_read(&a, &k, DimMode::Read)?;
                    self.reg_set(top, dst, r);
                }
                Instr::DimFetchW { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_r(top, key)?;
                    let r = match deref(&a) {
                        Value::Object(h)
                            if self.class_implements(h.borrow().class, b"ArrayAccess") =>
                        {
                            let step = CursorStep::Key(k.clone());
                            let cell = self.aa_fetch_w(&h, &step, k, DimFetch::W)?;
                            let v = cell.borrow().clone();
                            v
                        }
                        // The object of a property write (FETCH_DIM_W then
                        // ASSIGN_OBJ & co.): zend_wrong_string_offset_error.
                        Value::Str(_) => return Err(self.string_offset_as_object(top, &k)),
                        // A write fetch on null makes the element (null) quietly.
                        Value::Null => Value::Null,
                        _ => self.dim_read(&a, &k, DimMode::Read)?,
                    };
                    self.reg_set(top, dst, r);
                }
                Instr::DimFetchUnset { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_r(top, key)?;
                    let r = match deref(&a) {
                        Value::Array(_) => self.dim_read(&a, &k, DimMode::Is)?,
                        Value::Str(_) => return Err(self.string_offset_as_object(top, &k)),
                        Value::Null => Value::Null,
                        Value::Bool(false) => {
                            self.diag(
                                builtins::E_DEPRECATED,
                                "Automatic conversion of false to array is deprecated",
                            )?;
                            Value::Null
                        }
                        Value::Object(_) | Value::Closure(_) => {
                            self.dim_read(&a, &k, DimMode::Read)?
                        }
                        _ => {
                            return Err(Thrown::error(
                                "Error",
                                "Cannot unset offset in a non-array variable",
                            ))
                        }
                    };
                    self.reg_set(top, dst, r);
                }
                Instr::DimFetchIs { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_r(top, key)?;
                    let r = self.dim_read(&a, &k, DimMode::Is)?;
                    self.reg_set(top, dst, r);
                }
                Instr::DimFetchList { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_r(top, key)?;
                    let r = self.dim_read(&a, &k, DimMode::List)?;
                    self.reg_set(top, dst, r);
                }

                // -- lvalue cursor --
                Instr::LvalCvRw { cv } => {
                    if self.stack[top].regs[cv as usize].is_undef() {
                        self.undef_cv(top, cv)?;
                    }
                    self.cv_define(top, cv);
                    self.cursor_root = CursorRoot::Cv(cv);
                    self.cursor_global = None;
                    self.cursor_steps.clear();
                    self.rw_null_keys = 0;
                    self.cursor_null_key = false;
                }
                Instr::LvalCvUnset { cv } => {
                    // Unsetting below an undefined variable leaves it so.
                    self.cursor_root = if self.stack[top].regs[cv as usize].is_undef() {
                        self.undef_cv(top, cv)?;
                        CursorRoot::Cell(Rc::new(RefCell::new(Value::Null)))
                    } else {
                        CursorRoot::Cv(cv)
                    };
                    self.cursor_global = None;
                    self.cursor_steps.clear();
                    self.rw_null_keys = 0;
                    self.cursor_null_key = false;
                }
                // The variable is created when the write lands (after its
                // value was read: `$a[0] = $a` reads an undefined `$a`):
                // `assign_path` / `make_ref_path` see `Undef` as null.
                Instr::LvalCv { cv } => {
                    self.cursor_root = CursorRoot::Cv(cv);
                    self.cursor_global = None;
                    self.cursor_steps.clear();
                    self.rw_null_keys = 0;
                    self.cursor_null_key = false;
                }
                Instr::LvalKey { key } => {
                    let k = if self.stack[top].regs[key as usize].is_undef() {
                        self.lval_undef_key(top, key)?
                    } else {
                        self.reg_get(top, key)
                    };
                    if let Value::Null = k {
                        self.cursor_null_key = true;
                    }
                    self.cursor_steps.push(CursorStep::Key(k));
                }
                Instr::LvalAppend => self.cursor_steps.push(CursorStep::Append),
                Instr::LvalProp { name } => {
                    let p = proto.names[name as usize].clone();
                    self.cursor_steps.push(CursorStep::Prop(p));
                }
                Instr::LvalPropDyn { name } => {
                    let p = self.dyn_prop_name(top, name)?;
                    self.cursor_steps.push(CursorStep::Prop(p));
                }
                Instr::LvalStaticProp { class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = proto.names[name as usize].as_bytes().to_vec();
                    self.check_static_prop_access(top, cls, &pname)?;
                    let cell = self.static_prop_cell(cls, &pname)?;
                    self.cursor_root = CursorRoot::Cell(cell);
                    self.cursor_global = None;
                    self.cursor_steps.clear();
                    self.rw_null_keys = 0;
                    self.cursor_null_key = false;
                }
                Instr::LvalStaticPropDyn { class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = conv::to_string_bytes(&self.reg_r(top, name)?);
                    self.check_static_prop_access(top, cls, &pname)?;
                    let cell = self.static_prop_cell(cls, &pname)?;
                    self.cursor_root = CursorRoot::Cell(cell);
                    self.cursor_global = None;
                    self.cursor_steps.clear();
                    self.rw_null_keys = 0;
                    self.cursor_null_key = false;
                }
                Instr::LvalVarVar { name } => self.lval_var_var(top, name, false)?,
                Instr::LvalVarVarRw { name } => self.lval_var_var(top, name, true)?,
                Instr::BindVarVarRef { name, src } => {
                    let vname = conv::to_string_bytes(&self.reg_r(top, name)?);
                    let cell = match &self.stack[top].regs[src as usize] {
                        Value::Ref(rc) => rc.clone(),
                        // a by-value call result: assigned
                        v => {
                            let v = v.clone();
                            match self.var_var_slot(top, &vname) {
                                Ok(cv) => self.cv_assign(top, cv, v),
                                Err(cell) => {
                                    let mut c = cell.borrow_mut();
                                    match &mut *c {
                                        Value::Ref(rc) => *rc.borrow_mut() = v,
                                        slot => *slot = v,
                                    }
                                }
                            }
                            continue;
                        }
                    };
                    self.var_var_bind(top, &vname, cell);
                }
                Instr::UnsetVarVar { name } => {
                    let vname = conv::to_string_bytes(&self.reg_r(top, name)?);
                    self.var_var_unset(top, &vname);
                }
                Instr::FetchVarVarIs { dst, name } => {
                    let vname = conv::to_string_bytes(&self.reg_get(top, name));
                    let v = match self.var_var_lookup(top, &vname) {
                        Some(Ok(cv)) => self.reg_get(top, cv),
                        Some(Err(cell)) => deref(&cell.borrow()),
                        None => Value::Null,
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::FetchVarVar { dst, name } => {
                    let vname = conv::to_string_bytes(&self.reg_r(top, name)?);
                    let v = match self.var_var_lookup(top, &vname) {
                        Some(Ok(cv)) => self.reg_r(top, cv)?,
                        Some(Err(cell)) if !cell.borrow().is_undef() => deref(&cell.borrow()),
                        _ => {
                            let msg = [b"Undefined variable $".as_slice(), &vname].concat();
                            self.diag(builtins::E_WARNING, &msg)?;
                            Value::Null
                        }
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::FetchStaticPropDyn { dst, class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = conv::to_string_bytes(&self.reg_r(top, name)?);
                    self.check_static_prop_access(top, cls, &pname)?;
                    let cell = self.static_prop_cell(cls, &pname)?;
                    let v = cell.borrow().clone();
                    self.reg_set(top, dst, v);
                }
                Instr::UnsetStaticProp { class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = conv::to_string_bytes(&self.reg_r(top, name)?);
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Attempt to unset static property {}::${}",
                            String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                            String::from_utf8_lossy(&pname)
                        ),
                    ));
                }
                Instr::FetchStaticPropDynIs { dst, class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = conv::to_string_bytes(&self.reg_get(top, name));
                    let v = match self.class(cls).static_props.get(&pname[..]) {
                        Some(sp)
                            if self.static_prop_accessible(self.stack[top].defining_class, sp) =>
                        {
                            deref(&sp.cell.borrow())
                        }
                        _ => Value::Null,
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::LvalGlobal { name } => self.lval_global(top, name, false)?,
                Instr::LvalGlobalRw { name } => self.lval_global(top, name, true)?,
                Instr::FetchGlobal { dst, name } => {
                    let n = conv::to_string_bytes(&self.reg_r(top, name)?);
                    let v = match self.globals.get(&n[..]) {
                        Some(c) if !c.borrow().is_undef() => deref(&c.borrow()),
                        _ => {
                            let msg = format!(
                                "Undefined global variable ${}",
                                String::from_utf8_lossy(&n)
                            );
                            self.diag(builtins::E_WARNING, &msg)?;
                            Value::Null
                        }
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::FetchGlobals { dst } => {
                    let v = self.globals_array();
                    self.reg_set(top, dst, v);
                }
                Instr::UnsetGlobal { name } => {
                    let n = conv::to_string_bytes(&self.reg_r(top, name)?);
                    self.unset_global(&n);
                }
                Instr::FetchLvalRw { dst } => {
                    let v = self.cursor_read(top)?;
                    self.reg_set(top, dst, v); // cursor stays open for the AssignLval
                }
                Instr::AssignLval { src, dst } => {
                    // (After a compound assignment's FetchLvalRw raised them
                    // already, `rw_null_keys` counts them.)
                    if self.cursor_null_key && self.rw_null_keys == 0 {
                        self.w_null_key_diags(top)?;
                    }
                    let v = self.reg_r(top, src)?;
                    let steps = std::mem::take(&mut self.cursor_steps);
                    let r = if dst == src {
                        self.cursor_write(top, &steps, v)
                    } else {
                        crate::lvalue::take_string_offset_result();
                        let r = self.cursor_write(top, &steps, v.clone());
                        let res = crate::lvalue::take_string_offset_result().unwrap_or(v);
                        self.reg_set(top, dst, res);
                        r
                    };
                    self.cursor_release();
                    r?;
                }
                Instr::AssignLvalRef { src } => {
                    // Raw (non-deref) value: `src` holds a `Ref`, so the cursor
                    // target becomes a reference to the same cell.
                    let v = self.stack[top].regs[src as usize].clone();
                    let steps = std::mem::take(&mut self.cursor_steps);
                    // `$GLOBALS['x'] = &$v`: the symbol-table entry itself
                    // becomes the reference (like `$x = &$v` in global scope).
                    if let (true, Some(n), Value::Ref(rc)) =
                        (steps.is_empty(), self.cursor_global.take(), &v)
                    {
                        self.cursor_release();
                        self.ref_globals.insert(n.clone());
                        self.rebind_global(&n, rc.clone());
                        continue;
                    }
                    let r = self.cursor_write(top, &steps, v);
                    self.cursor_release();
                    r?;
                }
                Instr::MakeRefLval { dst } => {
                    let steps = std::mem::take(&mut self.cursor_steps);
                    let cell = self.cursor_make_ref(top, &steps);
                    self.cursor_release();
                    self.reg_set(top, dst, Value::Ref(cell?));
                }
                Instr::UnsetLval => {
                    let steps = std::mem::take(&mut self.cursor_steps);
                    let r = self.cursor_unset(top, &steps);
                    self.cursor_release();
                    r?;
                }
                Instr::IncDecLval { dst, pre, delta } => {
                    self.cursor_settle_offsets(top)?;
                    let steps = std::mem::take(&mut self.cursor_steps);
                    // `$obj[$k]++` on an ArrayAccess object: php fetches the
                    // element for RW (offsetGet) and increments that; there
                    // is no offsetSet.
                    if let MagicSplit::Offset(i, h) = self.cursor_magic_split(top, &steps) {
                        if i + 1 == steps.len() {
                            let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                            let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::Rw)?;
                            let old = deref(&cell.borrow());
                            let new = match self.inc_dec_vm(&old, delta as i64) {
                                Ok(n) => n,
                                Err(e) => {
                                    self.cursor_release();
                                    return Err(e);
                                }
                            };
                            *cell.borrow_mut() = new.clone();
                            self.cursor_release();
                            self.reg_set(top, dst, if pre { new } else { old });
                            continue;
                        }
                    }
                    let old = self.cursor_read_magic(top, &steps, true)?;
                    let new = self.inc_dec_vm(&old, delta as i64)?;
                    self.cursor_write(top, &steps, new.clone())?;
                    self.cursor_release();
                    self.reg_set(top, dst, if pre { new } else { old });
                }

                // -- foreach --
                Instr::FeReset {
                    arr_reg,
                    pos_reg,
                    src,
                    end,
                    delegate,
                } => self.fe_reset(top, arr_reg, pos_reg, src, end, delegate)?,
                Instr::FeFetch {
                    arr_reg,
                    pos_reg,
                    key_cv,
                    val_cv,
                    end,
                } => self.fe_fetch(top, arr_reg, pos_reg, key_cv, val_cv, end)?,

                Instr::FeResetRef {
                    arr_cv,
                    pos_reg,
                    end,
                } => self.fe_reset_ref(top, arr_cv, pos_reg, end)?,
                Instr::FeFetchRef {
                    arr_cv,
                    pos_reg,
                    key_cv,
                    val_cv,
                    end,
                } => self.fe_fetch_ref(top, arr_cv, pos_reg, key_cv, val_cv, end)?,

                Instr::ThrowMatchError { subject } => {
                    let v = self.reg_r(top, subject)?;
                    return Err(Thrown::error(
                        "UnhandledMatchError",
                        format!("Unhandled match case {}", self.match_case_desc(&v)),
                    ));
                }
                Instr::CheckThis { cv } => {
                    let ok = match &self.stack[top].regs[cv as usize] {
                        Value::Object(_) => true,
                        Value::Ref(rc) => matches!(*rc.borrow(), Value::Object(_)),
                        _ => false,
                    };
                    if !ok {
                        return Err(Thrown::error(
                            "Error",
                            "Using $this when not in object context",
                        ));
                    }
                }
                Instr::ConstExprProp { obj, name } => {
                    if name != Reg::MAX {
                        let n = self.reg_get(top, name);
                        let s = self.to_string_vm(&n)?;
                        self.reg_set(top, name, Value::str(s));
                    }
                    if let Value::Object(h) = self.reg_get(top, obj) {
                        let cls = h.borrow().class;
                        if self.class(cls).kind != rhp_values::ClassKind::Enum {
                            return Err(Thrown::error(
                                "Error",
                                "Fetching properties on non-enums in constant expressions is not allowed",
                            ));
                        }
                    }
                }
                Instr::ConstExprDim { arr } => {
                    if let Value::Object(_) = self.reg_get(top, arr) {
                        return Err(Thrown::error(
                            "Error",
                            "Cannot use [] on objects in constant expression",
                        ));
                    }
                }
                Instr::ThrowError { k } => {
                    let msg = match &proto.consts[k as usize] {
                        Value::Str(m) => String::from_utf8_lossy(m.as_bytes()).into_owned(),
                        _ => String::new(),
                    };
                    return Err(Thrown::error("Error", msg));
                }
                Instr::SendNoRef { dst, src } => {
                    if let Value::Ref(rc) = &self.stack[top].regs[src as usize] {
                        let v = Value::Ref(rc.clone());
                        self.reg_set(top, dst, v);
                        continue;
                    }
                    let v = self.reg_r(top, src)?;
                    self.diag(
                        builtins::E_NOTICE,
                        "Only variables should be passed by reference",
                    )?;
                    // The callee works on a private copy.
                    self.reg_set(top, dst, Value::Ref(Rc::new(RefCell::new(v))));
                }
                Instr::Eval { dst, src } => {
                    let v = self.do_eval(top, src)?;
                    self.reg_set(top, dst, v);
                }
                Instr::Throw { src } => {
                    let v = self.reg_r(top, src)?;
                    match v {
                        Value::Object(h) => {
                            let cls = h.borrow().class;
                            let is_throwable = self
                                .resolve_class_by_name(b"Throwable")
                                .map(|t| self.class(cls).ancestors.contains(&t))
                                .unwrap_or(false);
                            if is_throwable {
                                return Err(Thrown::Object(h));
                            }
                            // zend_throw_exception_internal.
                            return Err(Thrown::error(
                                "Error",
                                "Cannot throw objects that do not implement Throwable",
                            ));
                        }
                        Value::Closure(_) => {
                            return Err(Thrown::error(
                                "Error",
                                "Cannot throw objects that do not implement Throwable",
                            ))
                        }
                        _ => return Err(Thrown::error("Error", "Can only throw objects")),
                    }
                }
                Instr::FinallyEnd => {
                    let pc = self.stack[top].pc as u32 - 1;
                    let fin = proto
                        .exception_table
                        .iter()
                        .find(|r| r.finally.is_some() && r.finally_end == pc)
                        .and_then(|r| r.finally);
                    let cont = fin.and_then(|f| {
                        let pend = &mut self.stack[top].pending;
                        let i = pend.iter().position(|(k, _)| *k == f)?;
                        Some(pend.remove(i).1)
                    });
                    match cont {
                        Some(FinallyCont::Rethrow(t)) => return Err(t),
                        Some(FinallyCont::Return(v)) => {
                            // An outer finally still has to run first.
                            if let Some(f2) = enclosing_finally(&proto, pc, None) {
                                self.push_pending(top, f2, FinallyCont::Return(v));
                                self.stack[top].pc = f2 as usize;
                            } else if let Some(done) = self.do_return(v) {
                                return Ok(done);
                            }
                        }
                        Some(FinallyCont::ReturnRef(v, ret_pc)) => {
                            if let Some(f2) = enclosing_finally(&proto, pc, None) {
                                self.push_pending(top, f2, FinallyCont::ReturnRef(v, ret_pc));
                                self.stack[top].pc = f2 as usize;
                            } else {
                                // php checks the type again at the return,
                                // whose finally runs again when it fails.
                                self.stack[top].pc = ret_pc as usize + 1;
                                self.recheck_ref_return(top, &proto, &v)?;
                                if let Some(done) = self.do_return(v) {
                                    return Ok(done);
                                }
                            }
                        }
                        Some(FinallyCont::Jump(target)) => {
                            self.jump_through_finally(top, &proto, pc, target)
                        }
                        None => {}
                    }
                }
                Instr::SetPendingRet { src } => {
                    let mut v = self.reg_r(top, src)?;
                    if let (Some(ty), false) = (&proto.return_type, proto.is_generator) {
                        let ctx = self.stack[top].defining_class;
                        v = self.coerce_stringable(v, ty, proto.strict_types, ctx)?;
                        v = self.check_return_type(&proto, top, v)?;
                    }
                    let pc = self.stack[top].pc as u32 - 1;
                    self.drop_left_finallies(top, &proto, pc);
                    if let Some(f) = enclosing_finally(&proto, pc, None) {
                        self.push_pending(top, f, FinallyCont::Return(v));
                    }
                }
                Instr::SetPendingRetRef { src, kind } => {
                    let v = self.ret_ref_value(top, &proto, src, kind)?;
                    let pc = self.stack[top].pc as u32 - 1;
                    self.drop_left_finallies(top, &proto, pc);
                    if let Some(f) = enclosing_finally(&proto, pc, None) {
                        self.push_pending(top, f, FinallyCont::ReturnRef(v, pc));
                    }
                }
                Instr::JmpFinally { target } => {
                    let pc = self.stack[top].pc as u32 - 1;
                    self.drop_left_finallies(top, &proto, pc);
                    self.jump_through_finally(top, &proto, pc, target);
                }

                // -- OOP --
                Instr::New {
                    dst,
                    class,
                    base,
                    argc,
                    ..
                } => self.exec_new(top, dst, class, base, argc)?,
                Instr::FetchProp { dst, obj, name, .. } => {
                    let o = self.reg_r(top, obj)?;
                    let pname = proto.names[name as usize].clone();
                    let v = self.prop_fetch(top, o, pname)?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchPropDyn { dst, obj, name } => {
                    let o = self.reg_r(top, obj)?;
                    let pname = self.dyn_prop_name(top, name)?;
                    let v = self.prop_fetch(top, o, pname)?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchPropW { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let v = self.fetching_as(builtins::MagicFetch::Write, |vm| {
                        vm.prop_fetch(top, o, pname)
                    })?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchPropDynW { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = self.dyn_prop_name(top, name)?;
                    let v = self.fetching_as(builtins::MagicFetch::Write, |vm| {
                        vm.prop_fetch(top, o, pname)
                    })?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchPropIs { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let v = self.prop_fetch_is(top, o, pname)?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchPropDynIs { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = self.dyn_prop_name(top, name)?;
                    let v = self.prop_fetch_is(top, o, pname)?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchStaticPropIs { dst, class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = proto.names[name as usize].as_bytes().to_vec();
                    let accessible = match self.class(cls).static_props.get(&pname[..]) {
                        Some(sp) => self.static_prop_accessible(self.stack[top].defining_class, sp),
                        None => false,
                    };
                    // The cell via static_prop_cell: runs the class's
                    // deferred (non-literal) initializers first.
                    let v = if accessible {
                        deref(&self.static_prop_cell(cls, &pname)?.borrow())
                    } else {
                        Value::Null
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::StorePropDyn { obj, name, src } => {
                    // ZEND_ASSIGN_OBJ reads the value before the name.
                    let o = self.reg_get(top, obj);
                    let val = self.reg_r(top, src)?;
                    let pname = self.dyn_prop_name(top, name)?;
                    self.prop_store(top, o, pname, val)?;
                    if !self.destructibles.is_empty() {
                        self.dtor_check = true; // the old value may be released
                    }
                }
                Instr::CallMethodDyn {
                    dst,
                    obj,
                    name,
                    base,
                    argc,
                } => {
                    let o = self.reg_r(top, obj)?;
                    let mname = dyn_method_name(&self.reg_r(top, name)?)?;
                    if let Value::Closure(_) = o {
                        let args: Vec<Value> =
                            (0..argc).map(|i| self.reg_get(top, base + i)).collect();
                        let r = self.closure_method(&o, &mname, args)?;
                        self.reg_set(top, dst, r);
                        continue;
                    }
                    let Value::Object(h) = o else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to a member function {}() on {}",
                                String::from_utf8_lossy(&mname),
                                rhp_builtins::zval_value_name(self, &o)
                            ),
                        ));
                    };
                    let cls = h.borrow().class;
                    self.invoke_method(top, h, cls, &mname, dst, base, argc)?;
                }
                Instr::StoreProp { obj, name, src } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let val = self.reg_r(top, src)?;
                    self.prop_store(top, o, pname, val)?;
                    if !self.destructibles.is_empty() {
                        self.dtor_check = true; // the old value may be released
                    }
                }
                Instr::IssetDim { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_r(top, key)?;
                    if let Value::Object(h) = &a {
                        if self.class_implements(h.borrow().class, b"ArrayAccess") {
                            let r = self.aa_offset_exists(h, k)?;
                            self.reg_set(top, dst, Value::Bool(r));
                            continue;
                        }
                    }
                    // zend_std_has_dimension: an object without ArrayAccess
                    // throws (like empty() and `??` already do).
                    let cls_name = match deref(&a) {
                        Value::Object(h) => Some(self.class(h.borrow().class).name.clone()),
                        Value::Closure(_) => Some(PhpStr::from("Closure")),
                        _ => None,
                    };
                    if let Some(n) = cls_name {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Cannot use object of type {} as array",
                                String::from_utf8_lossy(n.as_bytes())
                            ),
                        ));
                    }
                    let r = self.isset_isempty_dim(&a, &k, false)?;
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::EmptyDim { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_r(top, key)?;
                    let v = match &a {
                        // zend_std_has_dimension(check_empty = 1).
                        Value::Object(h)
                            if self.class_implements(h.borrow().class, b"ArrayAccess") =>
                        {
                            self.pending_magic_fetch = builtins::MagicFetch::Empty;
                            if self.aa_offset_exists(h, k.clone())? {
                                self.aa_offset_get(h, k)?
                            } else {
                                Value::Null
                            }
                        }
                        _ => {
                            let r = self.isset_isempty_dim(&a, &k, true)?;
                            self.reg_set(top, dst, Value::Bool(r));
                            continue;
                        }
                    };
                    let b = self.truthy(&v)?;
                    self.reg_set(top, dst, Value::Bool(!b));
                }
                Instr::IssetProp { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let r = self.prop_isset(top, o, pname)?;
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::EmptyProp { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let r = self.prop_empty(top, o, pname)?;
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::EmptyPropDyn { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = self.dyn_prop_name(top, name)?;
                    let r = self.prop_empty(top, o, pname)?;
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::IssetPropDyn { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = self.dyn_prop_name(top, name)?;
                    let r = self.prop_isset(top, o, pname)?;
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::UnsetProp { obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    self.prop_unset(top, o, pname)?;
                    if !self.destructibles.is_empty() {
                        self.dtor_check = true;
                    }
                }
                Instr::UnsetPropDyn { obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = self.dyn_prop_name(top, name)?;
                    self.prop_unset(top, o, pname)?;
                    if !self.destructibles.is_empty() {
                        self.dtor_check = true;
                    }
                }
                Instr::FetchStaticProp {
                    dst, class, name, ..
                } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = proto.names[name as usize].as_bytes().to_vec();
                    self.check_static_prop_access(top, cls, &pname)?;
                    let cell = self.static_prop_cell(cls, &pname)?;
                    let v = cell.borrow().clone();
                    self.reg_set(top, dst, v);
                }
                Instr::FetchClassConst {
                    dst, class, name, ..
                } => {
                    let n = Value::Str(proto.names[name as usize].clone());
                    self.fetch_class_const(top, &proto, dst, class, n)?
                }
                Instr::FetchClassConstDyn { dst, class, name } => {
                    let n = self.reg_r(top, name)?;
                    self.fetch_class_const(top, &proto, dst, class, n)?
                }
                Instr::FetchClassName {
                    dst,
                    class,
                    const_expr,
                } => {
                    let name = self.fetch_class_name(top, class, const_expr)?;
                    self.reg_set(top, dst, Value::Str(name));
                }
                Instr::CallMethod {
                    dst,
                    obj,
                    name,
                    base,
                    argc,
                    ..
                } => {
                    // An object in a temporary (`(new A)->m()`) is released
                    // by the call (php frees it when the call returns).
                    let o = if obj >= proto.num_cvs && !self.holds_destructible(top, obj) {
                        self.take_temp(top, obj)
                    } else {
                        self.reg_r(top, obj)?
                    };
                    let mname = proto.names[name as usize].as_bytes();
                    if let Value::Closure(_) = o {
                        self.call_closure_method(top, &o, mname, dst, base, argc)?;
                        continue;
                    }
                    let Value::Object(h) = o else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to a member function {}() on {}",
                                String::from_utf8_lossy(mname),
                                rhp_builtins::zval_value_name(self, &o)
                            ),
                        ));
                    };
                    let cls = h.borrow().class;
                    self.invoke_method(top, h, cls, mname, dst, base, argc)?;
                }
                Instr::MethodArgIsRef {
                    dst,
                    obj,
                    name,
                    idx,
                } => {
                    let r = match self.reg_get(top, obj) {
                        Value::Object(h) => {
                            let cls = h.borrow().class;
                            let m = self.lookup_method(cls, proto.names[name as usize].as_bytes());
                            // A method the scope cannot call goes to `__call`
                            // (zend_std_get_method's trampoline): by value.
                            let scope = self.stack[top].defining_class;
                            m.is_some_and(|m| {
                                param_is_ref(&m.proto, idx)
                                    && (self.method_accessible(&m, scope)
                                        || self.lookup_method(cls, b"__call").is_none())
                            })
                        }
                        _ => false,
                    };
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::ResolveFunc {
                    dst,
                    name,
                    fallback,
                } => {
                    let f = self
                        .fn_names
                        .get(proto.names[name as usize].as_bytes())
                        .or_else(|| {
                            (fallback != rhp_bytecode::NO_NAME)
                                .then(|| {
                                    self.fn_names.get(proto.names[fallback as usize].as_bytes())
                                })
                                .flatten()
                        })
                        .map_or(-1, |&i| i as i64);
                    self.reg_set(top, dst, Value::Int(f));
                }
                Instr::ValueArgIsRef { dst, callee, idx } => {
                    let c = self.reg_get(top, callee);
                    let r = self
                        .callable_proto(&c)
                        .is_some_and(|(p, _)| crate::calls::param_is_ref(&p, idx));
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::FuncArgIsRef { dst, func, idx } => {
                    let r = self.resolved_param_is_ref(top, func, idx);
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::SendVarEx { dst, cv, func, idx } => {
                    let v = if self.resolved_param_is_ref(top, func, idx) {
                        Value::Ref(self.promote(top, cv))
                    } else {
                        self.reg_r(top, cv)?
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::PassArg { dst, src } => {
                    // the temporary moves into the window
                    let v = if src >= proto.num_cvs {
                        std::mem::replace(&mut self.stack[top].regs[src as usize], Value::Null)
                    } else {
                        self.stack[top].regs[src as usize].clone()
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::StaticArgIsRef {
                    dst,
                    class,
                    name,
                    idx,
                } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let m = self.lookup_method(cls, proto.names[name as usize].as_bytes());
                    let r = m.is_some_and(|m| param_is_ref(&m.proto, idx));
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::CallStatic {
                    dst,
                    class,
                    name,
                    base,
                    argc,
                    ..
                } => {
                    let mname = proto.names[name as usize].as_bytes();
                    let ctor = mname.eq_ignore_ascii_case(b"__construct");
                    self.do_call_static(top, class, mname, dst, base, argc, ctor)?;
                }
                Instr::CallParentHook {
                    dst,
                    name,
                    kind,
                    base,
                    argc,
                } => {
                    let pname = proto.names[name as usize].clone();
                    let args = if argc == rhp_bytecode::PACKED_ARGS {
                        crate::hooks::HookArgs::Array(base)
                    } else {
                        crate::hooks::HookArgs::Window { base, argc }
                    };
                    self.exec_parent_hook(top, dst, pname, kind, args)?;
                }
                Instr::CallStaticDyn {
                    dst,
                    class,
                    name,
                    base,
                    argc,
                } => {
                    // ZEND_INIT_STATIC_METHOD_CALL checks the class first.
                    if let ClassRef::Dynamic(r) = class {
                        if !matches!(
                            self.reg_r(top, r)?,
                            Value::Str(_) | Value::Object(_) | Value::Closure(_)
                        ) {
                            return Err(Thrown::error(
                                "Error",
                                "Class name must be a valid object or a string",
                            ));
                        }
                    }
                    let mname = dyn_method_name(&self.reg_r(top, name)?)?;
                    self.do_call_static(top, class, &mname, dst, base, argc, false)?;
                }
                Instr::InstanceOf { dst, val, class } => {
                    let v = self.reg_r(top, val)?;
                    let result = match v {
                        // ZEND_INSTANCEOF never autoloads.
                        Value::Object(h) => match self.resolve_class(top, class) {
                            Ok(cid) => self.class(h.borrow().class).ancestors.contains(&cid),
                            // self/parent/static without a scope throw
                            // (zend_fetch_class).
                            Err(e)
                                if matches!(
                                    class,
                                    ClassRef::SelfClass
                                        | ClassRef::ParentClass
                                        | ClassRef::StaticClass
                                ) =>
                            {
                                return Err(e)
                            }
                            Err(_) => false,
                        },
                        // Closures / arrow fns / first-class callables are
                        // instances of the built-in `Closure` class.
                        Value::Closure(_) => match class {
                            ClassRef::Name(nidx) => {
                                let n = proto.names[nidx as usize].as_bytes();
                                let seg = n.rsplit(|&b| b == b'\\').next().unwrap_or(n);
                                seg.eq_ignore_ascii_case(b"Closure")
                            }
                            _ => false,
                        },
                        _ => false,
                    };
                    self.reg_set(top, dst, Value::Bool(result));
                }
                Instr::CloneObj { dst, src } => self.exec_clone(top, dst, src)?,
                Instr::DeclareClass { class_idx } => {
                    self.declare_class_at((self.unit_base(top).classes + class_idx) as usize)?;
                }
                Instr::DeclareFunc { idx } => {
                    // The proto is stored in the closures table (file-local index).
                    let f = self.closures[(self.unit_base(top).closures + idx) as usize].clone();
                    let lc = f.proto.name.as_bytes().to_ascii_lowercase();
                    if self.fn_names.contains_key(&lc) {
                        // A preloaded file's declaration of a preloaded function
                        // was removed by preloading.
                        if self.preloaded.as_ref().is_some_and(|p| p.has_function(&lc)) {
                            continue;
                        }
                        // do_bind_function_error at run time: E_ERROR.
                        let msg = self.redeclare_function_msg(&lc, &f.proto.name);
                        return Err(self.fatal_error_here(1, msg.as_bytes()));
                    }
                    let i = self.functions.len();
                    self.functions.push(f);
                    self.add_fn_name(lc, i);
                }

                Instr::BindRef { dst_cv, src_cv } => {
                    let rc = self.promote(top, src_cv);
                    self.stack[top].regs[dst_cv as usize] = Value::Ref(rc);
                    self.sync_global_cv(top, dst_cv);
                }
                Instr::RefAssign { dst_cv, src } => {
                    // `src` holds a `Ref` (MakeRefLval): alias it raw. A
                    // by-value call result is assigned (after its notice).
                    let v = self.stack[top].regs[src as usize].clone();
                    if !matches!(v, Value::Ref(_)) {
                        self.cv_assign(top, dst_cv, v);
                        continue;
                    }
                    self.stack[top].regs[dst_cv as usize] = v;
                    self.sync_global_cv(top, dst_cv);
                }
                Instr::FetchListW { dst, src, key } => {
                    let k = self.reg_r(top, key)?;
                    let v = match self.stack[top].regs[src as usize].clone() {
                        Value::Ref(rc) => {
                            self.cursor_root = CursorRoot::Cell(rc);
                            self.cursor_global = None;
                            self.cursor_steps.clear();
                            self.rw_null_keys = 0;
                            self.cursor_null_key = matches!(k, Value::Null);
                            let cell = self.cursor_make_ref(top, &[CursorStep::Key(k)]);
                            self.cursor_release();
                            Value::Ref(cell?)
                        }
                        v => {
                            self.diag(
                                builtins::E_NOTICE,
                                "Attempting to set reference to non referenceable value",
                            )?;
                            let r = self.dim_read(&deref(&v), &k, DimMode::List)?;
                            Value::Ref(Rc::new(RefCell::new(r)))
                        }
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::RefFuncResult { src } => {
                    if !matches!(self.stack[top].regs[src as usize], Value::Ref(_)) {
                        self.diag(
                            builtins::E_NOTICE,
                            "Only variables should be assigned by reference",
                        )?;
                    }
                }
                Instr::MakeRef { dst, cv } => {
                    let rc = self.promote(top, cv);
                    self.stack[top].regs[dst as usize] = Value::Ref(rc);
                }
                Instr::MakeArgRef { dst, cv } => {
                    let rc = self.promote_slot(top, cv);
                    self.stack[top].regs[dst as usize] = Value::Ref(rc);
                }
                Instr::MakeRefVal { dst, src } => {
                    let v = self.reg_get(top, src);
                    self.reg_set(top, dst, Value::Ref(Rc::new(RefCell::new(v))));
                }

                Instr::CallV {
                    dst,
                    name,
                    args,
                    fallback,
                } => {
                    let target = self.call_target(top, &proto, pc, name, fallback)?;
                    self.call_v(top, dst, args, target, &proto.names[name as usize])?;
                }
                Instr::NewV { dst, class, args } => self.exec_new_v(top, dst, class, args)?,
                Instr::CallMethodV {
                    dst,
                    obj,
                    name,
                    args,
                } => {
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    self.call_method_args(top, obj, mname, args, dst)?;
                }
                Instr::CallMethodDynV {
                    dst,
                    obj,
                    name,
                    args,
                } => {
                    let mname = dyn_method_name(&self.reg_r(top, name)?)?;
                    self.call_method_args(top, obj, mname, args, dst)?;
                }
                Instr::CallStaticV {
                    dst,
                    class,
                    name,
                    args,
                } => {
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    let ctor = mname.eq_ignore_ascii_case(b"__construct");
                    self.call_static_args(top, class, mname, args, dst, ctor)?;
                }
                Instr::CallStaticDynV {
                    dst,
                    class,
                    name,
                    args,
                } => {
                    // ZEND_INIT_STATIC_METHOD_CALL checks the class first.
                    if let ClassRef::Dynamic(r) = class {
                        if !matches!(
                            self.reg_r(top, r)?,
                            Value::Str(_) | Value::Object(_) | Value::Closure(_)
                        ) {
                            return Err(Thrown::error(
                                "Error",
                                "Class name must be a valid object or a string",
                            ));
                        }
                    }
                    let mname = dyn_method_name(&self.reg_r(top, name)?)?;
                    self.call_static_args(top, class, mname, args, dst, false)?;
                }

                Instr::FetchConst {
                    dst,
                    name,
                    fallback,
                } => self.fetch_const(top, &proto, dst, name, fallback)?,

                Instr::Ret { src } => {
                    let mut v = self.reg_r(top, src)?;
                    // Enforce a declared return type (the callee's own file decides
                    // strict vs coercive).
                    if let (Some(ty), false) = (&proto.return_type, proto.is_generator) {
                        let ctx = self.stack[top].defining_class;
                        v = self.coerce_stringable(v, ty, proto.strict_types, ctx)?;
                        v = self.check_return_type(&proto, top, v)?;
                    }
                    if let Some(done) = self.do_return(v) {
                        return Ok(done);
                    }
                }
                Instr::RetRef { src, kind } => {
                    let v = self.ret_ref_value(top, &proto, src, kind)?;
                    if let Some(done) = self.do_return(v) {
                        return Ok(done);
                    }
                }
                Instr::ClearTemps { from, to } => {
                    let regs = &mut self.stack[top].regs[from as usize..to as usize];
                    if !self.destructibles.is_empty()
                        && regs
                            .iter()
                            .any(|v| matches!(v, Value::Object(_) | Value::Array(_)))
                    {
                        self.dtor_check = true;
                    }
                    for r in regs {
                        *r = Value::Null;
                    }
                }
                Instr::RetNull | Instr::Halt => {
                    if proto.return_type.is_some() && !proto.is_generator {
                        self.check_implicit_return(&proto, top)?;
                    }
                    if let Some(done) = self.do_return(Value::Null) {
                        return Ok(done);
                    }
                }
                Instr::Include { dst, path, kind } => {
                    let pv = self.reg_r(top, path)?;
                    let raw = match &pv {
                        Value::Object(_) | Value::Array(_) => self.to_string_vm(&pv)?,
                        v => conv::to_string_bytes(v),
                    };
                    let v = self.do_include(top, raw, kind)?;
                    self.reg_set(top, dst, v);
                }
                Instr::Exit { arg } => {
                    // php 8.4's exit(): no argument is status 0; otherwise
                    // Z_PARAM_STR_OR_LONG (a string is printed, status 0).
                    let code = if arg == Reg::MAX {
                        0
                    } else {
                        let v = deref(&self.reg_r(top, arg)?);
                        let strict = proto.strict_types;
                        match self.zpp_exit_status(&v, strict)? {
                            Value::Int(n) => n as i32,
                            v => {
                                let s = conv::to_string_bytes(&v);
                                if !s.is_empty() {
                                    // An output handler that throws makes
                                    // exit() throw instead (catchable).
                                    self.echo_out(&s)?;
                                }
                                0
                            }
                        }
                    };
                    return Err(Thrown::Exit(code));
                }
                Instr::Yield { dst, key, value } => {
                    self.exec_yield(top, &proto, dst, key, value)?;
                    return Ok(Value::Null);
                }
            }
        }
    }

    fn eval_cmp(&mut self, kind: CmpKind, a: &Value, b: &Value) -> Result<Value, Thrown> {
        use std::cmp::Ordering;
        Ok(match kind {
            CmpKind::Eq => Value::Bool(self.compare_values(a, b)? == Ordering::Equal),
            CmpKind::Ne => Value::Bool(self.compare_values(a, b)? != Ordering::Equal),
            CmpKind::Identical => Value::Bool(ops::strict_eq(a, b)),
            CmpKind::NotIdentical => Value::Bool(!ops::strict_eq(a, b)),
            CmpKind::Lt => Value::Bool(self.compare_values(a, b)?.is_lt()),
            CmpKind::Le => Value::Bool(self.compare_values(a, b)?.is_le()),
            // php evaluates `a > b` as `b < a` (ZEND_IS_SMALLER with swapped
            // operands), so uncomparable pairs are false in both directions.
            CmpKind::Gt => Value::Bool(self.compare_values(b, a)?.is_lt()),
            CmpKind::Ge => Value::Bool(self.compare_values(b, a)?.is_le()),
            CmpKind::Spaceship => Value::Int(match self.compare_values(a, b)? {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            }),
        })
    }

    /// Call a zero-argument method on an object (for the iterator protocol).
    fn call_iter_method(&mut self, h: &ObjectHandle, name: &[u8]) -> Result<Value, Thrown> {
        let cls = h.borrow().class;
        let m = self
            .lookup_method(cls, name)
            .ok_or_else(|| Thrown::error("Error", "Iterator method not found"))?;
        self.call_sync(&m, Some(h.clone()), cls, Vec::new())
    }

    /// `++` / `--` (`delta` +1 / -1): php's increment_function /
    /// decrement_function, diagnostics and Perl-style string increment
    /// included.
    fn inc_dec_vm(&mut self, v: &Value, delta: i64) -> Result<Value, Thrown> {
        let inc = delta > 0;
        let what = if inc { "Increment" } else { "Decrement" };
        match deref(v) {
            Value::Null if !inc => {
                self.diag(
                    builtins::E_WARNING,
                    "Decrement on type null has no effect, this will change in the next major version of PHP",
                )?;
                Ok(Value::Null)
            }
            Value::Bool(b) => {
                self.diag(
                    builtins::E_WARNING,
                    &format!(
                        "{what} on type bool has no effect, this will change in the next major version of PHP"
                    ),
                )?;
                Ok(Value::Bool(b))
            }
            Value::Str(s) if conv::numeric_string(s.as_bytes()).is_none() => {
                let b = s.as_bytes();
                if !inc {
                    if b.is_empty() {
                        self.diag(
                            builtins::E_DEPRECATED,
                            "Decrement on empty string is deprecated as non-numeric",
                        )?;
                        return Ok(Value::Int(-1));
                    }
                    self.diag(
                        builtins::E_DEPRECATED,
                        "Decrement on non-numeric string has no effect and is deprecated",
                    )?;
                    return Ok(Value::Str(s));
                }
                self.diag(
                    builtins::E_DEPRECATED,
                    "Increment on non-numeric string is deprecated, use str_increment() instead",
                )?;
                Ok(Value::str(perl_increment(b)))
            }
            arr @ (Value::Array(_) | Value::Resource(_) | Value::Closure(_)) => {
                Err(Thrown::type_error(format!(
                    "Cannot {} {}",
                    what.to_ascii_lowercase(),
                    rhp_builtins::zval_value_name(self, &arr)
                )))
            }
            // increment_function: `do_operation(ADD, 1)`, else the object
            // as a number (`cast_object(_IS_NUMBER)`), else a TypeError.
            obj @ Value::Object(_) => {
                let Value::Object(h) = &obj else {
                    unreachable!()
                };
                if let Some(f) = self.handlers_of(h).and_then(|t| t.do_operation) {
                    let op = if inc { Op::Add } else { Op::Sub };
                    if let Some(r) = f(self, op, &obj, Some(&Value::Int(1)))? {
                        return Ok(r);
                    }
                }
                match self.cast_object_vm(h, CastTarget::Number)? {
                    Some(n) => self.inc_dec_vm(&n, delta),
                    None => Err(Thrown::type_error(format!(
                        "Cannot {} {}",
                        what.to_ascii_lowercase(),
                        rhp_builtins::zval_value_name(self, &obj)
                    ))),
                }
            }
            other => Ok(inc_dec_value(&other, delta)),
        }
    }

    /// The entries `...$src` unpacks (ZEND_SEND_UNPACK when `args`, else
    /// ZEND_ADD_ARRAY_UNPACK): an array's elements as they are, or what a
    /// Traversable yields (numeric-string keys count as positional).
    fn unpack_entries(
        &mut self,
        top: usize,
        src: Reg,
        args: bool,
    ) -> Result<Vec<(ArrayKey, Value)>, Thrown> {
        let v = self.reg_r(top, src)?;
        let h = match v {
            // ZEND_ADD_ARRAY_UNPACK copies a reference only the source
            // holds as its value.
            Value::Array(a) if !args => {
                return Ok(a
                    .iter()
                    .map(|(k, v)| (k.clone(), rhp_values::ops::copy_elem(v)))
                    .collect())
            }
            Value::Array(a) => return Ok(a.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            Value::Object(h) if self.class_implements(h.borrow().class, b"Traversable") => h,
            other => {
                let msg = format!(
                    "Only arrays and Traversables can be unpacked, {} given",
                    rhp_builtins::zval_value_name(self, &other)
                );
                return Err(
                    if args || matches!(other, Value::Object(_) | Value::Closure(_)) {
                        Thrown::type_error(msg)
                    } else {
                        Thrown::error("Error", msg)
                    },
                );
            }
        };
        let mut it = h;
        while self.class_implements(it.borrow().class, b"IteratorAggregate") {
            match self.call_iter_method(&it, b"getiterator")? {
                Value::Object(next)
                    if self.class_implements(next.borrow().class, b"Traversable") =>
                {
                    it = next
                }
                _ => {
                    let cls = it.borrow().class;
                    return Err(Thrown::error(
                        "Exception",
                        format!(
                            "Objects returned by {}::getIterator() must be traversable or implement interface Iterator",
                            String::from_utf8_lossy(self.class(cls).name.as_bytes())
                        ),
                    ));
                }
            }
        }
        let id = it.borrow().id;
        let is_gen = self.generators.contains_key(&id);
        let mut out = Vec::new();
        if is_gen {
            self.gen_ensure_started(id)?;
        } else {
            self.call_iter_method(&it, b"rewind")?;
        }
        loop {
            let (k, v) = if is_gen {
                let g = &self.generators[&id];
                if g.status == GenStatus::Finished {
                    break;
                }
                (g.current_key.clone(), g.current_val.clone())
            } else {
                if !conv::to_bool(&self.call_iter_method(&it, b"valid")?) {
                    break;
                }
                let v = self.call_iter_method(&it, b"current")?;
                let k = self.call_iter_method(&it, b"key")?;
                (k, v)
            };
            let key = match deref(&k) {
                Value::Int(n) => ArrayKey::Int(n),
                Value::Str(s) => match ArrayKey::from_bytes(s.as_bytes()) {
                    ArrayKey::Int(n) => ArrayKey::Int(n),
                    _ => ArrayKey::Str(s),
                },
                _ => {
                    return Err(Thrown::error(
                        "Error",
                        if args {
                            "Keys must be of type int|string during argument unpacking"
                        } else {
                            "Keys must be of type int|string during array unpacking"
                        },
                    ))
                }
            };
            out.push((key, deref(&v)));
            if is_gen {
                self.advance_gen(id, Value::Null)?;
            } else {
                self.call_iter_method(&it, b"next")?;
            }
        }
        Ok(out)
    }

    /// Binary op that may throw (arithmetic). Two numbers take the operator
    /// directly; anything else goes through [`Vm::bin_slow`].
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn bin(
        &mut self,
        top: usize,
        dst: Reg,
        a: Reg,
        b: Reg,
        op: Op,
        f: fn(&Value, &Value) -> Result<Value, Thrown>,
    ) -> Result<(), Thrown> {
        let (x, y) = (self.reg_r(top, a)?, self.reg_r(top, b)?);
        let v = if matches!(x, Value::Int(_) | Value::Float(_))
            && matches!(y, Value::Int(_) | Value::Float(_))
        {
            f(&x, &y)?
        } else {
            self.bin_slow(op, f, &x, &y)?
        };
        self.reg_set(top, dst, v);
        Ok(())
    }

    /// `% << >> | & ^` past the int fast path: `long_op` (compare.rs).
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn bin_inf(&mut self, top: usize, dst: Reg, a: Reg, b: Reg, op: Op) -> Result<(), Thrown> {
        let (x, y) = (self.reg_r(top, a)?, self.reg_r(top, b)?);
        let v = self.long_op(op, &x, &y)?;
        self.reg_set(top, dst, v);
        Ok(())
    }

    /// `echo` / `print` / inline HTML: write through the output layer; an
    /// exception an output handler threw, or one from the
    /// `header_register_callback` callback the first output byte ran,
    /// surfaces at the statement.
    fn echo_out(&mut self, bytes: &[u8]) -> Result<(), Thrown> {
        self.output_write(bytes);
        match self.pending_throw.take() {
            Some(t) => Err(t),
            None => Ok(()),
        }
    }

    /// zend_register_constant's refusal: the name is a defined constant,
    /// `true` / `false` / `null` (any case) or `__COMPILER_HALT_OFFSET__`.
    pub(crate) fn const_taken(&self, name: &[u8]) -> bool {
        self.constants.contains_key(name)
            || name == b"__COMPILER_HALT_OFFSET__"
            || [&b"true"[..], b"false", b"null"]
                .iter()
                .any(|s| name.eq_ignore_ascii_case(s))
    }

    /// `FeReset`: start a by-value foreach over `src`.
    fn fe_reset(
        &mut self,
        top: usize,
        arr_reg: Reg,
        pos_reg: Reg,
        src: Reg,
        end: Pc,
        delegate: bool,
    ) -> Result<(), Thrown> {
        match self.reg_r(top, src)? {
            Value::Array(a) if !a.is_empty() => {
                self.reg_set(top, arr_reg, Value::Array(a));
                self.reg_set(top, pos_reg, Value::Int(0));
            }
            Value::Object(h) => {
                // Resolve the thing we actually iterate: IteratorAggregate
                // delegates to getIterator(); Generators / Iterators drive
                // the protocol in FeFetch; a plain object iterates its
                // accessible properties (snapshotted to an array).
                let mut obj = h;
                if self.class_implements(obj.borrow().class, b"IteratorAggregate") {
                    match self.call_iter_method(&obj, b"getiterator")? {
                        Value::Object(it) => obj = it,
                        _ => {
                            self.stack[top].pc = end as usize;
                            return Ok(());
                        }
                    }
                }
                let oid = obj.borrow().id;
                let ocls = obj.borrow().class;
                if self.generators.contains_key(&oid) {
                    if delegate {
                        // ZEND_YIELD_FROM attaches the generator
                        // as it stands; no rewind.
                        self.gen_ensure_started(oid)?;
                    } else {
                        self.gen_check_open(oid)?;
                        self.gen_rewind(oid)?;
                    }
                    self.reg_set(top, arr_reg, Value::Object(obj));
                    self.reg_set(top, pos_reg, Value::Int(0));
                } else if self.class_implements(ocls, b"Iterator") {
                    self.call_iter_method(&obj, b"rewind")?;
                    self.reg_set(top, arr_reg, Value::Object(obj));
                    self.reg_set(top, pos_reg, Value::Int(0));
                } else if self.classes[ocls as usize].hooked {
                    let obj = self.lazy_props_object(&obj)?;
                    let scope = self.stack[top].defining_class;
                    let st = self.hooked_iter_state(&obj, scope);
                    self.reg_set(top, arr_reg, Value::Object(obj));
                    self.reg_set(top, pos_reg, st);
                } else {
                    let obj = self.lazy_props_object(&obj)?;
                    let arr = self.object_prop_array(&obj, top);
                    if arr.is_empty() {
                        self.stack[top].pc = end as usize;
                    } else {
                        self.reg_set(top, arr_reg, Value::Array(arr));
                        self.reg_set(top, pos_reg, Value::Int(0));
                    }
                }
            }
            // An empty array skips the loop; anything else warns too.
            v => {
                self.stack[top].pc = end as usize;
                // (A Closure is an object without properties.)
                if !matches!(v, Value::Array(_) | Value::Closure(_)) {
                    self.foreach_type_warning(&v)?;
                }
            }
        }
        Ok(())
    }

    /// ZEND_FE_RESET_R/RW on something that is neither an array nor an object.
    #[cold]
    fn foreach_type_warning(&mut self, v: &Value) -> Result<(), Thrown> {
        let msg = format!(
            "foreach() argument must be of type array|object, {} given",
            builtins::zval_value_name(self, v)
        );
        self.diag(builtins::E_WARNING, &msg)
    }

    /// `FeFetch`: bind the next key/value of a by-value foreach, or jump to `end`.
    fn fe_fetch(
        &mut self,
        top: usize,
        arr_reg: Reg,
        pos_reg: Reg,
        key_cv: Reg,
        val_cv: Reg,
        end: Pc,
    ) -> Result<(), Thrown> {
        // Generator mode. The advance happens at the TOP of each
        // iteration except the first, so the generator's post-yield
        // code runs *after* the loop body (unlike an array's pos++).
        // `pos_reg` is 0 on the first fetch, 1 thereafter.
        if let Value::Object(h) = self.reg_get(top, arr_reg) {
            let id = h.borrow().id;
            if self.generators.contains_key(&id) {
                let first = matches!(self.reg_get(top, pos_reg), Value::Int(0));
                if !first {
                    self.advance_gen(id, Value::Null)?;
                }
                let g = &self.generators[&id];
                if g.status == GenStatus::Finished {
                    self.stack[top].pc = end as usize;
                } else {
                    let (k, v) = (g.current_key.clone(), deref(&g.current_val));
                    self.cv_assign(top, val_cv, v);
                    if key_cv != Reg::MAX {
                        self.cv_assign(top, key_cv, k);
                    }
                    self.reg_set(top, pos_reg, Value::Int(1));
                }
                return Ok(());
            }
            // A hooked plain object (see FeReset).
            if let st @ Value::Array(_) = self.reg_get(top, pos_reg) {
                match self.hooked_iter_next(&h, &st, false)? {
                    Some((st, k, v)) => {
                        self.cv_assign(top, val_cv, v);
                        if key_cv != Reg::MAX {
                            self.cv_assign(top, key_cv, k);
                        }
                        self.reg_set(top, pos_reg, st);
                    }
                    None => self.stack[top].pc = end as usize,
                }
                return Ok(());
            }
            // User Iterator object: rewind()/valid()/current()/key()/next().
            if self.class_implements(h.borrow().class, b"Iterator") {
                let first = matches!(self.reg_get(top, pos_reg), Value::Int(0));
                if !first {
                    self.call_iter_method(&h, b"next")?;
                }
                let valid = conv::to_bool(&self.call_iter_method(&h, b"valid")?);
                if !valid {
                    self.stack[top].pc = end as usize;
                } else {
                    let v = self.call_iter_method(&h, b"current")?;
                    self.cv_assign(top, val_cv, v);
                    if key_cv != Reg::MAX {
                        let k = self.call_iter_method(&h, b"key")?;
                        self.cv_assign(top, key_cv, k);
                    }
                    self.reg_set(top, pos_reg, Value::Int(1));
                }
                return Ok(());
            }
        }
        let pos = match self.reg_get(top, pos_reg) {
            Value::Int(n) => n as usize,
            _ => 0,
        };
        let snapshot = self.reg_get(top, arr_reg);
        let Value::Array(a) = snapshot else {
            self.stack[top].pc = end as usize;
            return Ok(());
        };
        match a.get_index(pos) {
            Some((k, v)) => {
                let (k, v) = (k.clone(), deref(v).clone());
                // Bind as a PHP variable: if the loop var is still a
                // reference (e.g. left over from a prior by-ref loop),
                // write through it — matching PHP's foreach gotcha.
                self.cv_assign(top, val_cv, v);
                if key_cv != Reg::MAX {
                    self.cv_assign(top, key_cv, k.to_value());
                }
                self.reg_set(top, pos_reg, Value::Int((pos + 1) as i64));
            }
            None => self.stack[top].pc = end as usize,
        }
        Ok(())
    }

    /// `FeResetRef`: start a by-reference foreach over CV `arr_cv`.
    fn fe_reset_ref(
        &mut self,
        top: usize,
        arr_cv: Reg,
        pos_reg: Reg,
        end: Pc,
    ) -> Result<(), Thrown> {
        if self.stack[top].regs[arr_cv as usize].is_undef() {
            self.undef_cv(top, arr_cv)?;
        }
        // A Traversable object by reference: only a by-reference
        // generator (directly or from getIterator()) can do it;
        // `pos_reg` then holds [generator, fetched-once flag].
        if let Value::Object(h) = deref(&self.stack[top].regs[arr_cv as usize]) {
            let mut obj = h;
            let mut traversable = false;
            while self.class_implements(obj.borrow().class, b"IteratorAggregate") {
                // A built-in ArrayObject iterates its storage itself.
                if self.spl_array_owner(&obj, b"getiterator").is_some() {
                    break;
                }
                traversable = true;
                match self.call_iter_method(&obj, b"getiterator")? {
                    Value::Object(it)
                        if self.class_implements(it.borrow().class, b"Traversable") =>
                    {
                        obj = it
                    }
                    _ => {
                        let cls = obj.borrow().class;
                        return Err(Thrown::error(
                            "Exception",
                            format!(
                                "Objects returned by {}::getIterator() must be traversable or implement interface Iterator",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes())
                            ),
                        ));
                    }
                }
            }
            let oid = obj.borrow().id;
            if let Some(g) = self.generators.get(&oid) {
                let by_value_gen = g.frame.as_ref().is_some_and(|f| !f.proto.returns_ref);
                self.gen_check_open(oid)?;
                if by_value_gen {
                    return Err(Thrown::error(
                        "Exception",
                        "You can only iterate a generator by-reference if it declared that it yields by-reference",
                    ));
                }
                self.gen_rewind(oid)?;
                let mut st = PhpArray::new();
                st.push(Value::Object(obj));
                st.push(Value::Bool(false));
                self.reg_set(top, pos_reg, Value::Array(st));
                return Ok(());
            }
            // ArrayObject / ArrayIterator (unless `current()` is
            // overridden): by reference over the backing array.
            let spl = self
                .spl_array_owner(&obj, b"getiterator")
                .or_else(|| self.spl_array_owner(&obj, b"current"));
            if let Some(owner) = spl {
                // `pos_reg` holds [storage reference, position].
                let cell = self.spl_array_storage(&obj, owner)?;
                let mut st = PhpArray::new();
                st.push(Value::Ref(cell));
                st.push(Value::Int(0));
                self.reg_set(top, pos_reg, Value::Array(st));
                return Ok(());
            }
            if traversable || self.class_implements(obj.borrow().class, b"Iterator") {
                return Err(Thrown::error(
                    "Error",
                    "An iterator cannot be used with foreach by reference",
                ));
            }
        }
        let nonempty = match deref(&self.stack[top].regs[arr_cv as usize]) {
            Value::Array(a) => !a.is_empty(),
            // A plain object iterates its properties by reference;
            // `FeFetchRef` ends the loop when none remain accessible.
            Value::Object(_) => true,
            Value::Closure(_) => false,
            v => {
                self.foreach_type_warning(&v)?;
                false
            }
        };
        if let Value::Object(h) = deref(&self.stack[top].regs[arr_cv as usize]) {
            // get_properties initializes a lazy object (a proxy iterates
            // its instance, see FeFetchRef).
            let h = self.lazy_props_object(&h)?;
            if self.classes[h.borrow().class as usize].hooked {
                let scope = self.stack[top].defining_class;
                let st = self.hooked_iter_state(&h, scope);
                self.reg_set(top, pos_reg, st);
                return Ok(());
            }
        }
        if nonempty {
            self.reg_set(top, pos_reg, Value::Int(0));
        } else {
            self.stack[top].pc = end as usize;
        }
        Ok(())
    }

    /// `FeFetchRef`: alias the loop variable to the next element of a by-reference
    /// foreach, or jump to `end`.
    fn fe_fetch_ref(
        &mut self,
        top: usize,
        arr_cv: Reg,
        pos_reg: Reg,
        key_cv: Reg,
        val_cv: Reg,
        end: Pc,
    ) -> Result<(), Thrown> {
        // By-reference generator (see FeResetRef): advance at the
        // top of each iteration but the first, then alias the loop
        // variable to the yielded reference.
        if let Value::Array(st) = self.reg_get(top, pos_reg) {
            // A hooked plain object: `[position, plan]` (see FeResetRef).
            if let Some(Value::Int(_)) = st.get_index(0).map(|e| e.1.clone()) {
                let Value::Object(h) = deref(&self.stack[top].regs[arr_cv as usize]) else {
                    self.stack[top].pc = end as usize;
                    return Ok(());
                };
                let h = self.lazy_target(&h);
                match self.hooked_iter_next(&h, &Value::Array(st), true)? {
                    Some((st, k, Value::Ref(rc))) => {
                        self.bind_ref_cv(top, val_cv, rc);
                        if key_cv != Reg::MAX {
                            self.cv_assign(top, key_cv, k);
                        }
                        self.reg_set(top, pos_reg, st);
                    }
                    _ => self.stack[top].pc = end as usize,
                }
                return Ok(());
            }
            // ArrayObject / ArrayIterator storage (see FeResetRef).
            if let (Some(Value::Ref(cell)), Some(Value::Int(pos))) = (
                st.get_index(0).map(|e| e.1.clone()),
                st.get_index(1).map(|e| e.1.clone()),
            ) {
                let fetched = foreach_promote(&mut cell.borrow_mut(), pos as usize);
                match fetched {
                    Some((k, elem_rc)) => {
                        self.bind_ref_cv(top, val_cv, elem_rc);
                        if key_cv != Reg::MAX {
                            self.cv_assign(top, key_cv, k.to_value());
                        }
                        let mut st = PhpArray::new();
                        st.push(Value::Ref(cell));
                        st.push(Value::Int(pos + 1));
                        self.reg_set(top, pos_reg, Value::Array(st));
                    }
                    None => self.stack[top].pc = end as usize,
                }
                return Ok(());
            }
            let (Some(Value::Object(g)), Some(Value::Bool(fetched))) = (
                st.get_index(0).map(|e| e.1.clone()),
                st.get_index(1).map(|e| e.1.clone()),
            ) else {
                self.stack[top].pc = end as usize;
                return Ok(());
            };
            let id = g.borrow().id;
            if fetched {
                self.advance_gen(id, Value::Null)?;
            }
            let gs = &self.generators[&id];
            if gs.status == GenStatus::Finished {
                self.stack[top].pc = end as usize;
                return Ok(());
            }
            let (k, v) = (gs.current_key.clone(), gs.current_val.clone());
            let rc = match v {
                Value::Ref(rc) => rc,
                other => Rc::new(RefCell::new(other)),
            };
            self.bind_ref_cv(top, val_cv, rc);
            if key_cv != Reg::MAX {
                self.cv_assign(top, key_cv, k);
            }
            let mut st = PhpArray::new();
            st.push(Value::Object(g));
            st.push(Value::Bool(true));
            self.reg_set(top, pos_reg, Value::Array(st));
            return Ok(());
        }
        let pos = match self.reg_get(top, pos_reg) {
            Value::Int(n) => n as usize,
            _ => 0,
        };
        // By-reference foreach over a plain object: alias each
        // accessible property slot so writes through the loop var are
        // visible on the object (matches PHP).
        if let Value::Object(h) = deref(&self.stack[top].regs[arr_cv as usize]).clone() {
            let h = self.lazy_props_object(&h)?;
            let caller = self.stack[top].defining_class;
            let cls = h.borrow().class;
            let mut b = h.borrow_mut();
            let n = b.properties.len();
            let mut idx = pos;
            let mut found: Option<PhpStr> = None;
            while idx < n {
                let key = b.properties.get_index(idx).unwrap().0;
                if let Some(name) = classes::slot_accessible(&self.classes, cls, caller, key) {
                    found = Some(name);
                    break;
                }
                idx += 1;
            }
            match found {
                Some(name) => {
                    let key = b.properties.get_index(idx).unwrap().0.clone();
                    if let Some(pe) = self.class(cls).props.get(&key).filter(|pe| pe.readonly) {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Cannot acquire reference to readonly property {}::${}",
                                String::from_utf8_lossy(
                                    self.class(pe.declaring_class).name.as_bytes()
                                ),
                                String::from_utf8_lossy(pe.name.as_bytes())
                            ),
                        ));
                    }
                    let slot = b.properties.get_index_mut(idx).unwrap().1;
                    let rc = match slot {
                        Value::Ref(rc) => rc.clone(),
                        other => {
                            let inner = std::mem::replace(other, Value::Null);
                            let rc = Rc::new(RefCell::new(inner));
                            *other = Value::Ref(rc.clone());
                            rc
                        }
                    };
                    drop(b);
                    self.bind_ref_cv(top, val_cv, rc);
                    if key_cv != Reg::MAX {
                        self.cv_assign(top, key_cv, Value::str(name.as_bytes().to_vec()));
                    }
                    self.reg_set(top, pos_reg, Value::Int((idx + 1) as i64));
                }
                None => {
                    drop(b);
                    self.stack[top].pc = end as usize;
                }
            }
            return Ok(());
        }
        // Promote the element at `pos` to a shared cell (through a
        // Ref if the source variable itself is a reference).
        let rc_opt = match &self.stack[top].regs[arr_cv as usize] {
            Value::Ref(rc) => Some(rc.clone()),
            _ => None,
        };
        let fetched = match rc_opt {
            Some(rc) => foreach_promote(&mut rc.borrow_mut(), pos),
            None => foreach_promote(&mut self.stack[top].regs[arr_cv as usize], pos),
        };
        match fetched {
            Some((k, elem_rc)) => {
                self.bind_ref_cv(top, val_cv, elem_rc);
                if key_cv != Reg::MAX {
                    self.cv_assign(top, key_cv, k.to_value());
                }
                self.reg_set(top, pos_reg, Value::Int((pos + 1) as i64));
            }
            None => self.stack[top].pc = end as usize,
        }
        Ok(())
    }

    /// `FetchClassConst`: `Class::NAME` (and `Class::class`) into `dst`.
    /// ZEND_FETCH_CLASS_NAME / ZEND_AST_CLASS_NAME.
    fn fetch_class_name(
        &mut self,
        top: usize,
        class: ClassRef,
        const_expr: bool,
    ) -> Result<PhpStr, Thrown> {
        let scope = self.stack[top].defining_class;
        let kind = match class {
            ClassRef::Dynamic(reg) => {
                let v = self.reg_r(top, reg)?;
                return match v {
                    Value::Object(h) => {
                        let cls = h.borrow().class;
                        Ok(self.class(cls).name.clone())
                    }
                    other => Err(Thrown::type_error(format!(
                        "Cannot use \"::class\" on {}",
                        rhp_builtins::zval_value_name(self, &other)
                    ))),
                };
            }
            ClassRef::Name(n) => {
                return Ok(self.stack[top].proto.names[n as usize].clone());
            }
            ClassRef::SelfClass => "self",
            ClassRef::ParentClass => "parent",
            ClassRef::StaticClass => "static",
        };
        let Some(scope) = scope else {
            return Err(Thrown::error(
                "Error",
                if const_expr {
                    "Cannot use \"self\" when no class scope is active".to_string()
                } else {
                    format!("Cannot use \"{kind}\" in the global scope")
                },
            ));
        };
        let cls = match class {
            ClassRef::SelfClass => scope,
            ClassRef::ParentClass => self.class(scope).parent.ok_or_else(|| {
                Thrown::error(
                    "Error",
                    "Cannot use \"parent\" when current class scope has no parent",
                )
            })?,
            _ => self.stack[top].called_class.unwrap_or(scope),
        };
        Ok(self.class(cls).name.clone())
    }

    fn fetch_class_const(
        &mut self,
        top: usize,
        proto: &FunctionProto,
        dst: Reg,
        class: ClassRef,
        name: Value,
    ) -> Result<(), Thrown> {
        let cls = self.resolve_class_autoload(top, class)?;
        let name_b = match name {
            Value::Str(s) => s.as_bytes().to_vec(),
            // zend_invalid_class_constant_type_error.
            other => {
                return Err(Thrown::type_error(format!(
                    "Cannot use value of type {} as class constant name",
                    match other {
                        Value::Null => "null",
                        Value::Bool(_) => "bool",
                        Value::Int(_) => "int",
                        Value::Float(_) => "float",
                        Value::Array(_) => "array",
                        _ => "object",
                    }
                )))
            }
        };
        if name_b == b"class" {
            let v = Value::str(self.class(cls).name.as_bytes().to_vec());
            self.reg_set(top, dst, v);
        } else {
            // php loads every constant of a backed enum on first
            // fetch (it builds the value -> case table).
            // (Not from inside a constant expression, which goes
            // through zend_get_class_constant_ex in php.)
            let in_const_expr = proto.flags.contains(rhp_bytecode::ProtoFlags::CONST_EXPR);
            if self.class(cls).enum_backing.is_some() && !self.class(cls).updated && !in_const_expr
            {
                self.update_class_constants(cls)?;
            }
            // Inside a constant expression php names the class as
            // written (`self::X`); the opcode path uses its name.
            let display: Vec<u8> = match class {
                ClassRef::Name(n) if in_const_expr => proto.names[n as usize].as_bytes().to_vec(),
                ClassRef::SelfClass if in_const_expr => b"self".to_vec(),
                ClassRef::ParentClass if in_const_expr => b"parent".to_vec(),
                ClassRef::StaticClass if in_const_expr => b"static".to_vec(),
                _ => self.class(cls).name.as_bytes().to_vec(),
            };
            match self.class(cls).constants.get(&name_b) {
                Some(c) => {
                    let (vis, decl) = (c.visibility, c.declaring_class);
                    let deprecated = c.deprecated.clone();
                    let ready = c.init.is_none().then(|| c.value.clone());
                    if !self.check_access(self.stack[top].defining_class, decl, vis) {
                        return Err(self.access_err("constant", decl, &name_b, vis));
                    }
                    if let Some(d) = deprecated {
                        self.class_const_deprecation(decl, &name_b, &d)?;
                    }
                    let v = if let Some(v) = ready {
                        v
                    } else {
                        self.class_const_value(cls, &name_b, &display, in_const_expr)?
                            .unwrap_or(Value::Null)
                    };
                    if let Value::Object(h) = &v {
                        self.touch_enum_case(h);
                    }
                    // A built-in class's `#[\Deprecated]` constant.
                    if self.is_internal_class(decl) {
                        let dname = self.class(decl).name.as_bytes();
                        if let Some(msg) = builtins::class_constant_deprecation(dname, &name_b) {
                            self.diag(builtins::E_DEPRECATED, &msg)?;
                        }
                    }
                    self.reg_set(top, dst, v);
                }
                None => {
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Undefined constant {}::{}",
                            String::from_utf8_lossy(&display),
                            String::from_utf8_lossy(&name_b)
                        ),
                    ))
                }
            }
        }
        Ok(())
    }

    /// The E_DEPRECATED fetching built-in constant `bare` raises (its
    /// CONST_DEPRECATED flag).
    pub(crate) fn internal_const_deprecation(&self, bare: &[u8]) -> Option<String> {
        let msg = match bare {
            b"E_STRICT" => {
                "Constant E_STRICT is deprecated since 8.4, the error level was removed".to_string()
            }
            b"MT_RAND_PHP" => "Constant MT_RAND_PHP is deprecated since 8.3, as it uses a biased non-standard variant of Mt19937".to_string(),
            _ => builtins::constant_deprecation(bare)?,
        };
        (self.constants.contains_key(bare) && !self.user_const_order.iter().any(|n| n == bare))
            .then_some(msg)
    }

    /// `FetchConst`: the global constant `name` (or `fallback`) into `dst`.
    fn fetch_const(
        &mut self,
        top: usize,
        proto: &FunctionProto,
        dst: Reg,
        name: NIdx,
        fallback: NIdx,
    ) -> Result<(), Thrown> {
        let cname = proto.names[name as usize].as_bytes();
        let key = builtins::zend::builtin_functions::const_key(cname);
        // The constant found: `name`, else the global `fallback`.
        let (v, hit): (Option<Value>, &[u8]) = match self.constants.get(&*key) {
            Some(v) => (Some(v.clone()), &key),
            None if fallback != rhp_bytecode::NO_NAME => {
                let f = proto.names[fallback as usize].as_bytes();
                (self.constants.get(f).cloned(), f)
            }
            None => (None, &key),
        };
        match v {
            Some(v) => {
                if let Some(msg) = self.internal_const_deprecation(hit) {
                    self.diag(builtins::E_DEPRECATED, &msg)?;
                }
                if !self.const_attrs.is_empty() {
                    let hit = hit.to_vec();
                    self.deprecated_user_constant(&hit)?;
                }
                self.reg_set(top, dst, v);
            }
            None => {
                return Err(Thrown::error(
                    "Error",
                    format!("Undefined constant \"{}\"", String::from_utf8_lossy(cname)),
                ))
            }
        }
        Ok(())
    }
}

/// The int in `v`, also behind a reference (not while it is borrowed:
/// the slow path then fails as it always did).
#[inline(always)]
fn int_of(v: &Value) -> Option<i64> {
    match v {
        Value::Int(n) => Some(*n),
        Value::Ref(rc) => match *rc.try_borrow().ok()? {
            Value::Int(n) => Some(n),
            _ => None,
        },
        _ => None,
    }
}

/// Add `delta` to the int in `slot` (behind a reference too) unless that
/// overflows; the old value. Not while the reference is borrowed (the
/// slow path then fails as it always did).
#[inline(always)]
fn int_in_place(slot: &mut Value, delta: i64) -> Option<i64> {
    match slot {
        Value::Int(n) => {
            let old = *n;
            *n = old.checked_add(delta)?;
            Some(old)
        }
        Value::Ref(rc) => match &mut *rc.try_borrow_mut().ok()? {
            Value::Int(n) => {
                let old = *n;
                *n = old.checked_add(delta)?;
                Some(old)
            }
            _ => None,
        },
        _ => None,
    }
}

/// A copy of a scalar, without the out-of-line clone.
#[inline(always)]
fn scalar_copy(v: &Value) -> Option<Value> {
    match *v {
        Value::Null => Some(Value::Null),
        Value::Bool(b) => Some(Value::Bool(b)),
        Value::Int(n) => Some(Value::Int(n)),
        Value::Float(f) => Some(Value::Float(f)),
        _ => None,
    }
}

/// Define an undefined variable cell as null (a write through it).
pub(crate) fn cell_define(cell: &RefCell<Value>) {
    if let Ok(mut c) = cell.try_borrow_mut() {
        if matches!(*c, Value::Undef) {
            *c = Value::Null;
        }
    }
}

/// A value with nothing to drop: overwriting it needs no drop glue and
/// never releases an object or array.
#[inline(always)]
fn is_scalar(v: &Value) -> bool {
    matches!(
        v,
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Undef
    )
}

/// php's increment_string: `"a"++` is `"b"`, `"Az"++` is `"Ba"`, `"zz"++` is
/// `"aaa"`; the carry stops at the first non-alphanumeric byte; `""` is `"1"`.
fn perl_increment(s: &[u8]) -> Vec<u8> {
    if s.is_empty() {
        return b"1".to_vec();
    }
    let mut v = s.to_vec();
    let mut pos = v.len();
    let mut carry = false;
    let mut last = 0u8;
    while pos > 0 {
        pos -= 1;
        let (lo, hi) = match v[pos] {
            b'a'..=b'z' => (b'a', b'z'),
            b'A'..=b'Z' => (b'A', b'Z'),
            b'0'..=b'9' => (b'0', b'9'),
            _ => {
                carry = false;
                break;
            }
        };
        last = lo;
        if v[pos] == hi {
            v[pos] = lo;
            carry = true;
        } else {
            v[pos] += 1;
            carry = false;
            break;
        }
    }
    if carry {
        v.insert(0, if last == b'0' { b'1' } else { last });
    }
    v
}

impl Vm {
    /// `MakeClosure`: a new Closure object over the captured window.
    #[inline(never)]
    fn make_closure(
        &mut self,
        top: usize,
        dst: Reg,
        fnidx: u32,
        base: Reg,
        ncaptures: u16,
    ) -> Result<(), Thrown> {
        // Raw (non-deref) read: a by-reference capture arrives as a
        // `Value::Ref` (from MakeRef) and must stay a Ref so writes
        // inside the closure reach the outer variable; by-value
        // captures were already deref'd into the window by Mov.
        let captures: Vec<Value> = (0..ncaptures)
            .map(|i| self.stack[top].regs[(base + i) as usize].copy_slot())
            .collect();
        // Bind the defining/static class scope so the closure body can
        // reach private members and resolve `self::`/`static::`.
        // `fnidx` is file-local; resolve into the flat table.
        let fnidx = self.unit_base(top).closures + fnidx;
        // A non-static closure binds the creating frame's `$this`
        // (zend_create_closure), whether or not it uses it.
        let bound_this = if self.closures[fnidx as usize].proto.is_static {
            None
        } else {
            self.current_this(top)
        };
        let cd = Rc::new(rhp_values::ClosureData {
            fnidx,
            captures,
            bound_this,
            bound_class: self.stack[top].defining_class,
            bound_static: self.stack[top].called_class,
            wrapped: None,
            id: rhp_values::alloc_object_id(),
            statics: Default::default(),
        });
        self.reg_set(top, dst, Value::Closure(cd));
        Ok(())
    }

    /// `MakeFcc`: a first-class callable Closure over `callee`.
    #[inline(never)]
    fn make_fcc(
        &mut self,
        top: usize,
        proto: &Rc<FunctionProto>,
        dst: Reg,
        callee: Reg,
        ns_name: NIdx,
    ) -> Result<(), Thrown> {
        // A callee built in a temporary (`[$o, 'm']`) is dead
        // after this: the closure owns it.
        let mut c = if callee >= proto.num_cvs {
            match std::mem::replace(&mut self.stack[top].regs[callee as usize], Value::Null) {
                Value::Ref(rc) => rc.borrow().clone(),
                Value::Undef => Value::Null,
                v => v,
            }
        } else {
            self.reg_r(top, callee)?
        };
        // `$closure->__invoke(...)` is that Closure.
        if let Some(cl) = builtins::zend::closures::closure_invoke_of(&c) {
            self.reg_set(top, dst, cl);
            return Ok(());
        }
        if let Value::Str(name) = &c {
            if let Some(n) = self.check_fcc_name(top, name, ns_name)? {
                c = Value::Str(n);
            }
        }
        // ZEND_CALLABLE_CONVERT keeps the call's object.
        let this = match self.resolve_callable(&c) {
            Ok(
                crate::calls::Callee::Method(_, this, _) | crate::calls::Callee::Magic(_, this, ..),
            ) => this,
            _ => None,
        };
        let cd = Rc::new(rhp_values::ClosureData {
            fnidx: u32::MAX,
            captures: Vec::new(),
            bound_this: this,
            bound_class: self.stack[top].defining_class,
            bound_static: self.stack[top].called_class,
            wrapped: Some(Box::new(c)),
            id: rhp_values::alloc_object_id(),
            statics: Default::default(),
        });
        self.reg_set(top, dst, Value::Closure(cd));
        Ok(())
    }

    /// `CallDyn`: `$f(...)` with the arguments in a window.
    #[inline(never)]
    fn call_dyn(
        &mut self,
        top: usize,
        dst: Reg,
        callee: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        let c = self.reg_r(top, callee)?;
        let strict = self.caller_strict();
        // A user closure binds by-reference parameters to the
        // variables passed (the window holds them as `Ref`s), as
        // call_callable_value_mode would run it; anything else
        // gets plain values.
        if let Value::Closure(cd) = &c {
            if cd.wrapped.is_none() {
                self.fcc_scope.set(None);
                let args = Args::Window { top, base, argc };
                let out = self.run_closure_sync(cd, args, Some(strict))?;
                self.reg_set(top, dst, out);
                return Ok(());
            }
            // A user function's first-class callable: a direct
            // call (by-reference parameters and return).
            if let Some(Value::Str(n)) = cd.wrapped.as_deref() {
                let n = n.as_bytes();
                let n = n.strip_prefix(b"\\").unwrap_or(n);
                if let Some(&i) = self.fn_names.get(&n.to_ascii_lowercase()) {
                    let callee = self.functions[i].clone();
                    if !callee.proto.is_internal() {
                        self.enter_user_call(top, callee, dst, base, argc)?;
                        return Ok(());
                    }
                }
            }
        }
        // A user function's name: a direct call (by-reference parameters
        // and return).
        if let Value::Str(s) = &c {
            let n = s.as_bytes();
            let n = n.strip_prefix(b"\\").unwrap_or(n);
            if !n.contains(&b':') {
                if let Some(&i) = self.fn_names.get(&n.to_ascii_lowercase()) {
                    let callee = self.functions[i].clone();
                    if !callee.proto.is_internal() {
                        self.enter_user_call(top, callee, dst, base, argc)?;
                        return Ok(());
                    }
                }
            }
        }
        // An invokable object: a call of its __invoke method, with
        // by-reference parameters and return.
        if let Value::Object(h) = &c {
            let cls = h.borrow().class;
            if self
                .lookup_method(cls, b"__invoke")
                .is_some_and(|m| !m.is_static && m.visibility == rhp_values::Visibility::Public)
            {
                self.invoke_method(top, h.clone(), cls, b"__invoke", dst, base, argc)?;
                return Ok(());
            }
        }
        // Passed raw (the window's references kept): the callee's
        // by-reference parameters bind them, by-value ones dereference.
        let args: Vec<Value> = (0..argc)
            .map(|i| self.stack[top].regs[(base + i) as usize].clone())
            .collect();
        self.dyn_call = true;
        let out = self.call_callable_value_mode(&c, args, Some(strict));
        self.dyn_call = false;
        let out = out?;
        self.reg_set(top, dst, out);
        Ok(())
    }
}

impl Vm {
    /// `BindStatic`: bind CV `cv` to static variable `name` (the running
    /// closure's own, else the function's), created with constant `init`.
    #[inline(never)]
    fn bind_static(
        &mut self,
        top: usize,
        proto: &Rc<FunctionProto>,
        cv: Reg,
        name: NIdx,
        init: u32,
    ) {
        if self.stack[top].closure.is_some() {
            let initv = if init == u32::MAX {
                Value::Null
            } else {
                proto.consts[init as usize].clone()
            };
            let n = &proto.names[name as usize];
            self.bind_closure_static(top, cv, n, Some(initv));
            return;
        }
        let sname = proto.names[name as usize].as_bytes().to_vec();
        let key = self.static_key(top, proto, sname);
        let statics = self.static_table(top);
        let cell = match statics.get(&key) {
            Some(c) => c.clone(),
            None => {
                let initv = if init == u32::MAX {
                    Value::Null
                } else {
                    proto.consts[init as usize].clone()
                };
                let c = Rc::new(RefCell::new(initv));
                statics.insert(key, c.clone());
                c
            }
        };
        self.stack[top].regs[cv as usize] = Value::Ref(cell);
        self.sync_global_cv(top, cv);
    }
}

impl Vm {
    /// An array literal's element: an object in a temporary moves into the
    /// array (the temporary is dead, and php frees it there).
    #[inline]
    fn elem_operand(&mut self, top: usize, proto: &FunctionProto, r: Reg) -> Result<Value, Thrown> {
        if r >= proto.num_cvs
            && matches!(
                self.stack[top].regs[r as usize],
                Value::Object(_) | Value::Closure(_)
            )
        {
            return Ok(std::mem::replace(
                &mut self.stack[top].regs[r as usize],
                Value::Null,
            ));
        }
        self.reg_r(top, r)
    }
}

impl Vm {
    /// Whether register `r` holds an object with a destructor. Such a
    /// temporary stays until overwritten: rhp destructs released objects in
    /// creation order, so freeing it with the callee's frame would destruct
    /// it before the callee's locals (php: locals first, then `$this`).
    #[inline(never)]
    fn holds_destructible(&self, top: usize, r: Reg) -> bool {
        match &self.stack[top].regs[r as usize] {
            Value::Object(h) => {
                let cls = h.borrow().class;
                self.class(cls)
                    .handlers
                    .is_some_and(|t| t.dtor_obj.is_some() || t.free_obj_host.is_some())
                    || self.lookup_method(cls, b"__destruct").is_some()
            }
            _ => false,
        }
    }

    /// Move a temporary's value out (dereferenced), leaving null.
    #[inline(never)]
    fn take_temp(&mut self, top: usize, r: Reg) -> Value {
        match std::mem::replace(&mut self.stack[top].regs[r as usize], Value::Null) {
            Value::Ref(rc) => rc.borrow().clone(),
            Value::Undef => Value::Null,
            v => v,
        }
    }

    /// `$closure->m(...)`: `__invoke` runs the closure (by-reference
    /// parameters; Closure::__invoke is internal, so it runs as called from
    /// internal code), anything else is one of Closure's methods.
    #[inline(never)]
    fn call_closure_method(
        &mut self,
        top: usize,
        o: &Value,
        mname: &[u8],
        dst: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        if let Value::Closure(cd) = o {
            if cd.wrapped.is_none() && mname.eq_ignore_ascii_case(b"__invoke") {
                self.fcc_scope.set(None);
                let args = Args::Window { top, base, argc };
                let out = self.run_closure_sync(cd, args, None)?;
                self.reg_set(top, dst, out);
                return Ok(());
            }
        }
        let args: Vec<Value> = (0..argc).map(|i| self.reg_get(top, base + i)).collect();
        let r = self.closure_method(o, mname, args)?;
        self.reg_set(top, dst, r);
        Ok(())
    }
}

impl Vm {
    /// Bind CV `cv` to the running closure's own static variable `name`,
    /// creating it with `init` (`None`: only if it exists). Whether it is
    /// bound.
    #[inline(never)]
    fn bind_closure_static(
        &mut self,
        top: usize,
        cv: Reg,
        name: &PhpStr,
        init: Option<Value>,
    ) -> bool {
        let Some(cd) = self.stack[top].closure.clone() else {
            return false;
        };
        let mut st = cd.statics.borrow_mut();
        let cell = match st.iter().find(|(k, _)| k == name) {
            Some((_, c)) => c.clone(),
            None => {
                let Some(v) = init else { return false };
                let c = Rc::new(RefCell::new(v));
                st.push((name.clone(), c.clone()));
                c
            }
        };
        self.stack[top].regs[cv as usize] = Value::Ref(cell);
        true
    }
}

/// Where php's code for the loop whose head (condition) is at `pc` starts
/// its body: rhp tests a while/for condition at the top, behind a
/// `Jmp` to the next instruction, and leaves through a `JmpFalse` to just
/// past the back edge; php tests it at the bottom. `None`: not a loop head.
fn loop_body_start(code: &[Instr], pc: usize) -> Option<usize> {
    let back_edge = |i: &Instr| matches!(i, Instr::Jmp { target } if *target as usize == pc);
    if pc == 0 || !back_edge(&code[pc - 1]) {
        return None;
    }
    for (k, i) in code.iter().enumerate().skip(pc) {
        match i {
            Instr::JmpFalse { target, .. }
                if (*target as usize) > 0
                    && code.get(*target as usize - 1).is_some_and(back_edge) =>
            {
                return Some(k + 1);
            }
            i if back_edge(i) => return Some(pc),
            _ => {}
        }
    }
    None
}
