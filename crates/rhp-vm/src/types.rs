//! Type declarations and conversions: parameter, return and property type
//! checks, scalar coercion, truthiness and string conversion.

use rhp_builtins::zend::object_handlers::CastTarget;
use rhp_builtins::Host;
use rhp_bytecode::FunctionProto;
use rhp_values::{conv, ArrayKey, ClassId, PhpStr, Thrown, Value};

use crate::{deref, Frame, Vm};

impl Vm {
    fn value_instanceof(&self, v: &Value, name: &[u8]) -> bool {
        match v {
            Value::Object(h) => match self.resolve_class_by_name(name) {
                Ok(cid) => self.classes[h.borrow().class as usize]
                    .ancestors
                    .contains(&cid),
                // No object is an instance of a class that is not loaded
                // (type checks do not autoload).
                Err(_) => false,
            },
            Value::Closure(_) => name.eq_ignore_ascii_case(b"Closure"),
            _ => false,
        }
    }

    fn kind_matches(&self, v: &Value, k: &rhp_bytecode::TypeKind, ctx: Option<ClassId>) -> bool {
        use rhp_bytecode::TypeKind as K;
        match k {
            K::Mixed => true,
            K::Int => matches!(v, Value::Int(_)),
            K::Float => matches!(v, Value::Float(_)),
            K::String => matches!(v, Value::Str(_)),
            K::Bool => matches!(v, Value::Bool(_)),
            K::True => matches!(v, Value::Bool(true)),
            K::False => matches!(v, Value::Bool(false)),
            K::Array => matches!(v, Value::Array(_)),
            K::Object => matches!(v, Value::Object(_) | Value::Closure(_)),
            K::Callable => {
                // zend_verify_arg_type checks a callable in the callee's
                // scope (its private methods count).
                let saved = self.fcc_scope.replace(Some((ctx, ctx)));
                let ok = self.is_callable_value(v);
                self.fcc_scope.set(saved);
                ok
            }
            K::Iterable => matches!(v, Value::Array(_)) || self.value_instanceof(v, b"Traversable"),
            K::Null => matches!(v, Value::Null),
            K::Named(n) => self.value_instanceof(v, n.as_bytes()),
            K::SelfType | K::StaticType => match (v, ctx) {
                (Value::Object(h), Some(c)) => self.classes[h.borrow().class as usize]
                    .ancestors
                    .contains(&c),
                _ => false,
            },
            K::Parent => match (v, ctx.and_then(|c| self.classes[c as usize].parent)) {
                (Value::Object(h), Some(p)) => self.classes[h.borrow().class as usize]
                    .ancestors
                    .contains(&p),
                _ => false,
            },
            K::Void | K::Never => false,
        }
    }

    fn matches_type(&self, v: &Value, ty: &rhp_bytecode::TypeHint, ctx: Option<ClassId>) -> bool {
        if matches!(v, Value::Null) && ty.nullable {
            return true;
        }
        // DNF: any intersection group fully matched satisfies the union.
        if ty.groups.iter().any(|g| {
            g.iter()
                .all(|n| self.kind_matches(v, &rhp_bytecode::TypeKind::Named(n.clone()), ctx))
        }) {
            return true;
        }
        if ty.kinds.is_empty() {
            // only-null / unknown: don't reject (a DNF union already failed)
            return ty.groups.is_empty();
        }
        if ty.intersection {
            ty.kinds.iter().all(|k| self.kind_matches(v, k, ctx))
        } else {
            ty.kinds.iter().any(|k| self.kind_matches(v, k, ctx))
        }
    }

    /// Check/coerce one argument against its declared type. `Ok(None)` = fine as
    /// is, `Ok(Some(v))` = coerced replacement, `Err(())` = type violation.
    pub(crate) fn check_one(
        &self,
        v: &Value,
        ty: &rhp_bytecode::TypeHint,
        strict: bool,
        ctx: Option<ClassId>,
    ) -> Result<Option<Value>, ()> {
        use rhp_bytecode::TypeKind as K;
        let d = deref(v);
        if self.matches_type(&d, ty, ctx) {
            return Ok(None);
        }
        if matches!(d, Value::Null) {
            return Err(()); // null but not nullable
        }
        // int -> float widening is allowed in both strict and coercive modes.
        if let Value::Int(n) = d {
            if ty.kinds.iter().any(|k| matches!(k, K::Float)) {
                return Ok(Some(Value::Float(n as f64)));
            }
        }
        if strict {
            return Err(());
        }
        // Coercive mode: php's zend_verify_weak_scalar_type_hint.
        weak_scalar_coerce(&d, ty).map(Some).ok_or(())
    }

    /// Coercive typing's `__toString` step (zend_verify_weak_scalar_type_hint
    /// for `string`): an object with `__toString` passed where a `string`
    /// (and not its class) is declared becomes its string form.
    pub(crate) fn coerce_stringable(
        &mut self,
        v: Value,
        ty: &rhp_bytecode::TypeHint,
        strict: bool,
        ctx: Option<ClassId>,
    ) -> Result<Value, Thrown> {
        use rhp_bytecode::TypeKind as K;
        if strict || !ty.kinds.contains(&K::String) {
            return Ok(v);
        }
        let Value::Object(h) = deref(&v) else {
            return Ok(v);
        };
        if self.matches_type(&v, ty, ctx) {
            return Ok(v);
        }
        // zend_parse_arg_str_weak: the object's cast_object.
        Ok(match self.cast_object_vm(&h, CastTarget::Str)? {
            Some(s) => Value::str(conv::to_string_bytes(&s)),
            None => v,
        })
    }

