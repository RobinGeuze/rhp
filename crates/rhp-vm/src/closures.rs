//! Closure objects (Zend/zend_closures.c): debug info, binding checks and
//! `Closure::getCurrent()`.

use std::rc::Rc;

use rhp_bytecode::{FunctionProto, Instr};
use rhp_values::ops::deref;
use rhp_values::{ClosureData, ObjectHandle, PhpArray, PhpStr, Thrown, Value};

use crate::Vm;

/// One parameter as the debug info lists it: name, by reference, optional.
type DebugParam = (Vec<u8>, bool, bool);

fn put(a: &mut PhpArray, k: &str, v: Value) {
    a.insert(rhp_values::ArrayKey::Str(PhpStr::from(k)), v);
}

/// The parameters of a user function (`arg_info`, `required_num_args`).
fn proto_params(p: &FunctionProto) -> Vec<DebugParam> {
    let required = p
        .params
        .iter()
        .rposition(|q| q.default.is_none() && !q.variadic)
        .map_or(0, |i| i + 1);
    p.params
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let name = p.names[q.name as usize].as_bytes().to_vec();
            (name, q.by_ref, i >= required)
        })
        .collect()
}

/// The parameters of a native function from its stub.
fn stub_params(ps: &[rhp_bytecode::signatures::Param]) -> Vec<DebugParam> {
    use rhp_bytecode::signatures::{PR, R, U, V};
    let required = ps
        .iter()
        .rposition(|q| q.default.is_empty() && q.flags & (U | V) == 0)
        .map_or(0, |i| i + 1);
    ps.iter()
        .enumerate()
        .map(|(i, q)| {
            (
                q.name.as_bytes().to_vec(),
                q.flags & (R | PR) != 0,
                i >= required,
            )
        })
        .collect()
}

/// A closure body's static variables in declaration order, with their
/// declared initial values (php's `op_array.static_variables`).
fn declared_statics(p: &FunctionProto) -> Vec<(PhpStr, Value)> {
    let mut out: Vec<(PhpStr, Value)> = Vec::new();
    for ins in &p.code {
        let (name, init) = match *ins {
            Instr::BindStatic { name, init, .. } => (
                name,
                if init == u32::MAX {
                    Value::Null
                } else {
                    p.consts[init as usize].clone()
                },
            ),
            Instr::BindStaticOrJmp { name, .. } | Instr::BindStaticInit { name, .. } => {
                (name, Value::Null)
            }
            _ => continue,
        };
        let n = &p.names[name as usize];
        if !out.iter().any(|(k, _)| k == n) {
            out.push((n.clone(), init));
        }
    }
    out
}

impl Vm {
    /// zend_closure_get_debug_info: what `var_dump`/`print_r` show for a
    /// Closure.
    pub(crate) fn closure_debug_info(&self, cd: &ClosureData) -> PhpArray {
        let mut a = PhpArray::new();
        let mut this = cd.bound_this.clone();
        let params = match &cd.wrapped {
            Some(w) => {
                let (name, params, fcc_this) = self.fcc_debug_target(w, cd).unwrap_or_default();
                put(&mut a, "function", Value::str(name));
                // A user function's static variables (shared with it).
                if let Some(st) = self.fcc_function_statics(w) {
                    put(&mut a, "static", Value::Array(st));
                }
                if this.is_none() {
                    this = fcc_this;
                }
                params
            }
            None => {
                let Some(f) = self.closures.get(cd.fnidx as usize) else {
                    return a;
                };
                let p = &f.proto;
                put(&mut a, "name", Value::Str(p.name.clone()));
                put(&mut a, "file", Value::Str(p.debug.file.clone()));
                put(&mut a, "line", Value::Int(p.meta.start_line as i64));
                // Captured variables are static variables in php, before
                // the declared ones.
                let mut st = PhpArray::new();
                for (i, v) in cd.captures.iter().enumerate() {
                    if p.this_cv == Some(i as u16) {
                        if this.is_none() {
                            if let Value::Object(h) = deref(v) {
                                this = Some(h);
                            }
                        }
                        continue;
                    }
                    if let Some(n) = p.cv_var_names.get(i) {
                        st.insert(rhp_values::ArrayKey::Str(n.clone()), deref(v));
                    }
                }
                let own = cd.statics.borrow();
                for (n, init) in declared_statics(p) {
                    let v = match own.iter().find(|(k, _)| *k == n) {
                        Some((_, c)) => deref(&c.borrow()),
                        None => init,
                    };
                    st.insert(rhp_values::ArrayKey::Str(n), v);
                }
                if !st.is_empty() {
                    put(&mut a, "static", Value::Array(st));
                }
                proto_params(p)
            }
        };
        if let Some(h) = this {
            put(&mut a, "this", Value::Object(h));
        }
        if !params.is_empty() {
            let mut ps = PhpArray::new();
            for (name, by_ref, optional) in params {
                let key = [if by_ref { &b"&$"[..] } else { b"$" }, &name].concat();
                let v = if optional { "<optional>" } else { "<required>" };
                ps.insert(
                    rhp_values::ArrayKey::Str(PhpStr::from_bytes(key)),
                    Value::str(v),
                );
            }
            put(&mut a, "parameter", Value::Array(ps));
        }
        a
    }

