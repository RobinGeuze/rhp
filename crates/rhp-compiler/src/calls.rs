//! Calls: function, method and static calls, argument passing and by-reference parameters.

use crate::parse::cst::{Access, Argument, Expression, Identifier, Variable};
use mago_span::HasSpan;
use rhp_bytecode::{CallFlags, ClassRef, Instr, NIdx, Reg};
use rhp_values::{Thrown, Value};

use crate::assign::is_globals_var;
use crate::builder::ProtoBuilder;
use crate::file::FileCtx;
use crate::names::NameKind;
use crate::oop::Sel;
use crate::prelude::prelude_classes;
use crate::util::{compile_error, strip_dollar};

impl ProtoBuilder<'_> {
    pub(crate) fn compile_call(&mut self, call: &crate::parse::cst::Call) -> Result<Reg, Thrown> {
        use crate::parse::cst::Call;
        match call {
            Call::Function(fc) => self.compile_function_call(fc),
            Call::Method(mc) => {
                self.compile_method_call(mc.object, &mc.method, &mc.argument_list, false)
            }
            Call::StaticMethod(smc) => self.compile_static_call(smc),
            Call::NullSafeMethod(mc) => {
                self.compile_method_call(mc.object, &mc.method, &mc.argument_list, true)
            }
        }
    }

    /// `throw new \Error($msg)` at the current line; the returned register
    /// is never written.
    fn emit_throw_error(&mut self, msg: &str) -> Reg {
        let base = self.alloc_temps(1);
        let k = self.intern_const(Value::str(msg));
        self.emit(Instr::LoadConst { dst: base, k });
        let class = ClassRef::Name(self.intern_name(b"Error"));
        let dst = self.alloc_temp();
        self.emit(Instr::New {
            dst,
            class,
            base,
            argc: 1,
        });
        self.emit(Instr::Throw { src: dst });
        dst
    }

    fn compile_function_call(
        &mut self,
        fc: &crate::parse::cst::FunctionCall,
    ) -> Result<Reg, Thrown> {
        let result_unused = std::mem::take(&mut self.call_result_unused);
        let callee_id: &Identifier = match fc.function {
            Expression::Identifier(id) => id,
            Expression::ConstantAccess(ca) => &ca.name,
            // `$f(...)`, `($expr)(...)`, `$arr[0](...)`: the callee is a runtime
            // value (closure / string name / `[obj, method]` / __invoke object).
            _ => return self.compile_dynamic_call(fc),
        };
        // Resolve the (possibly namespaced) callee to a primary name + optional
        // global fallback (PHP's fall-back-to-global for unqualified calls).
        let (name_bytes, fallback) = self.cx.resolve_name(callee_id, NameKind::Func);

        // Spread / named-argument call `foo(...$args)` / `foo(b: 1)`: marshal
        // args into an array, emit CallV (the VM places named arguments).
        // php special-compiles none of these (zend_try_compile_special_func
        // gives up on unpacking and named arguments), so no CallFlags.
        if has_spread(&fc.argument_list) || has_named(&fc.argument_list) {
            let known = self.known_params(&name_bytes, &fallback);
            let args = self.build_args_array_known(Some(&fc.argument_list), known.as_ref())?;
            let name = self.intern_name(&name_bytes);
            let fb = self.intern_fallback(&fallback);
            let dst = self.alloc_temp();
            self.emit(Instr::CallV {
                dst,
                name,
                args,
                fallback: fb,
            });
            return Ok(dst);
        }

        if fallback.is_none() && name_bytes.eq_ignore_ascii_case(b"sprintf") {
            if let Some(r) =
                self.sprintf_rope(&fc.argument_list.arguments.iter().collect::<Vec<_>>())?
            {
                return Ok(r);
            }
        }

        // Per-parameter by-ref flags of the (statically-known) callee: a user
        // function's recorded signature (tried under the namespaced name, then the
        // global fallback), or a known by-ref builtin (always global).
        let name_lc = name_bytes.to_ascii_lowercase();
        let global_lc = fallback.as_ref().map(|f| f.to_ascii_lowercase());
        let sig = self
            .cx
            .sigs
            .get(&name_lc)
            .cloned()
            .or_else(|| {
                global_lc
                    .as_ref()
                    .and_then(|g| self.cx.sigs.get(g).cloned())
            })
            .or_else(|| {
                builtin_by_ref(
                    self.cx,
                    global_lc.as_deref().unwrap_or(&name_lc),
                    fc.argument_list.arguments.len(),
                )
            });
        // A user function's parameter names, and a by-reference variadic's
        // later positions.
        let user_meta = self
            .cx
            .sig_names
            .get(&name_lc)
            .or_else(|| global_lc.as_ref().and_then(|g| self.cx.sig_names.get(g)))
            .cloned();
        let sig = match (sig, &user_meta) {
            (Some(mut s), Some((_, true))) if s.last() == Some(&true) => {
                s.resize(s.len().max(fc.argument_list.arguments.len()), true);
                Some(s)
            }
            (s, _) => s,
        };
        // The builtin whose stub gave `sig` (not a user function of the file).
        let builtin = (!self.cx.sigs.contains_key(&name_lc)
            && !global_lc
                .as_ref()
                .is_some_and(|g| self.cx.sigs.contains_key(g)))
        .then(|| builtin_sig(self.cx, global_lc.as_deref().unwrap_or(&name_lc)))
        .flatten();
        // `@prefer-ref` positions of the builtin (array_multisort, extract): a
        // non-variable argument is passed by value, silently.
        let prefer_ref = |i: usize| {
            use rhp_bytecode::signatures::Signature;
            builtin.is_some_and(|f| f.prefer_ref(i))
        };

        // Compile each arg into a value reg, or (for a by-ref param) note the
        // variable's CV so it can be materialized as a reference.
        enum ArgSlot<'e> {
            Value(Reg),
            Ref(Reg),
            /// A by-ref argument that is a property / array-element lvalue.
            RefLval(&'e Expression<'e>),
            /// A call result for a by-ref parameter (passed with a notice).
            NoRef(Reg),
            /// The result of a function that returns by reference.
            RefCall(Reg),
            /// SEND_VAR_EX of a variable to an unresolved function.
            VarEx {
                cv: Reg,
                func: Reg,
                idx: u16,
            },
            /// A reference or a value, passed on as it is.
            Pass(Reg),
        }
        // php compiles `in_array($x, [...], FLAG)` with a literal flag to
        // ZEND_IN_ARRAY, which never type-checks the flag.
        let in_array_flag = self.in_array_ct_strict(fc, &name_bytes, &fallback);
        // The unresolved callee, looked up (ResolveFunc) for the first
        // argument that needs to ask it about by-reference parameters.
        let mut callee: Option<Reg> = None;
        let mut slots = Vec::new();
        for (i, arg) in fc.argument_list.arguments.iter().enumerate() {
            let p = match arg {
                Argument::Positional(p) if p.ellipsis.is_none() => p,
                Argument::Positional(_) => {
                    return Err(Thrown::unsupported(
                        "argument spread",
                        "call-site spread is deferred",
                    ))
                }
                Argument::Named(_) => {
                    return Err(Thrown::unsupported("named arguments", "deferred"))
                }
            };
            let by_ref = sig
                .as_ref()
                .and_then(|s| s.get(i).copied())
                .unwrap_or(false);
            if by_ref {
                match p.value {
                    // `$GLOBALS` is a read-only copy (php 8.1): a temporary.
                    Expression::Variable(Variable::Direct(d))
                        if strip_dollar(d.name) != b"GLOBALS" =>
                    {
                        slots.push(ArgSlot::Ref(self.cv(strip_dollar(d.name))))
                    }
                    // `sort($a['k'])`, `array_push($this->items, …)`.
                    Expression::ArrayAccess(_)
                    | Expression::ArrayAppend(_)
                    | Expression::Access(Access::Property(_))
                    | Expression::Access(Access::StaticProperty(_)) => {
                        slots.push(ArgSlot::RefLval(p.value))
                    }
                    // A call result for a by-ref parameter: php passes it
                    // anyway with "Only variables should be passed by reference"
                    // (ZEND_SEND_VAR_NO_REF / SEND_FUNC_ARG). prefer-ref
                    // parameters accept a temporary silently.
                    Expression::Call(_) => {
                        let r = self.compile_expr(p.value)?;
                        if prefer_ref(i) {
                            slots.push(ArgSlot::Value(r))
                        } else if self.returns_ref_call(p.value) {
                            slots.push(ArgSlot::RefCall(r))
                        } else {
                            slots.push(ArgSlot::NoRef(r))
                        }
                    }
                    _ if prefer_ref(i) => slots.push(ArgSlot::Value(self.compile_expr(p.value)?)),
                    // zend_cannot_pass_by_reference: a catchable Error raised
                    // when the argument is sent, after its expression runs.
                    // A variadic parameter is not named.
                    _ if builtin.is_some() => {
                        let f = builtin.expect("checked");
                        self.compile_expr(p.value)?;
                        let named = f
                            .params
                            .get(i)
                            .filter(|p| !p.variadic())
                            .map(|p| format!(" (${})", p.name))
                            .unwrap_or_default();
                        let msg = format!(
                            "{}(): Argument #{}{named} could not be passed by reference",
                            f.name,
                            i + 1
                        );
                        slots.push(ArgSlot::Value(self.emit_throw_error(&msg)));
                    }
                    // zend_cannot_pass_by_reference, raised at run time.
                    _ => {
                        self.compile_expr(p.value)?;
                        let named = user_meta
                            .as_ref()
                            .filter(|(names, variadic)| !(*variadic && i + 1 >= names.len()))
                            .and_then(|(names, _)| names.get(i))
                            .map(|n| format!(" (${})", String::from_utf8_lossy(n)))
                            .unwrap_or_default();
                        let msg = format!(
                            "{}(): Argument #{}{named} could not be passed by reference",
                            String::from_utf8_lossy(&name_bytes),
                            i + 1
                        );
                        slots.push(ArgSlot::Value(self.emit_throw_error(&msg)));
                    }
                }
            } else if let (2, Some(b)) = (i, in_array_flag) {
                let dst = self.alloc_temp();
                self.emit(Instr::LoadBool { dst, b });
                slots.push(ArgSlot::Value(dst));
            } else if let (None, None, Some(var)) = (&sig, builtin, send_var_ex_name(p.value)) {
                // A function the compiler does not know (another file's, or
                // declared later): php's SEND_VAR_EX asks it at run time
                // whether the parameter is by reference.
                let cv = self.cv(var);
                let func = self.resolve_callee(&mut callee, &name_bytes, &fallback);
                let idx = i as u16;
                if !later_arg_may_write(&fc.argument_list, i) {
                    slots.push(ArgSlot::VarEx { cv, func, idx });
                } else {
                    // a later argument may change the variable: send it now
                    let t = self.alloc_temp();
                    self.emit(Instr::SendVarEx {
                        dst: t,
                        cv,
                        func,
                        idx,
                    });
                    slots.push(ArgSlot::Pass(t));
                }
            } else if sig.is_none()
                && builtin.is_none()
                && matches!(
                    p.value,
                    Expression::ArrayAccess(_)
                        | Expression::ArrayAppend(_)
                        | Expression::Access(Access::Property(_))
                        | Expression::Access(Access::StaticProperty(_))
                )
                && (variable_rooted_lvalue(p.value) || temp_rooted_lvalue(p.value))
            {
                // SEND_FUNC_ARG for an element or property of a variable
                let func = self.resolve_callee(&mut callee, &name_bytes, &fallback);
                let t = self.compile_func_arg(p.value, ArgRefTarget::Func(func), i as u16)?;
                slots.push(ArgSlot::Pass(t));
            } else {
                // A variable's value is taken when its argument is evaluated
                // (php's SEND_VAR), before later arguments can change it.
                // A file function not compiled yet is called by name: `[]`
                // in its argument fails at run time (SEND_FUNC_ARG).
                let late = builtin.is_none() && {
                    use mago_span::HasSpan;
                    let at = fc.span().start.offset;
                    [Some(&name_lc), global_lc.as_ref()]
                        .into_iter()
                        .flatten()
                        .find_map(|n| self.cx.sig_ends.get(n))
                        .is_some_and(|end| *end > at)
                };
                let saved = self.func_arg_read;
                self.func_arg_read |= late;
                let r = self.compile_expr(p.value);
                self.func_arg_read = saved;
                let r = r?;
                let r = self.snapshot_arg(&fc.argument_list, i, p.value, r);
                slots.push(ArgSlot::Value(r));
            }
        }
        let argc = slots.len() as u16;
        let base = self.alloc_temps(argc);
        for (i, slot) in slots.into_iter().enumerate() {
            let dst = base + i as Reg;
            match slot {
                // A temporary holding the argument is dead once it is in
                // the window.
                ArgSlot::Value(r) if r as usize >= self.cv_map.len() => {
                    self.emit(Instr::MovTake { dst, src: r })
                }
                ArgSlot::Value(r) => self.emit(Instr::Mov { dst, src: r }),
                ArgSlot::Ref(cv) => self.emit(Instr::MakeRef { dst, cv }),
                ArgSlot::NoRef(src) => self.emit(Instr::SendNoRef { dst, src }),
                ArgSlot::RefCall(src) => self.emit(Instr::MakeRefVal { dst, src }),
                ArgSlot::RefLval(expr) => {
                    let mut steps = Vec::new();
                    let root = self.collect_lvalue_steps(expr, &mut steps)?;
                    self.emit_cursor(root, &steps);
                    self.emit(Instr::MakeRefLval { dst });
                }
                ArgSlot::VarEx { cv, func, idx } => {
                    self.emit(Instr::SendVarEx { dst, cv, func, idx })
                }
                ArgSlot::Pass(src) => self.emit(Instr::PassArg { dst, src }),
            }
        }
        let mut flags = call_flags(&name_bytes, fallback.as_deref(), argc as usize);
        if result_unused {
            flags.insert(CallFlags::RESULT_UNUSED);
        }
        let name = self.intern_name(&name_bytes);
        let fb = self.intern_fallback(&fallback);
        let dst = self.alloc_temp();
        self.emit(Instr::Call {
            dst,
            name,
            base,
            argc,
            fallback: fb,
            flags,
        });
        Ok(dst)
    }

    /// zend_compile_func_sprintf: `sprintf` with a constant format of only
    /// `%s`, `%d` and `%%` (shorter than 256 bytes) and one argument per
    /// placeholder compiles to string concatenation (php's rope): the
    /// values are evaluated first, `%d` casts to int. `None` when the call
    /// doesn't have that shape.
    fn sprintf_rope(&mut self, args: &[&Argument]) -> Result<Option<Reg>, Thrown> {
        let Some(Argument::Positional(first)) = args.first().copied() else {
            return Ok(None);
        };
        let Ok(Value::Str(fmt)) = crate::fold::fold_const(self.cx, first.value) else {
            return Ok(None);
        };
        let fmt = fmt.as_bytes().to_vec();
        if fmt.len() >= 256 {
            return Ok(None);
        }
        // Literal runs and placeholders ('s' / 'd').
        enum Part {
            Lit(Vec<u8>),
            Val(u8),
        }
        let mut parts = Vec::new();
        let mut lit = Vec::new();
        let mut i = 0;
        while i < fmt.len() {
            if fmt[i] != b'%' {
                lit.push(fmt[i]);
                i += 1;
                continue;
            }
            match fmt.get(i + 1) {
                Some(b'%') => lit.push(b'%'),
                Some(&c @ (b's' | b'd')) => {
                    if !lit.is_empty() {
                        parts.push(Part::Lit(std::mem::take(&mut lit)));
                    }
                    parts.push(Part::Val(c));
                }
                _ => return Ok(None),
            }
            i += 2;
        }
        if !lit.is_empty() {
            parts.push(Part::Lit(lit));
        }
        let placeholders = parts.iter().filter(|p| matches!(p, Part::Val(_))).count();
        if placeholders != args.len() - 1 {
            return Ok(None);
        }
        let mut values = Vec::new();
        for a in &args[1..] {
            let Argument::Positional(p) = *a else {
                return Ok(None);
            };
            values.push(p.value);
        }
        let mut regs = Vec::new();
        for v in values {
            regs.push(self.compile_expr(v)?);
        }
        let mut regs = regs.into_iter();
        let mut acc: Option<Reg> = None;
        let single = parts.len() == 1;
        for part in parts {
            let r = match part {
                Part::Lit(b) => {
                    let dst = self.alloc_temp();
                    let k = self.intern_const(Value::str(b));
                    self.emit(Instr::LoadConst { dst, k });
                    dst
                }
                Part::Val(c) => {
                    let mut r = regs.next().expect("one value per placeholder");
                    if c == b'd' {
                        let dst = self.alloc_temp();
                        self.emit(Instr::CastInt { dst, a: r });
                        r = dst;
                    }
                    // A one-element rope is a string cast.
                    if single {
                        let dst = self.alloc_temp();
                        self.emit(Instr::CastString { dst, a: r });
                        r = dst;
                    }
                    r
                }
            };
            acc = Some(match acc {
                None => r,
                Some(a) => {
                    let dst = self.alloc_temp();
                    self.emit(Instr::Concat { dst, a, b: r });
                    dst
                }
            });
        }
        let dst = match acc {
            Some(r) => r,
            None => {
                let dst = self.alloc_temp();
                let k = self.intern_const(Value::str(""));
                self.emit(Instr::LoadConst { dst, k });
                dst
            }
        };
        Ok(Some(dst))
    }

    /// zend_compile_func_in_array: a compile-time-resolved `in_array` call
    /// whose haystack is a constant array of only strings and ints (strict)
    /// or only non-numeric strings (not strict), and whose third argument
    /// is a literal or a compile-time constant, becomes ZEND_IN_ARRAY with
    /// the flag's truth value. That value, when the call has this shape.
    fn in_array_ct_strict(
        &self,
        fc: &crate::parse::cst::FunctionCall,
        name: &[u8],
        fallback: &Option<Vec<u8>>,
    ) -> Option<bool> {
        let args = &fc.argument_list.arguments;
        if fallback.is_some() || !name.eq_ignore_ascii_case(b"in_array") || args.len() != 3 {
            return None;
        }
        let arg = |i: usize| match &args[i] {
            Argument::Positional(p) => Some(unparen(p.value)),
            Argument::Named(_) => None,
        };
        let flag = match arg(2)? {
            Expression::Literal(l) => crate::fold::literal_value(l),
            Expression::ConstantAccess(ca) => self.ct_constant(&ca.name)?,
            _ => return None,
        };
        // zend_is_true's warning, raised while compiling, whether or not
        // the call then folds.
        if matches!(flag, Value::Float(f) if f.is_nan()) {
            self.cx.compile_diags.borrow_mut().push((
                rhp_values::E_WARNING,
                "unexpected NAN value was coerced to bool".into(),
                self.cur_line,
            ));
        }
        let strict = rhp_values::conv::to_bool(&flag);
        self.in_array_haystack_ok(arg(1)?, strict).then_some(strict)
    }

    /// zend_compile_func_in_array's haystack: a constant array of only
    /// strings and ints (strict) or only non-numeric strings.
    pub(crate) fn in_array_haystack_ok(&self, haystack: &Expression, strict: bool) -> bool {
        if !matches!(haystack, Expression::Array(_) | Expression::LegacyArray(_)) {
            return false;
        }
        let Ok(Value::Array(a)) = crate::fold::fold_const(self.cx, haystack) else {
            return false;
        };
        a.iter().all(|(_, v)| match v {
            Value::Int(_) => strict,
            Value::Str(s) => strict || rhp_values::conv::numeric_string(s.as_bytes()).is_none(),
            _ => false,
        })
    }

    /// zend_try_ct_eval_const: `true`/`false`/`null` in any case, or a
    /// global builtin constant that is not deprecated.
    pub(crate) fn ct_constant(&self, id: &Identifier) -> Option<Value> {
        let namespaced_fq =
            matches!(id, Identifier::FullyQualified(f) if f.value[1..].contains(&b'\\'));
        if !namespaced_fq {
            match crate::names::identifier_name(id)
                .to_ascii_lowercase()
                .as_slice()
            {
                b"true" => return Some(Value::Bool(true)),
                b"false" => return Some(Value::Bool(false)),
                b"null" => return Some(Value::Null),
                _ => {}
            }
        }
        let (name, fallback) = self.cx.resolve_name(id, NameKind::Const);
        if fallback.is_some() {
            return None;
        }
        (self.cx.builtins.constant)(&name)
    }

    /// `$f(args)` — call a runtime callable value. The callee is evaluated to a
    /// register; arguments are passed by value (the signature is unknown at the
    /// call site, so by-ref parameters are not honoured for dynamic calls).
    fn compile_dynamic_call(
        &mut self,
        fc: &crate::parse::cst::FunctionCall,
    ) -> Result<Reg, Thrown> {
        // A string literal names the function as written, fully qualified
        // (zend_compile_dynamic_call: INIT_FCALL_BY_NAME, leading `\` cut).
        let mut lit = fc.function;
        while let Expression::Parenthesized(p) = lit {
            lit = p.expression;
        }
        let callee = match lit {
            Expression::Literal(crate::parse::cst::Literal::String(s))
                if s.value
                    .is_some_and(|v| v.starts_with(b"\\") && !v.windows(2).any(|w| w == b"::")) =>
            {
                let v = s.value.expect("checked");
                self.load_string_const(&v[1..])
            }
            _ => self.compile_expr(fc.function)?,
        };

        if has_spread(&fc.argument_list) || has_named(&fc.argument_list) {
            let args = self.build_args_array(Some(&fc.argument_list))?;
            let dst = self.alloc_temp();
            self.emit(Instr::CallDynV { dst, callee, args });
            return Ok(dst);
        }

        // The callee is only known at run time, so (as for method calls) a
        // simple-variable argument travels as a reference; a by-value
        // parameter dereferences it.
        let (base, argc) = self.compile_arg_window_t(
            Some(&fc.argument_list),
            &[],
            true,
            Some(ArgRefTarget::Value(callee)),
        )?;
        let dst = self.alloc_temp();
        self.emit(Instr::CallDyn {
            dst,
            callee,
            base,
            argc,
        });
        Ok(dst)
    }

    /// First-class callable syntax: `f(...)`, `$o->m(...)`, `C::m(...)`. Builds
    /// the underlying callable value, then wraps it as a `Closure` via `MakeFcc`.
    pub(crate) fn compile_fcc(
        &mut self,
        pa: &crate::parse::cst::PartialApplication,
    ) -> Result<Reg, Thrown> {
        use crate::parse::cst::PartialApplication as Pa;
        if !pa.is_first_class_callable() {
            return Err(Thrown::unsupported(
                "partial application with placeholders",
                "only the first-class callable form `f(...)` is supported",
            ));
        }
        // Build the callable Value into `callee`.
        let mut ns_name = None;
        let callee = match pa {
            Pa::Function(f) => match f.function {
                // Named function: the callable is its (resolved) name string.
                // `die` is the exit keyword (T_EXIT): `die(...)` is exit's.
                Expression::Identifier(Identifier::Local(l))
                    if l.value.eq_ignore_ascii_case(b"die") =>
                {
                    self.load_string_const(b"exit")
                }
                Expression::Identifier(id) => {
                    let (n, ns) = self.resolve_callable_name(id);
                    ns_name = ns;
                    self.load_string_const(&n)
                }
                Expression::ConstantAccess(ca) if matches!(&ca.name, Identifier::Local(l) if l.value.eq_ignore_ascii_case(b"die")) => {
                    self.load_string_const(b"exit")
                }
                Expression::ConstantAccess(ca) => {
                    let (n, ns) = self.resolve_callable_name(&ca.name);
                    ns_name = ns;
                    self.load_string_const(&n)
                }
                // Dynamic `$f(...)`: wrap the existing callable value.
                other => self.compile_expr(other)?,
            },
            Pa::Method(m) => {
                if crate::assign::is_short_circuited(m.object) {
                    return Err(compile_error(
                        "Cannot combine nullsafe operator with Closure creation",
                        self.cur_line,
                    ));
                }
                let obj = self.compile_expr(m.object)?;
                let name = self.selector_string(&m.method)?;
                self.build_pair(obj, &name)
            }
            Pa::StaticMethod(s) if is_parent_hook_call(s.class, &s.method) => {
                use mago_span::HasSpan;
                self.cx.fatal(
                    s.span().start.offset,
                    "Cannot create Closure for parent property hook call".into(),
                );
                self.alloc_temp()
            }
            Pa::StaticMethod(s) => {
                let class = match s.class {
                    Expression::Identifier(id) => {
                        let n = self.cx.resolve_class_name(id);
                        self.load_string_const(&n)
                    }
                    Expression::ConstantAccess(ca) => {
                        let n = self.cx.resolve_class_name(&ca.name);
                        self.load_string_const(&n)
                    }
                    // Resolved against the creation scope when called.
                    Expression::Self_(_) => self.load_string_const(b"self"),
                    Expression::Static(_) => self.load_string_const(b"static"),
                    Expression::Parent(_) => self.load_string_const(b"parent"),
                    other => self.compile_expr(other)?,
                };
                let name = self.selector_string(&s.method)?;
                self.build_pair(class, &name)
            }
        };
        let ns_name = self.intern_fallback(&ns_name);
        let dst = self.alloc_temp();
        self.emit(Instr::MakeFcc {
            dst,
            callee,
            ns_name,
        });
        Ok(dst)
    }

    /// Build a `PhpArray` of the call arguments (used when the call has a
    /// spread `...$x`): positional args are pushed, spreads are merged.
    pub(crate) fn build_args_array(
        &mut self,
        args: Option<&crate::parse::cst::ArgumentList>,
    ) -> Result<Reg, Thrown> {
        self.build_args_array_known(args, None)
    }

    /// The parameters of a function the compiler knows by name (a function
    /// of this file, or a builtin), for by-reference named arguments.
    fn known_params(&self, name: &[u8], fallback: &Option<Vec<u8>>) -> Option<KnownParams> {
        let name_lc = name.to_ascii_lowercase();
        let global_lc = fallback.as_ref().map(|f| f.to_ascii_lowercase());
        for key in std::iter::once(&name_lc).chain(global_lc.as_ref()) {
            if let (Some(by_ref), Some((names, variadic))) =
                (self.cx.sigs.get(key), self.cx.sig_names.get(key))
            {
                return Some(KnownParams {
                    fname: String::from_utf8_lossy(name).into_owned(),
                    names: names.clone(),
                    by_ref: by_ref.clone(),
                    prefer_ref: vec![false; by_ref.len()],
                    variadic: *variadic,
                });
            }
        }
        let f = builtin_sig(self.cx, global_lc.as_deref().unwrap_or(&name_lc))?;
        Some(KnownParams {
            fname: f.name.to_string(),
            names: f
                .params
                .iter()
                .map(|p| p.name.as_bytes().to_vec())
                .collect(),
            by_ref: f.params.iter().map(|p| p.by_ref()).collect(),
            prefer_ref: f.params.iter().map(|p| p.prefer_ref()).collect(),
            variadic: f.params.last().is_some_and(|p| p.variadic()),
        })
    }

    /// [`ProtoBuilder::build_args_array`] for a call to a known function:
    /// an argument for a by-reference parameter (found by position, or by
    /// name for a named one) travels as a reference.
    fn build_args_array_known(
        &mut self,
        args: Option<&crate::parse::cst::ArgumentList>,
        known: Option<&KnownParams>,
    ) -> Result<Reg, Thrown> {
        if let Some(list) = args {
            self.check_arg_order(list);
        }
        let dst = self.alloc_temp();
        self.emit(Instr::NewArray { dst, cap: 0 });
        if let Some(list) = args {
            let mut names: Vec<&[u8]> = Vec::new();
            let (mut pos, mut unpacked) = (0usize, false);
            for (ai, arg) in list.arguments.iter().enumerate() {
                // An unknown callee gets a variable by value when a later
                // argument may change it (php sends it before that runs).
                let late = known.is_none() && later_arg_may_write(list, ai);
                match arg {
                    // A repeated name: zend_handle_named_arg finds the slot
                    // taken when the second one is sent.
                    Argument::Named(n) if names.contains(&n.name.value) => {
                        self.compile_expr(n.value)?;
                        let msg = format!(
                            "Named parameter ${} overwrites previous argument",
                            String::from_utf8_lossy(n.name.value)
                        );
                        self.emit_throw_error(&msg);
                    }
                    Argument::Positional(p) if p.ellipsis.is_none() && !unpacked => {
                        let idx = known.and_then(|k| k.param(Some(pos), None));
                        pos += 1;
                        match self.compile_known_ref_arg(p.value, known, idx, late)? {
                            Some(r) => self.emit(Instr::ArrPushRef { arr: dst, val: r }),
                            None => {
                                let r = self.compile_expr(p.value)?;
                                self.emit(Instr::ArrPush { arr: dst, val: r });
                            }
                        }
                    }
                    Argument::Positional(p) => {
                        unpacked |= p.ellipsis.is_some();
                        let r = self.compile_expr(p.value)?;
                        if p.ellipsis.is_some() {
                            self.emit(Instr::ArgSpread { arr: dst, src: r });
                        } else {
                            self.emit(Instr::ArrPush { arr: dst, val: r });
                        }
                    }
                    // A named argument travels under its name (the callee's
                    // parameter list places it at run time).
                    Argument::Named(n) => {
                        names.push(n.name.value);
                        // After an unpack php no longer knows the callee.
                        let known = known.filter(|_| !unpacked);
                        let idx = known.and_then(|k| k.param(None, Some(n.name.value)));
                        let by_ref = self.compile_known_ref_arg(n.value, known, idx, late)?;
                        let r = match by_ref {
                            Some(r) => r,
                            None => self.compile_expr(n.value)?,
                        };
                        let k = self.intern_const(Value::str(n.name.value.to_vec()));
                        let key = self.alloc_temp();
                        self.emit(Instr::LoadConst { dst: key, k });
                        if unpacked {
                            // The unpack may have passed this name: add it
                            // as an unpack of its own, which checks that.
                            let one = self.alloc_temp();
                            self.emit(Instr::NewArray { dst: one, cap: 1 });
                            self.emit(Instr::ArrSet {
                                arr: one,
                                key,
                                val: r,
                            });
                            self.emit(Instr::ArgSpread { arr: dst, src: one });
                        } else if by_ref.is_some() {
                            self.emit(Instr::ArrSetRef {
                                arr: dst,
                                key,
                                val: r,
                            });
                        } else {
                            self.emit(Instr::ArrSet {
                                arr: dst,
                                key,
                                val: r,
                            });
                        }
                    }
                }
            }
        }
        Ok(dst)
    }

    /// An argument for by-reference parameter `idx` of a known callee, or
    /// for an unknown callee (`known` None), as a register holding a
    /// reference; `None` when it goes by value.
    /// php's SEND_REF for a variable; SEND_VAR_NO_REF for a call result
    /// (by value, with a notice); SEND_VAL for a temporary, which throws.
    fn compile_known_ref_arg(
        &mut self,
        e: &Expression,
        known: Option<&KnownParams>,
        idx: Option<usize>,
        later_write: bool,
    ) -> Result<Option<Reg>, Thrown> {
        let Some(k) = known else {
            // A callee only known at run time: as in a method's argument
            // window, a plain variable travels as a reference and a
            // by-value parameter dereferences it.
            let Some(var) = send_var_ex_name(e).filter(|_| !later_write) else {
                return Ok(None);
            };
            let cv = self.cv(var);
            let dst = self.alloc_temp();
            self.emit(Instr::MakeRef { dst, cv });
            return Ok(Some(dst));
        };
        let Some(i) = idx.filter(|&i| k.by_ref[i]) else {
            return Ok(None);
        };
        let dst = self.alloc_temp();
        match e {
            Expression::Variable(Variable::Direct(d))
                if !matches!(strip_dollar(d.name), b"GLOBALS" | b"this") =>
            {
                let cv = self.cv(strip_dollar(d.name));
                self.emit(Instr::MakeRef { dst, cv });
            }
            Expression::ArrayAccess(_)
            | Expression::ArrayAppend(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(e, &mut steps)?;
                self.emit_cursor(root, &steps);
                self.emit(Instr::MakeRefLval { dst });
            }
            _ if k.prefer_ref[i] => return Ok(None),
            Expression::Call(_) => {
                let src = self.compile_expr(e)?;
                if self.returns_ref_call(e) {
                    self.emit(Instr::MakeRefVal { dst, src });
                } else {
                    self.emit(Instr::SendNoRef { dst, src });
                }
            }
            _ => {
                self.compile_expr(e)?;
                let named = if k.variadic && i + 1 == k.names.len() {
                    String::new()
                } else {
                    format!(" (${})", String::from_utf8_lossy(&k.names[i]))
                };
                let msg = format!(
                    "{}(): Argument #{}{named} could not be passed by reference",
                    k.fname,
                    i + 1
                );
                self.emit_throw_error(&msg);
            }
        }
        Ok(Some(dst))
    }

    /// Whether `e` calls a function of this file that returns by reference
    /// (SEND_VAR_NO_REF finds a reference then: no notice).
    fn returns_ref_call(&self, e: &Expression) -> bool {
        let Expression::Call(crate::parse::cst::Call::Function(fc)) = e else {
            return false;
        };
        let id = match fc.function {
            Expression::Identifier(id) => id,
            Expression::ConstantAccess(ca) => &ca.name,
            _ => return false,
        };
        let (name, fallback) = self.cx.resolve_name(id, NameKind::Func);
        let lc = name.to_ascii_lowercase();
        if self.cx.sigs.contains_key(&lc) {
            return self.cx.ref_returning.contains(&lc);
        }
        fallback.is_some_and(|f| self.cx.ref_returning.contains(&f.to_ascii_lowercase()))
    }

    /// zend_compile_args' compile-time order checks: no positional argument
    /// after a named one or an unpack, no unpack after a named argument.
    pub(crate) fn check_arg_order(&self, list: &crate::parse::cst::ArgumentList) {
        let (mut named, mut unpack) = (false, false);
        // php reports the line it last compiled: the previous argument's.
        let mut prev = list.left_parenthesis.start.offset;
        for arg in list.arguments.iter() {
            let at = std::mem::replace(&mut prev, arg.span().start.offset);
            let msg = match arg {
                Argument::Named(_) => {
                    named = true;
                    continue;
                }
                Argument::Positional(p) if p.ellipsis.is_some() => {
                    if !named {
                        unpack = true;
                        continue;
                    }
                    "Cannot use argument unpacking after named arguments"
                }
                Argument::Positional(_) if unpack => {
                    "Cannot use positional argument after argument unpacking"
                }
                Argument::Positional(_) if named => {
                    "Cannot use positional argument after named argument"
                }
                Argument::Positional(_) => continue,
            };
            self.cx.fatal(at, msg.to_string());
            return;
        }
    }

    /// Marshal positional args into a contiguous register window.
    /// Compile a method/constructor argument list into a contiguous window.
    /// `ref_mask[i]` true means position `i` may be by-reference for some method
    /// of this name: if the argument is a simple variable, pass it as a `Ref`
    /// (by-value callees deref it in `bind_params`).
    /// `over_approx`: pass every simple-variable argument by reference (used for
    /// method / constructor calls, where the callee may live in another file so
    /// its by-ref signature is unknown at compile time; a by-value parameter just
    /// dereferences a copy, so this is safe and matches php's runtime by-ref).
    fn compile_arg_window_ex(
        &mut self,
        args: Option<&crate::parse::cst::ArgumentList>,
        ref_mask: &[bool],
        over_approx: bool,
    ) -> Result<(Reg, u16), Thrown> {
        self.compile_arg_window_t(args, ref_mask, over_approx, None)
    }

    /// [`compile_arg_window_ex`] for a call whose callee is resolved at run
    /// time: a property / array-element argument asks the callee (`target`)
    /// whether that parameter is by-reference (php's SEND_FUNC_ARG) and is
    /// then passed as a reference to the lvalue, else by value.
    pub(crate) fn compile_arg_window_t(
        &mut self,
        args: Option<&crate::parse::cst::ArgumentList>,
        ref_mask: &[bool],
        over_approx: bool,
        target: Option<ArgRefTarget>,
    ) -> Result<(Reg, u16), Thrown> {
        enum Slot {
            Value(Reg),
            Ref(Reg),
        }
        let mut slots = Vec::new();
        if let Some(list) = args {
            for (i, arg) in list.arguments.iter().enumerate() {
                let p = match arg {
                    Argument::Positional(p) if p.ellipsis.is_none() => p,
                    Argument::Positional(_) => {
                        return Err(Thrown::unsupported("argument spread", "deferred"))
                    }
                    Argument::Named(_) => {
                        return Err(Thrown::unsupported("named arguments", "deferred"))
                    }
                };
                if let Expression::Variable(Variable::Direct(d)) = p.value {
                    if (over_approx || ref_mask.get(i).copied().unwrap_or(false))
                        && !matches!(strip_dollar(d.name), b"GLOBALS" | b"this")
                    {
                        slots.push(Slot::Ref(self.cv(strip_dollar(d.name))));
                        continue;
                    }
                }
                if let Some(t) = target {
                    if matches!(
                        p.value,
                        Expression::ArrayAccess(_)
                            | Expression::ArrayAppend(_)
                            | Expression::Access(Access::Property(_))
                            | Expression::Access(Access::StaticProperty(_))
                    ) && (variable_rooted_lvalue(p.value) || temp_rooted_lvalue(p.value))
                    {
                        let r = self.compile_func_arg(p.value, t, i as u16)?;
                        slots.push(Slot::Ref(r));
                        continue;
                    }
                }
                let r = self.compile_expr(p.value)?;
                let r = self.snapshot_arg(list, i, p.value, r);
                slots.push(Slot::Value(r));
            }
        }
        let argc = slots.len() as u16;
        let base = self.alloc_temps(argc);
        for (i, slot) in slots.into_iter().enumerate() {
            let dst = base + i as Reg;
            match slot {
                // A temporary holding the argument is dead once it is in
                // the window.
                Slot::Value(r) if r as usize >= self.cv_map.len() => {
                    self.emit(Instr::MovTake { dst, src: r })
                }
                Slot::Value(r) => self.emit(Instr::Mov { dst, src: r }),
                Slot::Ref(cv) => self.emit(Instr::MakeArgRef { dst, cv }),
            }
        }
        Ok((base, argc))
    }

    /// One SEND_FUNC_ARG argument: `$t = callee-param-is-ref ? &lvalue :
    /// lvalue` into a temp (the caller passes it on with `MakeRef`, which
    /// a by-value parameter dereferences).
    fn compile_func_arg(
        &mut self,
        e: &Expression,
        target: ArgRefTarget,
        idx: u16,
    ) -> Result<Reg, Thrown> {
        let flag = self.alloc_temp();
        match target {
            ArgRefTarget::Method(obj, name) => self.emit(Instr::MethodArgIsRef {
                dst: flag,
                obj,
                name,
                idx,
            }),
            ArgRefTarget::Static(class, name) => self.emit(Instr::StaticArgIsRef {
                dst: flag,
                class,
                name,
                idx,
            }),
            ArgRefTarget::Func(func) => self.emit(Instr::FuncArgIsRef {
                dst: flag,
                func,
                idx,
            }),
            ArgRefTarget::Value(callee) => self.emit(Instr::ValueArgIsRef {
                dst: flag,
                callee,
                idx,
            }),
        }
        let t = self.alloc_temp();
        let jf = self.code.len();
        self.emit(Instr::Nop); // JmpFalse flag -> by_value
        if temp_rooted_lvalue(e) {
            let k = self.intern_const(Value::str(
                "Cannot use temporary expression in write context",
            ));
            self.emit(Instr::ThrowError { k });
        } else {
            let mut steps = Vec::new();
            let root = self.collect_lvalue_steps(e, &mut steps)?;
            self.emit_cursor(root, &steps);
            self.emit(Instr::MakeRefLval { dst: t });
        }
        let jend = self.code.len();
        self.emit(Instr::Nop); // Jmp -> end
        let by_value = self.here();
        self.code[jf] = Instr::JmpFalse {
            cond: flag,
            target: by_value,
        };
        // FETCH_DIM_FUNC_ARG reading: `[]` fails when the call runs.
        let saved = std::mem::replace(&mut self.func_arg_read, true);
        let v = self.compile_expr(e);
        self.func_arg_read = saved;
        self.emit(Instr::Mov { dst: t, src: v? });
        let end = self.here();
        self.code[jend] = Instr::Jmp { target: end };
        Ok(t)
    }

    /// A by-value argument that is a plain variable compiles to the variable's
    /// own register, which is only read when the argument window is filled
    /// after every argument ran. Snapshot it into a temp now when a later
    /// argument could change it (`f($m, g($m))` with a by-ref `g`): php
    /// evaluates arguments strictly left to right.
    fn snapshot_arg(
        &mut self,
        list: &crate::parse::cst::ArgumentList,
        i: usize,
        expr: &Expression,
        r: Reg,
    ) -> Reg {
        if !later_arg_may_write(list, i)
            || !matches!(expr, Expression::Variable(Variable::Direct(_)))
        {
            return r;
        }
        let t = self.alloc_temp();
        self.emit(Instr::Mov { dst: t, src: r });
        t
    }

    /// The by-ref mask for a method/constructor name (empty if none).
    pub(crate) fn method_ref_mask(&self, name_lc: &[u8]) -> Vec<bool> {
        self.cx.method_ref_mask(name_lc)
    }

    /// The by-ref mask for an interned method name.
    fn name_ref_mask(&self, name: NIdx) -> Vec<bool> {
        let lc = self.names[name as usize].as_bytes().to_ascii_lowercase();
        self.method_ref_mask(&lc)
    }

    fn compile_method_call(
        &mut self,
        object: &Expression,
        selector: &crate::parse::cst::ClassLikeMemberSelector,
        args: &crate::parse::cst::ArgumentList,
        null_safe: bool,
    ) -> Result<Reg, Thrown> {
        self.check_method_name(selector)?;
        let obj = self.compile_expr(object)?;
        // `$o?->m(...)`: a null object makes the whole chain null, without
        // evaluating the arguments.
        if null_safe {
            self.nullsafe_guard(obj);
        }
        {
            match self.compile_selector(selector)? {
                Sel::Static(name) => {
                    if has_spread(args) || has_named(args) {
                        let args = self.build_args_array(Some(args))?;
                        let dst = self.alloc_temp();
                        self.emit(Instr::CallMethodV {
                            dst,
                            obj,
                            name,
                            args,
                        });
                        return Ok(dst);
                    }
                    let mask = self.name_ref_mask(name);
                    let (base, argc) = self.compile_arg_window_t(
                        Some(args),
                        &mask,
                        true,
                        Some(ArgRefTarget::Method(obj, name)),
                    )?;
                    let dst = self.alloc_temp();
                    self.emit(Instr::CallMethod {
                        dst,
                        obj,
                        name,
                        base,
                        argc,
                    });
                    Ok(dst)
                }
                Sel::Dyn(name) => {
                    if has_spread(args) || has_named(args) {
                        let args = self.build_args_array(Some(args))?;
                        let dst = self.alloc_temp();
                        self.emit(Instr::CallMethodDynV {
                            dst,
                            obj,
                            name,
                            args,
                        });
                        return Ok(dst);
                    }
                    let (base, argc) = self.compile_arg_window_ex(Some(args), &[], true)?;
                    let dst = self.alloc_temp();
                    self.emit(Instr::CallMethodDyn {
                        dst,
                        obj,
                        name,
                        base,
                        argc,
                    });
                    Ok(dst)
                }
            }
        }
    }

    /// zend_compile_method_call: a constant method name must be a string.
    fn check_method_name(
        &self,
        sel: &crate::parse::cst::ClassLikeMemberSelector,
    ) -> Result<(), Thrown> {
        if let crate::parse::cst::ClassLikeMemberSelector::Expression(e) = sel {
            if crate::fold::fold_const(self.cx, e.expression)
                .is_ok_and(|v| !matches!(v, Value::Str(_)))
            {
                return Err(compile_error("Method name must be a string", self.cur_line));
            }
        }
        Ok(())
    }

    fn compile_static_call(
        &mut self,
        smc: &crate::parse::cst::StaticMethodCall,
    ) -> Result<Reg, Thrown> {
        if let Some(r) = self.compile_parent_hook_call(smc)? {
            return Ok(r);
        }
        let class = self.class_ref(smc.class)?;
        self.check_method_name(&smc.method)?;
        match self.compile_selector(&smc.method)? {
            Sel::Static(name) => {
                if has_spread(&smc.argument_list) || has_named(&smc.argument_list) {
                    let args = self.build_args_array(Some(&smc.argument_list))?;
                    let dst = self.alloc_temp();
                    self.emit(Instr::CallStaticV {
                        dst,
                        class,
                        name,
                        args,
                    });
                    return Ok(dst);
                }
                let mask = self.name_ref_mask(name);
                let (base, argc) = self.compile_arg_window_t(
                    Some(&smc.argument_list),
                    &mask,
                    true,
                    Some(ArgRefTarget::Static(class, name)),
                )?;
                let dst = self.alloc_temp();
                self.emit(Instr::CallStatic {
                    dst,
                    class,
                    name,
                    base,
                    argc,
                });
                Ok(dst)
            }
            Sel::Dyn(name) => {
                if has_spread(&smc.argument_list) || has_named(&smc.argument_list) {
                    let args = self.build_args_array(Some(&smc.argument_list))?;
                    let dst = self.alloc_temp();
                    self.emit(Instr::CallStaticDynV {
                        dst,
                        class,
                        name,
                        args,
                    });
                    return Ok(dst);
                }
                let (base, argc) =
                    self.compile_arg_window_ex(Some(&smc.argument_list), &[], true)?;
                let dst = self.alloc_temp();
                self.emit(Instr::CallStaticDyn {
                    dst,
                    class,
                    name,
                    base,
                    argc,
                });
                Ok(dst)
            }
        }
    }
}