    /// [`Vm::coerce_stringable`] over a just-bound frame's passed parameters.
    pub(crate) fn coerce_stringable_args(
        &mut self,
        proto: &FunctionProto,
        frame: &mut Frame,
        caller: Option<bool>,
        ctx: Option<ClassId>,
    ) -> Result<(), Thrown> {
        let strict = caller.unwrap_or(false);
        if strict {
            return Ok(());
        }
        for (i, p) in proto.params.iter().enumerate() {
            let Some(ty) = &p.type_hint else { continue };
            if p.variadic && !p.by_ref {
                // `string ...$xs`: each collected argument.
                let Value::Array(a) = &frame.regs[p.cv as usize] else {
                    continue;
                };
                if !a.iter().any(|(_, v)| matches!(v, Value::Object(_))) {
                    continue;
                }
                let mut a = a.clone();
                let keys: Vec<ArrayKey> = a.iter().map(|(k, _)| k.clone()).collect();
                for k in keys {
                    let cur = a.get(&k).cloned().unwrap_or(Value::Null);
                    if matches!(cur, Value::Object(_)) {
                        let v = self.coerce_stringable(cur, ty, false, ctx)?;
                        a.insert(k, v);
                    }
                }
                frame.regs[p.cv as usize] = Value::Array(a);
                continue;
            }
            if p.variadic || i >= frame.argc || (i < 64 && frame.holes & (1u64 << i) != 0) {
                continue;
            }
            if p.by_ref {
                // the referenced value is converted in place
                if let Value::Ref(rc) = &frame.regs[p.cv as usize] {
                    let rc = rc.clone();
                    let cur = rc.borrow().clone();
                    if matches!(cur, Value::Object(_)) {
                        let v = self.coerce_stringable(cur, ty, false, ctx)?;
                        *rc.borrow_mut() = v;
                    }
                }
                continue;
            }
            let cur = frame.regs[p.cv as usize].clone();
            if matches!(cur, Value::Object(_)) {
                frame.regs[p.cv as usize] = self.coerce_stringable(cur, ty, false, ctx)?;
            }
        }
        Ok(())
    }

    /// Enforce declared parameter types on a just-bound frame (php's
    /// `strict_types` is the CALLER's; `ctx` resolves `self`/`static`/`parent`).
    /// `caller` is `Some(strict_types)` for a call from user code, `None` for
    /// a call made by a builtin (coercive, and the error has no call site).
    pub(crate) fn check_arg_types(
        &mut self,
        proto: &FunctionProto,
        frame: &mut Frame,
        caller: Option<bool>,
        ctx: Option<ClassId>,
    ) -> Result<(), (usize, Thrown)> {
        let strict = caller.unwrap_or(false);
        for (i, p) in proto.params.iter().enumerate() {
            let Some(ty) = &p.type_hint else { continue };
            if p.variadic {
                self.check_variadic_types(proto, frame, i, caller, ctx)
                    .map_err(|t| (i, t))?;
                continue;
            }
            // A defaulted (not passed) param is trusted.
            if i >= frame.argc || (i < 64 && frame.holes & (1u64 << i) != 0) {
                continue;
            }
            // A by-reference parameter is checked on the referenced value,
            // which keeps a coerced one.
            let cur = match &frame.regs[p.cv as usize] {
                Value::Ref(rc) if p.by_ref => rc.borrow().clone(),
                v => v.clone(),
            };
            // zend_check_type_slow's zend_is_callable runs in the callee's
            // frame (its scope and `$this`), deprecations included.
            let callable = needs_callable_check(&ty.kinds, &cur);
            let checked = if callable {
                let this = proto.this_cv.and_then(|cv| match &frame.regs[cv as usize] {
                    Value::Object(h) => Some(h.clone()),
                    _ => None,
                });
                let saved = self
                    .fcc_scope
                    .replace(Some((frame.defining_class, frame.called_class)));
                let saved_this = self.fcc_this.replace(this);
                let mut deps = Vec::new();
                let _ = self.resolve_callable_ex(&cur, proto.is_internal(), &mut deps);
                let r = self.check_one(&cur, ty, strict, ctx);
                self.fcc_scope.set(saved);
                *self.fcc_this.borrow_mut() = saved_this;
                for d in deps {
                    if let Err(t) = self.diag(rhp_builtins::E_DEPRECATED, &d) {
                        return Err((i, t));
                    }
                }
                r
            } else {
                self.check_one(&cur, ty, strict, ctx)
            };
            match checked {
                Ok(None) => {}
                Ok(Some(coerced)) => {
                    let at = || {
                        (!proto.is_internal())
                            .then(|| (proto.debug.file.as_bytes().to_vec(), p.line as i64))
                    };
                    if let Err(t) = self.coercion_diag_at(&cur, &coerced, at) {
                        return Err((i, t));
                    }
                    match &frame.regs[p.cv as usize] {
                        Value::Ref(rc) if p.by_ref => *rc.borrow_mut() = coerced,
                        _ => frame.regs[p.cv as usize] = coerced,
                    }
                }
                // A prelude (built-in) function takes null for a scalar
                // parameter like an internal one in coercive mode: deprecated,
                // then coerced (zend_null_arg_deprecated).
                Err(())
                    if !strict
                        && proto.is_internal()
                        && matches!(deref(&cur), Value::Null)
                        && prelude_null_coercion(ty).is_some() =>
                {
                    let (tname, zero) = prelude_null_coercion(ty).unwrap();
                    let fname = self.func_display_name(proto, ctx);
                    let pname = String::from_utf8_lossy(proto.names[p.name as usize].as_bytes())
                        .into_owned();
                    let msg = format!(
                        "{fname}(): Passing null to parameter #{} (${pname}) of type {tname} is deprecated",
                        i + 1
                    );
                    if let Err(t) = self.diag(rhp_builtins::E_DEPRECATED, &msg) {
                        return Err((i, t));
                    }
                    frame.regs[p.cv as usize] = zero;
                }
                Err(()) => {
                    let label = format!(
                        " (${})",
                        String::from_utf8_lossy(proto.names[p.name as usize].as_bytes())
                    );
                    return Err((
                        i,
                        self.arg_type_error(proto, frame, i, &label, &cur, caller, ctx),
                    ));
                }
            }
        }
        Ok(())
    }

