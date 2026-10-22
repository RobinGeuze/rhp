//! The register bytecode VM.
//!
//! Executes a `phrust_bytecode::Program`. Every arithmetic/compare/concat/cast
//! opcode is a thin wrapper over the verified `phrust_semantics::ops` / value
//! `conv` functions, and calls go through `phrust_builtins` — so the VM and the
//! tree-walking oracle are semantically identical by construction, and any
//! differential mismatch localizes to a compiler-lowering bug (spec §2.3).
//!
//! Phase-1 scope: a single `{main}` frame, builtin calls, no user functions /
//! exceptions yet (the explicit call stack + exception table are in place for
//! those phases).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use phrust_bytecode::{
    ClassRef, CmpKind, Default as ParamDefault, FunctionProto, Instr, Program, Reg,
};
use phrust_builtins::{self as builtins, Host};
use phrust_semantics::{cast_to_array, inc_dec_value, ops};
use phrust_values::{conv, ArrayKey, ClassId, ClassKind, ObjectData, ObjectHandle, PhpArray, Thrown, Value};

/// A pending lvalue step (an evaluated key, an append, or an object property).
enum CursorStep {
    Key(Value),
    Append,
    Prop(phrust_values::PhpStr),
}

/// The virtual machine: output sink + call stack + global tables.
pub struct Vm {
    out: Vec<u8>,
    stack: Vec<Frame>,
    functions: Vec<Rc<FunctionProto>>,
    fn_names: HashMap<Vec<u8>, usize>, // lowercase name -> index into `functions`
    constants: HashMap<Vec<u8>, Value>,
    max_depth: usize,
    // lvalue cursor: a root in the current frame (CV or a static-prop cell) +
    // pending path steps. Cursor opcodes are emitted contiguously, so a single
    // cursor never nests.
    cursor_root: CursorRoot,
    cursor_steps: Vec<CursorStep>,
    // OOP class table (see `classes` module).
    classes: Vec<classes::ClassEntry>,
    class_ids: HashMap<Vec<u8>, ClassId>, // lowercase name -> id
    next_object_id: u32,
}

mod classes;

/// Where an lvalue cursor is rooted.
enum CursorRoot {
    /// A register (compiled variable) in the current frame.
    Cv(Reg),
    /// A shared static-property cell.
    Cell(Rc<RefCell<Value>>),
}

impl Host for Vm {
    fn write_out(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    fn class_name(&self, class_id: u32) -> Option<Vec<u8>> {
        self.classes.get(class_id as usize).map(|c| c.name.as_bytes().to_vec())
    }

    fn class_parent_name(&self, class_id: u32) -> Option<Vec<u8>> {
        let parent = self.classes.get(class_id as usize)?.parent?;
        Some(self.classes[parent as usize].name.as_bytes().to_vec())
    }

    fn call_callable(&mut self, callable: &Value, args: &[Value]) -> Result<Value, Thrown> {
        self.call_callable_value(callable, args.to_vec())
    }
}

/// One activation record.
struct Frame {
    proto: Rc<FunctionProto>,
    regs: Vec<Value>,
    pc: usize,
    /// Caller register that receives this frame's return value (unused for main).
    ret_dst: Reg,
    /// The class whose body this is (`self::`/`parent::`/`__CLASS__`); None outside methods.
    defining_class: Option<ClassId>,
    /// The late-static-binding class (`static::`); None outside methods.
    called_class: Option<ClassId>,
    /// What a `finally` block must do when it finishes (set by the unwinder).
    pending: Option<FinallyCont>,
    /// A synchronous (re-entrant) call: when this frame returns, `do_return`
    /// hands the value back to the Rust caller instead of delivering to a reg.
    capture: bool,
}

/// Result of unwinding: a handler was installed, or the exception escaped the
/// current (sub)run and should propagate to the Rust caller.
enum UnwindResult {
    Caught,
    Escaped(Thrown),
}

/// What to do after a `finally` block completes.
enum FinallyCont {
    /// An exception was in flight (no catch matched); re-raise after finally.
    Rethrow(Thrown),
    /// A `return` crossed out of the try/catch; complete it after finally.
    Return(Value),
}

impl Frame {
    fn new(proto: Rc<FunctionProto>, ret_dst: Reg) -> Self {
        let regs = vec![Value::Null; proto.num_registers as usize];
        Frame {
            proto,
            regs,
            pc: 0,
            ret_dst,
            defining_class: None,
            called_class: None,
            pending: None,
            capture: false,
        }
    }
}

impl Default for Vm {
    fn default() -> Self {
        Vm::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        let mut constants = HashMap::new();
        for (name, value) in builtins::constants() {
            constants.insert(name.as_bytes().to_vec(), value);
        }
        Vm {
            out: Vec::new(),
            stack: Vec::new(),
            functions: Vec::new(),
            fn_names: HashMap::new(),
            constants,
            max_depth: 1 << 16,
            cursor_root: CursorRoot::Cv(0),
            cursor_steps: Vec::new(),
            classes: Vec::new(),
            class_ids: HashMap::new(),
            next_object_id: 1,
        }
    }

    pub fn output(&self) -> &[u8] {
        &self.out
    }

    /// Execute a compiled program; returns the process exit code.
    pub fn run(&mut self, prog: &Program) -> Result<i32, Thrown> {
        // Install (hoist) all declared functions, keyed case-insensitively.
        for (idx, proto) in prog.functions.iter().enumerate() {
            self.functions.push(proto.clone());
            self.fn_names.insert(proto.name.as_bytes().to_ascii_lowercase(), idx);
        }
        // Link all declared classes (flatten inheritance, resolve forward refs).
        self.install_classes(&prog.classes)?;
        self.stack.push(Frame::new(prog.main.clone(), 0));
        match self.run_loop() {
            Ok(_) => Ok(0),
            Err(Thrown::Exit(code)) => Ok(code),
            Err(other) => Err(other),
        }
    }

    // -- register access: deref on read, write-through on a Ref slot --

    fn reg_get(&self, top: usize, r: Reg) -> Value {
        match &self.stack[top].regs[r as usize] {
            Value::Ref(rc) => rc.borrow().clone(),
            v => v.clone(),
        }
    }

