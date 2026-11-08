//! Method and callable resolution, after php's zend_std_get_method,
//! zend_std_get_static_method (zend_object_handlers.c), zend_is_callable_ex
//! (zend_API.c) and the dynamic-call initializers (zend_execute.c).

use rhp_values::{ArrayKey, ClassId, ClassKind, ObjectHandle, Thrown, Value, Visibility};

use crate::calls::Callee;
use crate::classes::ResolvedMethod;
use crate::{deref, Vm};

/// What a method lookup found.
pub(crate) enum Found {
    Method(ResolvedMethod),
    /// The class's `__call`, to run with the method name.
    Call(ResolvedMethod),
    /// The class's `__callStatic`, to run with the method name.
    CallStatic(ResolvedMethod),
}

/// The calling context a callable resolves in (zend_is_callable_at_frame's
/// frame): its scope, called scope and `$this`.
struct Frame {
    scope: Option<ClassId>,
    called: Option<ClassId>,
    this: Option<ObjectHandle>,
}

/// zend_fcall_info_cache while it is being filled.
#[derive(Default)]
struct Fcc {
    calling: Option<ClassId>,
    called: Option<ClassId>,
    object: Option<ObjectHandle>,
}

fn vis_word(v: Visibility) -> &'static str {
    match v {
        Visibility::Private => "private",
        Visibility::Protected => "protected",
        Visibility::Public => "public",
    }
}

/// The method name a `__call` trampoline gets: php cuts it at a NUL byte.
pub(crate) fn trampoline_name(name: &[u8]) -> Vec<u8> {
    match name.iter().position(|&b| b == 0) {
        Some(i) => name[..i].to_vec(),
        None => name.to_vec(),
    }
}

impl Vm {
    pub(crate) fn class_name_lossy(&self, c: ClassId) -> String {
        String::from_utf8_lossy(self.class(c).name.as_bytes()).into_owned()
    }

    pub(crate) fn instance_of(&self, c: ClassId, of: ClassId) -> bool {
        self.class(c).ancestors.contains(&of)
    }

    /// zend_get_function_root_class: a constructor's prototype only counts
    /// when it is abstract (see `inherit`).
    pub(crate) fn method_root(&self, m: &ResolvedMethod) -> ClassId {
        if m.proto.name.as_bytes().eq_ignore_ascii_case(b"__construct") {
            m.prototype.unwrap_or(m.defining_class)
        } else {
            m.root
        }
    }

    /// zend_check_protected: `scope` and `root` are on one inheritance line.
    pub(crate) fn check_protected(&self, root: ClassId, scope: Option<ClassId>) -> bool {
        scope.is_some_and(|s| self.instance_of(root, s) || self.instance_of(s, root))
    }

    /// zend_check_method_accessible.
    pub(crate) fn method_accessible(&self, m: &ResolvedMethod, scope: Option<ClassId>) -> bool {
        m.visibility == Visibility::Public
            || Some(m.defining_class) == scope
            || (m.visibility == Visibility::Protected
                && self.check_protected(self.method_root(m), scope))
    }

    /// zend_bad_method_call: "Call to private method A::m() from scope B".
    pub(crate) fn bad_method_call(
        &self,
        m: &ResolvedMethod,
        name: &[u8],
        scope: Option<ClassId>,
    ) -> Thrown {
        let from = match scope {
            Some(c) => format!("scope {}", self.class_name_lossy(c)),
            None => "global scope".to_string(),
        };
        Thrown::error(
            "Error",
            format!(
                "Call to {} method {}::{}() from {from}",
                vis_word(m.visibility),
                self.class_name_lossy(m.defining_class),
                String::from_utf8_lossy(name)
            ),
        )
    }

    /// zend_undefined_method.
    pub(crate) fn undefined_method(&self, cls: ClassId, name: &[u8]) -> Thrown {
        Thrown::error(
            "Error",
            format!(
                "Call to undefined method {}::{}()",
                self.class_name_lossy(cls),
                String::from_utf8_lossy(name)
            ),
        )
    }

    /// zend_abstract_method_call.
    fn abstract_method_call(&self, m: &ResolvedMethod) -> Thrown {
        Thrown::error(
            "Error",
            format!(
                "Cannot call abstract method {}::{}()",
                self.class_name_lossy(m.defining_class),
                String::from_utf8_lossy(m.proto.name.as_bytes())
            ),
        )
    }