    /// zend_verify_arg_error: "f(): Argument #N ($x) must be of type T, U
    /// given". php appends the call site when the caller is user code (the
    /// new frame has not been pushed yet: the caller is the top); a callback
    /// invoked by a builtin (`caller == None`) has none. Prelude (built-in
    /// PHP-source) functions report like internal ones: no call site.
    #[allow(clippy::too_many_arguments)]
    #[cold]
    #[inline(never)]
    fn arg_type_error(
        &self,
        proto: &FunctionProto,
        frame: &Frame,
        i: usize,
        label: &str,
        cur: &Value,
        caller: Option<bool>,
        ctx: Option<ClassId>,
    ) -> Thrown {
        let p = &proto.params[i.min(proto.params.len() - 1)];
        let ty = p.type_hint.as_ref().expect("typed parameter");
        let site = caller
            .filter(|_| !proto.is_internal())
            .and_then(|_| self.user_call_site())
            .map(|(file, line)| {
                format!(
                    ", called in {} on line {}",
                    String::from_utf8_lossy(&file),
                    line
                )
            });
        Thrown::error(
            "TypeError",
            format!(
                "{}(): Argument #{}{label} must be of type {}, {} given{}",
                self.func_c_name(proto, ctx),
                i + 1,
                self.type_string_resolved(ty, ctx, frame.called_class),
                rhp_builtins::zval_value_name(self, &deref(cur)),
                site.unwrap_or_default(),
            ),
        )
    }

    /// ZEND_RECV_VARIADIC's checks: each collected argument of the typed
    /// variadic parameter `i`, in order (a reference's value is coerced in
    /// place).
    #[inline(never)]
    fn check_variadic_types(
        &mut self,
        proto: &FunctionProto,
        frame: &mut Frame,
        i: usize,
        caller: Option<bool>,
        ctx: Option<ClassId>,
    ) -> Result<(), Thrown> {
        let p = &proto.params[i];
        let Some(ty) = &p.type_hint else {
            return Ok(());
        };
        let Value::Array(arr) = &frame.regs[p.cv as usize] else {
            return Ok(());
        };
        let strict = caller.unwrap_or(false);
        let items: Vec<(ArrayKey, Value)> =
            arr.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        // php numbers a named extra after the positional ones, all alike
        let positional = items
            .iter()
            .filter(|(k, _)| matches!(k, ArrayKey::Int(_)))
            .count();
        for (n, (k, v)) in items.into_iter().enumerate() {
            let cur = deref(&v);
            match self.check_one(&cur, ty, strict, ctx) {
                Ok(None) => {}
                Ok(Some(coerced)) => {
                    let at = || {
                        (!proto.is_internal())
                            .then(|| (proto.debug.file.as_bytes().to_vec(), p.line as i64))
                    };
                    self.coercion_diag_at(&cur, &coerced, at)?;
                    match &v {
                        Value::Ref(rc) => *rc.borrow_mut() = coerced,
                        _ => {
                            if let Value::Array(a) = &mut frame.regs[p.cv as usize] {
                                a.insert(k, coerced);
                            }
                        }
                    }
                }
                Err(()) => {
                    // zend_verify_variadic_arg_type: no parameter name
                    let at = match &k {
                        ArrayKey::Str(_) => i + positional,
                        ArrayKey::Int(_) => i + n,
                    };
                    return Err(self.arg_type_error(proto, frame, at, "", &cur, caller, ctx));
                }
            }
        }
        Ok(())
    }