    /// Plain register write (temporaries / instruction destinations): overwrites
    /// the slot. Never writes through a stale `Ref` left in a reused temp.
    fn reg_set(&mut self, top: usize, r: Reg, v: Value) {
        self.stack[top].regs[r as usize] = v;
    }

    /// Assign to a PHP *variable* (a CV): if the slot is a reference, write
    /// through it (so by-ref aliases see the change); otherwise overwrite.
    fn cv_assign(&mut self, top: usize, cv: Reg, v: Value) {
        let slot = &mut self.stack[top].regs[cv as usize];
        if let Value::Ref(rc) = slot {
            *rc.borrow_mut() = v;
        } else {
            *slot = v;
        }
    }

    /// Read the value at the current cursor (root + steps), non-mutating.
    fn cursor_read(&self, top: usize) -> Value {
        self.cursor_read_steps(top, &self.cursor_steps)
    }

    fn cursor_read_steps(&self, top: usize, steps: &[CursorStep]) -> Value {
        match &self.cursor_root {
            CursorRoot::Cv(r) => read_path(&self.stack[top].regs[*r as usize], steps),
            CursorRoot::Cell(rc) => read_path(&rc.borrow(), steps),
        }
    }

    /// Write `v` at the current cursor (root + steps).
    fn cursor_write(&mut self, top: usize, steps: &[CursorStep], v: Value) -> Result<(), Thrown> {
        match &self.cursor_root {
            CursorRoot::Cv(r) => {
                let r = *r as usize;
                assign_path(&mut self.stack[top].regs[r], steps, v)
            }
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                let r = assign_path(&mut rc.borrow_mut(), steps, v);
                r
            }
        }
    }

    /// The `$this` of the current frame, if it is bound to an object.
    fn current_this(&self, top: usize) -> Option<ObjectHandle> {
        let cv = self.stack[top].proto.this_cv?;
        match &self.stack[top].regs[cv as usize] {
            Value::Object(h) => Some(h.clone()),
            _ => None,
        }
    }

