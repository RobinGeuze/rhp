//! Calls: user frames and returns, argument binding, methods, callables and
//! builtins.

use std::rc::{Rc, Weak};

use rhp_builtins::{self as builtins};
use rhp_bytecode::{ClassRef, FunctionProto, NIdx, ParamDefault, Reg};
use rhp_values::{
    conv, ArrayKey, ClassId, ClassKind, ObjectHandle, PhpArray, PhpStr, Thrown, Value,
};

use crate::errors::UnwindResult;
use crate::{classes, deref, Frame, UnitFn, Vm};

/// Named call arguments, in call order.
type NamedArgs = Vec<(PhpStr, Value)>;

/// A resolved callable (see `Vm::resolve_callable`).
pub(crate) enum Callee {
    Closure(Rc<rhp_values::ClosureData>),
    Function(UnitFn),
    Builtin(String),
    /// A method with its `$this` (None: static) and called class.
    Method(classes::ResolvedMethod, Option<ObjectHandle>, ClassId),
    /// `__call` (with `$this`) / `__callStatic` for an unknown method name.
    Magic(
        classes::ResolvedMethod,
        Option<ObjectHandle>,
        ClassId,
        Vec<u8>,
    ),
    /// An enum's built-in `cases`/`from`/`tryFrom` (lowercased).
    EnumStatic(ClassId, Vec<u8>),
}

/// A call's arguments: a slice, or the caller's argument window (`argc`
/// registers from `base` in frame `top`, read raw: by-reference arguments
/// are `Value::Ref`s), bound without copying it first.
#[derive(Clone, Copy)]
pub(crate) enum Args<'a> {
    Slice(&'a [Value]),
    Window { top: usize, base: Reg, argc: u16 },
}

impl Args<'_> {
    fn len(&self) -> usize {
        match self {
            Args::Slice(s) => s.len(),
            Args::Window { argc, .. } => *argc as usize,
        }
    }
}

/// What a call by name (`Call`, `CallV`) runs.
#[derive(Clone, Copy)]
pub(crate) enum CallTarget {
    /// `Vm::functions[i]`.
    User(usize),
    Builtin(
        &'static builtins::Builtin,
        Option<&'static rhp_bytecode::signatures::Func>,
    ),
    /// No function by that name: the call fails as undefined.
    Missing,
}

/// A call site's resolution (see [`Vm::call_target`]).
pub(crate) struct CallSite {
    target: CallTarget,
    /// The [`Vm::fn_gen`] it was resolved at, or 0 when no later function
    /// declaration can change it.
    gen: u32,
    /// Holds the proto's allocation, so that no other proto takes its
    /// address (half the key) while the entry exists.
    _proto: Weak<FunctionProto>,
}

/// A builtin (native) call in progress, for exception traces: php shows the
/// internal function as its own trace frame, called from `caller`'s line, and
/// any user callback it invokes as `[internal function]`.
pub(crate) struct NativeCall {
    pub(crate) name: &'static str,
    /// The running builtin's argument slice, borrowed rather than copied so
    /// tracking a call costs nothing until a trace is actually built.
    /// Invariant: the entry is pushed just before the builtin is called and
    /// popped right after it returns, while the owning `Vec` (a local of
    /// `run_builtin`, only shared-borrowed by the builtin) is alive and
    /// unmodified — so the pointer is valid whenever it is in `native_calls`.
    pub(crate) args: *const [Value],
    /// Bit `i`: argument `i` is a scalar zpp converted to a string in
    /// place, so a trace shows it as one (the builtin got the scalar).
    pub(crate) shown_str: u64,
    /// Index of the frame that made the call.
    pub(crate) caller: usize,
    /// php 8.4 runs the call without a frame of its own
    /// (ZEND_FRAMELESS_ICALL), though traces still list it.
    pub(crate) frameless_icall: bool,
}

impl Vm {
    /// Whether parameter `idx` of the function `Instr::ResolveFunc` left in
    /// register `func` is by reference.
    pub(crate) fn resolved_param_is_ref(&self, top: usize, func: Reg, idx: u16) -> bool {
        match self.stack[top].regs[func as usize] {
            Value::Int(f) if f >= 0 => param_is_ref(&self.functions[f as usize].proto, idx),
            _ => false,
        }
    }

    /// What the call by name at `pc` of `proto`, run by frame `top`, calls.
    /// A call site keeps what it resolved to for the request (php's run-time
    /// cache slot), unless a later function declaration could change it: a
    /// call in a namespace that fell back to the global name, or a call that
    /// reached a builtin (rhp lets a later declaration take a builtin's
    /// name). An include's or eval's `{main}` is freed when it returns, so
    /// its calls are resolved every time.
    #[inline]
    pub(crate) fn call_target(
        &mut self,
        top: usize,
        proto: &Rc<FunctionProto>,
        pc: usize,
        name: NIdx,
        fallback: NIdx,
    ) -> Result<CallTarget, Thrown> {
        if self.stack[top].include_kind.is_none() {
            let key = (Rc::as_ptr(proto) as usize, pc as u32);
            if let Some(site) = self.call_sites.get(&key) {
                if site.gen == 0 || site.gen == self.fn_gen {
                    return Ok(site.target);
                }
            }
        }
        self.call_target_miss(top, proto, pc, name, fallback)
    }

    /// [`Vm::call_target`] when the site has no valid entry: resolve the
    /// call and remember it. Kept out of line so that the dispatch loop
    /// holds only the cache probe.
    #[cold]
    #[inline(never)]
    fn call_target_miss(
        &mut self,
        top: usize,
        proto: &Rc<FunctionProto>,
        pc: usize,
        name: NIdx,
        fallback: NIdx,
    ) -> Result<CallTarget, Thrown> {
        let (target, stable) = self.resolve_call(proto, name, fallback)?;
        if self.stack[top].include_kind.is_none() && !matches!(target, CallTarget::Missing) {
            let key = (Rc::as_ptr(proto) as usize, pc as u32);
            let site = CallSite {
                target,
                gen: if stable { 0 } else { self.fn_gen },
                _proto: Rc::downgrade(proto),
            };
            // Not counted: php allocates an op_array's run-time cache in one
            // piece, so memory_get_usage() doesn't move when a call site is
            // first reached.
            rhp_values::mem::uncounted(|| self.call_sites.insert(key, site));
        }
        Ok(target)
    }

    /// [`Vm::call_target`] without the cache. Tries the (possibly
    /// namespaced) `name`, then `fallback` (the global name) if set, against
    /// user functions; a builtin is looked up under the last name tried.
    /// The flag: the target is final for the request (a user function found
    /// under `name` itself).
    fn resolve_call(
        &self,
        proto: &FunctionProto,
        name: NIdx,
        fallback: NIdx,
    ) -> Result<(CallTarget, bool), Thrown> {
        let names = &proto.names;
        let primary = names[name as usize].as_bytes().to_ascii_lowercase();
        let (user, fname, own) = match self.fn_names.get(&primary) {
            Some(&i) => (Some(i), primary, true),
            None if fallback != rhp_bytecode::NO_NAME => {
                let global = names[fallback as usize].as_bytes().to_ascii_lowercase();
                (self.fn_names.get(&global).copied(), global, false)
            }
            None => (None, primary, false),
        };
        let Ok(user) = Self::engine_helper_call(proto, user, &fname) else {
            return Err(undefined_function(&names[name as usize]));
        };
        if let Some(reason) = builtins::is_refused(std::str::from_utf8(&fname).unwrap_or("")) {
            let shown = String::from_utf8_lossy(&fname).into_owned();
            return Err(Thrown::unsupported(format!("{shown}()"), reason));
        }
        if let Some(i) = user {
            return Ok((CallTarget::User(i), own));
        }
        let target = match builtins::lookup_lc(std::str::from_utf8(&fname).unwrap_or("")) {
            Some(b) => CallTarget::Builtin(b, rhp_bytecode::signatures::function(&fname)),
            None => CallTarget::Missing,
        };
        Ok((target, false))
    }

    /// `name(...)` / `$name(...)` where `name` is an engine helper's: php
    /// looks the function up when the closure is made (`ns_name`, the
    /// namespaced name `name` fell back from, first), and user code has no
    /// such function (see [`is_helper_function`]). Returns the namespaced
    /// name when that is the function found. Other names are still resolved
    /// when the closure is called.
    pub(crate) fn check_fcc_name(
        &self,
        top: usize,
        name: &PhpStr,
        ns_name: NIdx,
    ) -> Result<Option<PhpStr>, Thrown> {
        let b = name.as_bytes();
        let n = b.strip_prefix(b"\\").unwrap_or(b);
        if !is_helper_function(n)
            || n.windows(2).any(|w| w == b"::")
            || self.stack[top].proto.is_internal()
        {
            return Ok(None);
        }
        let ns = (ns_name != rhp_bytecode::NO_NAME)
            .then(|| &self.stack[top].proto.names[ns_name as usize]);
        if let Some(ns) = ns.filter(|ns| {
            self.fn_names
                .contains_key(&ns.as_bytes().to_ascii_lowercase())
        }) {
            return Ok(Some(ns.clone()));
        }
        if self.fn_names.contains_key(&n.to_ascii_lowercase()) {
            return Ok(None);
        }
        Err(undefined_function(ns.unwrap_or(name)))
    }

    /// Name user function `idx` in php's function table. Call sites that
    /// the new name can change are resolved again (see
    /// [`Vm::call_target`]).
    pub(crate) fn add_fn_name(&mut self, lc: Vec<u8>, idx: usize) {
        self.fn_names.insert(lc, idx);
        self.fn_gen = self.fn_gen.wrapping_add(1).max(1);
    }

    /// The user function a direct call from `caller` to `name` (resolved by
    /// [`Vm::resolve_call`] to `user`) runs, `None` for a builtin, or `Err`
    /// when the call must fail as undefined. For an engine helper's name
    /// (see [`is_helper_function`]) prelude code always gets the builtin (a
    /// user function of that name doesn't shadow it), user code never does.
    pub(crate) fn engine_helper_call(
        caller: &FunctionProto,
        user: Option<usize>,
        name: &[u8],
    ) -> Result<Option<usize>, ()> {
        if !is_helper_function(name) {
            Ok(user)
        } else if caller.is_internal() {
            Ok(None)
        } else {
            user.map(Some).ok_or(())
        }
    }