    /// php's RECV_INIT type check of a non-constant default just stored
    /// into parameter `index` (coerced under the caller's strict_types).
    pub(crate) fn recv_init_check(&mut self, top: usize, index: u16) -> Result<(), Thrown> {
        let proto = self.stack[top].proto.clone();
        let p = &proto.params[index as usize];
        let Some(ty) = &p.type_hint else {
            return Ok(());
        };
        let from_user = top > 0 && !self.stack[top].capture;
        let strict = from_user && self.stack[top - 1].proto.strict_types;
        let ctx = self.stack[top].defining_class;
        let cur = self.stack[top].regs[p.cv as usize].clone();
        match self.check_one(&cur, ty, strict, ctx) {
            Ok(None) => Ok(()),
            Ok(Some(v)) => {
                self.stack[top].regs[p.cv as usize] = v;
                Ok(())
            }
            Err(()) => {
                let fname = self.func_c_name(&proto, ctx);
                let pname = String::from_utf8_lossy(proto.names[p.name as usize].as_bytes());
                let site = if from_user && !self.is_internal_frame(top - 1) {
                    format!(
                        ", called in {} on line {}",
                        String::from_utf8_lossy(&self.current_file(top - 1)),
                        self.current_line(top - 1)
                    )
                } else {
                    String::new()
                };
                Err(Thrown::error(
                    "TypeError",
                    format!(
                        "{fname}(): Argument #{} (${pname}) must be of type {}, {} given{site}",
                        index + 1,
                        self.type_string_resolved(ty, ctx, self.stack[top].called_class),
                        rhp_builtins::zval_value_name(self, &deref(&cur)),
                    ),
                ))
            }
        }
    }

    /// Enforce a function's declared return type on `v`, coercing in coercive
    /// mode. `void`/`never`/`mixed` and unknowable types are not checked.
    /// Falling off the end of (or a bare `return;` in) a function with a
    /// declared return type: only `void` allows it (zend_verify_missing_return_type).
    pub(crate) fn check_implicit_return(
        &self,
        proto: &FunctionProto,
        top: usize,
    ) -> Result<(), Thrown> {
        use rhp_bytecode::TypeKind as K;
        let Some(ty) = &proto.return_type else {
            return Ok(());
        };
        if ty.kinds.contains(&K::Void) {
            return Ok(());
        }
        let ctx = self.stack[top].defining_class;
        let msg = if ty.kinds.contains(&K::Never) {
            // zend_verify_never_error.
            let fname = self.func_c_name(proto, ctx);
            let what = if ctx.is_some() { "method" } else { "function" };
            format!("{fname}(): never-returning {what} must not implicitly return")
        } else {
            let fname = self.func_display_name(proto, ctx);
            let called = self.stack[top].called_class;
            format!(
                "{fname}(): Return value must be of type {}, none returned",
                self.type_string_resolved(ty, ctx, called)
            )
        };
        Err(Thrown::error("TypeError", msg))
    }

    /// ZEND_RETURN_BY_REF (after ZEND_VERIFY_RETURN_TYPE): the value a
    /// by-reference function returns from register `src`. The type is
    /// checked on the referenced value, which keeps a coerced value; a value
    /// that is no reference notices and is returned in a new one.
    pub(crate) fn ret_ref_value(
        &mut self,
        top: usize,
        proto: &FunctionProto,
        src: rhp_bytecode::Reg,
        kind: u8,
    ) -> Result<Value, Thrown> {
        let raw = self.ret_ref_checked(top, proto, src, kind)?;
        self.ret_ref_wrap(raw, kind)
    }

    /// The type-checked value of a by-reference `return` (ZEND_VERIFY_RETURN_TYPE),
    /// before RETURN_BY_REF.
    pub(crate) fn ret_ref_checked(
        &mut self,
        top: usize,
        proto: &FunctionProto,
        src: rhp_bytecode::Reg,
        kind: u8,
    ) -> Result<Value, Thrown> {
        let typed = proto.return_type.is_some() && !proto.is_generator;
        if kind == rhp_bytecode::RET_REF_IMPLICIT {
            if typed {
                self.check_implicit_return(proto, top)?;
            }
            return Ok(Value::Null);
        }
        let raw = self.stack[top].regs[src as usize].clone();
        if typed {
            self.recheck_ref_return(top, proto, &raw)
        } else {
            Ok(raw)
        }
    }

    /// RETURN_BY_REF on a checked value: a value that is no reference
    /// notices and is returned in a new one.
    pub(crate) fn ret_ref_wrap(&mut self, raw: Value, kind: u8) -> Result<Value, Thrown> {
        use rhp_bytecode::{RET_REF_FUNC, RET_REF_IMPLICIT, RET_REF_VALUE};
        let is_ref = matches!(raw, Value::Ref(_));
        if kind == RET_REF_VALUE || kind == RET_REF_IMPLICIT || (kind == RET_REF_FUNC && !is_ref) {
            self.diag(
                rhp_builtins::E_NOTICE,
                "Only variable references should be returned by reference",
            )?;
            let v = match raw {
                Value::Undef => Value::Null,
                v => deref(&v),
            };
            return Ok(Value::Ref(std::rc::Rc::new(std::cell::RefCell::new(v))));
        }
        Ok(match raw {
            Value::Ref(_) => raw,
            v => Value::Ref(std::rc::Rc::new(std::cell::RefCell::new(v))),
        })
    }

