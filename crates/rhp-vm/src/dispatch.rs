//! The instruction loop (one `match` over `Instr`), register access and the
//! operator helpers it calls.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rhp_builtins as builtins;
use rhp_builtins::zend::object_handlers::{CastTarget, Op};
use rhp_bytecode::{CallFlags, ClassRef, CmpKind, FunctionProto, Instr, NIdx, Pc, Reg};
use rhp_values::ops::{self, cast_to_array, inc_dec_value};
use rhp_values::{conv, ArrayKey, ClassKind, ObjectHandle, PhpArray, PhpStr, Thrown, Value};

use crate::calls::{dyn_method_name, param_is_ref, undefined_function, CallTarget};
use crate::errors::{enclosing_finally, FinallyCont, UnwindResult};
use crate::generators::GenStatus;
use crate::lvalue::{
    aa_key, foreach_promote, isset_dim, CursorRoot, CursorStep, DimFetch, DimMode, MagicSplit,
};
use crate::types::value_name_lc;
use crate::{classes, deref, StaticKey, UnitBase, Vm};

impl Vm {
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

    /// Plain register write (temporaries / instruction destinations): overwrites
    /// the slot. Never writes through a stale `Ref` left in a reused temp.
    pub(crate) fn reg_set(&mut self, top: usize, r: Reg, v: Value) {
        let slot = &mut self.stack[top].regs[r as usize];
        if !self.destructibles.is_empty() && matches!(slot, Value::Object(_) | Value::Array(_)) {
            self.dtor_check = true;
        }
        *slot = v;
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
                            Ok(UnwindResult::Caught) => continue, // handler installed; re-enter
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
            let pc = self.stack[top].pc;
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
            let instr = proto.code[pc];
            self.stack[top].pc = pc + 1;

            // Run destructors of objects the program has just released.
            if self.dtor_check {
                self.dtor_check = false;
                if !self.destructibles.is_empty() {
                    self.collect_released()?;
                    self.stack[top].pc = pc;
                    continue;
                }
            }

            // pcntl async signals (php's `EG(vm_interrupt)` check): run queued
            // PHP signal handlers before the next instruction.
            if builtins::pcntl_interrupt_pending() {
                // A throwing handler raises at this (not yet run) instruction;
                // otherwise the instruction then runs as usual.
                builtins::pcntl_dispatch(self)?;
                self.stack[top].pc = pc;
                continue;
            }

            match instr {
                Instr::Nop => {}
                Instr::Mov { dst, src } => {
                    let v = self.reg_get(top, src);
                    self.reg_set(top, dst, v);
                }
                Instr::LoadConst { dst, k } => {
                    let v = proto.consts[k as usize].clone();
                    self.reg_set(top, dst, v);
                }
                Instr::LoadNull { dst } => {
                    // `unset($x)` in the global scope drops the symbol-table
                    // entry's binding (a reference to it keeps the old cell).
                    if self.stack[top].global_scope
                        && (dst as usize) < proto.cv_var_names.len()
                        && matches!(self.stack[top].regs[dst as usize], Value::Ref(_))
                    {
                        self.stack[top].regs[dst as usize] = Value::Null;
                        self.sync_global_cv(top, dst);
                        self.dtor_check = true;
                    } else {
                        self.reg_set(top, dst, Value::Null);
                    }
                }
                Instr::LoadBool { dst, b } => self.reg_set(top, dst, Value::Bool(b)),
                Instr::LoadInt { dst, n } => self.reg_set(top, dst, Value::Int(n)),

                Instr::Assign { cv, src } => {
                    let v = self.reg_get(top, src);
                    self.cv_assign(top, cv, v);
                }

                Instr::Add { dst, a, b } => self.bin(top, dst, a, b, Op::Add, ops::add)?,
                Instr::Sub { dst, a, b } => self.bin(top, dst, a, b, Op::Sub, ops::sub)?,
                Instr::Mul { dst, a, b } => self.bin(top, dst, a, b, Op::Mul, ops::mul)?,
                Instr::Div { dst, a, b } => self.bin(top, dst, a, b, Op::Div, ops::div)?,
                Instr::Mod { dst, a, b } => self.bin(top, dst, a, b, Op::Mod, ops::modulo)?,
                Instr::Pow { dst, a, b } => self.bin(top, dst, a, b, Op::Pow, ops::pow)?,
                Instr::BitAnd { dst, a, b } => {
                    self.bin_inf(top, dst, a, b, Op::BitAnd, |x, y| {
                        ops::bitwise(x, y, ops::BitOp::And)
                    })?
                }
                Instr::BitOr { dst, a, b } => self.bin_inf(top, dst, a, b, Op::BitOr, |x, y| {
                    ops::bitwise(x, y, ops::BitOp::Or)
                })?,
                Instr::BitXor { dst, a, b } => {
                    self.bin_inf(top, dst, a, b, Op::BitXor, |x, y| {
                        ops::bitwise(x, y, ops::BitOp::Xor)
                    })?
                }
                Instr::Shl { dst, a, b } => self.bin_inf(top, dst, a, b, Op::Shl, |x, y| {
                    ops::bitwise(x, y, ops::BitOp::Shl)
                })?,
                Instr::Shr { dst, a, b } => self.bin_inf(top, dst, a, b, Op::Shr, |x, y| {
                    ops::bitwise(x, y, ops::BitOp::Shr)
                })?,
                Instr::Concat { dst, a, b } => {
                    if self.concat_in_place(top, &proto.code, pc, proto.num_cvs, dst, a, b) {
                        continue;
                    }
                    let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
                    let v = if matches!(x, Value::Object(_) | Value::Array(_))
                        || matches!(y, Value::Object(_) | Value::Array(_))
                    {
                        self.concat_object(&x, &y)?
                    } else {
                        ops::concat(&x, &y)
                    };
                    self.reg_set(top, dst, v);
                }

                Instr::Neg { dst, a } => {
                    let v = self.reg_get(top, a);
                    let r = if let Value::Object(_) | Value::Closure(_) = v {
                        // php compiles `-$x` to `$x * -1`.
                        self.arith_object(Op::Mul, ops::mul, &v, &Value::Int(-1))?
                    } else {
                        ops::negate(&v)?
                    };
                    self.reg_set(top, dst, r);
                }
                Instr::BitNot { dst, a } => {
                    let v = self.reg_get(top, a);
                    if let Value::Object(h) = &v {
                        if let Some(f) = self.handlers_of(h).and_then(|t| t.do_operation) {
                            if let Some(r) = f(self, Op::BitNot, &v, None)? {
                                self.reg_set(top, dst, r);
                                continue;
                            }
                        }
                    }
                    if let Value::Object(_) | Value::Closure(_) = v {
                        return Err(Thrown::type_error(format!(
                            "Cannot perform bitwise not on {}",
                            rhp_builtins::zval_value_name(self, &v)
                        )));
                    }
                    self.reg_set(top, dst, ops::bitwise_not(&v));
                }
                Instr::Not { dst, a } => {
                    let v = self.reg_get(top, a);
                    let b = self.truthy(&v)?;
                    self.reg_set(top, dst, Value::Bool(!b));
                }
                Instr::IncDec {
                    dst,
                    cv,
                    pre,
                    delta,
                } => {
                    let cur = self.reg_get(top, cv);
                    let new = self.inc_dec_vm(&cur, delta as i64)?;
                    self.cv_assign(top, cv, new.clone());
                    self.reg_set(top, dst, if pre { new } else { cur });
                }

                Instr::CastInt { dst, a } => {
                    let v = self.reg_get(top, a);
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
                        _ => conv::to_int(&v),
                    };
                    self.reg_set(top, dst, Value::Int(n));
                }
                Instr::CastFloat { dst, a } => {
                    let v = self.reg_get(top, a);
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
                    let v = self.reg_get(top, a);
                    let s = self.to_string_vm(&v)?;
                    self.reg_set(top, dst, Value::str(s));
                }
                Instr::CastBool { dst, a } => {
                    let v = self.reg_get(top, a);
                    let b = self.truthy(&v)?;
                    self.reg_set(top, dst, Value::Bool(b));
                }
                Instr::CastArray { dst, a } => {
                    let v = self.reg_get(top, a);
                    let out = match &v {
                        // get_properties_for(ARRAY_CAST): an internal class's
                        // view (an ArrayObject's storage).
                        Value::Object(h) => {
                            let purpose = builtins::zend::object_handlers::PropPurpose::ArrayCast;
                            match builtins::zend::object_handlers::properties_for(self, h, purpose)?
                            {
                                Some(a) => Value::Array(a),
                                None => self.object_to_array_mangled(h),
                            }
                        }
                        _ => cast_to_array(v),
                    };
                    self.reg_set(top, dst, out);
                }
                Instr::CastObject { dst, a } => {
                    let v = self.reg_get(top, a);
                    let obj = self.cast_to_object(v);
                    self.reg_set(top, dst, obj);
                }

                Instr::Cmp { dst, a, b, kind } => {
                    let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
                    let r = self.eval_cmp(kind, &x, &y)?;
                    self.reg_set(top, dst, r);
                }
                Instr::BeginSilence => {
                    self.silence_stack.push((self.stack.len(), self.silence));
                    self.silence += 1;
                }
                Instr::EndSilence => {
                    if let Some((_, lvl)) = self.silence_stack.pop() {
                        self.silence = lvl;
                    }
                }

                Instr::Echo { src } => {
                    let v = self.reg_get(top, src);
                    let bytes = self.to_string_vm(&v)?;
                    self.echo_out(&bytes)?;
                }
                Instr::EchoConst { k } => {
                    let bytes = conv::to_string_bytes(&proto.consts[k as usize]);
                    self.echo_out(&bytes)?;
                }
                Instr::Print { dst, src } => {
                    let v = self.reg_get(top, src);
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
                Instr::JmpTrue { cond, target } => {
                    let c = self.reg_get(top, cond);
                    if self.truthy(&c)? {
                        self.stack[top].pc = target as usize;
                    }
                }
                Instr::JmpFalse { cond, target } => {
                    let c = self.reg_get(top, cond);
                    if !self.truthy(&c)? {
                        self.stack[top].pc = target as usize;
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
                    let v = self.reg_get(top, val);
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        a.push(v);
                    }
                }
                Instr::ArrSet { arr, key, val } => {
                    let k = self.reg_get(top, key);
                    let v = self.reg_get(top, val);
                    let key = ArrayKey::from_value(&k)
                        .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        a.insert(key, v);
                    }
                }
                Instr::ArrPushRef { arr, val } => {
                    let v = self.stack[top].regs[val as usize].clone();
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        a.push(v);
                    }
                }
                Instr::ArrSetRef { arr, key, val } => {
                    let k = self.reg_get(top, key);
                    let v = self.stack[top].regs[val as usize].clone();
                    let key = ArrayKey::from_value(&k)
                        .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        a.insert(key, v);
                    }
                }
                Instr::ArrSpread { arr, src } => {
                    let entries = self.unpack_entries(top, src, false)?;
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        for (k, iv) in entries {
                            match k {
                                ArrayKey::Int(_) => a.push(iv),
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
                        let out = self.call_builtin(top, builtin, sig, base, argc, inline, icall);
                        self.cuf_caller = None;
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
                    // Raw (non-deref) read: a by-reference capture arrives as a
                    // `Value::Ref` (from MakeRef) and must stay a Ref so writes
                    // inside the closure reach the outer variable; by-value
                    // captures were already deref'd into the window by Mov.
                    let captures: Vec<Value> = (0..ncaptures)
                        .map(|i| self.stack[top].regs[(base + i) as usize].clone())
                        .collect();
                    // Bind the defining/static class scope so the closure body can
                    // reach private members and resolve `self::`/`static::`.
                    let cd = Rc::new(rhp_values::ClosureData {
                        // `fnidx` is file-local; resolve into the flat table.
                        fnidx: self.unit_base(top).closures + fnidx,
                        captures,
                        bound_this: None,
                        bound_class: self.stack[top].defining_class,
                        bound_static: self.stack[top].called_class,
                        wrapped: None,
                    });
                    self.reg_set(top, dst, Value::Closure(cd));
                }
                Instr::BindGlobal { cv, name } => {
                    let gname = proto.names[name as usize].clone();
                    let cell = self.global_cell(gname.as_bytes());
                    self.ref_globals.insert(gname.as_bytes().to_vec());
                    self.stack[top].regs[cv as usize] = Value::Ref(cell);
                }
                Instr::BindGlobalDyn { name } => {
                    let gname = conv::to_string_bytes(&self.reg_get(top, name));
                    let cell = self.global_cell(&gname);
                    self.ref_globals.insert(gname.clone());
                    self.var_var_bind(top, &gname, cell);
                }
                Instr::BindStatic { cv, name, init } => {
                    let sname = proto.names[name as usize].as_bytes().to_vec();
                    let key = self.static_key(top, &proto, sname);
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
                Instr::BindStaticOrJmp { cv, name, target } => {
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
                    let sname = proto.names[name as usize].as_bytes().to_vec();
                    let key = self.static_key(top, &proto, sname);
                    let v = self.reg_get(top, src);
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
                    let key = builtins::const_key(cname);
                    let v = self.reg_get(top, src);
                    if self.const_taken(&key) {
                        let msg = builtins::const_redefined_message(cname);
                        self.diag(builtins::E_WARNING, &msg)?;
                    } else {
                        self.constants.insert(key.into_owned(), v);
                        self.user_const_order.push(cname.to_vec());
                    }
                }
                Instr::MakeFcc {
                    dst,
                    callee,
                    ns_name,
                } => {
                    let mut c = self.reg_get(top, callee);
                    if let Value::Str(name) = &c {
                        if let Some(n) = self.check_fcc_name(top, name, ns_name)? {
                            c = Value::Str(n);
                        }
                    }
                    let cd = Rc::new(rhp_values::ClosureData {
                        fnidx: u32::MAX,
                        captures: Vec::new(),
                        bound_this: None,
                        bound_class: self.stack[top].defining_class,
                        bound_static: self.stack[top].called_class,
                        wrapped: Some(Box::new(c)),
                    });
                    self.reg_set(top, dst, Value::Closure(cd));
                }
                Instr::CallDyn {
                    dst,
                    callee,
                    base,
                    argc,
                } => {
                    let c = self.reg_get(top, callee);
                    // A user closure binds by-reference parameters to the
                    // variables passed (the window holds them as `Ref`s);
                    // anything else gets plain values.
                    let raw = matches!(&c, Value::Closure(cd) if cd.wrapped.is_none());
                    let args: Vec<Value> = (0..argc)
                        .map(|i| {
                            if raw {
                                self.stack[top].regs[(base + i) as usize].clone()
                            } else {
                                self.reg_get(top, base + i)
                            }
                        })
                        .collect();
                    let strict = self.caller_strict();
                    let out = self.call_callable_value_mode(&c, args, Some(strict))?;
                    self.reg_set(top, dst, out);
                }
                Instr::CallDynV { dst, callee, args } => {
                    let c = self.reg_get(top, callee);
                    let arr = self.reg_get(top, args);
                    let (argvec, named) = self.split_call_args(&arr)?;
                    let strict = self.caller_strict();
                    let out = self.call_named(&c, argvec, named, Some(strict))?;
                    self.reg_set(top, dst, out);
                }

                // -- array element read --
                Instr::DimFetch { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
                    let r = self.dim_read(&a, &k, DimMode::Read)?;
                    self.reg_set(top, dst, r);
                }
                Instr::DimFetchW { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
                    let r = match deref(&a) {
                        Value::Object(h)
                            if self.class_implements(h.borrow().class, b"ArrayAccess") =>
                        {
                            let step = CursorStep::Key(k.clone());
                            let cell = self.aa_fetch_w(&h, &step, k, DimFetch::W)?;
                            let v = cell.borrow().clone();
                            v
                        }
                        _ => self.dim_read(&a, &k, DimMode::Read)?,
                    };
                    self.reg_set(top, dst, r);
                }
                Instr::DimFetchIs { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
                    let r = self.dim_read(&a, &k, DimMode::Is)?;
                    self.reg_set(top, dst, r);
                }
                Instr::DimFetchList { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
                    let r = self.dim_read(&a, &k, DimMode::List)?;
                    self.reg_set(top, dst, r);
                }

                // -- lvalue cursor --
                Instr::LvalCv { cv } => {
                    self.cursor_root = CursorRoot::Cv(cv);
                    self.cursor_global = None;
                    self.cursor_steps.clear();
                }
                Instr::LvalKey { key } => {
                    let k = self.reg_get(top, key);
                    self.cursor_steps.push(CursorStep::Key(k));
                }
                Instr::LvalAppend => self.cursor_steps.push(CursorStep::Append),
                Instr::LvalProp { name } => {
                    let p = proto.names[name as usize].clone();
                    self.cursor_steps.push(CursorStep::Prop(p));
                }
                Instr::LvalPropDyn { name } => {
                    let p = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
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
                }
                Instr::LvalStaticPropDyn { class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = conv::to_string_bytes(&self.reg_get(top, name));
                    self.check_static_prop_access(top, cls, &pname)?;
                    let cell = self.static_prop_cell(cls, &pname)?;
                    self.cursor_root = CursorRoot::Cell(cell);
                    self.cursor_global = None;
                    self.cursor_steps.clear();
                }
                Instr::LvalVarVar { name } => {
                    let vname = conv::to_string_bytes(&self.reg_get(top, name));
                    self.cursor_root = match self.var_var_slot(top, &vname) {
                        Ok(cv) => CursorRoot::Cv(cv),
                        Err(cell) => CursorRoot::Cell(cell),
                    };
                    self.cursor_global = None;
                    self.cursor_steps.clear();
                }
                Instr::BindVarVarRef { name, src } => {
                    let vname = conv::to_string_bytes(&self.reg_get(top, name));
                    let cell = match &self.stack[top].regs[src as usize] {
                        Value::Ref(rc) => rc.clone(),
                        v => Rc::new(RefCell::new(v.clone())),
                    };
                    self.var_var_bind(top, &vname, cell);
                }
                Instr::UnsetVarVar { name } => {
                    let vname = conv::to_string_bytes(&self.reg_get(top, name));
                    self.var_var_unset(top, &vname);
                }
                Instr::FetchVarVar { dst, name } => {
                    let vname = conv::to_string_bytes(&self.reg_get(top, name));
                    let v = match self.var_var_lookup(top, &vname) {
                        Some(Ok(cv)) => self.reg_get(top, cv),
                        Some(Err(cell)) => deref(&cell.borrow()),
                        None => Value::Null,
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::FetchStaticPropDyn { dst, class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = conv::to_string_bytes(&self.reg_get(top, name));
                    self.check_static_prop_access(top, cls, &pname)?;
                    let cell = self.static_prop_cell(cls, &pname)?;
                    let v = cell.borrow().clone();
                    self.reg_set(top, dst, v);
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
                Instr::LvalGlobal { name } => {
                    let n = conv::to_string_bytes(&self.reg_get(top, name));
                    self.cursor_root = CursorRoot::Cell(self.global_cell(&n));
                    self.cursor_global = Some(n);
                    self.cursor_steps.clear();
                }
                Instr::FetchGlobal { dst, name } => {
                    let n = conv::to_string_bytes(&self.reg_get(top, name));
                    let v = match self.globals.get(&n[..]) {
                        Some(c) => deref(&c.borrow()),
                        None => {
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
                    let n = conv::to_string_bytes(&self.reg_get(top, name));
                    if self.globals.contains_key(&n[..]) {
                        // Live global-scope frames lose the variable too.
                        self.rebind_global(&n, Rc::new(RefCell::new(Value::Null)));
                        self.globals.shift_remove(&n[..]);
                        self.ref_globals.remove(&n[..]);
                        self.dtor_check = true;
                    }
                }
                Instr::FetchLvalRw { dst } => {
                    let v = self.cursor_read(top)?;
                    self.reg_set(top, dst, v); // cursor stays open for the AssignLval
                }
                Instr::AssignLval { src } => {
                    let v = self.reg_get(top, src);
                    let steps = std::mem::take(&mut self.cursor_steps);
                    let r = self.cursor_write(top, &steps, v);
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
                    let v = self.reg_get(top, subject);
                    return Err(Thrown::error(
                        "UnhandledMatchError",
                        format!("Unhandled match case {}", match_case_desc(&v)),
                    ));
                }
                Instr::SendNoRef { dst, src } => {
                    let v = self.reg_get(top, src);
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
                    let v = self.reg_get(top, src);
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
                            return Err(Thrown::type_error(
                                "Can only throw objects that implement Throwable",
                            ));
                        }
                        _ => return Err(Thrown::type_error("Can only throw objects")),
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
                        Some(FinallyCont::Jump(target)) => {
                            self.jump_through_finally(top, &proto, pc, target)
                        }
                        None => {}
                    }
                }
                Instr::SetPendingRet { src } => {
                    let mut v = self.reg_get(top, src);
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
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let v = self.prop_fetch(top, o, pname)?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchPropDyn { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
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
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
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
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
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
                    let o = self.reg_get(top, obj);
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
                    let val = self.reg_get(top, src);
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
                    let o = self.reg_get(top, obj);
                    let mname = dyn_method_name(&self.reg_get(top, name))?;
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
                                value_name_lc(&o)
                            ),
                        ));
                    };
                    let cls = h.borrow().class;
                    self.invoke_method(top, h, cls, mname, dst, base, argc)?;
                }
                Instr::StoreProp { obj, name, src } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let val = self.reg_get(top, src);
                    self.prop_store(top, o, pname, val)?;
                    if !self.destructibles.is_empty() {
                        self.dtor_check = true; // the old value may be released
                    }
                }
                Instr::IssetDim { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
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
                    self.reg_set(top, dst, Value::Bool(isset_dim(&a, &k)));
                }
                Instr::EmptyDim { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
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
                        _ => self.dim_read(&a, &k, DimMode::Is)?,
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
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
                    let r = self.prop_empty(top, o, pname)?;
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::IssetPropDyn { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
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
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
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
                } => self.fetch_class_const(top, &proto, dst, class, name)?,
                Instr::CallMethod {
                    dst,
                    obj,
                    name,
                    base,
                    argc,
                    ..
                } => {
                    let o = self.reg_get(top, obj);
                    let mname = proto.names[name as usize].as_bytes().to_vec();
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
                                value_name_lc(&o)
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
                            m.is_some_and(|m| param_is_ref(&m.proto, idx))
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
                Instr::FuncArgIsRef { dst, func, idx } => {
                    let r = self.resolved_param_is_ref(top, func, idx);
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::SendVarEx { dst, cv, func, idx } => {
                    let v = if self.resolved_param_is_ref(top, func, idx) {
                        Value::Ref(self.promote(top, cv))
                    } else {
                        self.reg_get(top, cv)
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::PassArg { dst, src } => {
                    let v = self.stack[top].regs[src as usize].clone();
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
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    self.do_call_static(top, class, mname, dst, base, argc)?;
                }
                Instr::CallStaticDyn {
                    dst,
                    class,
                    name,
                    base,
                    argc,
                } => {
                    let mname = dyn_method_name(&self.reg_get(top, name))?;
                    self.do_call_static(top, class, mname, dst, base, argc)?;
                }
                Instr::InstanceOf { dst, val, class } => {
                    let v = self.reg_get(top, val);
                    let result = match v {
                        Value::Object(h) => match self.resolve_class_autoload(top, class) {
                            Ok(cid) => self.class(h.borrow().class).ancestors.contains(&cid),
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
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Cannot redeclare function {}()",
                                String::from_utf8_lossy(f.proto.name.as_bytes())
                            ),
                        ));
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
                    // `src` already holds a `Ref` (MakeRefLval); alias it raw.
                    let v = self.stack[top].regs[src as usize].clone();
                    self.stack[top].regs[dst_cv as usize] = v;
                    self.sync_global_cv(top, dst_cv);
                }
                Instr::MakeRef { dst, cv } => {
                    let rc = self.promote(top, cv);
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
                Instr::NewV { dst, class, args } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let entry = self.class(cls);
                    if entry.kind != ClassKind::Class || entry.flags.is_abstract {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Cannot instantiate {}",
                                String::from_utf8_lossy(entry.name.as_bytes())
                            ),
                        ));
                    }
                    self.check_ctor_access(top, cls)?;
                    self.update_class_constants(cls)?;
                    let obj = self.new_object(cls);
                    self.reg_set(top, dst, Value::Object(obj.clone()));
                    if let Some(m) = self.lookup_method(cls, b"__construct") {
                        let arr = self.reg_get(top, args);
                        let (argv, named) = self.split_call_args(&arr)?;
                        let argv =
                            self.place_named_args(&m.proto, Some(m.defining_class), argv, named)?;
                        self.push_method_frame(&m, Some(obj), cls, argv, Reg::MAX)?;
                    }
                }
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
                    let mname = dyn_method_name(&self.reg_get(top, name))?;
                    self.call_method_args(top, obj, mname, args, dst)?;
                }
                Instr::CallStaticV {
                    dst,
                    class,
                    name,
                    args,
                } => {
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    self.call_static_args(top, class, mname, args, dst)?;
                }
                Instr::CallStaticDynV {
                    dst,
                    class,
                    name,
                    args,
                } => {
                    let mname = dyn_method_name(&self.reg_get(top, name))?;
                    self.call_static_args(top, class, mname, args, dst)?;
                }

                Instr::FetchConst {
                    dst,
                    name,
                    fallback,
                } => self.fetch_const(top, &proto, dst, name, fallback)?,

                Instr::Ret { src } => {
                    let mut v = self.reg_get(top, src);
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
                Instr::RetRef { src } => {
                    // Return-by-reference: deliver the raw slot (a `Value::Ref`
                    // cell) without dereferencing, so the caller's `$x = &f()`
                    // aliases the same cell. No return-type coercion.
                    let v = self.stack[top].regs[src as usize].clone();
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
                    let raw = conv::to_string_bytes(&self.reg_get(top, path));
                    let v = self.do_include(top, raw, kind)?;
                    self.reg_set(top, dst, v);
                }
                Instr::Exit { arg } => {
                    // A string message is printed through the output buffers (code
                    // 0); an int is the code.
                    let code = match self.reg_get(top, arg) {
                        Value::Int(n) => n as i32,
                        Value::Str(s) => {
                            self.output_write(s.as_bytes());
                            0
                        }
                        Value::Null => 0,
                        other => {
                            let s = conv::to_string_bytes(&other);
                            self.output_write(&s);
                            0
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
            arr @ Value::Array(_) => Err(Thrown::type_error(format!(
                "Cannot {} {}",
                what.to_ascii_lowercase(),
                rhp_builtins::zval_value_name(self, &arr)
            ))),
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
        let v = self.reg_get(top, src);
        let h = match v {
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

    /// Binary op that may throw (arithmetic). An object operand goes
    /// through [`Vm::bin_object`]; plain values take the operator directly.
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
        let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
        let v = if matches!(x, Value::Object(_) | Value::Closure(_))
            || matches!(y, Value::Object(_) | Value::Closure(_))
        {
            self.bin_object(op, f, x, y)?
        } else {
            f(&x, &y)?
        };
        self.reg_set(top, dst, v);
        Ok(())
    }

    /// An operator with an object operand: php's order of `do_operation`
    /// and operand conversion (see `compare.rs`).
    #[inline(never)]
    fn bin_object(
        &mut self,
        op: Op,
        f: fn(&Value, &Value) -> Result<Value, Thrown>,
        x: Value,
        y: Value,
    ) -> Result<Value, Thrown> {
        match op {
            Op::Mod => self.long_op_object(op, &x, &y),
            _ => self.arith_object(op, f, &x, &y),
        }
    }

    /// Bitwise binary op. An object operand goes through
    /// [`Vm::bin_inf_object`].
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn bin_inf(
        &mut self,
        top: usize,
        dst: Reg,
        a: Reg,
        b: Reg,
        op: Op,
        f: fn(&Value, &Value) -> Value,
    ) -> Result<(), Thrown> {
        let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
        let v = if matches!(x, Value::Object(_) | Value::Closure(_))
            || matches!(y, Value::Object(_) | Value::Closure(_))
        {
            self.bin_inf_object(op, x, y)?
        } else {
            f(&x, &y)
        };
        self.reg_set(top, dst, v);
        Ok(())
    }

    /// A bitwise operator with an object operand (`convert_op1_op2_long`).
    #[inline(never)]
    fn bin_inf_object(&mut self, op: Op, x: Value, y: Value) -> Result<Value, Thrown> {
        self.long_op_object(op, &x, &y)
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
        match self.reg_get(top, src) {
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
                } else {
                    let arr = self.object_prop_array(&obj, top);
                    if arr.is_empty() {
                        self.stack[top].pc = end as usize;
                    } else {
                        self.reg_set(top, arr_reg, Value::Array(arr));
                        self.reg_set(top, pos_reg, Value::Int(0));
                    }
                }
            }
            // empty array or non-iterable: skip the loop
            _ => self.stack[top].pc = end as usize,
        }
        Ok(())
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
            _ => false,
        };
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
    fn fetch_class_const(
        &mut self,
        top: usize,
        proto: &FunctionProto,
        dst: Reg,
        class: ClassRef,
        name: NIdx,
    ) -> Result<(), Thrown> {
        let name_b = proto.names[name as usize].as_bytes().to_vec();
        let cls = self.resolve_class_autoload(top, class)?;
        if name_b == b"class" {
            let v = Value::str(self.class(cls).name.as_bytes().to_vec());
            self.reg_set(top, dst, v);
        } else {
            // php loads every constant of a backed enum on first
            // fetch (it builds the value -> case table).
            // (Not from inside a constant expression, which goes
            // through zend_get_class_constant_ex in php.)
            let in_const_expr = proto.name.as_bytes() == b"{init}";
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
                    if !self.check_access(self.stack[top].defining_class, decl, vis) {
                        return Err(self.access_err("constant", decl, &name_b, vis));
                    }
                    let v = if c.init.is_none() {
                        c.value.clone()
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
        let key = builtins::const_key(cname);
        let v = self.constants.get(&*key).cloned().or_else(|| {
            if fallback != rhp_bytecode::NO_NAME {
                self.constants
                    .get(proto.names[fallback as usize].as_bytes())
                    .cloned()
            } else {
                None
            }
        });
        match v {
            Some(v) => {
                let bare = cname.rsplit(|&c| c == b'\\').next().unwrap_or(cname);
                let deprecation = match bare {
                    b"E_STRICT" => Some(
                        "Constant E_STRICT is deprecated since 8.4, the error level was removed"
                            .to_string(),
                    ),
                    b"MT_RAND_PHP" => Some(
                        "Constant MT_RAND_PHP is deprecated since 8.3, as it uses a biased non-standard variant of Mt19937"
                            .to_string(),
                    ),
                    _ => builtins::constant_deprecation(bare),
                };
                if let Some(msg) = deprecation {
                    if self.constants.contains_key(bare) {
                        self.diag(builtins::E_DEPRECATED, &msg)?;
                    }
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

/// Describe the unmatched subject for an UnhandledMatchError message.
fn match_case_desc(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Str(_) => "of type string".to_string(),
        Value::Float(_) => "of type double".to_string(),
        Value::Null => "NULL".to_string(),
        other => format!("of type {}", other.type_name()),
    }
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