    /// Dispatch a method call on `h` by name (generator methods, user method with
    /// visibility check, `__call` fallback, else errors). Shared by `CallMethod`
    /// and `CallMethodDyn`.
    #[allow(clippy::too_many_arguments)] // VM call-dispatch frame setup
    pub(crate) fn invoke_method(
        &mut self,
        top: usize,
        h: ObjectHandle,
        cls: ClassId,
        mname: &[u8],
        dst: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        let gid = h.borrow().id;
        if self.generators.contains_key(&gid) {
            let args: Vec<Value> = (0..argc).map(|i| self.reg_get(top, base + i)).collect();
            if let Some(r) = self.generator_method(gid, &mname.to_ascii_lowercase(), &args) {
                let r = r?;
                self.reg_set(top, dst, r);
                return Ok(());
            }
        }
        let found = self.lookup_method(cls, mname);
        let accessible = found.as_ref().is_some_and(|m| {
            self.check_access(
                self.stack[top].defining_class,
                m.defining_class,
                m.visibility,
            )
        });
        if let (Some(m), true) = (&found, accessible) {
            let m = m.clone();
            self.enter_method(top, &m, Some(h), cls, dst, base, argc)?;
        } else if let Some(magic) = self.lookup_method(cls, b"__call") {
            let mut arr = rhp_values::PhpArray::new();
            for i in 0..argc {
                arr.push(self.reg_get(top, base + i));
            }
            let args = vec![Value::str(mname.to_vec()), Value::Array(arr)];
            let r = self.call_sync(&magic, Some(h.clone()), cls, args)?;
            self.reg_set(top, dst, r);
        } else if let Some(m) = found {
            return Err(self.access_err("method", m.defining_class, mname, m.visibility));
        } else {
            return Err(Thrown::error(
                "Error",
                format!(
                    "Call to undefined method {}::{}()",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                    String::from_utf8_lossy(mname)
                ),
            ));
        }
        Ok(())
    }