    /// zend_get_parent_private_method.
    fn parent_private_method(
        &self,
        scope: Option<ClassId>,
        cls: ClassId,
        name: &[u8],
    ) -> Option<ResolvedMethod> {
        let scope = scope.filter(|&s| s != cls && self.instance_of(cls, s))?;
        self.lookup_method(scope, name)
            .filter(|m| m.visibility == Visibility::Private && m.defining_class == scope)
    }

    /// zend_std_get_method: the method `$obj->name()` runs from `scope` on
    /// an object of class `cls`. `Ok(None)`: no such method and no
    /// `__call`.
    #[inline]
    pub(crate) fn get_method(
        &self,
        cls: ClassId,
        name: &[u8],
        scope: Option<ClassId>,
    ) -> Result<Option<Found>, Thrown> {
        match self.lookup_method(cls, name) {
            // Public and not shadowing a private, or the calling scope's own.
            Some(m)
                if !m.is_abstract
                    && ((m.visibility == Visibility::Public && !m.changed)
                        || Some(m.defining_class) == scope) =>
            {
                Ok(Some(Found::Method(m)))
            }
            m => self.get_method_slow(cls, name, scope, m),
        }
    }

    #[cold]
    #[inline(never)]
    fn get_method_slow(
        &self,
        cls: ClassId,
        name: &[u8],
        scope: Option<ClassId>,
        found: Option<ResolvedMethod>,
    ) -> Result<Option<Found>, Thrown> {
        let Some(mut fbc) = found else {
            // The class's get_method handler (PDO's driver methods).
            if let Some(helper) = self
                .class(cls)
                .handlers
                .and_then(|t| rhp_builtins::zend::object_handlers::driver_method(t, name))
            {
                if let Some(m) = self.lookup_method(cls, helper.as_bytes()) {
                    return Ok(Some(Found::Method(m)));
                }
            }
            return Ok(self.lookup_method(cls, b"__call").map(Found::Call));
        };
        if (fbc.visibility != Visibility::Public || fbc.changed)
            && Some(fbc.defining_class) != scope
        {
            let mut done = false;
            if fbc.changed {
                if let Some(p) = self.parent_private_method(scope, cls, name) {
                    fbc = p;
                    done = true;
                } else if fbc.visibility == Visibility::Public {
                    done = true;
                }
            }
            if !done
                && (fbc.visibility == Visibility::Private
                    || !self.check_protected(self.method_root(&fbc), scope))
            {
                return match self.lookup_method(cls, b"__call") {
                    Some(c) => Ok(Some(Found::Call(c))),
                    None => Err(self.bad_method_call(&fbc, name, scope)),
                };
            }
        }
        if fbc.is_abstract {
            return Err(self.abstract_method_call(&fbc));
        }
        Ok(Some(Found::Method(fbc)))
    }

    /// get_static_method_fallback: `__call` (of `$this`'s class) when the
    /// calling frame's `$this` is-a `cls`, else `__callStatic`.
    fn static_method_fallback(&self, cls: ClassId, this: Option<&ObjectHandle>) -> Option<Found> {
        if self.lookup_method(cls, b"__call").is_some() {
            if let Some(h) = this {
                let oc = h.borrow().class;
                if self.instance_of(oc, cls) {
                    return self.lookup_method(oc, b"__call").map(Found::Call);
                }
            }
        }
        self.lookup_method(cls, b"__callstatic")
            .map(Found::CallStatic)
    }