impl ProtoBuilder<'_> {
    /// zend_compile_parent_property_hook_call: `parent::$prop::get()` /
    /// `parent::$prop::set($v)`, only in that same hook of `$prop`.
    fn compile_parent_hook_call(
        &mut self,
        smc: &crate::parse::cst::StaticMethodCall,
    ) -> Result<Option<Reg>, Thrown> {
        use crate::parse::cst::{Access, ClassLikeMemberSelector, Variable};
        let Expression::Access(Access::StaticProperty(spa)) = smc.class else {
            return Ok(None);
        };
        let pname: Vec<u8> = match (spa.class, &spa.property) {
            (Expression::Parent(_), Variable::Direct(prop)) => {
                crate::util::strip_dollar(prop.name).to_vec()
            }
            // `parent::${0}`: a constant name.
            (Expression::Parent(_), Variable::Indirect(iv)) => match iv.expression {
                Expression::Literal(crate::parse::cst::Literal::String(s)) => match s.value {
                    Some(v) => v.to_vec(),
                    None => return Ok(None),
                },
                Expression::Literal(crate::parse::cst::Literal::Integer(i)) => match i.value {
                    Some(v) => v.to_string().into_bytes(),
                    None => return Ok(None),
                },
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        let ClassLikeMemberSelector::Identifier(m) = &smc.method else {
            return Ok(None);
        };
        let kind = if m.value.eq_ignore_ascii_case(b"get") {
            rhp_bytecode::HookKind::Get
        } else if m.value.eq_ignore_ascii_case(b"set") {
            rhp_bytecode::HookKind::Set
        } else {
            return Ok(None);
        };
        let at = {
            use mago_span::HasSpan;
            smc.span().start.offset
        };
        let shown = String::from_utf8_lossy(&pname).into_owned();
        let hname = String::from_utf8_lossy(m.value).into_owned();
        if self.class_name.is_none() {
            self.cx.fatal(
                at,
                "Cannot use \"parent\" when no class scope is active".into(),
            );
            return Ok(Some(self.alloc_temp()));
        }
        match self.hook.clone() {
            None => self.cx.fatal(
                at,
                format!("Must not use parent::${shown}::{hname}() outside a property hook"),
            ),
            Some((hp, _)) if hp != pname => self.cx.fatal(
                at,
                format!(
                    "Must not use parent::${shown}::{hname}() in a different property (${})",
                    String::from_utf8_lossy(&hp)
                ),
            ),
            Some((_, hk)) if hk != kind => self.cx.fatal(
                at,
                format!(
                    "Must not use parent::${shown}::{hname}() in a different property hook ({})",
                    hk.name()
                ),
            ),
            Some(_) => {}
        }
        let name = self.intern_name(&pname);
        if has_spread(&smc.argument_list) || has_named(&smc.argument_list) {
            let args = self.build_args_array(Some(&smc.argument_list))?;
            let dst = self.alloc_temp();
            self.emit(Instr::CallParentHook {
                dst,
                name,
                kind,
                base: args,
                argc: rhp_bytecode::PACKED_ARGS,
            });
            return Ok(Some(dst));
        }
        let (base, argc) = self.compile_arg_window_ex(Some(&smc.argument_list), &[], true)?;
        let dst = self.alloc_temp();
        self.emit(Instr::CallParentHook {
            dst,
            name,
            kind,
            base,
            argc,
        });
        Ok(Some(dst))
    }
}

/// `parent::$prop::get` / `::set` (unparenthesized).
fn is_parent_hook_call(
    class: &Expression,
    method: &crate::parse::cst::ClassLikeMemberSelector,
) -> bool {
    use crate::parse::cst::{Access, ClassLikeMemberSelector, Variable};
    matches!(class, Expression::Access(Access::StaticProperty(spa))
        if matches!((spa.class, &spa.property), (Expression::Parent(_), Variable::Direct(_))))
        && matches!(method, ClassLikeMemberSelector::Identifier(m)
            if m.value.eq_ignore_ascii_case(b"get") || m.value.eq_ignore_ascii_case(b"set"))
}

fn unparen<'a>(mut e: &'a Expression<'a>) -> &'a Expression<'a> {
    while let Expression::Parenthesized(p) = e {
        e = p.expression;
    }
    e
}