    /// The function a first-class-callable Closure wraps: its name as the
    /// debug info shows it (`Class::method` for a method), its parameters
    /// and the bound object.
    fn fcc_debug_target(
        &self,
        w: &Value,
        cd: &ClosureData,
    ) -> Option<(Vec<u8>, Vec<DebugParam>, Option<ObjectHandle>)> {
        let method = |cid: u32, m: &[u8], this: Option<ObjectHandle>| {
            let rm = self.lookup_method(cid, m)?;
            let cname = self.classes[rm.defining_class as usize].name.as_bytes();
            let name = [cname, b"::", rm.proto.name.as_bytes()].concat();
            let params = match rhp_bytecode::signatures::method(cname, rm.proto.name.as_bytes()) {
                Some(sig) if self.is_internal_class(rm.defining_class) => stub_params(sig.params),
                _ => proto_params(&rm.proto),
            };
            let this = if rm.is_static { None } else { this };
            Some((name, params, this))
        };
        match deref(w) {
            Value::Str(s) => {
                let n = s.as_bytes();
                let n = n.strip_prefix(b"\\").unwrap_or(n);
                if let Some(i) = n.windows(2).position(|p| p == b"::") {
                    let cid = self.scoped_class(&n[..i], cd)?;
                    return method(cid, &n[i + 2..], None);
                }
                let lc = n.to_ascii_lowercase();
                if let Some(&i) = self.fn_names.get(&lc) {
                    let p = &self.functions[i].proto;
                    let params = match rhp_bytecode::signatures::function(&lc) {
                        Some(sig) if p.is_internal() => stub_params(sig.params),
                        _ => proto_params(p),
                    };
                    return Some((p.name.as_bytes().to_vec(), params, None));
                }
                let sig = rhp_bytecode::signatures::function(&lc)?;
                Some((sig.name.as_bytes().to_vec(), stub_params(sig.params), None))
            }
            Value::Array(a) => {
                let v: Vec<Value> = a.iter().map(|(_, v)| deref(v)).collect();
                let [target, m] = &v[..] else { return None };
                let m = rhp_values::conv::to_string_bytes(m);
                match target {
                    Value::Object(h) => {
                        let cid = h.borrow().class;
                        method(cid, &m, Some(h.clone()))
                    }
                    Value::Closure(c) if m.eq_ignore_ascii_case(b"__invoke") => {
                        self.closure_invoke_target(c)
                    }
                    other => {
                        let n = rhp_values::conv::to_string_bytes(other);
                        let cid = self.scoped_class(&n, cd)?;
                        method(cid, &m, None)
                    }
                }
            }
            Value::Object(h) => {
                let cid = h.borrow().class;
                method(cid, b"__invoke", Some(h.clone()))
            }
            Value::Closure(c) => self.closure_invoke_target(&c),
            _ => None,
        }
    }