    /// The return type check of a by-reference function on `v`: a
    /// reference's value is coerced in place. A plain value comes back
    /// coerced.
    pub(crate) fn recheck_ref_return(
        &mut self,
        top: usize,
        proto: &FunctionProto,
        v: &Value,
    ) -> Result<Value, Thrown> {
        let Some(ty) = &proto.return_type else {
            return Ok(v.clone());
        };
        if proto.is_generator {
            return Ok(v.clone());
        }
        let ctx = self.stack[top].defining_class;
        let cur = match v {
            Value::Ref(rc) => rc.borrow().clone(),
            Value::Undef => Value::Null,
            v => v.clone(),
        };
        let nv = self.coerce_stringable(cur, ty, proto.strict_types, ctx)?;
        let nv = self.check_return_type(proto, top, nv)?;
        Ok(match v {
            Value::Ref(rc) => {
                *rc.borrow_mut() = nv;
                v.clone()
            }
            _ => nv,
        })
    }

    pub(crate) fn check_return_type(
        &mut self,
        proto: &FunctionProto,
        top: usize,
        v: Value,
    ) -> Result<Value, Thrown> {
        use rhp_bytecode::TypeKind as K;
        let ty = match &proto.return_type {
            Some(t) => t,
            None => return Ok(v),
        };
        if ty
            .kinds
            .iter()
            .any(|k| matches!(k, K::Void | K::Never | K::Mixed))
        {
            return Ok(v);
        }
        let ctx = self.stack[top].defining_class;
        let strict = proto.strict_types;
        // `static` is the called scope (zend_value_instanceof_static), not
        // the declaring class.
        let resolved;
        let checked = if ty.kinds.contains(&K::StaticType) {
            let called = self.stack[top]
                .called_class
                .or_else(|| self.current_this(top).map(|o| o.borrow().class));
            // Fast path (fluent `: static` methods): an instance of the
            // called scope passes without building the resolved type.
            if let (Some(c), Value::Object(h)) = (called, &deref(&v)) {
                if self.classes[h.borrow().class as usize]
                    .ancestors
                    .contains(&c)
                {
                    return Ok(v);
                }
            }
            let mut t = ty.clone();
            t.kinds.retain(|k| *k != K::StaticType);
            t.kinds.push(match called {
                Some(c) => K::Named(self.class(c).name.clone()),
                None => K::Never, // matches nothing
            });
            resolved = t;
            &resolved
        } else {
            ty
        };
        // zend_is_callable runs in the returning frame: its scope and
        // `$this` (an instance method named without an object is callable),
        // deprecations included.
        let r = if needs_callable_check(&checked.kinds, &v) {
            let this = self.current_this(top);
            let saved = self
                .fcc_scope
                .replace(Some((ctx, self.stack[top].called_class)));
            let saved_this = self.fcc_this.replace(this);
            let mut deps = Vec::new();
            let _ = self.resolve_callable_ex(&v, proto.is_internal(), &mut deps);
            let r = self.check_one(&v, checked, strict, ctx);
            self.fcc_scope.set(saved);
            *self.fcc_this.borrow_mut() = saved_this;
            for d in deps {
                self.diag(rhp_builtins::E_DEPRECATED, &d)?;
            }
            r
        } else {
            self.check_one(&v, checked, strict, ctx)
        };
        match r {
            Ok(None) => Ok(v),
            Ok(Some(coerced)) => {
                self.coercion_diag(&v, &coerced)?;
                Ok(coerced)
            }
            Err(()) => {
                let fname = self.func_display_name(proto, ctx);
                let called = self.stack[top].called_class;
                Err(Thrown::error(
                    "TypeError",
                    format!(
                        "{}(): Return value must be of type {}, {} returned",
                        fname,
                        self.type_string_resolved(ty, ctx, called),
                        rhp_builtins::zval_value_name(self, &deref(&v)),
                    ),
                ))
            }
        }
    }

    /// A property's type as zend_verify_property_type_error prints it: php
    /// resolves `self`/`parent` when compiling a named class, but an
    /// anonymous class keeps them as written.
    pub(crate) fn prop_type_string(
        &self,
        ty: &rhp_bytecode::TypeHint,
        decl: ClassId,
        called: Option<ClassId>,
    ) -> String {
        // A trait keeps them too (its own copy of the property).
        if crate::classes::is_anon_name(self.class(decl).name.as_bytes())
            || self.class(decl).kind == rhp_values::ClassKind::Trait
        {
            return self.type_string_resolved(ty, None, None);
        }
        self.type_string_resolved(ty, Some(decl), called)
    }