/// The stub signature of a builtin rhp implements: a native one, or a
/// function of the prelude.
fn builtin_sig(cx: &FileCtx, name_lc: &[u8]) -> Option<&'static rhp_bytecode::signatures::Func> {
    let f = rhp_bytecode::signatures::function(name_lc)?;
    let name = std::str::from_utf8(name_lc).ok()?;
    if (cx.builtins.native_function)(name) {
        return Some(f);
    }
    // While the prelude itself compiles, every function it calls exists.
    if cx.is_prelude {
        return Some(f);
    }
    let in_prelude = prelude_classes(cx.builtins)
        .ok()?
        .functions
        .iter()
        .any(|pf| pf.name.as_bytes().eq_ignore_ascii_case(name_lc));
    in_prelude.then_some(f)
}

/// By-reference flags for the first `argc` arguments of a builtin call, from
/// its stub signature. The call site emits `MakeRef` for each flagged
/// position.
fn builtin_by_ref(cx: &FileCtx, name_lc: &[u8], argc: usize) -> Option<Vec<bool>> {
    use rhp_bytecode::signatures::Signature;
    if !rhp_bytecode::signatures::function(name_lc)?.has_by_ref() {
        return None;
    }
    let f = builtin_sig(cx, name_lc)?;
    Some((0..argc.max(f.params.len())).map(|i| f.by_ref(i)).collect())
}