    /// The static variables of the user function a first-class callable
    /// wraps, if it has any.
    fn fcc_function_statics(&self, w: &Value) -> Option<PhpArray> {
        let Value::Str(n) = deref(w) else { return None };
        let n = n.as_bytes();
        let n = n.strip_prefix(b"\\").unwrap_or(n);
        let f = &self.functions[*self.fn_names.get(&n.to_ascii_lowercase())?];
        if f.proto.is_internal() {
            return None;
        }
        let mut st = PhpArray::new();
        for (name, init) in declared_statics(&f.proto) {
            let key = (
                Rc::as_ptr(&f.proto) as usize,
                f.unit,
                None,
                name.as_bytes().to_vec(),
            );
            let v = match self.statics.get(&key) {
                Some(c) => deref(&c.borrow()),
                None => init,
            };
            st.insert(rhp_values::ArrayKey::Str(name), v);
        }
        (!st.is_empty()).then_some(st)
    }

    /// A class named in a first-class callable: `self`, `parent` and
    /// `static` resolve against the scope the closure was created in.
    fn scoped_class(&self, name: &[u8], cd: &ClosureData) -> Option<u32> {
        let name = name.strip_prefix(b"\\").unwrap_or(name);
        match name.to_ascii_lowercase().as_slice() {
            b"self" => cd.bound_class,
            b"static" => cd.bound_static,
            b"parent" => self.classes[cd.bound_class? as usize].parent,
            _ => self.resolve_class_by_name(name).ok(),
        }
    }

    /// `Closure::__invoke` as a first-class callable target.
    fn closure_invoke_target(
        &self,
        c: &Rc<ClosureData>,
    ) -> Option<(Vec<u8>, Vec<DebugParam>, Option<ObjectHandle>)> {
        let params = match &c.wrapped {
            Some(w) => self.fcc_debug_target(w, c)?.1,
            None => proto_params(&self.closures.get(c.fnidx as usize)?.proto),
        };
        Some((b"Closure::__invoke".to_vec(), params, None))
    }
}

/// How a closure is rebound: `bind`/`bindTo` make a new Closure object,
/// `call` runs a temporary one (php's fake closure, no object handle).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum BindMode {
    Bind,
    Call,
}

impl Vm {
    /// The method a first-class-callable Closure wraps (its scope is the
    /// closure's scope), if it wraps one.
    fn fcc_method(&self, w: &Value, cd: &ClosureData) -> Option<crate::classes::ResolvedMethod> {
        match deref(w) {
            Value::Str(s) => {
                let n = s.as_bytes();
                let i = n.windows(2).position(|p| p == b"::")?;
                let cid = self.scoped_class(&n[..i], cd)?;
                self.lookup_method(cid, &n[i + 2..])
            }
            Value::Array(a) => {
                let v: Vec<Value> = a.iter().map(|(_, v)| deref(v)).collect();
                let [target, m] = &v[..] else { return None };
                let m = rhp_values::conv::to_string_bytes(m);
                let cid = match target {
                    Value::Object(h) => h.borrow().class,
                    Value::Closure(_) => return None,
                    other => self.scoped_class(&rhp_values::conv::to_string_bytes(other), cd)?,
                };
                self.lookup_method(cid, &m)
            }
            Value::Object(h) => {
                let cid = h.borrow().class;
                self.lookup_method(cid, b"__invoke")
            }
            _ => None,
        }
    }