    /// zend_type_to_string(_resolved): a declared type as php prints it in
    /// TypeErrors -- class names first (self/parent resolved against `scope`,
    /// static against `called`; with no scope they print as written), then
    /// the builtin types in php's fixed order.
    pub(crate) fn type_string_resolved(
        &self,
        ty: &rhp_bytecode::TypeHint,
        scope: Option<ClassId>,
        called: Option<ClassId>,
    ) -> String {
        use rhp_bytecode::TypeKind as K;
        let cname =
            |c: ClassId| String::from_utf8_lossy(self.class(c).name.as_bytes()).into_owned();
        let has = |k: &K| ty.kinds.contains(k);
        if has(&K::Mixed) {
            return "mixed".into();
        }
        let mut parts: Vec<String> = Vec::new();
        for k in &ty.kinds {
            match k {
                K::Named(n) => parts.push(String::from_utf8_lossy(n.as_bytes()).into_owned()),
                K::SelfType => parts.push(scope.map(cname).unwrap_or_else(|| "self".into())),
                K::Parent => parts.push(
                    scope
                        .and_then(|c| self.class(c).parent)
                        .map(cname)
                        .unwrap_or_else(|| "parent".into()),
                ),
                K::Iterable => parts.push("Traversable".into()),
                _ => {}
            }
        }
        interleave_groups(&mut parts, dnf_group_strings(ty), &ty.group_pos);
        if has(&K::StaticType) {
            let name = match (scope, called) {
                (Some(_), Some(c)) => cname(c),
                _ => "static".into(),
            };
            parts.push(name);
        }
        let array = has(&K::Array) || has(&K::Iterable);
        let bool_ = has(&K::Bool) || (has(&K::True) && has(&K::False));
        for (on, name) in [
            (has(&K::Callable), "callable"),
            (has(&K::Object), "object"),
            (array, "array"),
            (has(&K::String), "string"),
            (has(&K::Int), "int"),
            (has(&K::Float), "float"),
            (bool_, "bool"),
            (!bool_ && has(&K::False), "false"),
            (!bool_ && has(&K::True), "true"),
            (has(&K::Void), "void"),
            (has(&K::Never), "never"),
        ] {
            if on {
                parts.push(name.into());
            }
        }
        let sep = if ty.intersection { "&" } else { "|" };
        if ty.nullable || has(&K::Null) {
            if parts.is_empty() {
                return "null".into();
            }
            if parts.len() == 1 && ty.groups.is_empty() {
                return format!("?{}", parts[0]);
            }
            let mut out = parts.join(sep);
            out.push_str("|null");
            return out;
        }
        parts.join(sep)
    }

    /// `(object)` cast: wrap a value in a fresh `stdClass`. Objects pass through;
    /// arrays map each element to a property; null yields an empty object; a
    /// scalar yields `{ scalar: value }`.
    pub(crate) fn cast_to_object(&mut self, v: Value) -> Value {
        let v = deref(&v);
        if let Value::Object(_) | Value::Closure(_) = v {
            return v;
        }
        let cls = match self.resolve_class_by_name(b"stdClass") {
            Ok(c) => c,
            Err(_) => return v, // prelude missing (shouldn't happen)
        };
        let obj = self.new_object(cls);
        {
            let mut b = obj.borrow_mut();
            match v {
                Value::Null => {}
                Value::Array(a) => {
                    for (k, val) in a.iter() {
                        let name = match k {
                            ArrayKey::Int(n) => {
                                rhp_values::PhpStr::from_bytes(n.to_string().into_bytes())
                            }
                            ArrayKey::Str(s) => s.clone(),
                        };
                        b.props_mut().insert(name, deref(val));
                    }
                }
                scalar => {
                    b.props_mut()
                        .insert(rhp_values::PhpStr::from("scalar"), scalar);
                }
            }
        }
        Value::Object(obj)
    }

    /// Does `class_id`'s ancestor set include the named interface/class?
    pub(crate) fn class_implements(&self, class_id: ClassId, name: &[u8]) -> bool {
        match self.resolve_class_by_name(name) {
            Ok(target) => self.classes[class_id as usize].ancestors.contains(&target),
            Err(_) => false,
        }
    }

    /// The name of `$o->$name` (an object name converts through
    /// `__toString`).
    pub(crate) fn dyn_prop_name(
        &mut self,
        top: usize,
        name: rhp_bytecode::Reg,
    ) -> Result<PhpStr, Thrown> {
        let v = self.reg_r(top, name)?;
        match &v {
            Value::Str(s) => Ok(s.clone()),
            Value::Object(_) => Ok(PhpStr::from_bytes(self.to_string_vm(&v)?)),
            // zval_try_get_string.
            Value::Array(_) => {
                self.diag(rhp_builtins::E_WARNING, "Array to string conversion")?;
                Ok(PhpStr::from("Array"))
            }
            Value::Closure(_) => Err(Thrown::error(
                "Error",
                "Object of class Closure could not be converted to string",
            )),
            _ => Ok(PhpStr::from_bytes(conv::to_string_bytes(&v))),
        }
    }