/// What php's compiler decides about a call to `name` (as resolved) with
/// `argc` positional arguments. `fallback` is the global name an unqualified
/// call in a namespace falls back to; a builtin is only ever reached by its
/// global name.
fn call_flags(name: &[u8], fallback: Option<&[u8]>, argc: usize) -> CallFlags {
    let global = fallback.unwrap_or(name).to_ascii_lowercase();
    let mut flags = CallFlags::default();
    // Only for a name php resolves at compile time.
    if fallback.is_none() {
        if compiled_inline(&global, argc) {
            flags.insert(CallFlags::INLINE);
        }
        if global == b"call_user_func" || global == b"call_user_func_array" {
            flags.insert(CallFlags::USER_CALL);
        }
    }
    if frameless_icall(&global, argc) {
        flags.insert(CallFlags::ICALL);
    }
    flags
}

pub(crate) fn has_named(list: &crate::parse::cst::ArgumentList) -> bool {
    list.arguments
        .iter()
        .any(|a| matches!(a, Argument::Named(_)))
}

pub(crate) fn has_spread(list: &crate::parse::cst::ArgumentList) -> bool {
    list.arguments
        .iter()
        .any(|a| matches!(a, Argument::Positional(p) if p.ellipsis.is_some()))
}

/// A known callee's parameters (see `ProtoBuilder::known_params`).
pub(crate) struct KnownParams {
    fname: String,
    names: Vec<Vec<u8>>,
    by_ref: Vec<bool>,
    prefer_ref: Vec<bool>,
    variadic: bool,
}