    /// Shared body for `C::m()` / `C::$m()` static dispatch (resolves the class,
    /// handles enum built-ins, `__callStatic`, LSB, and `$this` forwarding).
    pub(crate) fn do_call_static(
        &mut self,
        top: usize,
        class: ClassRef,
        mname: &[u8],
        dst: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        let cls = self.resolve_class_autoload(top, class)?;
        let found = self.lookup_method(cls, mname);
        // Enum built-in static methods (cases/from/tryFrom) when not overridden.
        if found.is_none() && self.class(cls).kind == ClassKind::Enum {
            let lc = mname.to_ascii_lowercase();
            if self.is_enum_builtin(cls, &lc) {
                let args: Vec<Value> = (0..argc).map(|i| self.reg_get(top, base + i)).collect();
                let strict = self.stack[top].proto.strict_types;
                let r = self.enum_builtin_static(cls, &lc, args, strict)?;
                self.reg_set(top, dst, r);
                return Ok(());
            }
        }
        let accessible = found.as_ref().is_some_and(|m| {
            self.check_access(
                self.stack[top].defining_class,
                m.defining_class,
                m.visibility,
            )
        });
        if !(found.is_some() && accessible) {
            // Undefined / inaccessible static method -> __callStatic.
            if let Some(magic) = self.lookup_method(cls, b"__callstatic") {
                let mut arr = rhp_values::PhpArray::new();
                for i in 0..argc {
                    arr.push(self.reg_get(top, base + i));
                }
                let args = vec![Value::str(mname.to_vec()), Value::Array(arr)];
                let r = self.call_sync(&magic, None, cls, args)?;
                self.reg_set(top, dst, r);
                return Ok(());
            }
            return match found {
                Some(m) => Err(self.access_err("method", m.defining_class, mname, m.visibility)),
                None => Err(Thrown::error(
                    "Error",
                    format!(
                        "Call to undefined method {}::{}()",
                        String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                        String::from_utf8_lossy(mname)
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
        let this = self.static_call_this(top, &m, cls)?;
        self.enter_method(top, &m, this, called, dst, base, argc)
    }

    /// The `$this` a `C::m()` call forwards (ZEND_INIT_STATIC_METHOD_CALL):
    /// the caller's object when it is-a `cls`; a non-static method without
    /// one is "Non-static method C::m() cannot be called statically".
    fn static_call_this(
        &self,
        top: usize,
        m: &classes::ResolvedMethod,
        cls: ClassId,
    ) -> Result<Option<ObjectHandle>, Thrown> {
        if m.is_static {
            return Ok(None);
        }
        match self
            .current_this(top)
            .filter(|h| self.class(h.borrow().class).ancestors.contains(&cls))
        {
            Some(h) => Ok(Some(h)),
            None => Err(Thrown::error(
                "Error",
                format!(
                    "Non-static method {}::{}() cannot be called statically",
                    String::from_utf8_lossy(self.class(m.defining_class).name.as_bytes()),
                    String::from_utf8_lossy(m.proto.name.as_bytes())
                ),
            )),
        }
    }

    /// `$o->m(...)` with the arguments in an array (spread / named arguments):
    /// register `args` holds positional entries then named (string-keyed)
    /// ones. An undefined or inaccessible method goes to `__call` with the
    /// arguments array as is (php collects named arguments into it).
    pub(crate) fn call_method_args(
        &mut self,
        top: usize,
        obj: Reg,
        mname: Vec<u8>,
        args: Reg,
        dst: Reg,
    ) -> Result<(), Thrown> {
        let o = self.reg_get(top, obj);
        let arr = self.reg_get(top, args);
        if let Value::Closure(_) = o {
            let (mut argv, named) = self.split_call_args(&arr)?;
            let r = match mname.to_ascii_lowercase().as_slice() {
                b"__invoke" if !named.is_empty() => self.call_named(&o, argv, named, None)?,
                b"call" if !named.is_empty() && !argv.is_empty() => {
                    let new_this = argv.remove(0);
                    let bound = self.closure_call_target(&o, &new_this)?;
                    if matches!(bound, Value::Null) {
                        Value::Null
                    } else {
                        let r = self.call_named(&bound, argv, named, None);
                        self.closure_call_done(&o, &bound);
                        r?
                    }
                }
                _ => self.closure_method(&o, &mname, argv)?,
            };
            self.reg_set(top, dst, r);
            return Ok(());
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
        let gid = h.borrow().id;
        if self.generators.contains_key(&gid) {
            let (argv, _named) = self.split_call_args(&arr)?;
            if let Some(r) = self.generator_method(gid, &mname.to_ascii_lowercase(), &argv) {
                let r = r?;
                self.reg_set(top, dst, r);
                return Ok(());
            }
        }
        let cls = h.borrow().class;
        let found = self.lookup_method(cls, &mname);
        let accessible = found.as_ref().is_some_and(|m| {
            self.check_access(
                self.stack[top].defining_class,
                m.defining_class,
                m.visibility,
            )
        });
        match (found, accessible) {
            (Some(m), true) => {
                let (argv, named) = self.split_call_args(&arr)?;
                let argv = self.place_named_args(&m.proto, Some(m.defining_class), argv, named)?;
                self.push_method_frame(&m, Some(h.clone()), cls, Args::Slice(&argv), dst)
            }
            (found, _) => {
                if let Some(magic) = self.lookup_method(cls, b"__call") {
                    let arr = match deref(&arr) {
                        Value::Array(a) => Value::Array(a),
                        _ => Value::Array(PhpArray::new()),
                    };
                    let r =
                        self.call_sync(&magic, Some(h.clone()), cls, vec![Value::str(mname), arr])?;
                    self.reg_set(top, dst, r);
                    return Ok(());
                }
                Err(match found {
                    Some(m) => self.access_err("method", m.defining_class, &mname, m.visibility),
                    None => Thrown::error(
                        "Error",
                        format!(
                            "Call to undefined method {}::{}()",
                            String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                            String::from_utf8_lossy(&mname)
                        ),
                    ),
                })
            }
        }
    }

    /// `C::m(...)` with the arguments in an array (see [`Self::call_method_args`]):
    /// LSB and `$this` forwarding as for a window call; enum built-ins; an
    /// undefined or inaccessible method goes to `__callStatic`.
    pub(crate) fn call_static_args(
        &mut self,
        top: usize,
        class: ClassRef,
        mname: Vec<u8>,
        args: Reg,
        dst: Reg,
    ) -> Result<(), Thrown> {
        let cls = self.resolve_class_autoload(top, class)?;
        let arr = self.reg_get(top, args);
        let found = self.lookup_method(cls, &mname);
        if found.is_none() && self.class(cls).kind == ClassKind::Enum {
            let lc = mname.to_ascii_lowercase();
            if self.is_enum_builtin(cls, &lc) {
                let (argv, _named) = self.split_call_args(&arr)?;
                let strict = self.stack[top].proto.strict_types;
                let r = self.enum_builtin_static(cls, &lc, argv, strict)?;
                self.reg_set(top, dst, r);
                return Ok(());
            }
        }
        let accessible = found.as_ref().is_some_and(|m| {
            self.check_access(
                self.stack[top].defining_class,
                m.defining_class,
                m.visibility,
            )
        });
        let m = match (found, accessible) {
            (Some(m), true) => m,
            (found, _) => {
                if let Some(magic) = self.lookup_method(cls, b"__callstatic") {
                    let arr = match deref(&arr) {
                        Value::Array(a) => Value::Array(a),
                        _ => Value::Array(PhpArray::new()),
                    };
                    let r = self.call_sync(&magic, None, cls, vec![Value::str(mname), arr])?;
                    self.reg_set(top, dst, r);
                    return Ok(());
                }
                return Err(match found {
                    Some(m) => self.access_err("method", m.defining_class, &mname, m.visibility),
                    None => Thrown::error(
                        "Error",
                        format!(
                            "Call to undefined method {}::{}()",
                            String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                            String::from_utf8_lossy(&mname)
                        ),
                    ),
                });
            }
        };
        let called = match class {
            ClassRef::SelfClass | ClassRef::ParentClass | ClassRef::StaticClass => {
                self.stack[top].called_class.unwrap_or(cls)
            }
            _ => cls,
        };
        let this = self.static_call_this(top, &m, cls)?;
        let (argv, named) = self.split_call_args(&arr)?;
        let argv = self.place_named_args(&m.proto, Some(m.defining_class), argv, named)?;
        self.push_method_frame(&m, this, called, Args::Slice(&argv), dst)
    }

    /// The strict-types mode of the current caller (top of stack), for argument
    /// coercion of a call about to be made.
    pub(crate) fn caller_strict(&self) -> bool {
        self.stack
            .last()
            .map(|f| f.proto.strict_types)
            .unwrap_or(false)
    }

    /// Bind `args` into `frame` (which already carries its `$this`/scope) and
    /// enforce declared parameter types. On failure php raises the
    /// ArgumentCountError / TypeError from *inside* the callee: at the failing
    /// parameter's line, with the callee on the trace.
    pub(crate) fn bind_frame(
        &mut self,
        frame: &mut Frame,
        args: Args<'_>,
        caller: Option<bool>,
    ) -> Result<(), Thrown> {
        let proto = frame.proto.clone();
        let argc = args.len();
        frame.argc = argc;
        // A named-argument call armed `pending_named` for this frame.
        let named = self.pending_named.take();
        let count_error = self.internal_arg_count_error(&proto, frame, argc);
        let bound = match args {
            Args::Slice(s) => bind_params(
                &proto,
                s.len(),
                |i, by_ref| if by_ref { s[i].clone() } else { deref(&s[i]) },
                frame,
            ),
            Args::Window { top, base, argc } => {
                // The window's temporaries move into the callee (php moves
                // arguments into the callee's frame, so they die with it).
                let f = &mut self.stack[top];
                let movable = base as usize >= f.proto.num_cvs as usize;
                let w = &mut f.regs[base as usize..base as usize + argc as usize];
                bind_params(
                    &proto,
                    argc as usize,
                    |i, by_ref| {
                        if !movable {
                            return if by_ref { w[i].clone() } else { deref(&w[i]) };
                        }
                        match std::mem::replace(&mut w[i], Value::Null) {
                            Value::Ref(rc) if !by_ref => rc.borrow().clone(),
                            v => v,
                        }
                    },
                    frame,
                )
            }
        };
        let failed = match bound {
            _ if count_error.is_some() => count_error.map(|e| (0, e)),
            Err(missing) => Some((missing, self.arg_count_error(&proto, frame, argc))),
            Ok(()) => {
                if let Some((holes, extra)) = named {
                    frame.holes = holes;
                    if !extra.is_empty() {
                        if let Some(p) = proto.params.iter().find(|p| p.variadic) {
                            if let Value::Array(a) = &mut frame.regs[p.cv as usize] {
                                for (k, v) in extra {
                                    a.insert(ArrayKey::Str(k), deref(&v));
                                }
                            }
                        }
                    }
                }
                let scope = frame.defining_class;
                match self.coerce_stringable_args(&proto, frame, caller, scope) {
                    Err(e) => Some((0, e)),
                    Ok(()) => self.check_arg_types(&proto, frame, caller, scope).err(),
                }
            }
        };
        let Some((idx, err)) = failed else {
            return Ok(());
        };
        // A prelude callee has no file of its own: like an internal function,
        // the error belongs to the calling user frame (the throw site).
        let at = (!proto.is_internal()).then(|| {
            let line = proto.params.get(idx).map(|p| p.line).unwrap_or(0);
            (proto.debug.file.as_bytes().to_vec(), line)
        });
        let mut pushed = Frame::new(proto.clone(), frame.unit, Reg::MAX);
        std::mem::swap(&mut pushed, frame);
        self.stack.push(pushed);
        let t = self.materialize(err, at);
        let mut popped = self.stack.pop().expect("pushed frame");
        std::mem::swap(&mut popped, frame);
        Err(t)
    }

    /// zend_std_get_constructor's visibility check for `new C` run by frame
    /// `top`: "Call to private C::__construct() from global scope".
    /// Engine-internal (prelude) code may construct anything.
    pub(crate) fn check_ctor_access(&self, top: usize, cls: ClassId) -> Result<(), Thrown> {
        let Some(m) = self.lookup_method(cls, b"__construct") else {
            return Ok(());
        };
        if m.visibility == rhp_values::Visibility::Public || self.stack[top].proto.is_internal() {
            return Ok(());
        }
        let scope = self.stack[top].defining_class;
        if self.check_access(scope, m.defining_class, m.visibility) {
            return Ok(());
        }
        let v = if m.visibility == rhp_values::Visibility::Private {
            "private"
        } else {
            "protected"
        };
        let from = match scope {
            Some(c) => format!(
                "scope {}",
                String::from_utf8_lossy(self.class(c).name.as_bytes())
            ),
            None => "global scope".to_string(),
        };
        Err(Thrown::error(
            "Error",
            format!(
                "Call to {v} {}::{}() from {from}",
                String::from_utf8_lossy(self.class(m.defining_class).name.as_bytes()),
                String::from_utf8_lossy(m.proto.name.as_bytes())
            ),
        ))
    }

    /// zpp's ArgumentCountError for a prelude function or a built-in class's
    /// prelude method called with an argument count its stub signature
    /// rejects (prelude code calling it is not checked).
    fn internal_arg_count_error(
        &self,
        proto: &FunctionProto,
        frame: &Frame,
        passed: usize,
    ) -> Option<Thrown> {
        use rhp_bytecode::signatures::Signature;
        if !proto.is_internal() {
            return None;
        }
        let caller = self.stack.len().checked_sub(1)?;
        if self.is_internal_frame(caller) {
            return None;
        }
        let (name, min, max) = match frame.defining_class {
            Some(cls) if self.is_internal_class(cls) => {
                let cname = self.class(cls).name.as_bytes();
                // Their php bodies throw without parsing any argument, or
                // parse them quietly (DatePeriod's three signatures).
                if (cname.eq_ignore_ascii_case(b"WeakReference")
                    || cname.eq_ignore_ascii_case(b"DatePeriod"))
                    && proto.name.as_bytes().eq_ignore_ascii_case(b"__construct")
                {
                    return None;
                }
                let st = rhp_bytecode::signatures::method(cname, proto.name.as_bytes())?;
                let name = format!("{}::{}", String::from_utf8_lossy(cname), st.name);
                (name, st.min_args(), st.max_args())
            }
            Some(_) => return None,
            None => {
                let f = rhp_bytecode::signatures::function(proto.name.as_bytes())?;
                (f.name.to_string(), f.min_args(), f.max_args())
            }
        };
        if let Some(e) = zpp_count_error(&name, min, max, passed) {
            return Some(e);
        }
        // php takes the argument but the prelude body declares no such
        // parameter (and reads none through func_get_args): refuse it rather
        // than ignore it.
        if passed > proto.params.len() && !proto.params.iter().any(|p| p.variadic) {
            return Some(Thrown::unsupported(
                format!("{name}() argument #{}", proto.params.len() + 1),
                "not implemented",
            ));
        }
        None
    }

    /// "Too few arguments to function f(), N passed[ in F on line L] and
    /// {exactly|at least} M expected" (zend_missing_arg_error).
    fn arg_count_error(&self, proto: &FunctionProto, frame: &Frame, passed: usize) -> Thrown {
        let required = required_params(proto);
        let declared = proto.params.iter().filter(|p| !p.variadic).count();
        let which = if required == declared {
            "exactly"
        } else {
            "at least"
        };
        let name = self.func_display_name(proto, frame.defining_class);
        if proto.is_internal() {
            // A built-in implemented in the PHP prelude reports like an
            // internal function (zend_wrong_parameters_count_error).
            let which = if proto.params.iter().any(|p| p.variadic) {
                "at least"
            } else {
                which
            };
            return Thrown::error(
                "ArgumentCountError",
                format!(
                    "{name}() expects {which} {required} argument{}, {passed} given",
                    if required == 1 { "" } else { "s" }
                ),
            );
        }
        let msg = match self.user_call_site() {
            Some((file, line)) => format!(
                "Too few arguments to function {name}(), {passed} passed in {} on line {line} and {which} {required} expected",
                String::from_utf8_lossy(&file)
            ),
            None => format!(
                "Too few arguments to function {name}(), {passed} passed and {which} {required} expected"
            ),
        };
        Thrown::error("ArgumentCountError", msg)
    }

    /// The `$this` of the current frame, if it is bound to an object.
    pub(crate) fn current_this(&self, top: usize) -> Option<ObjectHandle> {
        let Some(cv) = self.stack[top].proto.this_cv else {
            // A closure whose body has no `$this` slot still has the bound
            // object (traces show `A->{closure}`).
            return self.stack[top].closure.as_ref()?.bound_this.clone();
        };
        match &self.stack[top].regs[cv as usize] {
            Value::Object(h) => Some(h.clone()),
            // Passed to a by-reference-capable argument slot, the register
            // was promoted to a reference cell.
            Value::Ref(rc) => match &*rc.borrow() {
                Value::Object(h) => Some(h.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Synchronously call a method and return its value (re-entrant: used by
    /// magic methods like `__toString`/`__get` mid-opcode). Exceptions thrown
    /// inside are handled within the sub-run; if uncaught there, they propagate.
    pub(crate) fn call_sync(
        &mut self,
        method: &classes::ResolvedMethod,
        this: Option<ObjectHandle>,
        called_class: ClassId,
        args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        self.call_sync_mode(method, this, called_class, args, None)
    }

    /// [`Self::call_sync`] with the caller's `strict_types` mode for the
    /// argument checks (an internal caller is always coercive).
    pub(crate) fn call_sync_mode(
        &mut self,
        method: &classes::ResolvedMethod,
        this: Option<ObjectHandle>,
        called_class: ClassId,
        args: Vec<Value>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        let f = UnitFn {
            proto: method.proto.clone(),
            unit: method.unit,
        };
        self.run_proto_sync(
            f,
            this,
            Some(method.defining_class),
            Some(called_class),
            args,
            caller,
        )
    }

    /// Push a capture frame for `proto` and drive it to completion, returning
    /// its value (re-entrant; exceptions handled within the sub-run). The
    /// call takes the pending magic fetch mode, which then holds for its
    /// own frame only ([`Host::magic_fetch`]).
    pub(crate) fn run_proto_sync(
        &mut self,
        f: UnitFn,
        this: Option<ObjectHandle>,
        defining_class: Option<ClassId>,
        called_class: Option<ClassId>,
        args: Vec<Value>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        let mode = std::mem::take(&mut self.pending_magic_fetch);
        let saved = std::mem::replace(&mut self.magic_fetch, (self.stack.len(), mode));
        let r = self.run_proto_frame(f, this, defining_class, called_class, args, caller);
        self.magic_fetch = saved;
        r
    }

    fn run_proto_frame(
        &mut self,
        f: UnitFn,
        this: Option<ObjectHandle>,
        defining_class: Option<ClassId>,
        called_class: Option<ClassId>,
        args: Vec<Value>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let proto = f.proto;
        let mut frame = Frame::new(proto.clone(), f.unit, 0);
        if let (Some(cv), Some(obj)) = (proto.this_cv, this) {
            frame.regs[cv as usize] = Value::Object(obj);
        }
        frame.defining_class = defining_class;
        frame.called_class = called_class;
        self.bind_frame(&mut frame, Args::Slice(&args), caller)?;
        self.bind_superglobals(&mut frame);
        if proto.is_generator {
            return Ok(self.make_generator(frame));
        }
        frame.capture = true;
        let base = self.stack.len();
        self.stack.push(frame);
        loop {
            match self.run_frames() {
                Ok(v) => return Ok(v),
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        // php's unwind_exit pops the callback's frames, so
                        // what the calling builtin raises next is its own.
                        if matches!(thrown, Thrown::Exit(_)) {
                            self.stack.truncate(base);
                        }
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

    /// Push a capture frame for a closure (captures bound to CVs `0..nc`, params
    /// after) and drive it to completion (re-entrant; exceptions handled within).
    pub(crate) fn run_closure_sync(
        &mut self,
        cd: &Rc<rhp_values::ClosureData>,
        args: Args<'_>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let f = &self.closures[cd.fnidx as usize];
        let proto = f.proto.clone();
        let mut frame = Frame::new(proto.clone(), f.unit, 0);
        for (i, cap) in cd.captures.iter().enumerate() {
            frame.regs[i] = cap.clone();
        }
        // A rebound closure (`Closure::bind`/`bindTo`) overrides the captured
        // `$this` with the new object.
        if let (Some(cv), Some(obj)) = (proto.this_cv, &cd.bound_this) {
            frame.regs[cv as usize] = Value::Object(obj.clone());
        }
        frame.defining_class = cd.bound_class;
        frame.called_class = cd.bound_static;
        frame.closure = Some(cd.clone());
        self.bind_frame(&mut frame, args, caller)?;
        self.bind_superglobals(&mut frame);
        if proto.is_generator {
            return Ok(self.make_generator(frame));
        }
        frame.capture = true;
        let base = self.stack.len();
        self.stack.push(frame);
        loop {
            match self.run_frames() {
                Ok(v) => return Ok(v),
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        // php's unwind_exit pops the callback's frames, so
                        // what the calling builtin raises next is its own.
                        if matches!(thrown, Thrown::Exit(_)) {
                            self.stack.truncate(base);
                        }
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

    /// `lc` names a built-in static method of enum `cls`: `cases()`, and
    /// `from()`/`tryFrom()` of a backed enum.
    fn is_enum_builtin(&self, cls: ClassId, lc: &[u8]) -> bool {
        match lc {
            b"cases" => true,
            b"from" | b"tryfrom" => self.class(cls).enum_backing.is_some(),
            _ => false,
        }
    }

    /// Built-in enum static methods: `cases()`, `from()`, `tryFrom()`,
    /// called from code in strict mode when `strict`.
    fn enum_builtin_static(
        &mut self,
        cls: ClassId,
        method_lc: &[u8],
        args: Vec<Value>,
        strict: bool,
    ) -> Result<Value, Thrown> {
        let want = usize::from(method_lc != b"cases");
        if args.len() != want {
            let name = String::from_utf8_lossy(self.class(cls).name.as_bytes()).into_owned();
            let m = match method_lc {
                b"from" => "from",
                b"tryfrom" => "tryFrom",
                _ => "cases",
            };
            return Err(
                zpp_count_error(&format!("{name}::{m}"), want, Some(want), args.len())
                    .expect("count mismatch"),
            );
        }
        // zend_enum_from_base parses its argument (int for an int-backed
        // enum, else string, or int|string in coercive mode) before it
        // loads the class.
        let mut args = args;
        if let (b"from" | b"tryfrom", Some(backing)) = (method_lc, self.class(cls).enum_backing) {
            let fname = format!(
                "{}::{}",
                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                if method_lc == b"from" {
                    "from"
                } else {
                    "tryFrom"
                }
            );
            let v = args.first().map(deref).unwrap_or(Value::Null);
            let int = backing == rhp_bytecode::EnumBacking::Int;
            let parsed = self.zpp_enum_value(&fname, int, &v, strict)?;
            if let Some(a) = args.first_mut() {
                *a = parsed;
            }
        }
        // Case values may be pending constant expressions: cases() evaluates
        // just the pending case constants (php's zend_enum_cases_func), while
        // from()/tryFrom() load the whole class (building the value table).
        if method_lc == b"cases" {
            let pending: Vec<Vec<u8>> = self
                .class(cls)
                .constants
                .iter()
                .filter(|(_, c)| matches!(c.init, Some(classes::ConstInit::EnumCase(_))))
                .map(|(k, _)| k.clone())
                .collect();
            let display = self.class(cls).name.as_bytes().to_vec();
            for name in pending {
                self.class_const_value(cls, &name, &display, false)?;
            }
        } else {
            self.update_class_constants(cls)?;
        }
        self.touch_enum_cases(cls);
        let enum_name = || String::from_utf8_lossy(self.class(cls).name.as_bytes()).into_owned();
        match method_lc {
            b"cases" => {
                let mut arr = rhp_values::PhpArray::new();
                for obj in &self.class(cls).enum_cases {
                    arr.push(Value::Object(obj.clone()));
                }
                Ok(Value::Array(arr))
            }
            b"from" | b"tryfrom" => {
                let is_try = method_lc == b"tryfrom";
                let backing = self.class(cls).enum_backing.ok_or_else(|| {
                    Thrown::error(
                        "Error",
                        format!(
                            "{}::{} is not a backed enum",
                            enum_name(),
                            if is_try { "tryFrom()" } else { "from()" }
                        ),
                    )
                })?;
                let input = match args.first() {
                    Some(Value::Ref(r)) => r.borrow().clone(),
                    Some(v) => v.clone(),
                    None => Value::Null,
                };
                // The declared parameter type is int|string; other types are a TypeError.
                use rhp_bytecode::EnumBacking;
                let matches = |cv: &Value| -> bool {
                    match (backing, &input) {
                        (EnumBacking::Int, Value::Int(n)) => matches!(cv, Value::Int(m) if m == n),
                        (EnumBacking::Str, Value::Str(s)) => {
                            matches!(cv, Value::Str(cs) if cs.as_bytes() == s.as_bytes())
                        }
                        // Cross-type (e.g. int value against a string backing) never matches.
                        _ => false,
                    }
                };
                let cases = self.class(cls).enum_cases.clone();
                let value_key = rhp_values::PhpStr::from("value");
                for obj in &cases {
                    let val = obj.borrow().properties.get(&value_key).cloned();
                    if let Some(v) = val {
                        if matches(&v) {
                            return Ok(Value::Object(obj.clone()));
                        }
                    }
                }
                if is_try {
                    return Ok(Value::Null);
                }
                // from(): no match -> ValueError.
                let shown = match &input {
                    Value::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
                    other => String::from_utf8_lossy(&conv::to_string_bytes(other)).into_owned(),
                };
                Err(Thrown::error(
                    "ValueError",
                    format!(
                        "{} is not a valid backing value for enum {}",
                        shown,
                        enum_name()
                    ),
                ))
            }
            _ => unreachable!("enum_builtin_static called with {:?}", method_lc),
        }
    }

    /// Invoke a PHP callable value synchronously (string function name,
    /// `[obj_or_class, method]`, an `__invoke` object, or a closure).
    /// Instance methods of a `Closure` value: `bindTo`, `call`, `__invoke`.
    pub(crate) fn closure_method(
        &mut self,
        closure: &Value,
        mname: &[u8],
        mut args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        let lc = mname.to_ascii_lowercase();
        if let Some(e) = native_method_count_error("Closure", &lc, args.len()) {
            return Err(e);
        }
        match lc.as_slice() {
            b"bindto" => {
                let new_this = args.first().cloned().unwrap_or(Value::Null);
                let scope = args.get(1).cloned().unwrap_or_else(|| Value::str("static"));
                self.bind_closure(
                    "Closure::bindTo",
                    1,
                    closure,
                    &new_this,
                    Some(&scope),
                    crate::closures::BindMode::Bind,
                )
            }
            b"call" => {
                // Temporarily binds `$this` AND the scope to the new object.
                let new_this = args.remove(0);
                let bound = self.closure_call_target(closure, &new_this)?;
                if matches!(bound, Value::Null) {
                    return Ok(Value::Null);
                }
                let r = self.call_callable_value(&bound, args);
                self.closure_call_done(closure, &bound);
                r
            }
            b"__invoke" => self.call_callable_value(closure, args),
            _ => Err(Thrown::error(
                "Error",
                format!(
                    "Call to undefined method Closure::{}()",
                    String::from_utf8_lossy(mname)
                ),
            )),
        }
    }

    /// The calling scope a callable resolves against (zend_is_callable's
    /// `scope` / called scope / `$this`): the innermost frame.
    pub(crate) fn callable_scope(
        &self,
    ) -> (Option<ClassId>, Option<ClassId>, Option<ObjectHandle>) {
        if let Some((scope, called)) = self.fcc_scope.get() {
            return (scope, called, None);
        }
        // Prelude (internal) methods act for their caller, as php's internal
        // functions do: the scope is the innermost user frame's.
        let n = self.stack.len();
        let user = (0..n).rev().find(|&i| !self.is_internal_frame(i));
        match user.or(n.checked_sub(1)) {
            Some(top) => (
                self.stack[top].defining_class,
                self.stack[top].called_class,
                self.current_this(top),
            ),
            None => (None, None, None),
        }
    }

    /// The class half of a `Class::method` / `[Class, method]` callable,
    /// with `self`/`parent`/`static` relative to the calling scope. Returns
    /// (class, called class).
    fn callable_class(&self, name: &[u8]) -> Result<(ClassId, ClassId), Vec<u8>> {
        let (scope, called, _) = self.callable_scope();
        let lc = name.to_ascii_lowercase();
        let no_scope =
            |w: &str| format!("cannot access \"{w}\" when no class scope is active").into_bytes();
        let forward = |cls: ClassId| match called {
            Some(c) if self.class(c).ancestors.contains(&cls) => c,
            _ => cls,
        };
        match lc.as_slice() {
            b"self" => scope
                .map(|c| (c, forward(c)))
                .ok_or_else(|| no_scope("self")),
            b"static" => called
                .or(scope)
                .map(|c| (c, c))
                .ok_or_else(|| no_scope("static")),
            b"parent" => {
                let c = scope.ok_or_else(|| no_scope("parent"))?;
                let p = self.class(c).parent.ok_or_else(|| {
                    b"cannot access \"parent\" when current class scope has no parent".to_vec()
                })?;
                Ok((p, forward(p)))
            }
            _ => self
                .resolve_class_by_name(name)
                .map(|c| (c, c))
                .map_err(|_| {
                    let n = name.strip_prefix(b"\\").unwrap_or(name);
                    [b"class \"", n, b"\" not found"].concat()
                }),
        }
    }

    /// The method half of a method callable (zend_is_callable_check_func).
    fn callable_method(
        &self,
        cls: ClassId,
        called: ClassId,
        obj: Option<ObjectHandle>,
        mname: &[u8],
    ) -> Result<Callee, Vec<u8>> {
        let (scope, _, this) = self.callable_scope();
        let cname = || self.class(cls).name.as_bytes();
        let magic = |obj: &Option<ObjectHandle>| -> Option<Callee> {
            let name = mname.to_vec();
            if let Some(o) = obj {
                if let Some(m) = self.lookup_method(cls, b"__call") {
                    return Some(Callee::Magic(m, Some(o.clone()), called, name));
                }
            }
            if let Some(m) = self.lookup_method(cls, b"__callstatic") {
                return Some(Callee::Magic(m, None, called, name));
            }
            None
        };
        // A non-static method called without an object borrows a compatible
        // `$this` from the calling scope.
        let obj =
            obj.or_else(|| this.filter(|h| self.class(h.borrow().class).ancestors.contains(&cls)));
        let Some(m) = self.lookup_method(cls, mname) else {
            if self.class(cls).kind == ClassKind::Enum {
                let lc = mname.to_ascii_lowercase();
                if self.is_enum_builtin(cls, &lc) {
                    return Ok(Callee::EnumStatic(cls, lc));
                }
            }
            return magic(&obj).ok_or_else(|| {
                [
                    b"class ",
                    cname(),
                    b" does not have a method \"",
                    mname,
                    b"\"",
                ]
                .concat()
            });
        };
        if !self.check_access(scope, m.defining_class, m.visibility) {
            if let Some(c) = magic(&obj) {
                return Ok(c);
            }
            let v = match m.visibility {
                rhp_values::Visibility::Private => "private",
                rhp_values::Visibility::Protected => "protected",
                rhp_values::Visibility::Public => "public",
            };
            return Err([
                b"cannot access ",
                v.as_bytes(),
                b" method ",
                cname(),
                b"::",
                m.proto.name.as_bytes(),
                b"()",
            ]
            .concat());
        }
        if m.is_static {
            return Ok(Callee::Method(m, None, called));
        }
        match obj {
            Some(o) => {
                let oc = o.borrow().class;
                Ok(Callee::Method(m, Some(o), oc))
            }
            None => Err([
                b"non-static method ",
                cname(),
                b"::",
                m.proto.name.as_bytes(),
                b"() cannot be called statically",
            ]
            .concat()),
        }
    }

    /// Resolve a callable value (zend_is_callable_ex), or say why it is not
    /// one in zend_is_callable_error's words.
    pub(crate) fn resolve_callable(&self, callable: &Value) -> Result<Callee, Vec<u8>> {
        match deref(callable) {
            Value::Closure(cd) => Ok(Callee::Closure(cd)),
            Value::Str(s) => {
                let b = s.as_bytes();
                // `Class::method` splits at the last "::".
                let split = (1..b.len())
                    .rev()
                    .find(|&i| b[i] == b':' && b[i - 1] == b':');
                if let Some(i) = split {
                    if i == 1 {
                        return Err(b"invalid function name".to_vec());
                    }
                    let (cls, called) = self.callable_class(&b[..i - 1])?;
                    return self.callable_method(cls, called, None, &b[i + 1..]);
                }
                let name = b.strip_prefix(b"\\").unwrap_or(b);
                if let Some(&idx) = self.fn_names.get(&name.to_ascii_lowercase()) {
                    return Ok(Callee::Function(self.functions[idx].clone()));
                }
                let n = String::from_utf8_lossy(name).into_owned();
                // Engine-internal `__*` builtins are not user-visible functions.
                if user_visible_builtin(name) {
                    return Ok(Callee::Builtin(n));
                }
                Err([b"function \"", b, b"\" not found or invalid function name"].concat())
            }
            Value::Array(a) => {
                if a.len() != 2 {
                    return Err(b"array callback must have exactly two members".to_vec());
                }
                let item0 = a.get(&ArrayKey::Int(0)).map(deref);
                let item1 = a.get(&ArrayKey::Int(1)).map(deref);
                let (Some(item0), Some(item1)) = (item0, item1) else {
                    return Err(b"array callback must have exactly two members".to_vec());
                };
                let (cls, called, obj) = match item0 {
                    // zend_closure_get_method: a Closure's `__invoke`.
                    Value::Closure(cd) => {
                        return match &item1 {
                            Value::Str(m) if m.as_bytes().eq_ignore_ascii_case(b"__invoke") => {
                                Ok(Callee::Closure(cd))
                            }
                            Value::Str(m) => Err([
                                &b"class Closure does not have a method \""[..],
                                m.as_bytes(),
                                b"\"",
                            ]
                            .concat()),
                            _ => Err(b"second array member is not a valid method".to_vec()),
                        };
                    }
                    Value::Object(h) => {
                        let c = h.borrow().class;
                        (c, c, Some(h))
                    }
                    Value::Str(cn) => {
                        let (c, called) = self.callable_class(cn.as_bytes())?;
                        (c, called, None)
                    }
                    _ => {
                        return Err(
                            b"first array member is not a valid class name or object".to_vec()
                        )
                    }
                };
                let Value::Str(m) = item1 else {
                    return Err(b"second array member is not a valid method".to_vec());
                };
                self.callable_method(cls, called, obj, m.as_bytes())
            }
            Value::Object(h) => {
                let cls = h.borrow().class;
                match self.lookup_method(cls, b"__invoke") {
                    Some(m) => Ok(Callee::Method(m, Some(h), cls)),
                    None => Err(b"no array or string given".to_vec()),
                }
            }
            _ => Err(b"no array or string given".to_vec()),
        }
    }

    /// Autoload the class half of a callable that names an unloaded class
    /// (php's zend_is_callable looks classes up with autoloading).
    pub(crate) fn autoload_callable_class(&mut self, callable: &Value) -> Result<(), Thrown> {
        let class: Option<Vec<u8>> = match deref(callable) {
            Value::Str(s) => {
                let b = s.as_bytes();
                (2..b.len())
                    .rev()
                    .find(|&i| b[i] == b':' && b[i - 1] == b':')
                    .map(|i| b[..i - 1].to_vec())
            }
            Value::Array(a) if a.len() == 2 => match a.get(&ArrayKey::Int(0)).map(deref) {
                Some(Value::Str(s)) => Some(s.as_bytes().to_vec()),
                _ => None,
            },
            _ => None,
        };
        if let Some(c) = class {
            let lc = c.to_ascii_lowercase();
            if !matches!(lc.as_slice(), b"self" | b"parent" | b"static")
                && self.resolve_class_by_name(&c).is_err()
            {
                self.try_autoload(&c)?;
            }
        }
        Ok(())
    }

    /// Invoke a PHP callable value synchronously (string function name,
    /// `Class::method`, `[obj_or_class, method]`, an `__invoke` object, or a
    /// closure). A value that is not callable raises the Error php's dynamic
    /// call (`$f()`) does.
    pub(crate) fn call_callable_value(
        &mut self,
        callable: &Value,
        args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        self.call_callable_value_mode(callable, args, None)
    }

    /// [`Vm::call_callable_value`] with the caller's mode for the argument
    /// checks: `Some(strict_types)` for a dynamic call from user code, `None`
    /// for a callback invoked by a builtin (always coercive, like php's
    /// internal callers, and no call site in the error).
    pub(crate) fn call_callable_value_mode(
        &mut self,
        callable: &Value,
        args: Vec<Value>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        let c = deref(callable);
        // `[$closure, '__invoke']`: Closure::__invoke is internal, so the
        // closure runs as called from internal code.
        if let Some(cl) = builtins::zend::closures::closure_invoke_of(&c) {
            return self.call_callable_value_mode(&cl, args, None);
        }
        // First-class callable: forward to the wrapped callable.
        if let Value::Closure(cd) = &c {
            if let Some(w) = &cd.wrapped {
                let w = (**w).clone();
                // Resolve it in the scope the callable was created in.
                let saved = self
                    .fcc_scope
                    .replace(Some((cd.bound_class, cd.bound_static)));
                let r = self.call_callable_value_mode(&w, args, caller);
                self.fcc_scope.set(saved);
                return r;
            }
        }
        let fcc = self.fcc_scope.take();
        self.autoload_callable_class(&c)?;
        self.fcc_scope.set(fcc);
        let resolved = self.resolve_callable(&c);
        // A first-class callable's creation scope applies to resolving it
        // only, not to whatever it then runs.
        self.fcc_scope.set(None);
        let callee = match resolved {
            Ok(callee) => callee,
            Err(_) => return Err(self.dynamic_call_error(&c)),
        };
        match callee {
            Callee::Closure(cd) => self.run_closure_sync(&cd, Args::Slice(&args), caller),
            Callee::Function(f) => self.run_proto_sync(f, None, None, None, args, caller),
            Callee::Builtin(name) => self.invoke_builtin(&name, args, caller == Some(true)),
            Callee::Method(m, this, called) => self.call_sync_mode(&m, this, called, args, caller),
            Callee::Magic(m, this, called, name) => {
                let mut arr = PhpArray::new();
                for a in args {
                    arr.push(a);
                }
                self.call_sync_mode(
                    &m,
                    this,
                    called,
                    vec![Value::str(name), Value::Array(arr)],
                    caller,
                )
            }
            Callee::EnumStatic(cls, lc) => {
                self.enum_builtin_static(cls, &lc, args, caller == Some(true))
            }
        }
    }

    /// The Error a dynamic call of a non-callable value raises
    /// (ZEND_INIT_DYNAMIC_CALL).
    fn dynamic_call_error(&self, c: &Value) -> Thrown {
        let err = |m: String| Thrown::error("Error", m);
        let method_err = |cls: ClassId, m: &[u8]| -> Thrown {
            match self.lookup_method(cls, m) {
                Some(rm) if !rm.is_static => err(format!(
                    "Non-static method {}::{}() cannot be called statically",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                    String::from_utf8_lossy(rm.proto.name.as_bytes())
                )),
                Some(rm) => {
                    let v = match rm.visibility {
                        rhp_values::Visibility::Private => "private",
                        rhp_values::Visibility::Protected => "protected",
                        rhp_values::Visibility::Public => "public",
                    };
                    let scope = match self.callable_scope().0 {
                        Some(c) => format!(
                            "scope {}",
                            String::from_utf8_lossy(self.class(c).name.as_bytes())
                        ),
                        None => "global scope".to_string(),
                    };
                    err(format!(
                        "Call to {v} method {}::{}() from {scope}",
                        String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                        String::from_utf8_lossy(rm.proto.name.as_bytes())
                    ))
                }
                None => err(format!(
                    "Call to undefined method {}::{}()",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                    String::from_utf8_lossy(m)
                )),
            }
        };
        let class_err = |n: &[u8]| {
            err(format!(
                "Class \"{}\" not found",
                String::from_utf8_lossy(n.strip_prefix(b"\\").unwrap_or(n))
            ))
        };
        match c {
            Value::Str(s) => {
                let b = s.as_bytes();
                match (1..b.len())
                    .rev()
                    .find(|&i| b[i] == b':' && b[i - 1] == b':')
                {
                    Some(i) => match self.callable_class(&b[..i - 1]) {
                        Ok((cls, _)) => method_err(cls, &b[i + 1..]),
                        Err(_) => class_err(&b[..i - 1]),
                    },
                    None => err(format!(
                        "Call to undefined function {}()",
                        String::from_utf8_lossy(b)
                    )),
                }
            }
            Value::Array(a) if a.len() == 2 => {
                let item0 = a.get(&ArrayKey::Int(0)).map(deref);
                let item1 = a.get(&ArrayKey::Int(1)).map(deref);
                match (item0, item1) {
                    (Some(Value::Object(h)), Some(Value::Str(m))) => {
                        method_err(h.borrow().class, m.as_bytes())
                    }
                    (Some(Value::Str(cn)), Some(Value::Str(m))) => {
                        match self.callable_class(cn.as_bytes()) {
                            Ok((cls, _)) => method_err(cls, m.as_bytes()),
                            Err(_) => class_err(cn.as_bytes()),
                        }
                    }
                    (Some(Value::Object(_) | Value::Str(_)), Some(_)) => {
                        err("Second array member is not a valid method".to_string())
                    }
                    (Some(_), Some(_)) => {
                        err("First array member is not a valid class name or object".to_string())
                    }
                    _ => err("Array callback must have exactly two elements".to_string()),
                }
            }
            Value::Array(_) => err("Array callback must have exactly two elements".to_string()),
            Value::Object(_) => err(format!(
                "Object of type {} is not callable",
                self.debug_class(c)
            )),
            v => err(format!(
                "Value of type {} is not callable",
                rhp_builtins::zval_type_name(self, v)
            )),
        }
    }

    fn debug_class(&self, v: &Value) -> String {
        match v {
            Value::Object(h) => {
                String::from_utf8_lossy(self.class(h.borrow().class).name.as_bytes()).into_owned()
            }
            _ => String::new(),
        }
    }

    /// Pop the current frame and deliver `value`. Returns `Some(value)` when the
    /// outermost frame returns (program done), else delivers to the caller.
    pub(crate) fn do_return(&mut self, value: Value) -> Option<Value> {
        let frame = self.stack.pop().expect("non-empty stack");
        if self
            .ctor_frames
            .last()
            .is_some_and(|c| c.0 == self.stack.len())
        {
            self.ctor_frames.pop();
        }
        if !self.destructibles.is_empty() {
            self.dtor_check = true;
        }
        self.include_writeback(&frame);
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
            None => {
                // {main} finished: keep its globals for the shutdown sequence.
                self.finished_main = Some(frame);
                Some(value)
            }
        }
    }

    /// Enter a user function: bind args to the callee's CV registers and push.
    pub(crate) fn enter_user_call(
        &mut self,
        top: usize,
        callee: UnitFn,
        dst: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        // Raw window: by-ref args arrive as `Value::Ref` and must be kept as
        // references; by-value binding derefs in `bind_params`.
        self.push_user_frame(callee, Args::Window { top, base, argc }, dst)
    }

    /// Push a frame for a user function.
    fn push_user_frame(&mut self, callee: UnitFn, args: Args<'_>, dst: Reg) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let is_generator = callee.proto.is_generator;
        let mut frame = Frame::new(callee.proto, callee.unit, dst);
        self.bind_frame(&mut frame, args, Some(self.caller_strict()))?;
        self.bind_superglobals(&mut frame);
        if is_generator {
            let gen = self.make_generator(frame);
            let caller = self.stack.len() - 1;
            if dst != Reg::MAX {
                self.reg_set(caller, dst, gen);
            }
            return Ok(());
        }
        self.stack.push(frame);
        Ok(())
    }

    /// Split a call's argument array (a spread / named-argument call) into
    /// positional arguments and named ones (string keys).
    pub(crate) fn split_call_args(&self, arr: &Value) -> Result<(Vec<Value>, NamedArgs), Thrown> {
        let mut pos = Vec::new();
        let mut named: NamedArgs = Vec::new();
        if let Value::Array(a) = deref(arr) {
            for (k, v) in a.iter() {
                match k {
                    ArrayKey::Int(_) => {
                        if !named.is_empty() {
                            return Err(Thrown::error(
                                "Error",
                                "Cannot use positional argument after named argument during unpacking",
                            ));
                        }
                        pos.push(v.clone());
                    }
                    ArrayKey::Str(s) => named.push((s.clone(), v.clone())),
                }
            }
        }
        Ok((pos, named))
    }

    /// php's `zend_handle_named_arg` + `zend_handle_undef_args`: place named
    /// arguments into `proto`'s parameter slots. Skipped parameters take
    /// their default (a constant one directly; a non-constant one through the
    /// prologue, via the hole mask); unknown names go to a variadic
    /// parameter. Arms `pending_named` for the frame push that must follow.
    pub(crate) fn place_named_args(
        &mut self,
        proto: &FunctionProto,
        class: Option<ClassId>,
        mut args: Vec<Value>,
        named: Vec<(PhpStr, Value)>,
    ) -> Result<Vec<Value>, Thrown> {
        if named.is_empty() {
            return Ok(args);
        }
        let positional = args.len();
        let variadic = proto.params.iter().position(|p| p.variadic);
        let mut slots: Vec<Option<Value>> = args.drain(..).map(Some).collect();
        let mut extra: Vec<(PhpStr, Value)> = Vec::new();
        for (name, v) in named {
            let idx = proto.params.iter().position(|p| {
                !p.variadic && proto.names[p.name as usize].as_bytes() == name.as_bytes()
            });
            match idx {
                Some(i) => {
                    if i < slots.len() && slots[i].is_some() {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Named parameter ${} overwrites previous argument",
                                String::from_utf8_lossy(name.as_bytes())
                            ),
                        ));
                    }
                    if slots.len() <= i {
                        slots.resize(i + 1, None);
                    }
                    slots[i] = Some(v);
                }
                None if variadic.is_some() => {
                    if extra.iter().any(|(k, _)| k.as_bytes() == name.as_bytes()) {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Named parameter ${} overwrites previous argument",
                                String::from_utf8_lossy(name.as_bytes())
                            ),
                        ));
                    }
                    extra.push((name, v));
                }
                None => {
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Unknown named parameter ${}",
                            String::from_utf8_lossy(name.as_bytes())
                        ),
                    ))
                }
            }
        }
        let mut holes = 0u64;
        let mut out = Vec::with_capacity(slots.len());
        for (i, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(v) => out.push(v),
                None if i < positional => unreachable!("positional slots are filled"),
                None => {
                    let p = &proto.params[i];
                    match &p.default {
                        Some(ParamDefault::Const(k)) => out.push(proto.consts[*k as usize].clone()),
                        Some(ParamDefault::Init) => out.push(Value::Null),
                        None => {
                            let fname = match class {
                                Some(c) => format!(
                                    "{}::{}",
                                    String::from_utf8_lossy(self.class(c).name.as_bytes()),
                                    String::from_utf8_lossy(proto.name.as_bytes())
                                ),
                                None => String::from_utf8_lossy(proto.name.as_bytes()).into_owned(),
                            };
                            return Err(Thrown::error(
                                "ArgumentCountError",
                                format!(
                                    "{}(): Argument #{} (${}) not passed",
                                    fname,
                                    i + 1,
                                    String::from_utf8_lossy(
                                        proto.names[p.name as usize].as_bytes()
                                    )
                                ),
                            ));
                        }
                    }
                    if i < 64 {
                        holes |= 1u64 << i;
                    }
                }
            }
        }
        self.pending_named = Some((holes, extra));
        Ok(out)
    }

    /// Call a callable value with positional and named arguments.
    pub(crate) fn call_named(
        &mut self,
        c: &Value,
        mut args: Vec<Value>,
        named: Vec<(PhpStr, Value)>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        if !named.is_empty() {
            self.autoload_callable_class(c)?;
            let Some((p, cls)) = self.callable_proto(c) else {
                return Err(Thrown::unsupported(
                    "named arguments to a builtin function",
                    "deferred",
                ));
            };
            args = self.place_named_args(&p, cls, args, named)?;
        }
        self.call_callable_value_mode(c, args, caller)
    }

    /// The user-code proto a callable value would run (for named-argument
    /// placement before a dynamic call), with its scope class.
    pub(crate) fn callable_proto(&self, c: &Value) -> Option<(Rc<FunctionProto>, Option<ClassId>)> {
        if let Value::Closure(cd) = deref(c) {
            return match &cd.wrapped {
                Some(w) => {
                    // A first-class callable resolves in its creation scope.
                    let saved = self
                        .fcc_scope
                        .replace(Some((cd.bound_class, cd.bound_static)));
                    let r = self.callable_proto(w);
                    self.fcc_scope.set(saved);
                    r
                }
                None => Some((self.closures[cd.fnidx as usize].proto.clone(), None)),
            };
        }
        match self.resolve_callable(c).ok()? {
            Callee::Function(f) => Some((f.proto, None)),
            Callee::Method(m, _, _) => Some((m.proto.clone(), Some(m.defining_class))),
            _ => None,
        }
    }

    // Out of line: a builtin call costs far more than the call, and keeping
    // it out of run_frames keeps the dispatch loop's code steady.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    pub(crate) fn call_builtin(
        &mut self,
        top: usize,
        builtin: &'static builtins::Builtin,
        sig: Option<&'static rhp_bytecode::signatures::Func>,
        base: Reg,
        argc: u16,
        frameless: bool,
        icall: bool,
    ) -> Result<Value, Thrown> {
        // By-ref builtins (sort/array_push/preg_match/…) need their by-reference
        // positions kept as the `Value::Ref` the call site built, so they can
        // write back to the caller's variable. Other args deref to plain values.
        use rhp_bytecode::signatures::Signature;
        // Arguments in temporaries move to the call (php frees them when the
        // call returns, so their object handles come back then).
        let ncv = self.stack[top].proto.num_cvs as usize;
        let args: Vec<Value> = (0..argc)
            .map(|i| {
                let r = (base + i) as usize;
                let by_ref = sig.is_some_and(|s| s.by_ref(i as usize));
                if r >= ncv {
                    let v = std::mem::replace(&mut self.stack[top].regs[r], Value::Null);
                    match v {
                        Value::Ref(rc) if !by_ref => rc.borrow().clone(),
                        v => v,
                    }
                } else if by_ref {
                    self.stack[top].regs[r].clone()
                } else {
                    self.reg_get(top, base + i)
                }
            })
            .collect();
        // A moved-in object or array dies when the call returns: check for
        // released destructible objects then, as a register overwrite would.
        if !self.destructibles.is_empty()
            && args
                .iter()
                .any(|a| matches!(a, Value::Object(_) | Value::Array(_)))
        {
            self.dtor_check = true;
        }
        let strict = self.stack[top].proto.strict_types;
        self.run_builtin_sig(builtin, sig, args, frameless, icall, strict)
    }

    /// Call a builtin by name with a pre-collected argument vector, its
    /// arguments parsed in strict mode when `strict` (a call from a
    /// `strict_types` file, not from another builtin).
    pub(crate) fn invoke_builtin(
        &mut self,
        name: &str,
        args: Vec<Value>,
        strict: bool,
    ) -> Result<Value, Thrown> {
        self.invoke_builtin_ex(name, args, false, strict)
    }

    /// Call a builtin. Unless `frameless` (php compiled the call to an opcode),
    /// the call is tracked as a native frame while it runs, and an engine error
    /// it raises is materialized with that frame on the trace.
    fn invoke_builtin_ex(
        &mut self,
        name: &str,
        args: Vec<Value>,
        frameless: bool,
        strict: bool,
    ) -> Result<Value, Thrown> {
        let Some(builtin) = builtins::lookup(name) else {
            return Err(Thrown::error(
                "Error",
                format!("Call to undefined function {name}()"),
            ));
        };
        self.run_builtin(builtin, args, frameless, strict)
    }

    /// [`Vm::invoke_builtin_ex`] for an already looked-up builtin.
    fn run_builtin(
        &mut self,
        builtin: &'static builtins::Builtin,
        args: Vec<Value>,
        frameless: bool,
        strict: bool,
    ) -> Result<Value, Thrown> {
        let sig = rhp_bytecode::signatures::function(builtin.name.as_bytes());
        self.run_builtin_sig(builtin, sig, args, frameless, false, strict)
    }

    /// [`Vm::run_builtin`] with the builtin's stub signature.
    fn run_builtin_sig(
        &mut self,
        builtin: &'static builtins::Builtin,
        sig: Option<&'static rhp_bytecode::signatures::Func>,
        mut args: Vec<Value>,
        frameless: bool,
        icall: bool,
        strict: bool,
    ) -> Result<Value, Thrown> {
        // Free streams whose last reference went away (php closes a resource
        // when its refcount drops to zero; what that throws is thrown there).
        self.reap_dropped_streams()?;
        let caller = self.stack.len().saturating_sub(1);
        let user = !self.stack.is_empty() && !self.is_internal_frame(caller);
        // ZEND_DO_FCALL's zend_deprecated_function, before the arguments
        // are parsed (also for a callback or a Reflection invoke).
        if let Some(f) = sig.filter(|f| f.flags & rhp_bytecode::signatures::DEPRECATED != 0) {
            builtins::deprecated_function(self, f.name)?;
        }
        // php's zpp runs in the builtin's frame: a count or parse error is
        // raised as the call's, with the arguments as converted so far.
        let (zpp_error, shown_str) = match builtin_arg_count_error(builtin, args.len()) {
            Some(e) => (Some(e), 0),
            None => {
                match self.coerce_builtin_args(builtin.name, sig, &mut args, user, strict, icall) {
                    Ok(shown) => (None, shown),
                    Err((e, shown)) => (Some(e), shown),
                }
            }
        };
        // A builtin called from engine-internal (prelude) code is not a php
        // frame, and php compiles a few calls to opcodes (no frame either).
        if frameless || !user {
            let r = match zpp_error {
                Some(e) => Err(e),
                None => (builtin.func)(self, &args),
            };
            // ZEND_ARRAY_KEY_EXISTS: zend_illegal_array_offset_access.
            let r = match r {
                Err(Thrown::Error {
                    class: "TypeError",
                    message,
                }) if frameless
                    && message
                        == "array_key_exists(): Argument #1 ($key) must be a valid array offset type" =>
                {
                    Err(Thrown::type_error(format!(
                        "Cannot access offset of type {} on array",
                        builtins::zval_type_name(self, &args[0])
                    )))
                }
                r => r,
            };
            self.flush_stream_diags(builtin.name);
            // (php's opcodes for these have no interrupt check: an
            // out-of-memory error waits for the next one.)
            return match self.pending_throw.take() {
                Some(t) => Err(t),
                None => r,
            };
        }
        self.native_calls.push(NativeCall {
            name: builtin.name,
            args: args.as_slice() as *const [Value],
            shown_str,
            caller,
            frameless_icall: icall,
        });
        let r = match zpp_error {
            Some(e) => Err(e),
            None => (builtin.func)(self, &args),
        };
        // What a stream operation queued without reaching a host.
        self.flush_stream_diags(builtin.name);
        let r = match r {
            Err(e @ Thrown::Error { .. }) => Err(self.materialize(e, None)),
            other => other,
        };
        // An exception a user error handler threw during the call wins.
        let r = match self.pending_throw.take() {
            Some(t) => Err(t),
            None => r,
        };
        // php's ZEND_VM_FCALL_INTERRUPT_CHECK: an out-of-memory error the
        // call ran into is raised now, with the call still on the trace.
        let r = self.oom_interrupt().and(r);
        self.native_calls.pop();
        // `args` outlives the entry just popped (see `NativeCall::args`).
        drop(args);
        r
    }

    /// `New`: instantiate `class` into `dst` and push its constructor frame.
    pub(crate) fn exec_new(
        &mut self,
        top: usize,
        dst: Reg,
        class: ClassRef,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        let cls = self.resolve_class_autoload(top, class)?;
        let entry = self.class(cls);
        if entry.kind != ClassKind::Class || entry.flags.is_abstract {
            let what = if entry.flags.is_abstract {
                "abstract class"
            } else {
                match entry.kind {
                    ClassKind::Enum => "enum",
                    ClassKind::Interface => "interface",
                    ClassKind::Trait => "trait",
                    ClassKind::Class => "class",
                }
            };
            return Err(Thrown::error(
                "Error",
                format!(
                    "Cannot instantiate {} {}",
                    what,
                    String::from_utf8_lossy(entry.name.as_bytes())
                ),
            ));
        }
        if entry.name.as_bytes() == b"Closure" {
            // zend_closure_new
            return Err(Thrown::error(
                "Error",
                "Instantiation of class Closure is not allowed",
            ));
        }
        if self.is_directory_class(cls) {
            // php's Directory get_constructor handler.
            return Err(Thrown::error(
                "Error",
                "Cannot directly construct Directory, use dir() instead",
            ));
        }
        self.check_ctor_access(top, cls)?;
        self.update_class_constants(cls)?;
        let obj = self.new_object(cls);
        self.reg_set(top, dst, Value::Object(obj.clone()));
        // Run __construct, discarding its return (into the arg window).
        if let Some(m) = self.lookup_method(cls, b"__construct") {
            let ptr = Rc::as_ptr(&obj) as usize;
            let tracked = !self.destructibles.is_empty()
                && self
                    .destructibles
                    .last()
                    .is_some_and(|d| Rc::as_ptr(d) as usize == ptr);
            self.enter_method(top, &m, Some(obj), cls, Reg::MAX, base, argc)?;
            if tracked {
                self.ctor_frames.push((self.stack.len() - 1, ptr));
            }
        }
        Ok(())
    }

    /// `CallV`: call `target` (the function `name` resolved to) with the
    /// packed arguments array in `args`. A builtin runs as an ordinary
    /// internal call: a callback it makes is coercive, whatever the caller's
    /// `strict_types`.
    pub(crate) fn call_v(
        &mut self,
        top: usize,
        dst: Reg,
        args: Reg,
        target: CallTarget,
        name: &PhpStr,
    ) -> Result<(), Thrown> {
        let arr = self.reg_get(top, args);
        let (argv, named) = self.split_call_args(&arr)?;
        match target {
            CallTarget::User(i) => {
                let callee = self.functions[i].clone();
                let argv = self.place_named_args(&callee.proto, None, argv, named)?;
                self.push_user_frame(callee, Args::Slice(&argv), dst)?;
            }
            _ if !named.is_empty() => {
                // `call_user_func($f, ...named)` forwards them.
                let cuf = matches!(target, CallTarget::Builtin(b, _) if b.name == "call_user_func");
                if cuf && !argv.is_empty() {
                    let out = self.call_named(&argv[0], argv[1..].to_vec(), named, None)?;
                    self.reg_set(top, dst, out);
                } else {
                    return Err(Thrown::unsupported(
                        "named arguments to a builtin function",
                        "deferred",
                    ));
                }
            }
            CallTarget::Builtin(builtin, sig) => {
                let strict = self.caller_strict();
                let out = self.run_builtin_sig(builtin, sig, argv, false, false, strict)?;
                self.reg_set(top, dst, out);
            }
            CallTarget::Missing => return Err(undefined_function(name)),
        }
        Ok(())
    }
}

/// The engine's helpers behind the prelude classes are the builtins named
/// `__…`. php has no function by such a name, so they exist only for
/// built-in code: a direct call from the prelude reaches them, and nothing
/// else does or shows them (callables, function_exists, is_callable,
/// get_defined_functions, Reflection, first-class callables). A user
/// function may take such a name.
pub(crate) fn is_helper_function(name: &[u8]) -> bool {
    name.starts_with(b"__")
}

/// Whether `name` is a builtin visible to user code (not an engine helper).
pub(crate) fn user_visible_builtin(name: &[u8]) -> bool {
    !is_helper_function(name) && builtins::lookup(&String::from_utf8_lossy(name)).is_some()
}

/// "Call to undefined function NAME()" with the name as written at the call
/// site (namespaced for an unqualified call inside a namespace).
pub(crate) fn undefined_function(name: &rhp_values::PhpStr) -> Thrown {
    Thrown::error(
        "Error",
        format!(
            "Call to undefined function {}()",
            String::from_utf8_lossy(name.as_bytes())
        ),
    )
}

/// Functions whose php body accepts one of two exact argument counts and
/// checks that itself, before any type check.
fn two_form_arity(name: &str) -> Option<[usize; 2]> {
    match name {
        // ZEND_NUM_ARGS() == 0 returns early, then zpp(2, 2).
        "rand" | "mt_rand" => Some([0, 2]),
        // A 2-argument call takes an array $from; the error says "exactly".
        "strtr" => Some([2, 3]),
        _ => None,
    }
}

/// zend_wrong_parameters_count_error: "f() expects exactly 1 argument, 0 given".
/// The bounds are the registration's: php's (the stub's), with the larger
/// minimum the array_u* callback functions check in their bodies.
fn builtin_arg_count_error(b: &builtins::Builtin, n: usize) -> Option<Thrown> {
    if let Some(counts) = two_form_arity(b.name) {
        if !counts.contains(&n) {
            // The smallest non-zero count when below it, else the largest.
            let low = if counts[0] == 0 { counts[1] } else { counts[0] };
            let bound = if n < low { low } else { counts[1] };
            return Some(Thrown::error(
                "ArgumentCountError",
                format!(
                    "{}() expects exactly {bound} argument{}, {n} given",
                    b.name,
                    if bound == 1 { "" } else { "s" }
                ),
            ));
        }
    }
    zpp_count_error(b.name, b.min_args, b.max_args, n)
}

/// zend_wrong_parameters_count_error for `name` with php's bounds.
fn zpp_count_error(name: &str, min: usize, max: Option<usize>, passed: usize) -> Option<Thrown> {
    let exact = max == Some(min);
    let (which, bound) = match max {
        _ if passed < min => (if exact { "exactly" } else { "at least" }, min),
        Some(max) if passed > max => (if exact { "exactly" } else { "at most" }, max),
        _ => return None,
    };
    Some(Thrown::error(
        "ArgumentCountError",
        format!(
            "{name}() expects {which} {bound} argument{}, {passed} given",
            if bound == 1 { "" } else { "s" }
        ),
    ))
}

/// The stub's arity check for a method the VM implements natively
/// (Generator, Closure).
pub(crate) fn native_method_count_error(
    class: &str,
    method: &[u8],
    passed: usize,
) -> Option<Thrown> {
    use rhp_bytecode::signatures::Signature;
    let st = rhp_bytecode::signatures::method(class.as_bytes(), method)?;
    zpp_count_error(
        &format!("{class}::{}", st.name),
        st.min_args(),
        st.max_args(),
        passed,
    )
}

/// The method name of `$o->$m()` / `C::$m()`: php requires a string.
pub(crate) fn dyn_method_name(v: &Value) -> Result<Vec<u8>, Thrown> {
    match deref(v) {
        Value::Str(s) => Ok(s.as_bytes().to_vec()),
        _ => Err(Thrown::error("Error", "Method name must be a string")),
    }
}

/// A `__`-prefixed method name that is not one of php's magic methods.
pub(crate) fn is_engine_helper_name(name: &[u8]) -> bool {
    const MAGIC: &[&[u8]] = &[
        b"__construct",
        b"__destruct",
        b"__call",
        b"__callstatic",
        b"__get",
        b"__set",
        b"__isset",
        b"__unset",
        b"__sleep",
        b"__wakeup",
        b"__serialize",
        b"__unserialize",
        b"__tostring",
        b"__invoke",
        b"__set_state",
        b"__clone",
        b"__debuginfo",
    ];
    let lc = name.to_ascii_lowercase();
    lc.starts_with(b"__") && !MAGIC.contains(&lc.as_slice())
}

/// A magic or ArrayAccess method of a built-in class whose php stub has no
/// such method: the prelude's stand-in for php's property, dimension and
/// `get_method` handlers (SimpleXML, DOM, XMLReader, SPL's forwarding).
pub(crate) fn is_handler_accessor(class: &[u8], method: &[u8]) -> bool {
    const ACCESSORS: &[&[u8]] = &[
        b"__get",
        b"__set",
        b"__isset",
        b"__unset",
        b"__call",
        b"__callstatic",
        b"offsetget",
        b"offsetset",
        b"offsetexists",
        b"offsetunset",
    ];
    ACCESSORS.contains(&method.to_ascii_lowercase().as_slice())
        && rhp_bytecode::signatures::class(class).is_some()
        && rhp_bytecode::signatures::method(class, method).is_none()
}

/// Bind positional `args` to a callee's parameter CV registers (by value), with
/// constant defaults and variadic collection; surplus args (non-variadic callee)
/// are kept in `extra_args`. `Err(i)`: required parameter `i` was not passed.
fn bind_params(
    proto: &FunctionProto,
    n: usize,
    mut arg: impl FnMut(usize, bool) -> Value,
    frame: &mut Frame,
) -> Result<(), usize> {
    // `arg(i, by_ref)`: argument `i`, a by-reference one as passed (the
    // incoming Ref aliasing the caller's variable), else its value.
    let regs = &mut frame.regs;
    for (i, p) in proto.params.iter().enumerate() {
        if p.variadic {
            // Collect the remaining args into an array (by value).
            let mut arr = PhpArray::new();
            for j in i..n {
                arr.push(arg(j, false));
            }
            regs[p.cv as usize] = Value::Array(arr);
            return Ok(()); // variadic is always the last parameter
        } else if i < n {
            regs[p.cv as usize] = arg(i, p.by_ref);
        } else if let Some(def) = &p.default {
            match def {
                ParamDefault::Const(k) => regs[p.cv as usize] = proto.consts[*k as usize].clone(),
                ParamDefault::Init => {} // non-constant defaults: deferred
            }
        } else {
            return Err(i);
        }
    }
    if n > proto.params.len() {
        frame.extra_args = (proto.params.len()..n).map(|j| arg(j, false)).collect();
    }
    Ok(())
}

fn required_params(proto: &FunctionProto) -> usize {
    proto
        .params
        .iter()
        .filter(|p| p.default.is_none() && !p.variadic)
        .count()
}

/// Whether argument `idx` binds to a by-reference parameter of `p` (a
/// by-reference variadic covers every argument from its position on).
pub(crate) fn param_is_ref(p: &FunctionProto, idx: u16) -> bool {
    let i = idx as usize;
    match p.params.get(i) {
        Some(param) => param.by_ref,
        None => p.params.last().is_some_and(|l| l.variadic && l.by_ref),
    }
}