    /// zend_std_get_static_method: the method `C::name()` run by frame `top`
    /// calls (its scope; its `$this` for the `__call` fallback).
    /// `Ok(None)`: undefined.
    pub(crate) fn get_static_method(
        &mut self,
        cls: ClassId,
        name: &[u8],
        top: usize,
    ) -> Result<Option<Found>, Thrown> {
        let scope = self.stack[top].defining_class;
        let found = match self.lookup_method(cls, name) {
            Some(m) if self.method_accessible(&m, scope) => {
                if !m.is_abstract && self.class(m.defining_class).kind != ClassKind::Trait {
                    return Ok(Some(Found::Method(m)));
                }
                Some(Found::Method(m))
            }
            Some(m) => match self.static_method_fallback(cls, self.current_this(top).as_ref()) {
                Some(f) => Some(f),
                None => return Err(self.bad_method_call(&m, name, scope)),
            },
            None => self.static_method_fallback(cls, self.current_this(top).as_ref()),
        };
        if let Some(Found::Call(m) | Found::CallStatic(m)) = &found {
            // An abstract trampoline (zend_get_call_trampoline_func keeps
            // the flag), named after the called method.
            if m.is_abstract {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot call abstract method {}::{}()",
                        self.class_name_lossy(m.defining_class),
                        String::from_utf8_lossy(name)
                    ),
                ));
            }
            // The trampoline's scope is the called class.
            if self.class(cls).kind == ClassKind::Trait {
                let msg = format!(
                    "Calling static trait method {}::{} is deprecated, it should only be called on a class using the trait",
                    self.class_name_lossy(cls),
                    String::from_utf8_lossy(name)
                );
                self.diag(rhp_builtins::E_DEPRECATED, &msg)?;
            }
        }
        if let Some(Found::Method(m)) = &found {
            if m.is_abstract {
                return Err(self.abstract_method_call(m));
            }
            if self.class(m.defining_class).kind == ClassKind::Trait {
                let msg = format!(
                    "Calling static trait method {}::{} is deprecated, it should only be called on a class using the trait",
                    self.class_name_lossy(m.defining_class),
                    String::from_utf8_lossy(m.proto.name.as_bytes())
                );
                self.diag(rhp_builtins::E_DEPRECATED, &msg)?;
            }
        }
        Ok(found)
    }

    /// The frame a callable resolves in (zend_is_callable_ex: the innermost
    /// user frame, or a first-class callable's creation scope).
    fn callable_frame(&self) -> Frame {
        let (scope, called, this) = self.callable_scope();
        Frame {
            scope,
            called,
            this,
        }
    }

    /// zend_is_callable_check_class.
    #[allow(clippy::too_many_arguments)]
    fn callable_check_class(
        &self,
        name: &[u8],
        scope: Option<ClassId>,
        frame: &Frame,
        fcc: &mut Fcc,
        strict_class: &mut bool,
        suppress: bool,
        deps: &mut Vec<String>,
    ) -> Result<(), Vec<u8>> {
        *strict_class = false;
        let lc = name.to_ascii_lowercase();
        let no_scope =
            |w: &str| format!("cannot access \"{w}\" when no class scope is active").into_bytes();
        let forward = |c: ClassId| match frame.called {
            Some(called) if self.instance_of(called, c) => called,
            _ => c,
        };
        match lc.as_slice() {
            b"self" | b"parent" => {
                let s =
                    scope.ok_or_else(|| no_scope(if lc == b"self" { "self" } else { "parent" }))?;
                let c = if lc == b"self" {
                    s
                } else {
                    self.class(s).parent.ok_or_else(|| {
                        b"cannot access \"parent\" when current class scope has no parent".to_vec()
                    })?
                };
                if !suppress {
                    deps.push(format!(
                        "Use of \"{}\" in callables is deprecated",
                        if lc == b"self" { "self" } else { "parent" }
                    ));
                }
                fcc.called = Some(forward(c));
                fcc.calling = Some(c);
                if fcc.object.is_none() {
                    fcc.object = frame.this.clone();
                }
                *strict_class = lc == b"parent";
            }
            b"static" => {
                let c = frame.called.ok_or_else(|| no_scope("static"))?;
                if !suppress {
                    deps.push("Use of \"static\" in callables is deprecated".to_string());
                }
                fcc.called = Some(c);
                fcc.calling = Some(c);
                if fcc.object.is_none() {
                    fcc.object = frame.this.clone();
                }
                *strict_class = true;
            }
            _ => {
                let Ok(ce) = self.resolve_class_by_name(name) else {
                    return Err([b"class \"", name, b"\" not found"].concat());
                };
                fcc.calling = Some(ce);
                match (frame.scope, &fcc.object) {
                    (Some(s), None) => {
                        let bound = frame.this.as_ref().filter(|o| {
                            self.instance_of(o.borrow().class, s) && self.instance_of(s, ce)
                        });
                        match bound {
                            Some(o) => {
                                fcc.called = Some(o.borrow().class);
                                fcc.object = Some(o.clone());
                            }
                            None => fcc.called = Some(ce),
                        }
                    }
                    (_, o) => fcc.called = Some(o.as_ref().map_or(ce, |o| o.borrow().class)),
                }
                *strict_class = true;
            }
        }
        Ok(())
    }

    /// zend_is_callable_check_func: the function or method half of a
    /// callable (`fcc.calling` holds the array callable's class, if any).
    #[allow(clippy::too_many_arguments)]
    fn callable_check_func(
        &self,
        callable: &[u8],
        frame: &Frame,
        fcc: &mut Fcc,
        mut strict_class: bool,
        suppress: bool,
        deps: &mut Vec<String>,
    ) -> Result<Callee, Vec<u8>> {
        let ce_org = fcc.calling.take();
        if ce_org.is_none() {
            let name = callable.strip_prefix(b"\\").unwrap_or(callable);
            if let Some(&idx) = self.fn_names.get(&name.to_ascii_lowercase()) {
                return Ok(Callee::Function(self.functions[idx].clone()));
            }
            if crate::calls::user_visible_builtin(name) {
                return Ok(Callee::Builtin(String::from_utf8_lossy(name).into_owned()));
            }
        }
        // Split at the last "::".
        let colon = callable.iter().rposition(|&b| b == b':');
        let split = colon.filter(|&i| i > 0 && callable[i - 1] == b':');
        let mname: &[u8] = if let Some(i) = split {
            let clen = i - 1;
            if clen == 0 {
                return Err(b"invalid function name".to_vec());
            }
            let scope = ce_org.or(frame.scope);
            self.callable_check_class(
                &callable[..clen],
                scope,
                frame,
                fcc,
                &mut strict_class,
                suppress || ce_org.is_some(),
                deps,
            )?;
            let calling = fcc.calling.expect("class resolved");
            if let Some(org) = ce_org {
                if !self.instance_of(org, calling) {
                    return Err([
                        b"class ",
                        self.class(org).name.as_bytes(),
                        b" is not a subclass of ",
                        self.class(calling).name.as_bytes(),
                    ]
                    .concat());
                }
                if !suppress {
                    deps.push(format!(
                        "Callables of the form [\"{}\", \"{}\"] are deprecated",
                        self.class_name_lossy(org),
                        String::from_utf8_lossy(callable)
                    ));
                }
            }
            &callable[i + 1..]
        } else if let Some(org) = ce_org {
            fcc.calling = Some(org);
            callable
        } else {
            return Err([
                b"function \"",
                callable,
                b"\" not found or invalid function name",
            ]
            .concat());
        };
        let lm = mname.to_ascii_lowercase();
        let calling = fcc.calling;
        let mut fbc: Option<Found> = None;
        let mut via_handler = false;
        let mut lookup_failed = true;
        if let (true, Some(c), true) = (strict_class, calling, lm == b"__construct") {
            fbc = self.lookup_method(c, b"__construct").map(Found::Method);
            lookup_failed = false;
        } else if let Some(m) = calling.and_then(|c| self.lookup_method(c, &lm)) {
            lookup_failed = false;
            let mut m = m;
            if m.changed && !strict_class {
                if let Some(s) = frame.scope {
                    if self.instance_of(m.defining_class, s) {
                        if let Some(p) = self.lookup_method(s, &lm).filter(|p| {
                            p.visibility == Visibility::Private && p.defining_class == s
                        }) {
                            m = p;
                        }
                    }
                }
            }
            let c = calling.expect("checked");
            let has_magic = || {
                let magic: &[u8] = if fcc.object.is_some() {
                    b"__call"
                } else {
                    b"__callstatic"
                };
                self.lookup_method(c, magic).is_some()
            };
            if m.visibility != Visibility::Public
                && !self.method_accessible(&m, frame.scope)
                && has_magic()
            {
                lookup_failed = true;
            } else {
                fbc = Some(Found::Method(m));
            }
        }
        if lookup_failed {
            // get_function_via_handler
            let own_class = calling.is_some() && calling == ce_org;
            if let (Some(obj), true) = (fcc.object.clone(), own_class) {
                let org = ce_org.expect("own class");
                if strict_class && self.lookup_method(org, b"__call").is_some() {
                    fbc = self.lookup_method(org, b"__call").map(Found::Call);
                    via_handler = true;
                } else {
                    let oc = obj.borrow().class;
                    if let Ok(Some(f)) = self.get_method(oc, mname, frame.scope) {
                        let ok = match &f {
                            Found::Method(m) | Found::Call(m) | Found::CallStatic(m) => {
                                !strict_class || self.instance_of(org, m.defining_class)
                            }
                        };
                        if ok {
                            via_handler = !matches!(f, Found::Method(_));
                            fbc = Some(f);
                        }
                    }
                }
            } else if let Some(c) = calling {
                if self.class(c).kind == ClassKind::Enum && self.is_enum_builtin(c, &lm) {
                    return Ok(Callee::EnumStatic(c, lm));
                }
                let f = match self.lookup_method(c, &lm) {
                    Some(m) if self.method_accessible(&m, frame.scope) => Some(Found::Method(m)),
                    _ => self.static_method_fallback(c, frame.this.as_ref()),
                };
                if let Some(f) = f {
                    via_handler = !matches!(f, Found::Method(_));
                    if via_handler && fcc.object.is_none() {
                        fcc.object = frame
                            .this
                            .clone()
                            .filter(|o| self.instance_of(o.borrow().class, c));
                    }
                    fbc = Some(f);
                }
            }
        }
        let Some(fbc) = fbc else {
            return Err(match calling {
                Some(c) => [
                    b"class ",
                    self.class(c).name.as_bytes(),
                    b" does not have a method \"",
                    mname,
                    b"\"",
                ]
                .concat(),
                None => [b"function ", mname, b"() does not exist"].concat(),
            });
        };
        if let (Some(c), Found::Method(m)) = (calling, &fbc) {
            if !via_handler {
                let msg = |pre: &[u8], post: &[u8]| {
                    [
                        pre,
                        self.class(c).name.as_bytes(),
                        b"::",
                        m.proto.name.as_bytes(),
                        post,
                    ]
                    .concat()
                };
                if m.is_abstract {
                    return Err(msg(b"cannot call abstract method ", b"()"));
                }
                if fcc.object.is_none() && !m.is_static {
                    return Err(msg(
                        b"non-static method ",
                        b"() cannot be called statically",
                    ));
                }
                if !self.method_accessible(m, frame.scope) {
                    let pre = format!("cannot access {} method ", vis_word(m.visibility));
                    return Err(msg(pre.as_bytes(), b"()"));
                }
            }
        }
        if let Some(o) = &fcc.object {
            fcc.called = Some(o.borrow().class);
        }
        let called = fcc.called.or(calling).expect("method callable has a class");
        Ok(match fbc {
            Found::Method(m) => {
                let this = if m.is_static { None } else { fcc.object.take() };
                Callee::Method(m, this, called)
            }
            Found::Call(m) => Callee::Magic(m, fcc.object.take(), called, trampoline_name(mname)),
            Found::CallStatic(m) => Callee::Magic(m, None, called, trampoline_name(mname)),
        })
    }

    /// zend_is_callable_at_frame: resolve a callable value, or say why it
    /// is not one in zend_is_callable_error's words. Deprecations it raises
    /// are pushed to `deps` (none when `suppress`).
    pub(crate) fn resolve_callable_ex(
        &self,
        callable: &Value,
        suppress: bool,
        deps: &mut Vec<String>,
    ) -> Result<Callee, Vec<u8>> {
        let mut fcc = Fcc::default();
        match deref(callable) {
            Value::Closure(cd) => Ok(Callee::Closure(cd)),
            Value::Str(s) => {
                let frame = self.callable_frame();
                self.callable_check_func(s.as_bytes(), &frame, &mut fcc, false, suppress, deps)
            }
            Value::Array(a) => {
                let frame = self.callable_frame();
                if a.len() != 2 {
                    return Err(b"array callback must have exactly two members".to_vec());
                }
                let (Some(obj), Some(method)) = (
                    a.get(&ArrayKey::Int(0)).map(deref),
                    a.get(&ArrayKey::Int(1)).map(deref),
                ) else {
                    return Err(b"array callback has to contain indices 0 and 1".to_vec());
                };
                if !matches!(obj, Value::Str(_) | Value::Object(_) | Value::Closure(_)) {
                    return Err(b"first array member is not a valid class name or object".to_vec());
                }
                let Value::Str(m) = method else {
                    return Err(b"second array member is not a valid method".to_vec());
                };
                let mut strict_class = false;
                match obj {
                    // zend_closure_get_method: a Closure's `__invoke`.
                    Value::Closure(cd) => {
                        return if m.as_bytes().eq_ignore_ascii_case(b"__invoke") {
                            Ok(Callee::Closure(cd))
                        } else if let Some(c) = self.closure_std_method(cd, m.as_bytes()) {
                            Ok(c)
                        } else {
                            Err([
                                &b"class Closure does not have a method \""[..],
                                m.as_bytes(),
                                b"\"",
                            ]
                            .concat())
                        };
                    }
                    Value::Str(cn) => self.callable_check_class(
                        cn.as_bytes(),
                        frame.scope,
                        &frame,
                        &mut fcc,
                        &mut strict_class,
                        suppress,
                        deps,
                    )?,
                    Value::Object(h) => {
                        fcc.calling = Some(h.borrow().class);
                        fcc.object = Some(h);
                    }
                    _ => unreachable!(),
                }
                self.callable_check_func(
                    m.as_bytes(),
                    &frame,
                    &mut fcc,
                    strict_class,
                    suppress,
                    deps,
                )
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

    /// zend_closure_get_method's fallback to Closure's own methods: the
    /// static ones run as declared, `bindTo`/`call` on the closure.
    fn closure_std_method(
        &self,
        cd: std::rc::Rc<rhp_values::ClosureData>,
        m: &[u8],
    ) -> Option<Callee> {
        let lc = m.to_ascii_lowercase();
        if matches!(lc.as_slice(), b"bindto" | b"call") {
            return Some(Callee::ClosureMethod(cd, lc));
        }
        let cls = self.closure_class()?;
        let rm = self.lookup_method(cls, &lc)?;
        (rm.is_static && rm.visibility == Visibility::Public)
            .then_some(Callee::Method(rm, None, cls))
    }

    /// [`Vm::resolve_callable_ex`] without deprecations: for a callable
    /// already checked (zpp's), or a check php makes silently.
    pub(crate) fn resolve_callable(&self, callable: &Value) -> Result<Callee, Vec<u8>> {
        self.resolve_callable_ex(callable, true, &mut Vec::new())
    }

    /// zend_is_callable_ex with its deprecations raised: autoloads the
    /// class a callable names first. `Ok(Err(why))`: not callable.
    pub(crate) fn check_callable_ex(
        &mut self,
        callable: &Value,
        suppress: bool,
    ) -> Result<Result<Callee, String>, Thrown> {
        self.autoload_callable_class(callable)?;
        let mut deps = Vec::new();
        let r = self.resolve_callable_ex(callable, suppress, &mut deps);
        for d in deps {
            if let Err(t) = self.diag(rhp_builtins::E_DEPRECATED, &d) {
                self.callable_dep_thrown = true;
                return Err(t);
            }
        }
        Ok(r.map_err(|e| String::from_utf8_lossy(&e).into_owned()))
    }

    /// A dynamic call's callee (`$f()` on a string or array:
    /// zend_init_dynamic_call_string / _array). Unlike a callable check,
    /// `self`/`parent`/`static` are not special and a non-static method
    /// never borrows `$this`.
    pub(crate) fn resolve_dynamic_call(&mut self, c: &Value) -> Result<Callee, Thrown> {
        let err = |m: String| Thrown::error("Error", m);
        let top = self.stack.len() - 1;
        let scope = self.stack[top].defining_class;
        match c {
            Value::Str(s) => {
                let b = s.as_bytes();
                let colon = b.iter().rposition(|&x| x == b':');
                if let Some(i) = colon.filter(|&i| i > 0 && b[i - 1] == b':') {
                    let cls = self.fetch_class_or_throw(&b[..i - 1])?;
                    let mname = &b[i + 1..];
                    let found = self.get_static_method(cls, mname, top)?;
                    return match found {
                        None => Err(self.undefined_method(cls, mname)),
                        Some(Found::Method(m)) if m.is_static => Ok(Callee::Method(m, None, cls)),
                        Some(Found::Method(m)) => {
                            Err(self
                                .non_static_method_call(m.defining_class, m.proto.name.as_bytes()))
                        }
                        Some(Found::Call(m)) => {
                            Err(self.non_static_method_call(m.defining_class, mname))
                        }
                        Some(Found::CallStatic(m)) => {
                            Ok(Callee::Magic(m, None, cls, trampoline_name(mname)))
                        }
                    };
                }
                let name = b.strip_prefix(b"\\").unwrap_or(b);
                if let Some(&idx) = self.fn_names.get(&name.to_ascii_lowercase()) {
                    return Ok(Callee::Function(self.functions[idx].clone()));
                }
                if crate::calls::user_visible_builtin(name) {
                    return Ok(Callee::Builtin(String::from_utf8_lossy(name).into_owned()));
                }
                Err(err(format!(
                    "Call to undefined function {}()",
                    String::from_utf8_lossy(b)
                )))
            }
            Value::Array(a) => {
                if a.len() != 2 {
                    return Err(err("Array callback must have exactly two elements".into()));
                }
                let (Some(obj), Some(method)) = (
                    a.get(&ArrayKey::Int(0)).map(deref),
                    a.get(&ArrayKey::Int(1)).map(deref),
                ) else {
                    return Err(err("Array callback has to contain indices 0 and 1".into()));
                };
                if !matches!(obj, Value::Str(_) | Value::Object(_) | Value::Closure(_)) {
                    return Err(err(
                        "First array member is not a valid class name or object".into(),
                    ));
                }
                let Value::Str(m) = method else {
                    return Err(err("Second array member is not a valid method".into()));
                };
                let mname = m.as_bytes();
                match obj {
                    Value::Str(cn) => {
                        let cls = self.fetch_class_or_throw(cn.as_bytes())?;
                        if self.class(cls).kind == ClassKind::Enum {
                            let lc = mname.to_ascii_lowercase();
                            if self.lookup_method(cls, mname).is_none()
                                && self.is_enum_builtin(cls, &lc)
                            {
                                return Ok(Callee::EnumStatic(cls, lc));
                            }
                        }
                        match self.get_static_method(cls, mname, top)? {
                            None => Err(self.undefined_method(cls, mname)),
                            Some(Found::Method(m)) if m.is_static => {
                                Ok(Callee::Method(m, None, cls))
                            }
                            Some(Found::Method(m)) => Err(self
                                .non_static_method_call(m.defining_class, m.proto.name.as_bytes())),
                            Some(Found::Call(m)) => {
                                Err(self.non_static_method_call(m.defining_class, mname))
                            }
                            Some(Found::CallStatic(m)) => {
                                Ok(Callee::Magic(m, None, cls, trampoline_name(mname)))
                            }
                        }
                    }
                    Value::Closure(cd) => {
                        if mname.eq_ignore_ascii_case(b"__invoke") {
                            Ok(Callee::Closure(cd))
                        } else if let Some(c) = self.closure_std_method(cd, mname) {
                            Ok(c)
                        } else {
                            Err(err(format!(
                                "Call to undefined method Closure::{}()",
                                String::from_utf8_lossy(mname)
                            )))
                        }
                    }
                    Value::Object(h) => {
                        let oc = h.borrow().class;
                        match self.get_method(oc, mname, scope)? {
                            None => Err(self.undefined_method(oc, mname)),
                            Some(Found::Method(m)) if m.is_static => {
                                Ok(Callee::Method(m, None, oc))
                            }
                            Some(Found::Method(m)) => Ok(Callee::Method(m, Some(h), oc)),
                            Some(Found::Call(m)) | Some(Found::CallStatic(m)) => {
                                Ok(Callee::Magic(m, Some(h), oc, trampoline_name(mname)))
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Value::Object(h) => {
                let cls = h.borrow().class;
                match self.lookup_method(cls, b"__invoke") {
                    Some(m) => Ok(Callee::Method(m, Some(h.clone()), cls)),
                    None => Err(err(format!(
                        "Object of type {} is not callable",
                        self.class_name_lossy(cls)
                    ))),
                }
            }
            v => Err(err(format!(
                "Value of type {} is not callable",
                rhp_builtins::zval_type_name(self, v)
            ))),
        }
    }

    /// zend_non_static_method_call.
    pub(crate) fn non_static_method_call(&self, cls: ClassId, name: &[u8]) -> Thrown {
        Thrown::error(
            "Error",
            format!(
                "Non-static method {}::{}() cannot be called statically",
                self.class_name_lossy(cls),
                String::from_utf8_lossy(name)
            ),
        )
    }

    /// zend_fetch_class_by_name with autoloading and the "Class not found"
    /// exception.
    fn fetch_class_or_throw(&mut self, name: &[u8]) -> Result<ClassId, Thrown> {
        if let Ok(c) = self.resolve_class_by_name(name) {
            return Ok(c);
        }
        self.try_autoload(name)?;
        self.resolve_class_by_name(name).map_err(|_| {
            let n = name.strip_prefix(b"\\").unwrap_or(name);
            Thrown::error(
                "Error",
                format!("Class \"{}\" not found", String::from_utf8_lossy(n)),
            )
        })
    }
}