impl KnownParams {
    /// The parameter a positional argument `pos` or a named one binds to
    /// (a variadic parameter takes the rest and unknown names).
    fn param(&self, pos: Option<usize>, name: Option<&[u8]>) -> Option<usize> {
        let n = self.names.len();
        let fixed = n - usize::from(self.variadic);
        let i = match (pos, name) {
            (Some(i), _) if i < fixed => return Some(i),
            (_, Some(name)) => self.names[..fixed].iter().position(|p| p == name),
            _ => None,
        };
        i.or((self.variadic).then(|| n - 1))
    }
}

/// The callee a SEND_FUNC_ARG check asks: a method on the object in a
/// register, or a static method / constructor of a class.
#[derive(Clone, Copy)]
pub(crate) enum ArgRefTarget {
    Method(Reg, NIdx),
    Static(ClassRef, NIdx),
    /// The function [`Instr::ResolveFunc`] put in the register.
    Func(Reg),
    /// A callable value (`$f(...)`).
    Value(Reg),
}

/// The variable name (without `$`) of a plain-variable argument php sends
/// with SEND_VAR_EX to a function it cannot resolve (`$this` and `$GLOBALS`
/// are passed by value).
fn send_var_ex_name<'a>(e: &Expression<'a>) -> Option<&'a [u8]> {
    match e {
        Expression::Variable(Variable::Direct(d))
            if !matches!(strip_dollar(d.name), b"GLOBALS" | b"this") =>
        {
            Some(strip_dollar(d.name))
        }
        _ => None,
    }
}