    fn cursor_unset(&mut self, top: usize, steps: &[CursorStep]) {
        match &self.cursor_root {
            CursorRoot::Cv(r) => unset_path(&mut self.stack[top].regs[*r as usize], steps),
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                unset_path(&mut rc.borrow_mut(), steps);
            }
        }
    }

    /// Promote a CV slot to a shared reference cell, returning the cell. If the
    /// slot already holds a `Ref`, its cell is reused.
    fn promote(&mut self, top: usize, cv: Reg) -> Rc<RefCell<Value>> {
        let slot = &mut self.stack[top].regs[cv as usize];
        match slot {
            Value::Ref(rc) => rc.clone(),
            other => {
                let v = std::mem::replace(other, Value::Null);
                let rc = Rc::new(RefCell::new(v));
                *other = Value::Ref(rc.clone());
                rc
            }
        }
    }

    /// Drive execution, catching thrown exceptions and routing them through the
    /// exception table (per-frame handlers / finally), unwinding frames until a
    /// handler is found or the program's top frame is reached.
    fn run_loop(&mut self) -> Result<Value, Thrown> {
        loop {
            match self.run_frames() {
                Ok(v) => return Ok(v),
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        self.stack.clear();
                        return Err(thrown); // Exit / Unsupported: straight to the SAPI
                    }
                    match self.unwind(thrown, 0)? {
                        UnwindResult::Caught => {} // handler installed; re-enter
                        UnwindResult::Escaped(t) => return Err(t), // uncaught
                    }
                }
            }
        }
    }

    fn run_frames(&mut self) -> Result<Value, Thrown> {
        loop {
            let top = self.stack.len() - 1;
            let proto = self.stack[top].proto.clone();
            let pc = self.stack[top].pc;
            if pc >= proto.code.len() {
                // fell off the end == RetNull
                if let Some(done) = self.do_return(Value::Null) {
                    return Ok(done);
                }
                continue;
            }
            let instr = proto.code[pc];
            self.stack[top].pc = pc + 1;

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
                Instr::LoadNull { dst } => self.reg_set(top, dst, Value::Null),
                Instr::LoadBool { dst, b } => self.reg_set(top, dst, Value::Bool(b)),
                Instr::LoadInt { dst, n } => self.reg_set(top, dst, Value::Int(n)),

                Instr::Assign { cv, src } => {
                    let v = self.reg_get(top, src);
                    self.cv_assign(top, cv, v);
                }

                Instr::Add { dst, a, b } => self.bin(top, dst, a, b, ops::add)?,
                Instr::Sub { dst, a, b } => self.bin(top, dst, a, b, ops::sub)?,
                Instr::Mul { dst, a, b } => self.bin(top, dst, a, b, ops::mul)?,
                Instr::Div { dst, a, b } => self.bin(top, dst, a, b, ops::div)?,
                Instr::Mod { dst, a, b } => self.bin(top, dst, a, b, ops::modulo)?,
                Instr::Pow { dst, a, b } => self.bin(top, dst, a, b, ops::pow)?,
                Instr::BitAnd { dst, a, b } => self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::And)),
                Instr::BitOr { dst, a, b } => self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Or)),
                Instr::BitXor { dst, a, b } => self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Xor)),
                Instr::Shl { dst, a, b } => self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Shl)),
                Instr::Shr { dst, a, b } => self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Shr)),
                Instr::Concat { dst, a, b } => {
                    let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
                    let v = if matches!(x, Value::Object(_)) || matches!(y, Value::Object(_)) {
                        let mut bytes = self.to_string_vm(&x)?;
                        bytes.extend_from_slice(&self.to_string_vm(&y)?);
                        Value::str(bytes)
                    } else {
                        ops::concat(&x, &y)
                    };
                    self.reg_set(top, dst, v);
                }

                Instr::Neg { dst, a } => {
                    let v = self.reg_get(top, a);
                    let r = ops::negate(&v)?;
                    self.reg_set(top, dst, r);
                }
                Instr::BitNot { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, ops::bitwise_not(&v));
                }
                Instr::Not { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, Value::Bool(!conv::to_bool(&v)));
                }
                Instr::IncDec { dst, cv, pre, delta } => {
                    let cur = self.reg_get(top, cv);
                    let new = inc_dec_value(&cur, delta as i64);
                    self.cv_assign(top, cv, new.clone());
                    self.reg_set(top, dst, if pre { new } else { cur });
                }

                Instr::CastInt { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, Value::Int(conv::to_int(&v)));
                }
                Instr::CastFloat { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, Value::Float(conv::to_float(&v)));
                }
                Instr::CastString { dst, a } => {
                    let v = self.reg_get(top, a);
                    let s = self.to_string_vm(&v)?;
                    self.reg_set(top, dst, Value::str(s));
                }
                Instr::CastBool { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, Value::Bool(conv::to_bool(&v)));
                }
                Instr::CastArray { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, cast_to_array(v));
                }

                Instr::Cmp { dst, a, b, kind } => {
                    let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
                    self.reg_set(top, dst, eval_cmp(kind, &x, &y));
                }

                Instr::Echo { src } => {
                    let v = self.reg_get(top, src);
                    let bytes = self.to_string_vm(&v)?;
                    self.write_out(&bytes);
                }
                Instr::EchoConst { k } => {
                    let bytes = conv::to_string_bytes(&proto.consts[k as usize]);
                    self.write_out(&bytes);
                }
                Instr::Print { dst, src } => {
                    let v = self.reg_get(top, src);
                    let bytes = self.to_string_vm(&v)?;
                    self.write_out(&bytes);
                    self.reg_set(top, dst, Value::Int(1));
                }

                Instr::Jmp { target } => self.stack[top].pc = target as usize,
                Instr::JmpTrue { cond, target } => {
                    if conv::to_bool(&self.reg_get(top, cond)) {
                        self.stack[top].pc = target as usize;
                    }
                }
                Instr::JmpFalse { cond, target } => {
                    if !conv::to_bool(&self.reg_get(top, cond)) {
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
                    self.reg_set(top, dst, Value::Array(phrust_values::PhpArray::new()));
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
                Instr::ArrSpread { arr, src } => {
                    let spread = self.reg_get(top, src);
                    if let Value::Array(inner) = spread {
                        if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                            for (k, iv) in inner.iter() {
                                match k {
                                    ArrayKey::Int(_) => a.push(iv.clone()),
                                    ArrayKey::Str(_) => a.insert(k.clone(), iv.clone()),
                                }
                            }
                        }
                    }
                }

                Instr::Call { dst, name, base, argc } => {
                    let fname = proto.names[name as usize].as_bytes().to_ascii_lowercase();
                    if let Some(reason) = builtins::is_refused(
                        std::str::from_utf8(&fname).unwrap_or(""),
                    ) {
                        let shown = String::from_utf8_lossy(&fname).into_owned();
                        return Err(Thrown::unsupported(format!("{shown}()"), reason));
                    }
                    if let Some(&fnidx) = self.fn_names.get(&fname) {
                        // User function: push a frame (args bound to its CVs).
                        let callee = self.functions[fnidx].clone();
                        self.enter_user_call(top, callee, dst, base, argc)?;
                    } else {
                        // Builtin.
                        let name_str = String::from_utf8_lossy(&fname).into_owned();
                        let out = self.call_builtin(top, &name_str, base, argc)?;
                        self.reg_set(top, dst, out);
                    }
                }

                // -- array element read --
                Instr::DimFetch { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
                    self.reg_set(top, dst, dim_fetch(&a, &k));
                }

                // -- lvalue cursor --
                Instr::LvalCv { cv } => {
                    self.cursor_root = CursorRoot::Cv(cv);
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
                Instr::LvalStaticProp { class, name } => {
                    let cls = self.resolve_class(top, class)?;
                    let pname = proto.names[name as usize].as_bytes().to_vec();
                    let cell = self.static_prop_cell(cls, &pname)?;
                    self.cursor_root = CursorRoot::Cell(cell);
                    self.cursor_steps.clear();
                }
                Instr::FetchLvalRw { dst } => {
                    let v = self.cursor_read(top);
                    self.reg_set(top, dst, v); // cursor stays open for the AssignLval
                }
                Instr::AssignLval { src } => {
                    let v = self.reg_get(top, src);
                    let steps = std::mem::take(&mut self.cursor_steps);
                    self.cursor_write(top, &steps, v)?;
                }
                Instr::UnsetLval => {
                    let steps = std::mem::take(&mut self.cursor_steps);
                    self.cursor_unset(top, &steps);
                }
                Instr::IncDecLval { dst, pre, delta } => {
                    let steps = std::mem::take(&mut self.cursor_steps);
                    let old = self.cursor_read_steps(top, &steps);
                    let new = inc_dec_value(&old, delta as i64);
                    self.cursor_write(top, &steps, new.clone())?;
                    self.reg_set(top, dst, if pre { new } else { old });
                }

                // -- foreach --
                Instr::FeReset { arr_reg, pos_reg, src, end } => {
                    match self.reg_get(top, src) {
                        Value::Array(a) if !a.is_empty() => {
                            self.reg_set(top, arr_reg, Value::Array(a));
                            self.reg_set(top, pos_reg, Value::Int(0));
                        }
                        // empty array or non-array: skip the loop
                        _ => self.stack[top].pc = end as usize,
                    }
                }
                Instr::FeFetch { arr_reg, pos_reg, key_cv, val_cv, end } => {
                    let pos = match self.reg_get(top, pos_reg) {
                        Value::Int(n) => n as usize,
                        _ => 0,
                    };
                    let snapshot = self.reg_get(top, arr_reg);
                    let Value::Array(a) = snapshot else {
                        self.stack[top].pc = end as usize;
                        continue;
                    };
                    match a.get_index(pos) {
                        Some((k, v)) => {
                            let (k, v) = (k.clone(), v.clone());
                            self.reg_set(top, val_cv, v);
                            if key_cv != Reg::MAX {
                                self.reg_set(top, key_cv, array_key_to_value(&k));
                            }
                            self.reg_set(top, pos_reg, Value::Int((pos + 1) as i64));
                        }
                        None => self.stack[top].pc = end as usize,
                    }
                }

                Instr::ThrowMatchError { subject } => {
                    let v = self.reg_get(top, subject);
                    return Err(Thrown::error(
                        "UnhandledMatchError",
                        format!("Unhandled match case {}", match_case_desc(&v)),
                    ));
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
                Instr::FinallyEnd => match self.stack[top].pending.take() {
                    Some(FinallyCont::Rethrow(t)) => return Err(t),
                    Some(FinallyCont::Return(v)) => {
                        if let Some(done) = self.do_return(v) {
                            return Ok(done);
                        }
                    }
                    None => {}
                },
                Instr::SetPendingRet { src } => {
                    let v = self.reg_get(top, src);
                    self.stack[top].pending = Some(FinallyCont::Return(v));
                }

                // -- OOP --
                Instr::New { dst, class, base, argc, .. } => {
                    let cls = self.resolve_class(top, class)?;
                    let entry = self.class(cls);
                    if entry.kind != ClassKind::Class || entry.flags.is_abstract {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Cannot instantiate {} {}",
                                if entry.flags.is_abstract { "abstract class" } else { "interface/trait" },
                                String::from_utf8_lossy(entry.name.as_bytes())
                            ),
                        ));
                    }
                    let obj = self.new_object(cls);
                    self.reg_set(top, dst, Value::Object(obj.clone()));
                    // Run __construct, discarding its return (into the arg window).
                    if let Some(m) = self.lookup_method(cls, b"__construct") {
                        self.enter_method(top, &m, Some(obj), cls, Reg::MAX, base, argc)?;
                    }
                }
                Instr::FetchProp { dst, obj, name, .. } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let v = match o {
                        Value::Object(h) => {
                            let cls = h.borrow().class;
                            if let Some(pe) = self.class(cls).props.get(&pname) {
                                let (vis, decl) = (pe.visibility, pe.declaring_class);
                                if !self.check_access(self.stack[top].defining_class, decl, vis) {
                                    return Err(self.access_err("property", decl, pname.as_bytes(), vis));
                                }
                            }
                            match h.borrow().properties.get(&pname).cloned() {
                                Some(v) => v,
                                // Undefined property: __get fallback, else Null.
                                None => match self.lookup_method(cls, b"__get") {
                                    Some(getter) => {
                                        let arg = Value::str(pname.as_bytes().to_vec());
                                        self.call_sync(&getter, Some(h.clone()), cls, vec![arg])?
                                    }
                                    None => Value::Null,
                                },
                            }
                        }
                        _ => Value::Null, // php warns "property on <type>"
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::FetchStaticProp { dst, class, name, .. } => {
                    let cls = self.resolve_class(top, class)?;
                    let pname = proto.names[name as usize].as_bytes().to_vec();
                    if let Some(sp) = self.class(cls).static_props.get(&pname) {
                        let (vis, decl) = (sp.visibility, sp.declaring_class);
                        if !self.check_access(self.stack[top].defining_class, decl, vis) {
                            return Err(self.access_err("property", decl, &pname, vis));
                        }
                    }
                    let cell = self.static_prop_cell(cls, &pname)?;
                    let v = cell.borrow().clone();
                    self.reg_set(top, dst, v);
                }
                Instr::FetchClassConst { dst, class, name, .. } => {
                    let name_b = proto.names[name as usize].as_bytes().to_vec();
                    let cls = self.resolve_class(top, class)?;
                    if name_b == b"class" {
                        let v = Value::str(self.class(cls).name.as_bytes().to_vec());
                        self.reg_set(top, dst, v);
                    } else {
                        match self.class(cls).constants.get(&name_b) {
                            Some(c) => {
                                let (vis, decl) = (c.visibility, c.declaring_class);
                                if !self.check_access(self.stack[top].defining_class, decl, vis) {
                                    return Err(self.access_err("constant", decl, &name_b, vis));
                                }
                                let v = c.value.clone();
                                self.reg_set(top, dst, v);
                            }
                            None => {
                                return Err(Thrown::error(
                                    "Error",
                                    format!(
                                        "Undefined constant {}::{}",
                                        String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                        String::from_utf8_lossy(&name_b)
                                    ),
                                ))
                            }
                        }
                    }
                }
                Instr::CallMethod { dst, obj, name, base, argc, .. } => {
                    let o = self.reg_get(top, obj);
                    let Value::Object(h) = o else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to a member function {}() on {}",
                                String::from_utf8_lossy(proto.names[name as usize].as_bytes()),
                                type_lc(&o)
                            ),
                        ));
                    };
                    let cls = h.borrow().class;
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    let found = self.lookup_method(cls, &mname);
                    let accessible = found.as_ref().is_some_and(|m| {
                        self.check_access(self.stack[top].defining_class, m.defining_class, m.visibility)
                    });
                    if let (Some(m), true) = (&found, accessible) {
                        let m = m.clone();
                        self.enter_method(top, &m, Some(h.clone()), cls, dst, base, argc)?;
                    } else if let Some(magic) = self.lookup_method(cls, b"__call") {
                        // Undefined or inaccessible method -> __call($name, $args).
                        let mut arr = phrust_values::PhpArray::new();
                        for i in 0..argc {
                            arr.push(self.reg_get(top, base + i));
                        }
                        let args = vec![Value::str(mname.clone()), Value::Array(arr)];
                        let r = self.call_sync(&magic, Some(h.clone()), cls, args)?;
                        self.reg_set(top, dst, r);
                    } else if let Some(m) = found {
                        return Err(self.access_err("method", m.defining_class, &mname, m.visibility));
                    } else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to undefined method {}::{}()",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                String::from_utf8_lossy(&mname)
                            ),
                        ));
                    }
                }
                Instr::CallStatic { dst, class, name, base, argc, .. } => {
                    let cls = self.resolve_class(top, class)?;
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    let found = self.lookup_method(cls, &mname);
                    let accessible = found.as_ref().is_some_and(|m| {
                        self.check_access(self.stack[top].defining_class, m.defining_class, m.visibility)
                    });
                    if !(found.is_some() && accessible) {
                        // Undefined / inaccessible static method -> __callStatic.
                        if let Some(magic) = self.lookup_method(cls, b"__callstatic") {
                            let mut arr = phrust_values::PhpArray::new();
                            for i in 0..argc {
                                arr.push(self.reg_get(top, base + i));
                            }
                            let args = vec![Value::str(mname.clone()), Value::Array(arr)];
                            let r = self.call_sync(&magic, None, cls, args)?;
                            self.reg_set(top, dst, r);
                            continue;
                        }
                        return match found {
                            Some(m) => {
                                Err(self.access_err("method", m.defining_class, &mname, m.visibility))
                            }
                            None => Err(Thrown::error(
                                "Error",
                                format!(
                                    "Call to undefined method {}::{}()",
                                    String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                    String::from_utf8_lossy(&mname)
                                ),
                            )),
                        };
                    }
                    let m = found.unwrap();
                    // LSB: self/parent/static forward the called-class; Name/Dynamic set it.
                    let called = match class {
                        ClassRef::SelfClass | ClassRef::ParentClass | ClassRef::StaticClass => {
                            self.stack[top].called_class.unwrap_or(cls)
                        }
                        _ => cls,
                    };
                    // Forward $this to a non-static method iff it is-a the target class.
                    let this = if m.is_static {
                        None
                    } else {
                        self.current_this(top)
                            .filter(|h| self.class(h.borrow().class).ancestors.contains(&cls))
                    };
                    self.enter_method(top, &m, this, called, dst, base, argc)?;
                }
                Instr::InstanceOf { dst, val, class } => {
                    let v = self.reg_get(top, val);
                    let result = match v {
                        Value::Object(h) => match self.resolve_class(top, class) {
                            Ok(cid) => self.class(h.borrow().class).ancestors.contains(&cid),
                            Err(_) => false,
                        },
                        _ => false,
                    };
                    self.reg_set(top, dst, Value::Bool(result));
                }
                Instr::CloneObj { dst, src } => {
                    let v = self.reg_get(top, src);
                    let Value::Object(h) = v else {
                        return Err(Thrown::error("Error", "__clone method called on non-object"));
                    };
                    let (cls, props) = {
                        let od = h.borrow();
                        (od.class, od.properties.clone())
                    };
                    let id = self.next_object_id;
                    self.next_object_id += 1;
                    let new_obj: ObjectHandle =
                        Rc::new(RefCell::new(ObjectData { class: cls, id, properties: props }));
                    self.reg_set(top, dst, Value::Object(new_obj.clone()));
                    if let Some(m) = self.lookup_method(cls, b"__clone") {
                        self.enter_method(top, &m, Some(new_obj), cls, Reg::MAX, src, 0)?;
                    }
                }
                Instr::DeclareClass { .. } => {
                    return Err(Thrown::unsupported(
                        "conditional class declaration",
                        "only top-level class declarations for now",
                    ))
                }

                Instr::BindRef { dst_cv, src_cv } => {
                    let rc = self.promote(top, src_cv);
                    self.stack[top].regs[dst_cv as usize] = Value::Ref(rc);
                }
                Instr::MakeRef { dst, cv } => {
                    let rc = self.promote(top, cv);
                    self.stack[top].regs[dst as usize] = Value::Ref(rc);
                }

                Instr::CallV { dst, name, args } => {
                    let fname = proto.names[name as usize].as_bytes().to_ascii_lowercase();
                    if let Some(reason) =
                        builtins::is_refused(std::str::from_utf8(&fname).unwrap_or(""))
                    {
                        let shown = String::from_utf8_lossy(&fname).into_owned();
                        return Err(Thrown::unsupported(format!("{shown}()"), reason));
                    }
                    let arr = self.reg_get(top, args);
                    let argv = self.spread_args(&arr);
                    if let Some(&fnidx) = self.fn_names.get(&fname) {
                        let callee = self.functions[fnidx].clone();
                        self.push_user_frame(callee, argv, dst)?;
                    } else {
                        let name_str = String::from_utf8_lossy(&fname).into_owned();
                        let out = self.invoke_builtin(&name_str, argv)?;
                        self.reg_set(top, dst, out);
                    }
                }
                Instr::NewV { dst, class, args } => {
                    let cls = self.resolve_class(top, class)?;
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
                    let obj = self.new_object(cls);
                    self.reg_set(top, dst, Value::Object(obj.clone()));
                    if let Some(m) = self.lookup_method(cls, b"__construct") {
                        let arr = self.reg_get(top, args);
                        let argv = self.spread_args(&arr);
                        self.push_method_frame(&m, Some(obj), cls, argv, Reg::MAX)?;
                    }
                }
                Instr::CallMethodV { dst, obj, name, args } => {
                    let o = self.reg_get(top, obj);
                    let Value::Object(h) = o else {
                        return Err(Thrown::type_error("Call to a member function on non-object"));
                    };
                    let cls = h.borrow().class;
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    let Some(m) = self.lookup_method(cls, &mname) else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to undefined method {}::{}()",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                String::from_utf8_lossy(&mname)
                            ),
                        ));
                    };
                    if !self.check_access(self.stack[top].defining_class, m.defining_class, m.visibility) {
                        return Err(self.access_err("method", m.defining_class, &mname, m.visibility));
                    }
                    let arr = self.reg_get(top, args);
                    let argv = self.spread_args(&arr);
                    self.push_method_frame(&m, Some(h.clone()), cls, argv, dst)?;
                }
                Instr::CallStaticV { dst, class, name, args } => {
                    let cls = self.resolve_class(top, class)?;
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    let Some(m) = self.lookup_method(cls, &mname) else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to undefined method {}::{}()",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                String::from_utf8_lossy(&mname)
                            ),
                        ));
                    };
                    let called = match class {
                        ClassRef::SelfClass | ClassRef::ParentClass | ClassRef::StaticClass => {
                            self.stack[top].called_class.unwrap_or(cls)
                        }
                        _ => cls,
                    };
                    let this = if m.is_static {
                        None
                    } else {
                        self.current_this(top)
                            .filter(|h| self.class(h.borrow().class).ancestors.contains(&cls))
                    };
                    let arr = self.reg_get(top, args);
                    let argv = self.spread_args(&arr);
                    self.push_method_frame(&m, this, called, argv, dst)?;
                }

                Instr::FetchConst { dst, name } => {
                    let cname = proto.names[name as usize].as_bytes().to_vec();
                    match self.constants.get(&cname) {
                        Some(v) => {
                            let v = v.clone();
                            self.reg_set(top, dst, v);
                        }
                        None => {
                            return Err(Thrown::error(
                                "Error",
                                format!("Undefined constant \"{}\"", String::from_utf8_lossy(&cname)),
                            ))
                        }
                    }
                }

                Instr::Refuse { feature, .. } => {
                    let f = proto.names[feature as usize].as_bytes().to_vec();
                    return Err(Thrown::unsupported(
                        String::from_utf8_lossy(&f).into_owned(),
                        "refused construct",
                    ));
                }

                Instr::Ret { src } => {
                    let v = self.reg_get(top, src);
                    if let Some(done) = self.do_return(v) {
                        return Ok(done);
                    }
                }
                Instr::RetNull | Instr::Halt => {
                    if let Some(done) = self.do_return(Value::Null) {
                        return Ok(done);
                    }
                }
            }
        }
    }

    /// Unwind a thrown exception through the exception table: find a matching
    /// catch (bind + jump) or finally (re-raise after) in the current frame,
    /// else pop and continue in the caller. Returns `Ok` when a handler is
    /// installed (re-enter `run_frames`), `Err` when it escapes the top frame.
    fn unwind(&mut self, thrown: Thrown, floor: usize) -> Result<UnwindResult, Thrown> {
        loop {
            if self.stack.len() <= floor {
                return Ok(UnwindResult::Escaped(thrown)); // escaped the (sub)run
            }
            let top = self.stack.len() - 1;
            let proto = self.stack[top].proto.clone();
            // The throwing pc has already been advanced past the faulting instr.
            let throw_pc = (self.stack[top].pc as u32).saturating_sub(1);
            if let Some(region) =
                proto.exception_table.iter().find(|r| throw_pc >= r.start && throw_pc < r.end)
            {
                let region = region.clone();
                if let Some(cls) = self.thrown_class_id(&thrown) {
                    for arm in &region.catches {
                        if arm.classes.iter().any(|cn| self.class_matches(cls, cn.as_bytes())) {
                            let obj = self.thrown_to_object(&thrown)?;
                            if let Some(vr) = arm.var_reg {
                                self.stack[top].regs[vr as usize] = obj;
                            }
                            self.stack[top].pc = arm.target as usize;
                            return Ok(UnwindResult::Caught);
                        }
                    }
                }
                if let Some(fin) = region.finally {
                    self.stack[top].pending = Some(FinallyCont::Rethrow(thrown));
                    self.stack[top].pc = fin as usize;
                    return Ok(UnwindResult::Caught);
                }
            }
            self.stack.pop();
        }
    }

    /// Synchronously call a method and return its value (re-entrant: used by
    /// magic methods like `__toString`/`__get` mid-opcode). Exceptions thrown
    /// inside are handled within the sub-run; if uncaught there, they propagate.
    fn call_sync(
        &mut self,
        method: &classes::ResolvedMethod,
        this: Option<ObjectHandle>,
        called_class: ClassId,
        args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        let proto = method.proto.clone();
        self.run_proto_sync(proto, this, Some(method.defining_class), Some(called_class), args)
    }

    /// Push a capture frame for `proto` and drive it to completion, returning
    /// its value (re-entrant; exceptions handled within the sub-run).
    fn run_proto_sync(
        &mut self,
        proto: Rc<FunctionProto>,
        this: Option<ObjectHandle>,
        defining_class: Option<ClassId>,
        called_class: Option<ClassId>,
        args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let mut frame = Frame::new(proto.clone(), 0);
        bind_params(&proto, &args, &mut frame.regs)?;
        if let (Some(cv), Some(obj)) = (proto.this_cv, this) {
            frame.regs[cv as usize] = Value::Object(obj);
        }
        frame.defining_class = defining_class;
        frame.called_class = called_class;
        frame.capture = true;
        let base = self.stack.len();
        self.stack.push(frame);
        loop {
            match self.run_frames() {
                Ok(v) => return Ok(v),
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        return Err(thrown);
                    }
                    match self.unwind(thrown, base)? {
                        UnwindResult::Caught => {}
                        UnwindResult::Escaped(t) => return Err(t),
                    }
                }
            }
        }
    }

    /// Invoke a PHP callable value synchronously (string function name,
    /// `[obj_or_class, method]`, or an `__invoke` object).
    fn call_callable_value(&mut self, callable: &Value, args: Vec<Value>) -> Result<Value, Thrown> {
        let c = match callable {
            Value::Ref(r) => r.borrow().clone(),
            other => other.clone(),
        };
        match c {
            Value::Str(s) => {
                let lower = s.as_bytes().to_ascii_lowercase();
                if let Some(&fnidx) = self.fn_names.get(&lower) {
                    let proto = self.functions[fnidx].clone();
                    self.run_proto_sync(proto, None, None, None, args)
                } else {
                    let name = String::from_utf8_lossy(s.as_bytes()).into_owned();
                    self.invoke_builtin(&name, args)
                }
            }
            Value::Array(a) if a.len() == 2 => {
                let item0 = a.get(&ArrayKey::Int(0)).cloned().unwrap_or(Value::Null);
                let method = a.get(&ArrayKey::Int(1)).cloned().unwrap_or(Value::Null);
                let mname = conv::to_string_bytes(&method);
                match item0 {
                    Value::Object(h) => {
                        let cls = h.borrow().class;
                        let m = self.lookup_method(cls, &mname).ok_or_else(|| {
                            Thrown::error("Error", "Call to undefined method in callable")
                        })?;
                        self.call_sync(&m, Some(h), cls, args)
                    }
                    Value::Str(cn) => {
                        let cls = self.resolve_class_by_name(cn.as_bytes())?;
                        let m = self.lookup_method(cls, &mname).ok_or_else(|| {
                            Thrown::error("Error", "Call to undefined method in callable")
                        })?;
                        self.call_sync(&m, None, cls, args)
                    }
                    _ => Err(Thrown::type_error("Invalid callable array")),
                }
            }
            Value::Object(h) => {
                let cls = h.borrow().class;
                match self.lookup_method(cls, b"__invoke") {
                    Some(m) => self.call_sync(&m, Some(h), cls, args),
                    None => Err(Thrown::type_error("Object is not callable (no __invoke)")),
                }
            }
            _ => Err(Thrown::type_error("Value is not callable")),
        }
    }

    /// Stringify a value, invoking `__toString` on objects that define it.
    fn to_string_vm(&mut self, v: &Value) -> Result<Vec<u8>, Thrown> {
        let d = match v {
            Value::Ref(r) => r.borrow().clone(),
            other => other.clone(),
        };
        if let Value::Object(h) = &d {
            let cls = h.borrow().class;
            if let Some(m) = self.lookup_method(cls, b"__tostring") {
                let r = self.call_sync(&m, Some(h.clone()), cls, Vec::new())?;
                return Ok(conv::to_string_bytes(&r));
            }
            return Err(Thrown::error(
                "Error",
                format!(
                    "Object of class {} could not be converted to string",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes())
                ),
            ));
        }
        Ok(conv::to_string_bytes(&d))
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
                let obj = self.new_object(cls);
                obj.borrow_mut()
                    .properties
                    .insert(phrust_values::PhpStr::from("message"), Value::str(message.clone()));
                Ok(Value::Object(obj))
            }
            _ => Err(thrown.clone()),
        }
    }

    /// Pop the current frame and deliver `value`. Returns `Some(value)` when the
    /// outermost frame returns (program done), else delivers to the caller.
    fn do_return(&mut self, value: Value) -> Option<Value> {
        let frame = self.stack.pop().expect("non-empty stack");
        // A synchronous (re-entrant) call returns its value to the Rust caller.
        if frame.capture {
            return Some(value);
        }
        match self.stack.len().checked_sub(1) {
            Some(caller) => {
                // Reg::MAX means "discard" (constructor / __clone return value).
                if frame.ret_dst != Reg::MAX {
                    self.reg_set(caller, frame.ret_dst, value);
                }
                None
            }
            None => Some(value),
        }
    }

    /// Enter a user function: bind args to the callee's CV registers and push.
    fn enter_user_call(
        &mut self,
        top: usize,
        callee: Rc<FunctionProto>,
        dst: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error(
                "Error",
                "Maximum call stack depth exceeded",
            ));
        }
        // Raw (non-deref) copy: by-ref args arrive as `Value::Ref` and must be
        // kept as references; by-value binding derefs in `bind_params`.
        let args: Vec<Value> =
            (0..argc).map(|i| self.stack[top].regs[(base + i) as usize].clone()).collect();
        self.push_user_frame(callee, args, dst)
    }

    /// Push a frame for a user function with pre-collected args.
    fn push_user_frame(
        &mut self,
        callee: Rc<FunctionProto>,
        args: Vec<Value>,
        dst: Reg,
    ) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let mut frame = Frame::new(callee.clone(), dst);
        bind_params(&callee, &args, &mut frame.regs)?;
        self.stack.push(frame);
        Ok(())
    }

    /// Flatten an array value into a positional argument vector (for spread).
    fn spread_args(&self, arr: &Value) -> Vec<Value> {
        match arr {
            Value::Array(a) => a.iter().map(|(_, v)| v.clone()).collect(),
            Value::Ref(r) => self.spread_args(&r.borrow()),
            _ => Vec::new(),
        }
    }

    /// Binary op that may throw (arithmetic).
    fn bin(
        &mut self,
        top: usize,
        dst: Reg,
        a: Reg,
        b: Reg,
        f: fn(&Value, &Value) -> Result<Value, Thrown>,
    ) -> Result<(), Thrown> {
        let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
        let v = f(&x, &y)?;
        self.reg_set(top, dst, v);
        Ok(())
    }

    /// Infallible binary op (bitwise/concat).
    fn bin_inf(&mut self, top: usize, dst: Reg, a: Reg, b: Reg, f: fn(&Value, &Value) -> Value) {
        let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
        let v = f(&x, &y);
        self.reg_set(top, dst, v);
    }

    fn call_builtin(
        &mut self,
        top: usize,
        name: &str,
        base: Reg,
        argc: u16,
    ) -> Result<Value, Thrown> {
        let args: Vec<Value> = (0..argc).map(|i| self.reg_get(top, base + i)).collect();
        self.invoke_builtin(name, args)
    }

    /// Call a builtin by name with a pre-collected argument vector.
    fn invoke_builtin(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Thrown> {
        let Some(builtin) = builtins::lookup(name) else {
            return Err(Thrown::error("Error", format!("Call to undefined function {name}()")));
        };
        if args.len() < builtin.min_args {
            return Err(Thrown::error(
                "ArgumentCountError",
                format!(
                    "Too few arguments to function {name}(), {} passed and at least {} expected",
                    args.len(),
                    builtin.min_args
                ),
            ));
        }
        if let Some(max) = builtin.max_args {
            if args.len() > max {
                return Err(Thrown::error(
                    "ArgumentCountError",
                    format!("Too many arguments to function {name}()"),
                ));
            }
        }
        (builtin.func)(self, &args)
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

// --- array element read + lvalue path operations ---------------------------

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        other => other.clone(),
    }
}