    /// Stringify a value, invoking `__toString` on objects that define it.
    // Takes `&mut self` because stringifying an object may run its `__toString`.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_string_vm(&mut self, v: &Value) -> Result<Vec<u8>, Thrown> {
        let d = match v {
            Value::Ref(r) => r.borrow().clone(),
            other => other.clone(),
        };
        if let Value::Object(h) = &d {
            // zval_get_string: the object's cast_object (`__toString`, or an
            // internal class's handler).
            if let Some(r) = self.cast_object_vm(h, CastTarget::Str)? {
                return Ok(conv::to_string_bytes(&r));
            }
            let cls = h.borrow().class;
            return Err(Thrown::error(
                "Error",
                format!(
                    "Object of class {} could not be converted to string",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes())
                ),
            ));
        }
        match d {
            Value::Array(_) => self.diag(rhp_builtins::E_WARNING, "Array to string conversion")?,
            Value::Float(f) if f.is_nan() => self.nan_warning("string")?,
            _ => {}
        }
        if let Value::Closure(_) = d {
            return Err(Thrown::error(
                "Error",
                "Object of class Closure could not be converted to string",
            ));
        }
        Ok(conv::to_string_bytes(&d))
    }

    /// `(bool) $v`: an internal class's `cast_object(_IS_BOOL)` (an empty
    /// SimpleXML list, a zero GMP is false), else php's truthiness.
    #[inline]
    pub(crate) fn truthy(&mut self, v: &Value) -> Result<bool, Thrown> {
        match v {
            Value::Object(_) => self.truthy_object(v),
            Value::Float(f) if f.is_nan() => {
                self.nan_warning("bool")?;
                Ok(true)
            }
            _ => Ok(conv::to_bool(v)),
        }
    }

    /// The diagnostic of a weak scalar coercion `check_one` did, raised
    /// at `at` (a user function's parameter) or the current line.
    pub(crate) fn coercion_diag_at(
        &mut self,
        orig: &Value,
        coerced: &Value,
        at: impl FnOnce() -> Option<(Vec<u8>, i64)>,
    ) -> Result<(), Thrown> {
        let Some((level, msg)) = coercion_message(orig, coerced) else {
            return Ok(());
        };
        match at() {
            Some((file, line)) => {
                self.raise_error_at(level, &msg, file, line);
                match self.pending_throw.take() {
                    Some(t) => Err(t),
                    None => Ok(()),
                }
            }
            None => self.diag(level, &msg),
        }
    }

    pub(crate) fn coercion_diag(&mut self, orig: &Value, coerced: &Value) -> Result<(), Thrown> {
        self.coercion_diag_at(orig, coerced, || None)
    }

    /// php 8.5's `zend_nan_coerced_to_type_warning`.
    #[cold]
    pub(crate) fn nan_warning(&mut self, ty: &str) -> Result<(), Thrown> {
        self.diag(
            rhp_builtins::E_WARNING,
            &format!("unexpected NAN value was coerced to {ty}"),
        )
    }

    /// `zend_dval_to_lval`: php 8.5 warns when the float is out of range
    /// (then wraps modulo 2^64) or not finite (then 0).
    pub(crate) fn dval_to_lval(&mut self, d: f64) -> Result<i64, Thrown> {
        if !d.is_finite()
            || !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&d)
        {
            self.oob_float(d)?;
        }
        Ok(rhp_values::float_to_int(d))
    }

    /// `zend_oob_double_to_long_error`.
    #[cold]
    pub(crate) fn oob_float(&mut self, d: f64) -> Result<(), Thrown> {
        let shown = rhp_values::fmt::float_serialize(d);
        self.diag(
            rhp_builtins::E_WARNING,
            &[
                b"The float ".as_slice(),
                &shown,
                b" is not representable as an int, cast occurred",
            ]
            .concat(),
        )
    }

    #[inline(never)]
    fn truthy_object(&mut self, v: &Value) -> Result<bool, Thrown> {
        Ok(match self.handler_cast(v, CastTarget::Bool)? {
            Some(b) => conv::to_bool(&b),
            None => true,
        })
    }
}

/// The diagnostic of a weak scalar coercion: zend_parse_arg_long_weak's
/// lost-fraction deprecation, the NAN warning of the string and bool
/// coercions.
fn coercion_message(orig: &Value, coerced: &Value) -> Option<(i64, Vec<u8>)> {
    let lost = |what: &[u8], shown: &[u8]| {
        [
            b"Implicit conversion from ".as_slice(),
            what,
            shown,
            b" to int loses precision",
        ]
        .concat()
    };
    match (deref(orig), coerced) {
        (Value::Float(f), Value::Int(n)) if *n as f64 != f => Some((
            rhp_builtins::E_DEPRECATED,
            lost(b"float ", &rhp_values::fmt::float_serialize(f)),
        )),
        (Value::Str(s), Value::Int(n)) => match conv::numeric_string(s.as_bytes()) {
            Some(rhp_values::conv::Num::Float(f)) if *n as f64 != f => Some((
                rhp_builtins::E_DEPRECATED,
                lost(b"float-string ", &[b"\"", s.as_bytes(), b"\""].concat()),
            )),
            _ => None,
        },
        (Value::Float(f), Value::Str(_)) if f.is_nan() => Some((
            rhp_builtins::E_WARNING,
            b"unexpected NAN value was coerced to string".to_vec(),
        )),
        (Value::Float(f), Value::Bool(_)) if f.is_nan() => Some((
            rhp_builtins::E_WARNING,
            b"unexpected NAN value was coerced to bool".to_vec(),
        )),
        _ => None,
    }
}