/// Whether an argument after the `i`th may write a variable (anything but
/// a literal, a constant or a plain variable).
fn later_arg_may_write(list: &crate::parse::cst::ArgumentList, i: usize) -> bool {
    list.arguments.iter().skip(i + 1).any(|a| {
        let value = match a {
            Argument::Positional(q) => q.value,
            Argument::Named(n) => n.value,
        };
        !matches!(
            value,
            Expression::Literal(_)
                | Expression::ConstantAccess(_)
                | Expression::Variable(Variable::Direct(_))
        )
    })
}

/// Whether `e` is a `$var[...]` / `$var->p` / `C::$p[...]` chain rooted at a
/// variable other than `$GLOBALS` or at a static property: the arguments
/// SEND_FUNC_ARG can bind by reference (a chain on a call result or other
/// temporary is passed by value).
fn variable_rooted_lvalue(e: &Expression) -> bool {
    match e {
        Expression::Variable(Variable::Direct(_)) => !is_globals_var(e),
        Expression::Access(Access::StaticProperty(_)) => true,
        Expression::ArrayAccess(a) => variable_rooted_lvalue(a.array),
        Expression::ArrayAppend(a) => variable_rooted_lvalue(a.array),
        Expression::Access(Access::Property(p)) => variable_rooted_lvalue(p.object),
        _ => false,
    }
}