/// `$container[$key]` read: array element (Null if absent) or string byte.
fn dim_fetch(container: &Value, key: &Value) -> Value {
    match deref(container) {
        Value::Array(a) => match ArrayKey::from_value(key) {
            Some(k) => a.get(&k).cloned().unwrap_or(Value::Null),
            None => Value::Null,
        },
        Value::Str(s) => {
            let bytes = s.as_bytes();
            let len = bytes.len() as i64;
            let idx = conv::to_int(key);
            let i = if idx < 0 { len + idx } else { idx };
            if i >= 0 && i < len {
                Value::str(vec![bytes[i as usize]])
            } else {
                Value::str("")
            }
        }
        _ => Value::Null,
    }
}

/// Read the value at a cursor path (Null if any step is absent); non-mutating.
fn read_path(slot: &Value, steps: &[CursorStep]) -> Value {
    if let Value::Ref(r) = slot {
        return read_path(&r.borrow(), steps);
    }
    let Some((head, rest)) = steps.split_first() else {
        return slot.clone();
    };
    match (slot, head) {
        (Value::Array(a), CursorStep::Key(k)) => match ArrayKey::from_value(k) {
            Some(key) => a.get(&key).map(|c| read_path(c, rest)).unwrap_or(Value::Null),
            None => Value::Null,
        },
        (Value::Object(h), CursorStep::Prop(name)) => {
            let od = h.borrow();
            od.properties.get(name).map(|c| read_path(c, rest)).unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

/// Make `slot` an array (autovivifying null/"" ), or error on a non-empty scalar.
fn autovivify(slot: &mut Value) -> Result<(), Thrown> {
    match slot {
        Value::Array(_) => Ok(()),
        Value::Null => {
            *slot = Value::Array(PhpArray::new());
            Ok(())
        }
        Value::Str(s) if s.is_empty() => {
            *slot = Value::Array(PhpArray::new());
            Ok(())
        }
        _ => Err(Thrown::type_error("Cannot use a scalar value as an array")),
    }
}

/// Write `value` at a cursor path, autovivifying + COW-bubbling along the way.
fn assign_path(slot: &mut Value, steps: &[CursorStep], value: Value) -> Result<(), Thrown> {
    // Descend through a reference to its underlying storage.
    if let Value::Ref(rc) = slot {
        let mut inner = rc.borrow_mut();
        return assign_path(&mut inner, steps, value);
    }
    let Some((head, rest)) = steps.split_first() else {
        *slot = value;
        return Ok(());
    };
    if let CursorStep::Prop(name) = head {
        // Descend into an object property (handle semantics: no COW).
        let handle = match slot {
            Value::Object(h) => h.clone(),
            Value::Null => {
                return Err(Thrown::error("Error", "Attempt to assign property on null"))
            }
            other => {
                return Err(Thrown::error(
                    "Error",
                    format!("Attempt to assign property on {}", type_lc(other)),
                ))
            }
        };
        let mut od = handle.borrow_mut();
        let child = od.properties.entry(name.clone()).or_insert(Value::Null);
        return assign_path(child, rest, value);
    }
    autovivify(slot)?;
    let Value::Array(arr) = slot else { unreachable!("autovivify ensures array") };
    let child = match head {
        CursorStep::Key(k) => {
            let key = ArrayKey::from_value(k)
                .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
            arr.get_or_insert_mut(key)
        }
        CursorStep::Append => arr.append_mut(),
        CursorStep::Prop(_) => unreachable!("handled above"),
    };
    assign_path(child, rest, value)
}

/// Remove the leaf key of a cursor path (COW), descending to the parent first.
fn unset_path(slot: &mut Value, steps: &[CursorStep]) {
    if let Value::Ref(rc) = slot {
        let mut inner = rc.borrow_mut();
        return unset_path(&mut inner, steps);
    }
    let Some((head, rest)) = steps.split_first() else { return };
    match head {
        CursorStep::Prop(name) => {
            let Value::Object(h) = slot else { return };
            let h = h.clone();
            let mut od = h.borrow_mut();
            if rest.is_empty() {
                od.properties.shift_remove(name);
            } else if let Some(child) = od.properties.get_mut(name) {
                unset_path(child, rest);
            }
        }
        CursorStep::Key(k) => {
            let Value::Array(arr) = slot else { return };
            let Some(key) = ArrayKey::from_value(k) else { return };
            if rest.is_empty() {
                arr.remove(&key);
            } else if let Some(child) = arr.get_mut(&key) {
                unset_path(child, rest);
            }
        }
        CursorStep::Append => {}
    }
}

fn type_lc(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Ref(_) => "reference",
    }
}

fn array_key_to_value(k: &ArrayKey) -> Value {
    match k {
        ArrayKey::Int(n) => Value::Int(*n),
        ArrayKey::Str(s) => Value::Str(s.clone()),
    }
}

/// Bind positional `args` to a callee's parameter CV registers (by value), with
/// constant defaults and variadic collection. By-ref params and named args are
/// not yet supported.
fn bind_params(proto: &FunctionProto, args: &[Value], regs: &mut [Value]) -> Result<(), Thrown> {
    for (i, p) in proto.params.iter().enumerate() {
        if p.variadic {
            // Collect the remaining args into an array (by value).
            let mut arr = PhpArray::new();
            if i < args.len() {
                for a in &args[i..] {
                    arr.push(deref(a));
                }
            }
            regs[p.cv as usize] = Value::Array(arr);
            return Ok(()); // variadic is always the last parameter
        } else if i < args.len() {
            // By-ref params keep the incoming Ref (aliasing the caller's var);
            // by-value params receive a dereferenced copy.
            regs[p.cv as usize] = if p.by_ref { args[i].clone() } else { deref(&args[i]) };
        } else if let Some(def) = &p.default {
            match def {
                ParamDefault::Const(k) => regs[p.cv as usize] = proto.consts[*k as usize].clone(),
                ParamDefault::Init(_) => {} // non-constant defaults: deferred
            }
        } else {
            return Err(Thrown::error(
                "ArgumentCountError",
                format!(
                    "Too few arguments to function {}(), {} passed and at least {} expected",
                    String::from_utf8_lossy(proto.name.as_bytes()),
                    args.len(),
                    required_params(proto),
                ),
            ));
        }
    }
    Ok(())
}

fn required_params(proto: &FunctionProto) -> usize {
    proto.params.iter().filter(|p| p.default.is_none() && !p.variadic).count()
}

fn eval_cmp(kind: CmpKind, a: &Value, b: &Value) -> Value {
    match kind {
        CmpKind::Eq => Value::Bool(ops::loose_eq(a, b)),
        CmpKind::Ne => Value::Bool(!ops::loose_eq(a, b)),
        CmpKind::Identical => Value::Bool(ops::strict_eq(a, b)),
        CmpKind::NotIdentical => Value::Bool(!ops::strict_eq(a, b)),
        CmpKind::Lt => Value::Bool(ops::loose_compare(a, b).is_lt()),
        CmpKind::Le => Value::Bool(ops::loose_compare(a, b).is_le()),
        CmpKind::Gt => Value::Bool(ops::loose_compare(a, b).is_gt()),
        CmpKind::Ge => Value::Bool(ops::loose_compare(a, b).is_ge()),
        CmpKind::Spaceship => ops::spaceship(a, b),
    }
}