/// php's `zend_verify_weak_scalar_type_hint`: coerce a scalar to the first of
/// int, float, string, bool (in that preference order) the type accepts.
/// `None` if no member accepts it. (The E_DEPRECATED for a fractional float
/// to int is not emitted.)
fn weak_scalar_coerce(d: &Value, ty: &rhp_bytecode::TypeHint) -> Option<Value> {
    use rhp_bytecode::TypeKind as K;
    use rhp_values::conv::Num;
    let has = |want: fn(&K) -> bool| ty.kinds.iter().any(want);
    let (t_int, t_float, t_str) = (
        has(|k| matches!(k, K::Int)),
        has(|k| matches!(k, K::Float)),
        has(|k| matches!(k, K::String)),
    );
    let t_bool = has(|k| matches!(k, K::Bool))
        || (has(|k| matches!(k, K::True)) && has(|k| matches!(k, K::False)));
    // ZEND_DOUBLE_FITS_LONG + NaN check, then truncation.
    let f2i = |f: f64| -> Option<i64> {
        if f.is_nan() || !(-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f) {
            None
        } else {
            Some(f as i64)
        }
    };
    if t_int {
        match d {
            // int|float and a string: is_numeric_string decides.
            Value::Str(s) if t_float => match conv::numeric_string(s.as_bytes()) {
                Some(Num::Int(n)) => return Some(Value::Int(n)),
                Some(Num::Float(f)) => return Some(Value::Float(f)),
                None => {}
            },
            Value::Float(f) => {
                if let Some(n) = f2i(*f) {
                    return Some(Value::Int(n));
                }
            }
            Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
                Some(Num::Int(n)) => return Some(Value::Int(n)),
                Some(Num::Float(f)) => {
                    if let Some(n) = f2i(f) {
                        return Some(Value::Int(n));
                    }
                }
                None => {}
            },
            Value::Bool(b) => return Some(Value::Int(*b as i64)),
            _ => {}
        }
    }
    if t_float {
        match d {
            Value::Int(n) => return Some(Value::Float(*n as f64)),
            Value::Bool(b) => return Some(Value::Float(*b as i64 as f64)),
            Value::Str(s) => {
                if let Some(num) = conv::numeric_string(s.as_bytes()) {
                    return Some(Value::Float(num.as_f64()));
                }
            }
            _ => {}
        }
    }
    if t_str && matches!(d, Value::Int(_) | Value::Float(_) | Value::Bool(_)) {
        return Some(Value::str(conv::to_string_bytes(d)));
    }
    if t_bool && matches!(d, Value::Int(_) | Value::Float(_) | Value::Str(_)) {
        return Some(Value::Bool(conv::to_bool(d)));
    }
    None
}

/// The `(A&B)` members of a DNF type, as php prints them.
fn dnf_group_strings(ty: &rhp_bytecode::TypeHint) -> Vec<String> {
    ty.groups
        .iter()
        .map(|g| {
            let names: Vec<String> = g
                .iter()
                .map(|n| String::from_utf8_lossy(n.as_bytes()).into_owned())
                .collect();
            format!("({})", names.join("&"))
        })
        .collect()
}

/// Put a DNF type's `(A&B)` groups among its class members (`classes`)
/// at their declared positions.
pub(crate) fn interleave_groups<T>(classes: &mut Vec<T>, groups: Vec<T>, pos: &[u16]) {
    for (i, g) in groups.into_iter().enumerate() {
        let at = pos.get(i).map_or(i, |&p| p as usize + i);
        classes.insert(at.min(classes.len()), g);
    }
}

/// The type name and zero value a null argument coerces to for a scalar
/// parameter of an internal function, or `None` when the type accepts null
/// or is not a single scalar (unions print in php's own order).
fn prelude_null_coercion(ty: &rhp_bytecode::TypeHint) -> Option<(String, Value)> {
    use rhp_bytecode::TypeKind as K;
    if ty.nullable || ty.intersection {
        return None;
    }
    // Z_PARAM_NUMBER
    if let [K::Int, K::Float] | [K::Float, K::Int] = ty.kinds[..] {
        return Some(("int|float".into(), Value::Int(0)));
    }
    // Z_PARAM_OBJ_OR_STR / Z_PARAM_OBJ_OF_CLASS_OR_STR
    match &ty.kinds[..] {
        [K::Object, K::String] | [K::String, K::Object] => {
            return Some(("object|string".into(), Value::str("")));
        }
        [K::Named(c), K::String] | [K::String, K::Named(c)] => {
            return Some((
                format!("{}|string", String::from_utf8_lossy(c.as_bytes())),
                Value::str(""),
            ));
        }
        _ => {}
    }
    if ty.kinds.len() != 1 {
        return None;
    }
    match ty.kinds[0] {
        K::Int => Some(("int".into(), Value::Int(0))),
        K::Float => Some(("float".into(), Value::Float(0.0))),
        K::String => Some(("string".into(), Value::str(""))),
        K::Bool => Some(("bool".into(), Value::Bool(false))),
        _ => None,
    }
}

/// Whether zend_check_type_slow reaches zend_is_callable for `v`: a string
/// or array the type takes as such (`callable|string`) is not looked up.
fn needs_callable_check(kinds: &[rhp_bytecode::TypeKind], v: &Value) -> bool {
    use rhp_bytecode::TypeKind as K;
    kinds.contains(&K::Callable)
        && match deref(v) {
            Value::Str(_) => !kinds.contains(&K::String),
            Value::Array(_) => !kinds.iter().any(|k| matches!(k, K::Array | K::Iterable)),
            _ => false,
        }
}