/// An element or property of a temporary (`[0, 1][0]`, `(new A)->p`): no
/// container to reference.
fn temp_rooted_lvalue(e: &Expression) -> bool {
    match e {
        Expression::Parenthesized(p) => temp_rooted_lvalue(p.expression),
        Expression::ArrayAccess(a) => temp_root(a.array),
        Expression::ArrayAppend(a) => temp_root(a.array),
        Expression::Access(Access::Property(p)) => temp_root(p.object),
        _ => false,
    }
}

fn temp_root(e: &Expression) -> bool {
    match e {
        Expression::Variable(_) | Expression::Access(Access::StaticProperty(_)) => false,
        Expression::Call(_) | Expression::Pipe(_) => false,
        Expression::Access(Access::NullSafeProperty(_)) => false,
        Expression::ArrayAccess(_)
        | Expression::ArrayAppend(_)
        | Expression::Access(Access::Property(_))
        | Expression::Parenthesized(_) => temp_rooted_lvalue(e),
        _ => true,
    }
}

/// Calls php compiles to dedicated opcodes instead of a function call
/// (zend_try_compile_special_func): they never appear as a trace frame, and a
/// callback they invoke is called from the caller's own line.
pub(crate) fn compiled_inline(lcname: &[u8], argc: usize) -> bool {
    match lcname {
        b"strlen" | b"is_null" | b"is_bool" | b"is_long" | b"is_int" | b"is_integer"
        | b"is_float" | b"is_double" | b"is_string" | b"is_array" | b"is_object"
        | b"is_resource" | b"is_scalar" | b"boolval" | b"intval" | b"floatval" | b"doubleval"
        | b"strval" | b"gettype" | b"count" | b"sizeof" => argc == 1,
        b"get_class" => argc <= 1,
        b"get_called_class" | b"func_num_args" | b"func_get_args" => argc == 0,
        b"array_key_exists" | b"call_user_func_array" => argc == 2,
        b"call_user_func" => argc >= 1,
        _ => false,
    }
}

/// php 8.4's frameless internal calls (ZEND_FRAMELESS_FUNCTION, by arity):
/// they get no execute_data, so what they run sees their caller's frame.
pub(crate) fn frameless_icall(lcname: &[u8], argc: usize) -> bool {
    match lcname {
        b"dechex" | b"is_numeric" => argc == 1,
        b"class_exists" | b"dirname" | b"implode" | b"trim" => argc == 1 || argc == 2,
        b"in_array" | b"strpos" | b"strstr" | b"strtr" | b"substr" => argc == 2 || argc == 3,
        b"max" | b"min" | b"preg_match" | b"property_exists" | b"str_contains"
        | b"str_starts_with" => argc == 2,
        b"preg_replace" | b"str_replace" => argc == 3,
        _ => false,
    }
}
