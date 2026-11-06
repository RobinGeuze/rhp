//! Generators: suspended frames and the `Generator` methods.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_bytecode::{FunctionProto, Reg};
use rhp_values::{ArrayKey, ObjectData, PhpArray, PhpStr, Thrown, Value};

use crate::calls::native_method_count_error;
use crate::errors::UnwindResult;
use crate::{deref, Frame, Vm};

/// Status of a generator.
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum GenStatus {
    NotStarted,
    Suspended,
    Running,
    Finished,
}

/// A generator's suspended execution state.
pub(crate) struct GenState {
    /// The suspended frame (taken out while running). `None` while running/finished.
    pub(crate) frame: Option<Frame>,
    pub(crate) status: GenStatus,
    pub(crate) current_key: Value,
    pub(crate) current_val: Value,
    return_val: Value,
    /// Next auto-increment integer key for keyless `yield`.
    auto_key: i64,
    /// Register in the frame that receives the `send()` value on resume.
    resume_dst: Reg,
    /// php's ZEND_GENERATOR_AT_FIRST_YIELD: started, and not resumed since
    /// (only then can it be rewound).
    at_first_yield: bool,
    /// The generator function and its class (Generator::__debugInfo).
    func: (Rc<FunctionProto>, Option<rhp_values::ClassId>),
}

impl Vm {
    /// Wrap a prepared (but not pushed) frame as a `Generator` object; the body
    /// runs lazily under the generator protocol.
    pub(crate) fn make_generator(&mut self, mut frame: Frame) -> Value {
        frame.ret_dst = Reg::MAX;
        let func = (frame.proto.clone(), frame.defining_class);
        let cls = self
            .resolve_class_by_name(b"Generator")
            .expect("Generator prelude class");
        let id = self.alloc_object_id();
        let obj = Rc::new(RefCell::new(ObjectData::new(
            cls,
            id,
            indexmap::IndexMap::with_hasher(rustc_hash::FxBuildHasher),
        )));
        self.generators.insert(
            id,
            GenState {
                frame: Some(frame),
                status: GenStatus::NotStarted,
                current_key: Value::Null,
                current_val: Value::Null,
                return_val: Value::Null,
                auto_key: 0,
                resume_dst: 0,
                at_first_yield: false,
                func,
            },
        );
        Value::Object(obj)
    }

    /// Advance generator `id` to its next `yield` (or completion), injecting
    /// `send` as the result of the yield expression it was suspended on.
    pub(crate) fn advance_gen(&mut self, id: u32, send: Value) -> Result<(), Thrown> {
        self.resume_gen(id, Ok(send))
    }