    /// `Closure::bind`, `bindTo` and `call` (do_closure_bind,
    /// zend_valid_closure_binding): a warning and null for a binding php
    /// refuses. `func` names the method for argument errors; `first` is the
    /// argument number of `$newThis`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bind_closure(
        &mut self,
        func: &str,
        first: usize,
        closure: &Value,
        new_this: &Value,
        scope: Option<&Value>,
        mode: BindMode,
    ) -> Result<Value, Thrown> {
        let type_err = |vm: &Self, n: usize, name: &str, ty: &str, v: &Value| {
            Thrown::type_error(format!(
                "{func}(): Argument #{n} (${name}) must be of type {ty}, {} given",
                rhp_builtins::zval_type_name(vm, v)
            ))
        };
        let cd = match deref(closure) {
            Value::Closure(cd) => cd,
            other => return Err(type_err(self, 1, "closure", "Closure", &other)),
        };
        let new_this_h = match deref(new_this) {
            Value::Object(h) => Some(h),
            Value::Null if mode == BindMode::Bind => None,
            other => {
                let ty = if mode == BindMode::Call {
                    "object"
                } else {
                    "?object"
                };
                return Err(type_err(self, first, "newThis", ty, &other));
            }
        };
        let closure_cls = self.resolve_class_by_name(b"Closure").ok();
        // The function's own scope: the closure's, or the wrapped method's.
        let fake = cd.wrapped.as_ref().map(|w| self.fcc_method(w, &cd));
        let (func_scope, func_static) = match &fake {
            Some(Some(m)) => (Some(m.defining_class), m.is_static),
            Some(None) => (None, false),
            None => (
                cd.bound_class,
                self.closures
                    .get(cd.fnidx as usize)
                    .is_some_and(|f| f.proto.is_static),
            ),
        };
        let mut scope_cls = match mode {
            BindMode::Call => new_this_h.as_ref().map(|h| h.borrow().class),
            BindMode::Bind => match scope.map(deref) {
                None => func_scope,
                Some(Value::Null) => None,
                Some(Value::Object(h)) => Some(h.borrow().class),
                Some(Value::Closure(_)) => closure_cls,
                Some(v @ (Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_))) => {
                    let name = rhp_values::conv::to_string_bytes(&v);
                    if name == b"static" {
                        func_scope
                    } else {
                        match rhp_builtins::Host::resolve_class_autoload(self, &name)? {
                            Some(c) => Some(c),
                            None => {
                                self.diag(
                                    rhp_builtins::E_WARNING,
                                    format!(
                                        "Class \"{}\" not found",
                                        String::from_utf8_lossy(&name)
                                    )
                                    .as_bytes(),
                                )?;
                                return Ok(Value::Null);
                            }
                        }
                    }
                }
                Some(other) => {
                    return Err(type_err(
                        self,
                        first + 1,
                        "newScope",
                        "object|string|null",
                        &other,
                    ))
                }
            },
        };
        // zend_valid_closure_binding
        let has_this = cd.bound_this.is_some();
        let uses_this = cd.wrapped.is_none()
            && self
                .closures
                .get(cd.fnidx as usize)
                .is_some_and(|f| f.proto.flags.contains(rhp_bytecode::ProtoFlags::USES_THIS));
        let refuse = if let Some(nt) = &new_this_h {
            let nt_cls = nt.borrow().class;
            if func_static {
                Some("Cannot bind an instance to a static closure".to_string())
            } else if let (Some(_), Some(s)) = (&fake, func_scope) {
                (nt_cls != s && !self.class(nt_cls).ancestors.contains(&s)).then(|| {
                    format!(
                        "Cannot bind method {}::{}() to object of class {}",
                        self.class_name_str(s),
                        String::from_utf8_lossy(
                            fake.as_ref()
                                .and_then(|m| m.as_ref())
                                .map_or(&b""[..], |m| m.proto.name.as_bytes())
                        ),
                        self.class_name_str(nt_cls)
                    )
                })
            } else {
                None
            }
        } else if fake.is_some() && func_scope.is_some() && !func_static {
            Some("Cannot unbind $this of method".to_string())
        } else if fake.is_none() && has_this && uses_this {
            Some("Cannot unbind $this of closure using $this".to_string())
        } else {
            None
        };
        let refuse = refuse.or_else(|| match scope_cls {
            Some(s) if Some(s) != func_scope && self.is_internal_class(s) => Some(format!(
                "Cannot bind closure to scope of internal class {}",
                self.class_name_str(s)
            )),
            _ => None,
        });
        let refuse = refuse.or_else(|| {
            (fake.is_some() && scope_cls != func_scope).then(|| {
                if func_scope.is_none() {
                    "Cannot rebind scope of closure created from function".to_string()
                } else {
                    "Cannot rebind scope of closure created from method".to_string()
                }
            })
        });
        if let Some(msg) = refuse {
            self.diag(
                rhp_builtins::E_WARNING,
                format!("{msg}, this will be an error in PHP 9").as_bytes(),
            )?;
            return Ok(Value::Null);
        }
        // zend_create_closure: binding an object without a scope uses the
        // dummy scope Closure.
        if scope_cls.is_none() && new_this_h.is_some() && cd.wrapped.is_none() {
            scope_cls = closure_cls;
        }
        // zend_create_closure_ex: an internal free function keeps no `$this`.
        let mut new_this_h = new_this_h;
        if matches!(fake, Some(None)) && self.wraps_internal_function(&cd) {
            new_this_h = None;
        }
        let called = new_this_h.as_ref().map(|h| h.borrow().class).or(scope_cls);
        // A method's first-class callable runs on the new object.
        let wrapped = cd.wrapped.as_ref().map(|w| match (deref(w), &new_this_h) {
            (Value::Array(a), Some(nt))
                if matches!(a.iter().next(), Some((_, Value::Object(_)))) =>
            {
                let mut pair = PhpArray::new();
                pair.push(Value::Object(nt.clone()));
                pair.push(
                    a.iter()
                        .nth(1)
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Value::Null),
                );
                Box::new(Value::Array(pair))
            }
            _ => w.clone(),
        });
        let (id, statics) = match mode {
            BindMode::Bind => (rhp_values::alloc_object_id(), cd.statics_copy()),
            // php's fake closure shares the static variables and has no handle.
            BindMode::Call => (0, std::cell::RefCell::new(cd.statics.borrow().clone())),
        };
        // A nested arrow function's `$this` is captured: it follows the
        // new binding (gone when unbound).
        let mut captures = cd.captures.clone();
        if cd.wrapped.is_none() {
            let this_cv = self
                .closures
                .get(cd.fnidx as usize)
                .and_then(|f| f.proto.this_cv);
            if let Some(slot) = this_cv.and_then(|cv| captures.get_mut(cv as usize)) {
                *slot = new_this_h.clone().map_or(Value::Null, Value::Object);
            }
        }
        Ok(Value::Closure(Rc::new(ClosureData {
            fnidx: cd.fnidx,
            captures,
            bound_this: new_this_h,
            bound_class: scope_cls,
            bound_static: called,
            wrapped,
            id,
            statics,
        })))
    }

    /// Whether a first-class callable wraps a function that is not a user
    /// function (`strlen(...)`).
    fn wraps_internal_function(&self, cd: &ClosureData) -> bool {
        let Some(Value::Str(n)) = cd.wrapped.as_deref().map(deref) else {
            return false;
        };
        let n = n.as_bytes();
        let n = n.strip_prefix(b"\\").unwrap_or(n);
        if n.windows(2).any(|p| p == b"::") {
            return false;
        }
        self.fn_names
            .get(&n.to_ascii_lowercase())
            .is_none_or(|&i| self.functions[i].proto.is_internal())
    }

    fn class_name_str(&self, c: u32) -> String {
        String::from_utf8_lossy(self.classes[c as usize].name.as_bytes()).into_owned()
    }
}

impl Vm {
    /// `Closure::call($newThis)`: the temporary closure bound to `$newThis`
    /// and its class, or null when php refuses the binding.
    pub(crate) fn closure_call_target(
        &mut self,
        closure: &Value,
        new_this: &Value,
    ) -> Result<Value, Thrown> {
        self.bind_closure("Closure::call", 1, closure, new_this, None, BindMode::Call)
    }

    /// After `Closure::call`: static variables the temporary closure
    /// created become the closure's own (php's fake closure shares them).
    pub(crate) fn closure_call_done(&self, closure: &Value, bound: &Value) {
        let (Value::Closure(cd), Value::Closure(tmp)) = (deref(closure), bound) else {
            return;
        };
        let mut own = cd.statics.borrow_mut();
        for (k, c) in tmp.statics.borrow().iter() {
            if !own.iter().any(|(n, _)| n == k) {
                own.push((k.clone(), c.clone()));
            }
        }
    }
}