    /// [`Self::advance_gen`], or raise `Err` at the suspended `yield`
    /// (Generator::throw).
    fn resume_gen(&mut self, id: u32, send: Result<Value, Thrown>) -> Result<(), Thrown> {
        if self.generators.get(&id).map(|g| g.status) == Some(GenStatus::Running) {
            // Suspended in a fiber while it runs (zend_generator_resume).
            return Err(Thrown::error(
                "Error",
                "Cannot resume an already running generator",
            ));
        }
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let (mut frame, status, resume_dst) = {
            let g = self.generators.get_mut(&id).unwrap();
            if g.status == GenStatus::Finished {
                return Ok(());
            }
            g.at_first_yield = false;
            (
                g.frame.take().expect("suspended generator frame"),
                g.status,
                g.resume_dst,
            )
        };
        let mut throw = None;
        match send {
            Ok(v) if status == GenStatus::Suspended => frame.regs[resume_dst as usize] = v,
            Ok(_) => {}
            Err(t) => throw = Some(t),
        }
        frame.capture = true;
        self.generators.get_mut(&id).unwrap().status = GenStatus::Running;
        let base = self.stack.len();
        self.stack.push(frame);
        self.gen_stack.push(id);
        let saved_yield = self.yield_out.take();
        let result = loop {
            let step = match throw.take() {
                Some(t) => Err(t),
                None => self.run_frames(),
            };
            match step {
                Ok(v) => {
                    if let Some((k, val, dst)) = self.yield_out.take() {
                        let frame = self.stack.pop().unwrap();
                        let g = self.generators.get_mut(&id).unwrap();
                        g.frame = Some(frame);
                        g.resume_dst = dst;
                        g.current_key = k;
                        g.current_val = val;
                        g.status = GenStatus::Suspended;
                        break Ok(());
                    } else {
                        let g = self.generators.get_mut(&id).unwrap();
                        g.return_val = v;
                        g.current_key = Value::Null;
                        g.current_val = Value::Null;
                        g.status = GenStatus::Finished;
                        break Ok(());
                    }
                }
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        break Err(thrown);
                    }
                    match self.unwind(thrown, base)? {
                        UnwindResult::Caught => {}
                        UnwindResult::Escaped(t) => {
                            self.generators.get_mut(&id).unwrap().status = GenStatus::Finished;
                            break Err(t);
                        }
                    }
                }
            }
        };
        self.gen_stack.pop();
        self.yield_out = saved_yield;
        result
    }

    /// Ensure a generator has started (run to its first yield).
    pub(crate) fn gen_ensure_started(&mut self, id: u32) -> Result<(), Thrown> {
        if self.generators.get(&id).map(|g| g.status) == Some(GenStatus::NotStarted) {
            self.advance_gen(id, Value::Null)?;
            if let Some(g) = self.generators.get_mut(&id) {
                g.at_first_yield = true;
            }
        }
        Ok(())
    }

    /// zend_generator_rewind: start the generator; one resumed past its
    /// first `yield` cannot go back.
    pub(crate) fn gen_rewind(&mut self, id: u32) -> Result<(), Thrown> {
        self.gen_ensure_started(id)?;
        if self.generators.get(&id).is_some_and(|g| !g.at_first_yield) {
            return Err(Thrown::error(
                "Exception",
                "Cannot rewind a generator that was already run",
            ));
        }
        Ok(())
    }

    /// zend_generator_get_iterator's first check: a finished generator
    /// cannot be iterated again.
    pub(crate) fn gen_check_open(&self, id: u32) -> Result<(), Thrown> {
        if self.generators.get(&id).map(|g| g.status) == Some(GenStatus::Finished) {
            return Err(Thrown::error(
                "Exception",
                "Cannot traverse an already closed generator",
            ));
        }
        Ok(())
    }

    /// Dispatch a built-in `Generator` method. Returns `None` if `name` is not a
    /// generator method (so the caller falls through to normal dispatch).
    pub(crate) fn generator_method(
        &mut self,
        id: u32,
        name_lc: &[u8],
        args: &[Value],
    ) -> Option<Result<Value, Thrown>> {
        if matches!(
            name_lc,
            b"current"
                | b"key"
                | b"valid"
                | b"next"
                | b"send"
                | b"rewind"
                | b"getreturn"
                | b"throw"
                | b"__debuginfo"
        ) {
            if let Some(e) = native_method_count_error("Generator", name_lc, args.len()) {
                return Some(Err(e));
            }
        }
        Some(match name_lc {
            b"current" => (|| {
                self.gen_ensure_started(id)?;
                Ok(deref(&self.generators[&id].current_val))
            })(),
            b"key" => (|| {
                self.gen_ensure_started(id)?;
                Ok(self.generators[&id].current_key.clone())
            })(),
            b"valid" => (|| {
                self.gen_ensure_started(id)?;
                Ok(Value::Bool(
                    self.generators[&id].status != GenStatus::Finished,
                ))
            })(),
            b"next" => (|| {
                self.gen_ensure_started(id)?;
                self.advance_gen(id, Value::Null)?;
                Ok(Value::Null)
            })(),
            b"send" => (|| {
                let v = args.first().cloned().unwrap_or(Value::Null);
                // Sending to a not-yet-started generator first runs to the first yield.
                if self.generators[&id].status == GenStatus::NotStarted {
                    self.advance_gen(id, Value::Null)?;
                }
                self.advance_gen(id, v)?;
                Ok(deref(&self.generators[&id].current_val))
            })(),
            b"rewind" => (|| {
                self.gen_rewind(id)?;
                Ok(Value::Null)
            })(),
            b"getreturn" => Ok(self.generators[&id].return_val.clone()),
            b"throw" => (|| {
                let e = match args.first().map(deref) {
                    Some(Value::Object(e))
                        if self.class_implements(e.borrow().class, b"Throwable") =>
                    {
                        e
                    }
                    other => {
                        let v = other.unwrap_or(Value::Null);
                        let given = rhp_builtins::zval_type_name(self, &v);
                        return Err(Thrown::type_error(format!(
                            "Generator::throw(): Argument #1 ($exception) must be of type Throwable, {given} given"
                        )));
                    }
                };
                if self.generators[&id].status == GenStatus::Running {
                    return Err(Thrown::error(
                        "Error",
                        "Cannot resume an already running generator",
                    ));
                }
                self.gen_ensure_started(id)?;
                if self.generators[&id].status == GenStatus::Finished {
                    return Err(Thrown::Object(e));
                }
                self.resume_gen(id, Err(Thrown::Object(e)))?;
                Ok(deref(&self.generators[&id].current_val))
            })(),
            b"__debuginfo" => Ok(Value::Array(self.generator_debug_info(id))),
            _ => return None,
        })
    }

    /// Generator::__debugInfo: `["function" => name]`, `Class::name` for
    /// a function with a class scope.
    pub(crate) fn generator_debug_info(&self, id: u32) -> PhpArray {
        let mut a = PhpArray::new();
        if let Some(g) = self.generators.get(&id) {
            let (proto, class) = &g.func;
            let name = match class {
                Some(c) => {
                    let mut n = self.class(*c).name.as_bytes().to_vec();
                    n.extend_from_slice(b"::");
                    n.extend_from_slice(proto.name.as_bytes());
                    PhpStr::from_bytes(n)
                }
                None => proto.name.clone(),
            };
            a.insert(ArrayKey::Str(PhpStr::from("function")), Value::Str(name));
        }
        a
    }

    /// `Yield`: hand (key, value, resume register) to the generator driver in
    /// `yield_out`; the caller then suspends the frame.
    pub(crate) fn exec_yield(
        &mut self,
        top: usize,
        proto: &FunctionProto,
        dst: Reg,
        key: Reg,
        value: Reg,
    ) -> Result<(), Thrown> {
        // Suspend: hand (key, value, resume-dst) to the generator
        // driver, leaving this frame on the stack for it to reclaim.
        // A by-reference generator (`function &gen()`) yields a
        // variable as a reference to it (php's ZEND_YIELD with
        // ZEND_ACC_RETURN_REFERENCE), so `foreach (gen() as &$v)`
        // writes reach it; any other value is yielded as is.
        let val = if proto.returns_ref && value < proto.num_cvs {
            Value::Ref(self.promote(top, value))
        } else if proto.returns_ref {
            // A reference made by MakeRef/MakeRefLval, or a plain
            // value (php: "Only variable references…" notice).
            let v = self.stack[top].regs[value as usize].clone();
            if !matches!(v, Value::Ref(_)) {
                self.diag(
                    rhp_builtins::E_NOTICE,
                    "Only variable references should be yielded by reference",
                )?;
            }
            v
        } else {
            self.reg_r(top, value)?
        };
        let id = *self.gen_stack.last().expect("yield outside a generator");
        let key_val = if key == Reg::MAX {
            let g = self.generators.get_mut(&id).unwrap();
            let k = Value::Int(g.auto_key);
            g.auto_key += 1;
            k
        } else {
            let kv = self.reg_r(top, key)?;
            if let Value::Int(n) = &kv {
                let g = self.generators.get_mut(&id).unwrap();
                if *n >= g.auto_key {
                    g.auto_key = n + 1;
                }
            }
            kv
        };
        self.yield_out = Some((key_val, val, dst));
        Ok(())
    }
}
