//! Reflection metadata queries (`Host::reflect_op`): the native half of the
//! PHP-source `Reflection*` classes (see `rhp-builtins/src/ext/reflection`)
//! and of the class/function introspection builtins (`get_declared_classes`,
//! `get_class_methods`, …). Each op returns plain php arrays describing a
//! class, function/method/closure, property, constant or attribute; the PHP
//! side wraps them in the Reflection objects.

use std::rc::Rc;

use rhp_bytecode::{AttributeProto, ConstInitProto, DeclMeta, FunctionProto, TypeHint, TypeKind};
use rhp_values::{ArrayKey, ClassId, ClassKind, PhpArray, PhpStr, Thrown, Value, Visibility};

use crate::classes::ResolvedMethod;
use crate::{deref, UnitFn, UnitId, Vm};

use rhp_builtins::signatures::{self as sigs, Signature};

fn put(a: &mut PhpArray, k: &str, v: Value) {
    a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
}

fn s(b: &[u8]) -> Value {
    Value::str(b.to_vec())
}

fn lc(b: &[u8]) -> Vec<u8> {
    b.to_ascii_lowercase()
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(deref).unwrap_or(Value::Null)
}

fn arg_bytes(args: &[Value], i: usize) -> Vec<u8> {
    rhp_values::conv::to_string_bytes(&arg(args, i))
}

fn vis_flag(v: Visibility) -> i64 {
    match v {
        Visibility::Public => 1,
        Visibility::Protected => 2,
        Visibility::Private => 4,
    }
}

fn opt_str(v: &Option<PhpStr>) -> Value {
    match v {
        Some(s) => Value::Str(s.clone()),
        None => Value::Bool(false),
    }
}

/// Attribute target flags (php's `ZEND_ATTRIBUTE_TARGET_*`).
pub(crate) const T_CLASS: i64 = 1;
pub(crate) const T_FUNCTION: i64 = 2;
pub(crate) const T_METHOD: i64 = 4;
pub(crate) const T_PROPERTY: i64 = 8;
pub(crate) const T_CLASS_CONST: i64 = 16;
pub(crate) const T_PARAMETER: i64 = 32;

/// A resolved function-like reflection subject.
#[derive(Clone)]
enum FnSubject {
    /// A user (or prelude) function.
    Func(UnitFn),
    /// A builtin (native) function.
    Builtin(&'static str),
    /// A method, as found on `class` (the reflected class).
    Method { class: ClassId, m: ResolvedMethod },
    /// An enum's engine-provided static method (`cases` / `from` / `tryFrom`).
    EnumBuiltin { class: ClassId, name: &'static str },
    /// A closure object (`proto` is its body).
    Closure {
        cd: Rc<rhp_values::ClosureData>,
        proto: Rc<FunctionProto>,
    },
}

impl Vm {
    /// Dispatch one reflection metadata op (see the module docs).
    pub(crate) fn reflect_op(&mut self, op: &[u8], args: &[Value]) -> Result<Value, Thrown> {
        match op {
            b"class" => {
                let name = arg_bytes(args, 0);
                let autoload = rhp_values::conv::to_bool(&arg(args, 1));
                let cid = if autoload {
                    match self.resolve_class_by_name(&name) {
                        Ok(c) => Some(c),
                        Err(_) => {
                            if self.try_autoload(&name)? {
                                self.resolve_class_by_name(&name).ok()
                            } else {
                                None
                            }
                        }
                    }
                } else {
                    self.resolve_class_by_name(&name).ok()
                };
                Ok(match cid {
                    Some(c) => self.class_info(c),
                    None => Value::Null,
                })
            }
            b"class_of" => match arg(args, 0) {
                Value::Object(h) => {
                    let c = h.borrow().class;
                    Ok(s(self.classes[c as usize].name.as_bytes()))
                }
                Value::Closure(_) => Ok(s(b"Closure")),
                _ => Ok(Value::Null),
            },
            b"methods" => {
                let cid = self.refl_class(args)?;
                Ok(self.class_methods(cid))
            }
            b"methods_visible" => {
                // get_class_methods(): names callable from the calling scope.
                let cid = self.refl_class(args)?;
                let scope = self.stack.last().and_then(|f| f.defining_class);
                let internal = self.is_internal_class(cid);
                let mut out = PhpArray::new();
                for m in self.classes[cid as usize].methods.values() {
                    if internal && crate::calls::is_engine_helper_name(m.proto.name.as_bytes()) {
                        continue;
                    }
                    if m.visibility == Visibility::Private && m.defining_class != cid {
                        if scope != Some(m.defining_class) {
                            continue;
                        }
                    } else if !self.check_access(scope, m.defining_class, m.visibility) {
                        continue;
                    }
                    out.push(Value::Str(m.proto.name.clone()));
                }
                for n in self.enum_builtin_names(cid) {
                    out.push(s(n.as_bytes()));
                }
                Ok(Value::Array(out))
            }
            b"ref_id" => {
                // ReflectionReference::fromArrayElement: an id for a reference
                // element, null for a plain value.
                let Value::Array(a) = arg(args, 0) else {
                    return Ok(Value::Null);
                };
                let key = match arg(args, 1) {
                    Value::Int(i) => ArrayKey::Int(i),
                    other => ArrayKey::from_bytes(&rhp_values::conv::to_string_bytes(&other)),
                };
                Ok(match a.get(&key) {
                    Some(Value::Ref(r)) => {
                        let p = Rc::as_ptr(r) as usize as u64;
                        s(format!("{:040x}", p.wrapping_mul(0x9E37_79B9_7F4A_7C15)).as_bytes())
                    }
                    _ => Value::Null,
                })
            }
            b"props" => {
                let cid = self.refl_class(args)?;
                let mut out = PhpArray::new();
                for (name, decl, is_static) in self.prop_order(cid) {
                    let mut d = PhpArray::new();
                    d.push(Value::Str(name));
                    d.push(s(self.classes[decl as usize].name.as_bytes()));
                    d.push(Value::Bool(is_static));
                    out.push(Value::Array(d));
                }
                Ok(Value::Array(out))
            }
            b"consts" => {
                let cid = self.refl_class(args)?;
                let mut out = PhpArray::new();
                let e = &self.classes[cid as usize];
                let mut names: Vec<(Vec<u8>, ClassId)> = e
                    .constants
                    .iter()
                    // A parent's private constants are not inherited.
                    .filter(|(_, c)| {
                        c.declaring_class == cid
                            || !matches!(c.visibility, Visibility::Private)
                            || e.traits.contains(&c.declaring_class)
                    })
                    .map(|(k, c)| (k.clone(), c.declaring_class))
                    .collect();
                // Own constants and enum cases in declaration order first.
                let own_off = |k: &[u8]| -> Option<u32> {
                    e.proto
                        .constants
                        .iter()
                        .find(|c| c.name.as_bytes() == k)
                        .map(|c| c.meta.offset)
                        .or_else(|| {
                            e.proto
                                .enum_cases
                                .iter()
                                .find(|c| c.name.as_bytes() == k)
                                .map(|c| c.meta.offset)
                        })
                };
                #[allow(clippy::type_complexity)]
                let mut keyed: Vec<(u8, u32, usize, (Vec<u8>, ClassId))> = names
                    .drain(..)
                    .enumerate()
                    .map(|(i, n)| match own_off(&n.0) {
                        Some(o) => (0, o, i, n),
                        None => (1, 0, i, n),
                    })
                    .collect();
                keyed.sort_by_key(|(a, b, c, _)| (*a, *b, *c));
                let names: Vec<(Vec<u8>, ClassId)> = keyed.into_iter().map(|t| t.3).collect();
                for (k, decl) in names {
                    let mut d = PhpArray::new();
                    d.push(s(&k));
                    d.push(s(self.classes[decl as usize].name.as_bytes()));
                    out.push(Value::Array(d));
                }
                Ok(Value::Array(out))
            }
            b"const" => {
                let cid = self.refl_class(args)?;
                let name = arg_bytes(args, 1);
                self.const_info(cid, &name)
            }
            b"prop" => {
                let cid = self.refl_class(args)?;
                let name = arg_bytes(args, 1);
                Ok(self.prop_info(cid, &name))
            }
            b"fn" => match self.fn_subject(&arg(args, 0))? {
                Some(sub) => Ok(self.fn_info(&sub)),
                None => Ok(Value::Null),
            },
            b"attr_args" => self.attr_args(args),
            b"deprecated" => {
                let msg = arg_bytes(args, 0);
                self.raise_error(rhp_builtins::E_DEPRECATED, &String::from_utf8_lossy(&msg));
                match self.pending_throw.take() {
                    Some(t) => Err(t),
                    None => Ok(Value::Null),
                }
            }
            b"param_default" => {
                let Some(sub) = self.fn_subject(&arg(args, 0))? else {
                    return Ok(Value::Null);
                };
                let idx = rhp_values::conv::to_int(&arg(args, 1)) as usize;
                self.param_default(&sub, idx)
            }
            b"declared" => {
                let want = arg_bytes(args, 0);
                // Built-in classes first, then class-table (compile) order.
                let mut ids: Vec<usize> = (0..self.classes.len()).collect();
                ids.sort_by_key(|&id| {
                    let user = id >= self.internal_classes;
                    (user, if user { self.class_order[id] } else { 0 }, id)
                });
                let mut out = PhpArray::new();
                for id in ids {
                    let e = &self.classes[id];
                    let ok = match want.as_slice() {
                        b"interface" => e.kind == ClassKind::Interface,
                        b"trait" => e.kind == ClassKind::Trait,
                        _ => matches!(e.kind, ClassKind::Class | ClassKind::Enum),
                    };
                    if ok {
                        out.push(Value::Str(e.name.clone()));
                    }
                }
                Ok(Value::Array(out))
            }
            b"functions" => {
                let mut internal: Vec<Vec<u8>> = rhp_builtins::registry()
                    .keys()
                    .filter(|n| crate::calls::user_visible_builtin(n.as_bytes()))
                    .map(|n| n.as_bytes().to_ascii_lowercase())
                    .collect();
                let mut user = PhpArray::new();
                for f in &self.functions {
                    let name = lc(f.proto.name.as_bytes());
                    if f.proto.is_internal() {
                        internal.push(name);
                    } else {
                        user.push(s(&name));
                    }
                }
                internal.sort();
                internal.dedup();
                let mut i = PhpArray::new();
                for n in internal {
                    i.push(s(&n));
                }
                let mut out = PhpArray::new();
                put(&mut out, "internal", Value::Array(i));
                put(&mut out, "user", Value::Array(user));
                Ok(Value::Array(out))
            }
            b"included" => {
                let mut out = PhpArray::new();
                // The main script counts when it is a real file (not `-r`
                // code or a script read from stdin).
                let main = self.main_script_path.clone().or_else(|| {
                    let f = self.stack.first()?.proto.debug.file.as_bytes();
                    rhp_builtins::vcwd::os_path(f).is_file().then(|| f.to_vec())
                });
                if let Some(m) = &main {
                    out.push(s(m));
                }
                for f in &self.included_files {
                    if Some(f) != main.as_ref() {
                        out.push(s(f));
                    }
                }
                Ok(Value::Array(out))
            }
            b"is_a" => {
                // (class, other): instanceof between class names.
                let a = self.resolve_class_by_name(&arg_bytes(args, 0)).ok();
                let b = self.resolve_class_by_name(&arg_bytes(args, 1)).ok();
                Ok(Value::Bool(match (a, b) {
                    (Some(a), Some(b)) => self.classes[a as usize].ancestors.contains(&b),
                    _ => false,
                }))
            }
            b"scope" => {
                // The calling user frame's class scope (for visibility filters).
                let scope = self.stack.last().and_then(|f| f.defining_class);
                Ok(match scope {
                    Some(c) => s(self.classes[c as usize].name.as_bytes()),
                    None => Value::Null,
                })
            }
            b"static_props" => {
                let cid = self.refl_class(args)?;
                self.update_class_constants(cid)?;
                let mut out = PhpArray::new();
                let e = &self.classes[cid as usize];
                for (n, _, st) in self.prop_order(cid) {
                    if !st {
                        continue;
                    }
                    if let Some(sp) = e.static_props.get(n.as_bytes()) {
                        out.insert(ArrayKey::from_bytes(n.as_bytes()), deref(&sp.cell.borrow()));
                    }
                }
                Ok(Value::Array(out))
            }
            b"set_static" => {
                let cid = self.refl_class(args)?;
                self.update_class_constants(cid)?;
                let name = arg_bytes(args, 1);
                let v = arg(args, 2);
                match self.classes[cid as usize].static_props.get(&name) {
                    Some(sp) => {
                        let cell = sp.cell.clone();
                        let mut c = cell.borrow_mut();
                        match &mut *c {
                            Value::Ref(r) => *r.borrow_mut() = v,
                            slot => *slot = v,
                        }
                        Ok(Value::Bool(true))
                    }
                    None => Ok(Value::Bool(false)),
                }
            }
            b"default_props" => {
                let cid = self.refl_class(args)?;
                self.update_class_constants(cid)?;
                Ok(self.default_props(cid, rhp_values::conv::to_bool(&arg(args, 1))))
            }
            b"is_init" => {
                // (object, class, name): is the declared slot initialized?
                let obj = arg(args, 0);
                let Value::Object(h) = obj else {
                    return Ok(Value::Bool(false));
                };
                let cid = self.refl_class(args.get(1..).unwrap_or(&[]))?;
                let name = PhpStr::from_bytes(arg_bytes(args, 2));
                let key = self.decl_prop_key(cid, &name);
                let b = h.borrow();
                Ok(Value::Bool(match key {
                    Some(k) => b.properties.contains_key(&k),
                    None => b.properties.contains_key(&name),
                }))
            }
            b"obj_props" => {
                // Dynamic property names of an object (for ReflectionObject).
                let Value::Object(h) = arg(args, 0) else {
                    return Ok(Value::Array(PhpArray::new()));
                };
                let b = h.borrow();
                let entry = &self.classes[b.class as usize];
                let mut out = PhpArray::new();
                for k in b.properties.keys() {
                    if !entry.props.contains_key(k) && k.as_bytes().first() != Some(&0) {
                        out.push(Value::Str(k.clone()));
                    }
                }
                Ok(Value::Array(out))
            }
            b"closure" => {
                // A Closure for a method (bound to the object) or function.
                let sub = self.fn_subject(&arg(args, 0))?;
                let this = match arg(args, 1) {
                    Value::Object(h) => Some(h),
                    _ => None,
                };
                Ok(match sub {
                    Some(FnSubject::Closure { cd, .. }) => Value::Closure(cd),
                    Some(FnSubject::Method { class, m }) => {
                        let called = this.as_ref().map(|h| h.borrow().class).unwrap_or(class);
                        self.method_closure(&m, this, called)
                    }
                    Some(FnSubject::Func(f)) => self.fcc_value(Value::Str(f.proto.name.clone())),
                    Some(FnSubject::EnumBuiltin { class, name }) => {
                        let mut a = PhpArray::new();
                        a.push(Value::Str(self.classes[class as usize].name.clone()));
                        a.push(s(name.as_bytes()));
                        self.fcc_value(Value::Array(a))
                    }
                    Some(FnSubject::Builtin(n)) => self.fcc_value(s(n.as_bytes())),
                    None => Value::Null,
                })
            }
            _ => Err(Thrown::error(
                "Error",
                format!("unknown reflection op {}", String::from_utf8_lossy(op)),
            )),
        }
    }

    /// A first-class-callable Closure wrapping `callable`.
    fn fcc_value(&self, callable: Value) -> Value {
        Value::Closure(Rc::new(rhp_values::ClosureData {
            fnidx: 0,
            captures: Vec::new(),
            bound_this: None,
            bound_class: None,
            bound_static: None,
            wrapped: Some(Box::new(callable)),
        }))
    }

    /// `ReflectionMethod::getClosure($obj)`: a closure over the method body.
    fn method_closure(
        &mut self,
        m: &ResolvedMethod,
        this: Option<rhp_values::ObjectHandle>,
        called: ClassId,
    ) -> Value {
        let mut pair = PhpArray::new();
        match &this {
            Some(h) => pair.push(Value::Object(h.clone())),
            None => pair.push(Value::Str(self.classes[called as usize].name.clone())),
        }
        pair.push(Value::Str(m.proto.name.clone()));
        Value::Closure(Rc::new(rhp_values::ClosureData {
            fnidx: 0,
            captures: Vec::new(),
            bound_this: this,
            bound_class: Some(m.defining_class),
            bound_static: Some(called),
            wrapped: Some(Box::new(Value::Array(pair))),
        }))
    }

    fn refl_class(&self, args: &[Value]) -> Result<ClassId, Thrown> {
        let name = arg_bytes(args, 0);
        self.resolve_class_by_name(&name)
    }

    pub(crate) fn is_internal_class(&self, cid: ClassId) -> bool {
        (cid as usize) < self.internal_classes
    }

    /// Reflection's extension name for a built-in class.
    fn class_extension(&self, cid: ClassId) -> Option<&'static str> {
        if !self.is_internal_class(cid) {
            return None;
        }
        let n = self.classes[cid as usize].name.as_bytes();
        Some(sigs::class(n).map_or("Core", |c| c.ext))
    }

    fn class_info(&self, cid: ClassId) -> Value {
        let e = &self.classes[cid as usize];
        let p = &e.proto;
        let full = e.name.as_bytes().to_vec();
        let (short, ns) = match full.iter().rposition(|&b| b == b'\\') {
            Some(i) => (full[i + 1..].to_vec(), full[..i].to_vec()),
            None => (full.clone(), Vec::new()),
        };
        let internal = self.is_internal_class(cid);
        let mut a = PhpArray::new();
        put(&mut a, "name", s(&full));
        put(&mut a, "short", s(&short));
        put(&mut a, "ns", s(&ns));
        let kind = match e.kind {
            ClassKind::Class => "class",
            ClassKind::Interface => "interface",
            ClassKind::Trait => "trait",
            ClassKind::Enum => "enum",
        };
        put(&mut a, "kind", s(kind.as_bytes()));
        put(&mut a, "internal", Value::Bool(internal));
        if internal {
            put(&mut a, "file", Value::Bool(false));
            put(&mut a, "start", Value::Bool(false));
            put(&mut a, "end", Value::Bool(false));
            put(&mut a, "doc", Value::Bool(false));
        } else {
            put(&mut a, "file", Value::Str(p.decl_file.clone()));
            put(&mut a, "start", Value::Int(p.meta.start_line as i64));
            put(&mut a, "end", Value::Int(p.meta.end_line as i64));
            put(&mut a, "doc", opt_str(&p.meta.doc_comment));
        }
        let anon = full.windows(10).any(|w| w == b"@anonymous");
        put(&mut a, "anon", Value::Bool(anon));
        put(&mut a, "create_object", Value::Bool(e.create_object));
        // php: ZEND_ACC_EXPLICIT_ABSTRACT_CLASS (64) for `abstract class`,
        // ZEND_ACC_IMPLICIT_ABSTRACT_CLASS (16) for a class with abstract
        // methods / an interface / a trait; final 32; readonly 65536.
        let has_abstract = e.methods.values().any(|m| m.is_abstract);
        let explicit = e.flags.is_abstract;
        let mut mods = 0i64;
        if explicit {
            mods |= 64;
        } else if e.kind == ClassKind::Interface || e.kind == ClassKind::Trait || has_abstract {
            // getModifiers() masks to the explicit flags only.
        }
        let is_final = e.flags.is_final && e.kind != ClassKind::Interface;
        if is_final {
            mods |= 32;
        }
        if e.flags.is_readonly {
            mods |= 65536;
        }
        put(&mut a, "modifiers", Value::Int(mods));
        put(&mut a, "abstract", Value::Bool(explicit || has_abstract));
        put(&mut a, "final", Value::Bool(is_final));
        put(&mut a, "readonly", Value::Bool(e.flags.is_readonly));
        let instantiable = e.kind == ClassKind::Class
            && !explicit
            && !has_abstract
            && match e.methods.get(&b"__construct"[..]) {
                Some(m) => m.visibility == Visibility::Public,
                None => true,
            };
        put(&mut a, "instantiable", Value::Bool(instantiable));
        let cloneable = e.kind == ClassKind::Class
            && !explicit
            && !has_abstract
            && match e.methods.get(&b"__clone"[..]) {
                Some(m) => m.visibility == Visibility::Public,
                // (php's Exception / Error declare a private __clone.)
                None => !self.uncloneable_class(cid),
            };
        put(&mut a, "cloneable", Value::Bool(cloneable));
        put(
            &mut a,
            "parent",
            match e.parent {
                Some(pid) => s(self.classes[pid as usize].name.as_bytes()),
                None => Value::Bool(false),
            },
        );
        let mut ifaces = PhpArray::new();
        for &i in &e.interfaces {
            ifaces.push(s(self.classes[i as usize].name.as_bytes()));
        }
        put(&mut a, "interfaces", Value::Array(ifaces));
        let mut traits = PhpArray::new();
        for &t in &e.traits {
            traits.push(s(self.classes[t as usize].name.as_bytes()));
        }
        put(&mut a, "traits", Value::Array(traits));
        put(
            &mut a,
            "ext",
            match self.class_extension(cid) {
                Some(x) => s(x.as_bytes()),
                None => Value::Bool(false),
            },
        );
        put(
            &mut a,
            "backing",
            match e.enum_backing {
                Some(rhp_bytecode::EnumBacking::Int) => s(b"int"),
                Some(rhp_bytecode::EnumBacking::Str) => s(b"string"),
                None => Value::Null,
            },
        );
        let traversable = self
            .resolve_class_by_name(b"Traversable")
            .is_ok_and(|t| e.ancestors.contains(&t));
        put(
            &mut a,
            "iterable",
            Value::Bool(traversable && e.kind == ClassKind::Class && !explicit && !has_abstract),
        );
        let mut cases = PhpArray::new();
        for c in &p.enum_cases {
            cases.push(Value::Str(c.name.clone()));
        }
        put(&mut a, "cases", Value::Array(cases));
        put(
            &mut a,
            "attrs",
            self.attrs_desc(&p.meta.attributes, T_CLASS, |i| {
                let mut k = PhpArray::new();
                k.push(s(b"class"));
                k.push(Value::Str(e.name.clone()));
                k.push(Value::Int(i as i64));
                Value::Array(k)
            }),
        );
        Value::Array(a)
    }

    /// `[name, declaringClass]` pairs in php's `getMethods()` order (own,
    /// trait, inherited; a parent's private methods are not listed).
    fn class_methods(&self, cid: ClassId) -> Value {
        let mut out = PhpArray::new();
        for m in self.classes[cid as usize].methods.values() {
            if m.visibility == Visibility::Private && m.defining_class != cid {
                // php does not list a parent's private methods.
                continue;
            }
            if self.is_internal_class(m.defining_class)
                && crate::calls::is_engine_helper_name(m.proto.name.as_bytes())
            {
                continue; // engine-internal helper of a prelude class
            }
            let mut d = PhpArray::new();
            d.push(Value::Str(m.proto.name.clone()));
            d.push(s(self.classes[m.defining_class as usize].name.as_bytes()));
            out.push(Value::Array(d));
        }
        for n in self.enum_builtin_names(cid) {
            let mut d = PhpArray::new();
            d.push(s(n.as_bytes()));
            d.push(Value::Str(self.classes[cid as usize].name.clone()));
            out.push(Value::Array(d));
        }
        Value::Array(out)
    }

    /// The engine-provided static methods of an enum.
    fn enum_builtin_names(&self, cid: ClassId) -> Vec<&'static str> {
        let e = &self.classes[cid as usize];
        if e.kind != ClassKind::Enum {
            return Vec::new();
        }
        if e.enum_backing.is_some() {
            vec!["cases", "from", "tryFrom"]
        } else {
            vec!["cases"]
        }
    }

    /// php's `properties_info` order for `cid`: own declarations (instance and
    /// static interleaved in declaration order), trait properties, then the
    /// parent's (minus redeclared names and the parent's privates).
    pub(crate) fn prop_order(&self, cid: ClassId) -> Vec<(PhpStr, ClassId, bool)> {
        self.prop_order_ex(cid, false)
    }

    /// [`Vm::prop_order`]; `with_private` also keeps the ancestors' private
    /// properties (php's `properties_info` holds them; `get_class_vars`
    /// shows them to their declaring scope).
    fn prop_order_ex(&self, cid: ClassId, with_private: bool) -> Vec<(PhpStr, ClassId, bool)> {
        let e = &self.classes[cid as usize];
        let mut out: Vec<(PhpStr, ClassId, bool)> = Vec::new();
        let own = own_props_ordered(&e.proto);
        let internal = self.is_internal_class(cid);
        // A built-in class's table is filled by inheritance first.
        if internal {
            if let Some(p) = e.parent {
                out = self.prop_order_ex(p, with_private);
                // A parent's private properties are not inherited.
                if !with_private {
                    out.retain(|(n, decl, st)| {
                        self.prop_vis(*decl, n, *st) != Some(Visibility::Private)
                    });
                }
            }
        }
        for (n, st) in own {
            // A prelude class's `__` state slots are engine-internal.
            if internal && n.as_bytes().starts_with(b"__") {
                continue;
            }
            if !out.iter().any(|(x, _, _)| *x == n) {
                out.push((n, cid, st));
            }
        }
        for &t in &e.traits {
            for (n, _, st) in self.prop_order(t) {
                if !out.iter().any(|(x, _, _)| *x == n) {
                    out.push((n, cid, st));
                }
            }
        }
        if let Some(p) = e.parent {
            for (n, decl, st) in self.prop_order_ex(p, with_private) {
                if out.iter().any(|(x, _, _)| *x == n) {
                    continue;
                }
                if !with_private && self.prop_vis(decl, &n, st) == Some(Visibility::Private) {
                    continue;
                }
                out.push((n, decl, st));
            }
        }
        out
    }

    /// Declared visibility of property `name` on its declaring class.
    fn prop_vis(&self, decl: ClassId, name: &PhpStr, is_static: bool) -> Option<Visibility> {
        let e = &self.classes[decl as usize];
        if is_static {
            e.static_props.get(name.as_bytes()).map(|sp| sp.visibility)
        } else {
            let key = e.prop_visible.get(name)?;
            e.props.get(key).map(|p| p.visibility)
        }
    }

    /// The storage key of the declared instance property `name` as seen on
    /// class `cid` (a private of `cid` itself wins).
    fn decl_prop_key(&self, cid: ClassId, name: &PhpStr) -> Option<PhpStr> {
        let e = &self.classes[cid as usize];
        if let Some(k) = e.own_private.get(name) {
            return Some(k.clone());
        }
        e.prop_visible.get(name).cloned()
    }

    /// Find the compiled declaration of property `name` of class `cid`
    /// (searching its traits for a trait-imported one). The last field is
    /// the class or trait that declares it.
    fn find_prop_meta(&self, cid: ClassId, name: &[u8]) -> Option<(DeclMeta, bool, bool, ClassId)> {
        let p = &self.classes[cid as usize].proto;
        if let Some(pp) = p.props.iter().find(|pp| pp.name.as_bytes() == name) {
            return Some((
                pp.meta.clone(),
                pp.promoted,
                !matches!(pp.default, rhp_bytecode::PropDefaultProto::None),
                cid,
            ));
        }
        if let Some(sp) = p.static_props.iter().find(|sp| sp.name.as_bytes() == name) {
            return Some((sp.meta.clone(), false, sp.has_default, cid));
        }
        for &t in &self.classes[cid as usize].traits {
            if let Some(r) = self.find_prop_meta(t, name) {
                return Some(r);
            }
        }
        None
    }

    /// [`Vm::find_prop_meta`] for a class constant or enum case.
    fn find_const_meta(&self, cid: ClassId, name: &[u8]) -> Option<(DeclMeta, bool, ClassId)> {
        let p = &self.classes[cid as usize].proto;
        if let Some(c) = p.constants.iter().find(|c| c.name.as_bytes() == name) {
            return Some((c.meta.clone(), c.is_final, cid));
        }
        if let Some(c) = p.enum_cases.iter().find(|c| c.name.as_bytes() == name) {
            return Some((c.meta.clone(), false, cid));
        }
        for &t in &self.classes[cid as usize].traits {
            if let Some(r) = self.find_const_meta(t, name) {
                return Some(r);
            }
        }
        None
    }

    fn prop_info(&self, cid: ClassId, name: &[u8]) -> Value {
        let e = &self.classes[cid as usize];
        let pname = PhpStr::from_bytes(name.to_vec());
        // (declaring class, visibility, set visibility, type, readonly, static, default)
        let found = if let Some(key) = self.decl_prop_key(cid, &pname) {
            let pe = &e.props[&key];
            Some((
                pe.declaring_class,
                pe.visibility,
                pe.set_visibility,
                pe.ty.clone(),
                pe.readonly,
                false,
                if pe.init.is_some() {
                    Value::Null
                } else {
                    pe.default.clone()
                },
            ))
        } else {
            e.static_props.get(name).map(|sp| {
                (
                    sp.declaring_class,
                    sp.visibility,
                    None,
                    sp.ty.clone(),
                    false,
                    true,
                    deref(&sp.default.borrow()),
                )
            })
        };
        let Some((decl, vis, setvis, ty, readonly, is_static, default)) = found else {
            return Value::Null;
        };
        // A parent's private property is not visible from the child.
        if vis == Visibility::Private && decl != cid {
            return Value::Null;
        }
        let (meta, promoted, has_default_decl, _) =
            self.find_prop_meta(decl, name).unwrap_or_default();
        let internal = self.is_internal_class(decl);
        let mut a = PhpArray::new();
        put(&mut a, "name", s(name));
        put(
            &mut a,
            "class",
            s(self.classes[decl as usize].name.as_bytes()),
        );
        put(&mut a, "static", Value::Bool(is_static));
        put(&mut a, "vis", Value::Int(vis_flag(vis)));
        let mut mods = vis_flag(vis);
        if is_static {
            mods |= 16;
        }
        if readonly {
            mods |= 128;
        }
        match setvis {
            Some(Visibility::Private) => mods |= 4096,
            Some(Visibility::Protected) => mods |= 2048,
            _ => {}
        }
        // php: a readonly property is implicitly protected(set) when public.
        if readonly && setvis.is_none() && vis == Visibility::Public {
            mods |= 2048;
        }
        put(&mut a, "modifiers", Value::Int(mods));
        put(&mut a, "readonly", Value::Bool(readonly));
        put(&mut a, "promoted", Value::Bool(promoted));
        let own = self.classes[decl as usize]
            .proto
            .props
            .iter()
            .any(|pp| pp.name.as_bytes() == name)
            || self.classes[decl as usize]
                .proto
                .static_props
                .iter()
                .any(|sp| sp.name.as_bytes() == name);
        let ctx = own.then_some(decl).and_then(|c| self.known_scope(c));
        put(
            &mut a,
            "type",
            match &ty {
                Some(t) => self.type_desc(t, ctx),
                None => Value::Null,
            },
        );
        // php: an untyped property always has a (null) default; a typed one
        // only when declared with an initializer; promoted ones never.
        let has_default = !promoted && (ty.is_none() || has_default_decl) && !is_static
            || (is_static && (ty.is_none() || has_default_decl));
        put(&mut a, "hasDefault", Value::Bool(has_default));
        put(
            &mut a,
            "default",
            if has_default { default } else { Value::Null },
        );
        put(
            &mut a,
            "doc",
            if internal {
                Value::Bool(false)
            } else {
                opt_str(&meta.doc_comment)
            },
        );
        let owner = self.classes[decl as usize].name.clone();
        put(
            &mut a,
            "attrs",
            self.attrs_desc(&meta.attributes, T_PROPERTY, |i| {
                let mut k = PhpArray::new();
                k.push(s(b"prop"));
                k.push(Value::Str(owner.clone()));
                k.push(s(name));
                k.push(Value::Int(i as i64));
                Value::Array(k)
            }),
        );
        Value::Array(a)
    }

    fn const_info(&mut self, cid: ClassId, name: &[u8]) -> Result<Value, Thrown> {
        let Some(c) = self.classes[cid as usize].constants.get(name).cloned() else {
            return Ok(Value::Null);
        };
        let decl = c.declaring_class;
        if decl != cid
            && matches!(c.visibility, Visibility::Private)
            && !self.classes[cid as usize].traits.contains(&decl)
        {
            return Ok(Value::Null);
        }
        let display = self.classes[cid as usize].name.as_bytes().to_vec();
        if c.init.is_some() {
            // Evaluate a pending constant expression (php's
            // zend_update_class_constant).
            self.class_const_value(cid, name, &display, false)?;
        }
        let value = self.classes[cid as usize]
            .constants
            .get(name)
            .map(|c| c.value.clone())
            .unwrap_or(Value::Null);
        let (meta, is_final, _) = self.find_const_meta(decl, name).unwrap_or_default();
        let is_case = self.classes[decl as usize]
            .proto
            .enum_cases
            .iter()
            .any(|ec| ec.name.as_bytes() == name);
        let internal = self.is_internal_class(decl);
        let mut a = PhpArray::new();
        put(&mut a, "name", s(name));
        put(
            &mut a,
            "class",
            s(self.classes[decl as usize].name.as_bytes()),
        );
        put(&mut a, "value", value);
        put(&mut a, "vis", Value::Int(vis_flag(c.visibility)));
        let mut mods = vis_flag(c.visibility);
        if is_final || is_case {
            mods |= 32;
        }
        put(&mut a, "modifiers", Value::Int(mods));
        put(&mut a, "final", Value::Bool(is_final || is_case));
        put(&mut a, "case", Value::Bool(is_case));
        put(
            &mut a,
            "type",
            match &c.ty {
                Some(t) => self.type_desc(t, self.known_scope(decl)),
                None => Value::Null,
            },
        );
        put(
            &mut a,
            "doc",
            if internal {
                Value::Bool(false)
            } else {
                opt_str(&meta.doc_comment)
            },
        );
        let owner = self.classes[decl as usize].name.clone();
        put(
            &mut a,
            "attrs",
            self.attrs_desc(&meta.attributes, T_CLASS_CONST, |i| {
                let mut k = PhpArray::new();
                k.push(s(b"const"));
                k.push(Value::Str(owner.clone()));
                k.push(s(name));
                k.push(Value::Int(i as i64));
                Value::Array(k)
            }),
        );
        Ok(Value::Array(a))
    }

    /// `getDefaultProperties()` (statics first) / `get_class_vars()`
    /// (`vars` = true: instance first, visibility-filtered by the caller).
    fn default_props(&self, cid: ClassId, vars: bool) -> Value {
        let order = self.prop_order_ex(cid, vars);
        let scope = self.stack.last().and_then(|f| f.defining_class);
        let mut statics = PhpArray::new();
        let mut inst = PhpArray::new();
        for (name, decl, is_static) in order {
            let vis = self.prop_vis(decl, &name, is_static);
            if vis.is_none() {
                continue;
            }
            if vars {
                let ok = match vis {
                    Some(Visibility::Public) => true,
                    Some(Visibility::Protected) => scope.is_some_and(|sc| {
                        self.classes[sc as usize].ancestors.contains(&decl)
                            || self.classes[decl as usize].ancestors.contains(&sc)
                    }),
                    _ => scope == Some(decl),
                };
                if !ok {
                    continue;
                }
            }
            let e = &self.classes[cid as usize];
            // A typed property without a default is uninitialized:
            // get_class_vars lists it as null, getDefaultProperties skips it.
            if is_static {
                if let Some(sp) = e.static_props.get(name.as_bytes()) {
                    let uninit = sp.ty.is_some()
                        && !self
                            .find_prop_meta(sp.declaring_class, name.as_bytes())
                            .is_some_and(|(_, _, has, _)| has);
                    if uninit && !vars {
                        continue;
                    }
                    statics.insert(
                        ArrayKey::from_bytes(name.as_bytes()),
                        if uninit {
                            Value::Null
                        } else {
                            deref(&sp.default.borrow())
                        },
                    );
                }
            } else if let Some(key) = self.decl_prop_key(cid, &name) {
                let pe = &e.props[&key];
                let v = if pe.starts_uninit() {
                    if !vars {
                        continue;
                    }
                    Value::Null
                } else {
                    pe.default.clone()
                };
                inst.insert(ArrayKey::from_bytes(name.as_bytes()), v);
            }
        }
        let (first, second) = if vars {
            (inst, statics)
        } else {
            (statics, inst)
        };
        let mut out = first;
        for (k, v) in second.iter() {
            out.insert(k.clone(), v.clone());
        }
        Value::Array(out)
    }

    /// Attribute descriptors: `[name, target, repeated, key]` where `key`
    /// locates the attribute again for `attr_args`.
    fn attrs_desc(
        &self,
        attrs: &[AttributeProto],
        target: i64,
        key: impl Fn(usize) -> Value,
    ) -> Value {
        let mut out = PhpArray::new();
        for (i, at) in attrs.iter().enumerate() {
            let repeated = attrs
                .iter()
                .filter(|o| o.name.as_bytes().eq_ignore_ascii_case(at.name.as_bytes()))
                .count()
                > 1;
            let mut d = PhpArray::new();
            d.push(Value::Str(at.name.clone()));
            d.push(Value::Int(target));
            d.push(Value::Bool(repeated));
            d.push(key(i));
            out.push(Value::Array(d));
        }
        Value::Array(out)
    }

    /// Evaluate an attribute's argument array; `args[0]` is its key
    /// (`[kind, owner…, index]`).
    fn attr_args(&mut self, args: &[Value]) -> Result<Value, Thrown> {
        let Value::Array(key) = arg(args, 0) else {
            return Ok(Value::Array(PhpArray::new()));
        };
        let mut k: Vec<Value> = key.iter().map(|(_, v)| v.clone()).collect();
        // A forged key (the prelude's helpers are callable from PHP) must
        // not index out of bounds.
        if k.len() < 4 {
            k.resize(4, Value::Null);
        }
        let kind = rhp_values::conv::to_string_bytes(&k[0]);
        let idx_of = |v: &Value| rhp_values::conv::to_int(v) as usize;
        // (attribute, scope class, unit)
        let found: Option<(AttributeProto, Option<ClassId>, UnitId)> = match kind.as_slice() {
            b"class" => {
                let cid = self.resolve_class_by_name(&rhp_values::conv::to_string_bytes(&k[1]))?;
                self.classes[cid as usize]
                    .proto
                    .meta
                    .attributes
                    .get(idx_of(&k[2]))
                    .cloned()
                    .map(|a| (a, Some(cid), self.classes[cid as usize].unit))
            }
            b"prop" | b"const" => {
                let cid = self.resolve_class_by_name(&rhp_values::conv::to_string_bytes(&k[1]))?;
                let name = rhp_values::conv::to_string_bytes(&k[2]);
                // A trait member's thunks run in the trait's unit.
                let meta = if kind == b"prop" {
                    self.find_prop_meta(cid, &name).map(|m| (m.0, m.3))
                } else {
                    self.find_const_meta(cid, &name).map(|m| (m.0, m.2))
                };
                meta.and_then(|(m, owner)| {
                    let a = m.attributes.get(idx_of(&k[3])).cloned()?;
                    Some((a, Some(cid), self.classes[owner as usize].unit))
                })
            }
            b"fn" | b"param" => {
                let sub = self.fn_subject(&k[1])?;
                let scope = match &sub {
                    Some(FnSubject::Method { m, .. }) => Some(m.defining_class),
                    Some(FnSubject::Closure { cd, .. }) => cd.bound_class,
                    _ => None,
                };
                let unit = sub.as_ref().map_or(0, |s| self.subject_unit(s));
                let proto = sub.as_ref().and_then(|s| self.subject_proto(s));
                proto
                    .and_then(|p| {
                        if kind == b"fn" {
                            p.meta.attributes.get(idx_of(&k[2])).cloned()
                        } else {
                            p.params
                                .get(idx_of(&k[2]))
                                .and_then(|pi| pi.attributes.get(idx_of(&k[3])).cloned())
                        }
                    })
                    .map(|a| (a, scope, unit))
            }
            _ => None,
        };
        let Some((at, scope, unit)) = found else {
            return Ok(Value::Array(PhpArray::new()));
        };
        match at.args {
            None => Ok(Value::Array(PhpArray::new())),
            Some(ConstInitProto::Const(v)) => Ok(v),
            Some(ConstInitProto::Thunk(proto)) => {
                let f = UnitFn { proto, unit };
                self.run_proto_sync(f, None, scope, scope, Vec::new(), None)
            }
        }
    }

    /// Resolve a function locator: a function name, a Closure, or a
    /// `[class|object, method]` pair.
    fn fn_subject(&mut self, loc: &Value) -> Result<Option<FnSubject>, Thrown> {
        Ok(match loc {
            Value::Str(n) => {
                let name = n.as_bytes();
                let name = name.strip_prefix(b"\\").unwrap_or(name);
                if let Some(&i) = self.fn_names.get(&lc(name)) {
                    Some(FnSubject::Func(self.functions[i].clone()))
                } else if crate::calls::user_visible_builtin(name) {
                    rhp_builtins::registry()
                        .get_key_value(String::from_utf8_lossy(&lc(name)).as_ref())
                        .map(|(k, _)| FnSubject::Builtin(k))
                } else {
                    None
                }
            }
            Value::Closure(cd) => match &cd.wrapped {
                Some(w) => {
                    let w = (**w).clone();
                    self.fn_subject(&w)?
                }
                None => self
                    .closures
                    .get(cd.fnidx as usize)
                    .map(|f| FnSubject::Closure {
                        cd: cd.clone(),
                        proto: f.proto.clone(),
                    }),
            },
            Value::Array(a) => {
                let v: Vec<Value> = a.iter().map(|(_, v)| deref(v)).collect();
                if v.len() != 2 {
                    return Ok(None);
                }
                let cid = match &v[0] {
                    Value::Object(h) => h.borrow().class,
                    other => {
                        let n = rhp_values::conv::to_string_bytes(other);
                        match self.resolve_class_by_name(&n) {
                            Ok(c) => c,
                            Err(_) => return Ok(None),
                        }
                    }
                };
                let mname = rhp_values::conv::to_string_bytes(&v[1]);
                match self.lookup_method(cid, &mname) {
                    // An engine-internal helper of a prelude class is not a
                    // user-visible method.
                    Some(m)
                        if self.is_internal_class(m.defining_class)
                            && crate::calls::is_engine_helper_name(m.proto.name.as_bytes()) =>
                    {
                        None
                    }
                    Some(m) => Some(FnSubject::Method { class: cid, m }),
                    None => self
                        .enum_builtin_names(cid)
                        .into_iter()
                        .find(|n| n.as_bytes().eq_ignore_ascii_case(&mname))
                        .map(|name| FnSubject::EnumBuiltin { class: cid, name }),
                }
            }
            _ => None,
        })
    }

    fn subject_proto(&self, sub: &FnSubject) -> Option<Rc<FunctionProto>> {
        match sub {
            FnSubject::Func(f) => Some(f.proto.clone()),
            FnSubject::Method { m, .. } => Some(m.proto.clone()),
            FnSubject::Closure { proto, .. } => Some(proto.clone()),
            FnSubject::Builtin(_) | FnSubject::EnumBuiltin { .. } => None,
        }
    }

    /// The unit a user function's nested thunks (parameter defaults,
    /// attribute arguments) run in.
    fn subject_unit(&self, sub: &FnSubject) -> UnitId {
        match sub {
            FnSubject::Func(f) => f.unit,
            FnSubject::Method { m, .. } => m.unit,
            FnSubject::Closure { cd, .. } => self.closures[cd.fnidx as usize].unit,
            FnSubject::Builtin(_) | FnSubject::EnumBuiltin { .. } => 0,
        }
    }

    /// The stub row of an internal function (native or prelude) or of a
    /// built-in class's method.
    fn subject_stub_sig(&self, sub: &FnSubject) -> Option<StubSig> {
        match sub {
            FnSubject::Builtin(name) => sigs::function(name.as_bytes()).map(StubSig::of_fn),
            FnSubject::Func(f) if f.proto.is_internal() => {
                sigs::function(f.proto.name.as_bytes()).map(StubSig::of_fn)
            }
            FnSubject::Method { m, .. } if self.is_internal_class(m.defining_class) => {
                sigs::method(
                    self.classes[m.defining_class as usize].name.as_bytes(),
                    m.proto.name.as_bytes(),
                )
                .map(|m| StubSig {
                    ret: m.ret,
                    tentative: m.tentative(),
                    deprecated: m.deprecated(),
                    params: m.params,
                })
            }
            _ => None,
        }
    }

    fn subject_stub(&self, sub: &FnSubject) -> Option<&'static [sigs::Param]> {
        self.subject_stub_sig(sub).map(|s| s.params)
    }

    /// A stub default's value (zend_get_default_from_internal_arg_info).
    fn stub_default(&mut self, text: &str) -> Result<Value, Thrown> {
        crate::stub_default::eval(text, &mut |n| match n {
            crate::stub_default::Name::Const(c) => {
                let c = c.trim_start_matches('\\');
                self.constants
                    .get(c.as_bytes())
                    .cloned()
                    .ok_or_else(|| Thrown::error("Error", format!("Undefined constant \"{c}\"")))
            }
            crate::stub_default::Name::ClassConst(c, k) => {
                let c = c.trim_start_matches('\\');
                let cid = self.resolve_class_by_name(c.as_bytes())?;
                self.class_const_value(cid, k.as_bytes(), c.as_bytes(), false)?
                    .ok_or_else(|| Thrown::error("Error", format!("Undefined constant {c}::{k}")))
            }
        })
    }

    fn param_default(&mut self, sub: &FnSubject, idx: usize) -> Result<Value, Thrown> {
        if let Some(ps) = self.subject_stub(sub) {
            return match ps.get(idx).and_then(sigs::Param::default) {
                Some(text) => self.stub_default(text),
                None => Ok(Value::Null),
            };
        }
        let Some(proto) = self.subject_proto(sub) else {
            return Ok(Value::Null);
        };
        let Some(p) = proto.params.get(idx) else {
            return Ok(Value::Null);
        };
        match &p.default {
            Some(rhp_bytecode::ParamDefault::Const(k)) => Ok(proto.consts[*k as usize].clone()),
            _ => match &p.default_thunk {
                Some(t) => {
                    let scope = match sub {
                        FnSubject::Method { m, .. } => Some(m.defining_class),
                        FnSubject::Closure { cd, .. } => cd.bound_class,
                        _ => None,
                    };
                    let f = UnitFn {
                        proto: t.clone(),
                        unit: self.subject_unit(sub),
                    };
                    self.run_proto_sync(f, None, scope, scope, Vec::new(), None)
                }
                None => Ok(Value::Null),
            },
        }
    }

    fn fn_info(&self, sub: &FnSubject) -> Value {
        let mut a = PhpArray::new();
        let (proto, internal) = match sub {
            FnSubject::Builtin(name) => return self.builtin_fn_info(name),
            FnSubject::EnumBuiltin { class, name } => return self.enum_builtin_info(*class, name),
            FnSubject::Func(f) => (f.proto.clone(), f.proto.is_internal()),
            FnSubject::Method { m, .. } => {
                (m.proto.clone(), self.is_internal_class(m.defining_class))
            }
            FnSubject::Closure { proto, .. } => (proto.clone(), false),
        };
        let name: Vec<u8> = match sub {
            FnSubject::Closure { .. } => {
                // php 8.4+: `{closure:scope:line}` (the proto carries it).
                proto.name.as_bytes().to_vec()
            }
            _ => proto.name.as_bytes().to_vec(),
        };
        let (short, ns) = match sub {
            FnSubject::Func(_) => match name.iter().rposition(|&b| b == b'\\') {
                Some(i) => (name[i + 1..].to_vec(), name[..i].to_vec()),
                None => (name.clone(), Vec::new()),
            },
            _ => (name.clone(), Vec::new()),
        };
        // `self` / `parent` in a method's types resolve at compile time when
        // the scope is known (not a trait method, closure or anonymous class).
        let scope_ctx = match sub {
            FnSubject::Method { m, .. } => {
                let dc = m.defining_class;
                let own = self.classes[dc as usize]
                    .proto
                    .methods
                    .iter()
                    .any(|mp| Rc::ptr_eq(&mp.proto, &m.proto));
                if own {
                    self.known_scope(dc)
                } else {
                    None
                }
            }
            _ => None,
        };
        put(&mut a, "name", s(&name));
        put(&mut a, "short", s(&short));
        put(&mut a, "ns", s(&ns));
        put(&mut a, "internal", Value::Bool(internal));
        let ext = match sub {
            _ if !internal => None,
            FnSubject::Method { m, .. } => self.class_extension(m.defining_class),
            _ => Some(sigs::function(&name).map_or("standard", |f| f.ext)),
        };
        put(
            &mut a,
            "ext",
            ext.map_or(Value::Bool(false), |x| s(x.as_bytes())),
        );
        if internal {
            put(&mut a, "file", Value::Bool(false));
            put(&mut a, "start", Value::Bool(false));
            put(&mut a, "end", Value::Bool(false));
            put(&mut a, "doc", Value::Bool(false));
        } else {
            put(&mut a, "file", Value::Str(proto.debug.file.clone()));
            put(&mut a, "start", Value::Int(proto.meta.start_line as i64));
            put(&mut a, "end", Value::Int(proto.meta.end_line as i64));
            put(&mut a, "doc", opt_str(&proto.meta.doc_comment));
        }
        put(&mut a, "returnsRef", Value::Bool(proto.returns_ref));
        put(&mut a, "generator", Value::Bool(proto.is_generator));
        put(
            &mut a,
            "variadic",
            Value::Bool(proto.params.iter().any(|p| p.variadic)),
        );
        let ret = match &proto.return_type {
            Some(t) => self.type_desc(t, scope_ctx),
            None => Value::Null,
        };
        // A prelude function's or built-in class method's signature is
        // php's, from its stub.
        let stub = self.subject_stub_sig(sub);
        if let Some(st) = &stub {
            let ret = match st.ret {
                "" => Value::Null,
                r => self.stub_type_desc(r, false),
            };
            let (r, t) = if st.tentative {
                (Value::Null, ret)
            } else {
                (ret, Value::Null)
            };
            put(&mut a, "ret", r);
            put(&mut a, "tentative", t);
            put(&mut a, "deprecated", Value::Bool(st.deprecated));
            put(
                &mut a,
                "variadic",
                Value::Bool(st.params.last().is_some_and(sigs::Param::variadic)),
            );
        }
        // Without a stub row, an internal interface method's return type is
        // tentative, except in the interfaces php 8.0 introduced with real
        // types.
        let tentative = internal
            && matches!(sub, FnSubject::Method { m, .. } if m.is_abstract
            && !matches!(
                self.classes[m.defining_class as usize].name.as_bytes(),
                b"Stringable" | b"Throwable" | b"Reflector"
            ));
        if stub.is_some() {
            // (set above)
        } else if tentative {
            put(&mut a, "ret", Value::Null);
            put(&mut a, "tentative", ret);
        } else {
            put(&mut a, "ret", ret);
            put(&mut a, "tentative", Value::Null);
        }
        let mut is_static = proto.is_static;
        match sub {
            FnSubject::Method { class, m } => {
                is_static = m.is_static;
                put(
                    &mut a,
                    "class",
                    s(self.classes[m.defining_class as usize].name.as_bytes()),
                );
                put(
                    &mut a,
                    "reflClass",
                    s(self.classes[*class as usize].name.as_bytes()),
                );
                put(&mut a, "vis", Value::Int(vis_flag(m.visibility)));
                let mut mods = vis_flag(m.visibility);
                if m.is_static {
                    mods |= 16;
                }
                if m.is_final {
                    mods |= 32;
                }
                if m.is_abstract {
                    mods |= 64;
                }
                put(&mut a, "modifiers", Value::Int(mods));
                put(&mut a, "abstract", Value::Bool(m.is_abstract));
                put(&mut a, "final", Value::Bool(m.is_final));
                // The prototype: the same-named method of an ancestor
                // (parent chain first, then interfaces).
                let lname = lc(proto.name.as_bytes());
                let e = &self.classes[m.defining_class as usize];
                let mut proto_cls: Option<ClassId> = None;
                let mut cur = e.parent;
                while let Some(pc) = cur {
                    if let Some(pm) = self.classes[pc as usize].methods.get(&lname) {
                        if pm.visibility != Visibility::Private {
                            proto_cls = Some(pm.defining_class);
                            // Keep walking to the root declaration.
                            cur = self.classes[pm.defining_class as usize].parent;
                            continue;
                        }
                    }
                    break;
                }
                if proto_cls.is_none() {
                    for &i in &e.interfaces {
                        if i != m.defining_class
                            && self.classes[i as usize].methods.contains_key(&lname)
                        {
                            proto_cls =
                                Some(self.classes[i as usize].methods[&lname].defining_class);
                            break;
                        }
                    }
                }
                put(
                    &mut a,
                    "prototype",
                    match proto_cls {
                        Some(c) => s(self.classes[c as usize].name.as_bytes()),
                        None => Value::Null,
                    },
                );
            }
            FnSubject::Closure { cd, proto } => {
                put(&mut a, "closure", Value::Bool(true));
                put(
                    &mut a,
                    "this",
                    match &cd.bound_this {
                        Some(h) => Value::Object(h.clone()),
                        None => {
                            // `$this` captured like a variable.
                            match proto.this_cv {
                                Some(r) if (r as usize) < cd.captures.len() => {
                                    match deref(&cd.captures[r as usize]) {
                                        o @ Value::Object(_) => o,
                                        _ => Value::Null,
                                    }
                                }
                                _ => Value::Null,
                            }
                        }
                    },
                );
                put(
                    &mut a,
                    "scope",
                    match cd.bound_class {
                        Some(c) => s(self.classes[c as usize].name.as_bytes()),
                        None => Value::Null,
                    },
                );
                put(
                    &mut a,
                    "calledScope",
                    match cd.bound_static.or(cd.bound_class) {
                        Some(c) => s(self.classes[c as usize].name.as_bytes()),
                        None => Value::Null,
                    },
                );
                let mut used = PhpArray::new();
                for i in 0..proto.num_captures as usize {
                    let n = proto
                        .cv_var_names
                        .get(i)
                        .map(|n| n.as_bytes().to_vec())
                        .unwrap_or_default();
                    if n == b"this" {
                        continue;
                    }
                    let v = cd.captures.get(i).map(deref).unwrap_or(Value::Null);
                    used.insert(ArrayKey::from_bytes(&n), v);
                }
                put(&mut a, "used", Value::Array(used));
            }
            _ => {}
        }
        put(&mut a, "static", Value::Bool(is_static));
        let loc = self.subject_locator(sub);
        let target = match sub {
            FnSubject::Method { .. } => T_METHOD,
            _ => T_FUNCTION,
        };
        let loc2 = loc.clone();
        put(
            &mut a,
            "attrs",
            self.attrs_desc(&proto.meta.attributes, target, move |i| {
                let mut k = PhpArray::new();
                k.push(s(b"fn"));
                k.push(loc2.clone());
                k.push(Value::Int(i as i64));
                Value::Array(k)
            }),
        );
        if let Some(st) = &stub {
            let (params, nreq) = self.stub_params_desc(st.params);
            put(&mut a, "params", Value::Array(params));
            put(&mut a, "nreq", Value::Int(nreq as i64));
            put(&mut a, "loc", loc);
            return Value::Array(a);
        }
        let mut params = PhpArray::new();
        let nreq = proto
            .params
            .iter()
            .rposition(|p| p.default.is_none() && !p.variadic)
            .map(|i| i + 1)
            .unwrap_or(0);
        for (i, p) in proto.params.iter().enumerate() {
            let mut d = PhpArray::new();
            put(
                &mut d,
                "name",
                Value::Str(proto.names[p.name as usize].clone()),
            );
            put(&mut d, "pos", Value::Int(i as i64));
            // php: a parameter with a default before a required one is not
            // optional.
            put(&mut d, "optional", Value::Bool(i >= nreq));
            put(&mut d, "variadic", Value::Bool(p.variadic));
            put(&mut d, "byref", Value::Bool(p.by_ref));
            put(&mut d, "promoted", Value::Bool(p.promoted));
            let has_default = p.default.is_some();
            put(&mut d, "hasDefault", Value::Bool(has_default));
            put(
                &mut d,
                "defaultConst",
                match &p.default_const {
                    Some(n) => Value::Str(n.clone()),
                    None => Value::Null,
                },
            );
            put(
                &mut d,
                "type",
                match &p.type_hint {
                    Some(t) => {
                        // php: a `= null` default makes the type implicitly
                        // nullable.
                        let implicit_null = matches!(
                            &p.default,
                            Some(rhp_bytecode::ParamDefault::Const(k))
                                if matches!(proto.consts[*k as usize], Value::Null)
                        );
                        if implicit_null && !t.nullable {
                            let mut t2 = t.clone();
                            t2.nullable = true;
                            if t2.intersection {
                                let g: Vec<PhpStr> = t2
                                    .kinds
                                    .iter()
                                    .filter_map(|k| match k {
                                        TypeKind::Named(n) => Some(n.clone()),
                                        _ => None,
                                    })
                                    .collect();
                                t2.intersection = false;
                                t2.kinds.clear();
                                t2.groups.push(g);
                            }
                            self.type_desc(&t2, scope_ctx)
                        } else {
                            self.type_desc(t, scope_ctx)
                        }
                    }
                    None => Value::Null,
                },
            );
            let loc3 = loc.clone();
            put(
                &mut d,
                "attrs",
                self.attrs_desc(&p.attributes, T_PARAMETER, move |j| {
                    let mut k = PhpArray::new();
                    k.push(s(b"param"));
                    k.push(loc3.clone());
                    k.push(Value::Int(i as i64));
                    k.push(Value::Int(j as i64));
                    Value::Array(k)
                }),
            );
            params.push(Value::Array(d));
        }
        put(&mut a, "params", Value::Array(params));
        put(&mut a, "nreq", Value::Int(nreq as i64));
        put(&mut a, "loc", loc);
        Value::Array(a)
    }

    /// A value that `fn_subject` resolves back to `sub`.
    fn subject_locator(&self, sub: &FnSubject) -> Value {
        match sub {
            FnSubject::Func(f) => Value::Str(f.proto.name.clone()),
            FnSubject::Builtin(n) => s(n.as_bytes()),
            FnSubject::EnumBuiltin { class, name } => {
                let mut a = PhpArray::new();
                a.push(Value::Str(self.classes[*class as usize].name.clone()));
                a.push(s(name.as_bytes()));
                Value::Array(a)
            }
            FnSubject::Closure { cd, .. } => Value::Closure(cd.clone()),
            FnSubject::Method { m, .. } => {
                let mut a = PhpArray::new();
                a.push(s(self.classes[m.defining_class as usize].name.as_bytes()));
                a.push(Value::Str(m.proto.name.clone()));
                Value::Array(a)
            }
        }
    }

    /// `Enum::cases()` / `from()` / `tryFrom()`.
    fn enum_builtin_info(&self, cid: ClassId, name: &'static str) -> Value {
        let cname = self.classes[cid as usize].name.clone();
        let mut a = PhpArray::new();
        put(&mut a, "ext", s(b"Core"));
        put(&mut a, "name", s(name.as_bytes()));
        put(&mut a, "short", s(name.as_bytes()));
        put(&mut a, "ns", s(b""));
        put(&mut a, "internal", Value::Bool(true));
        put(&mut a, "file", Value::Bool(false));
        put(&mut a, "start", Value::Bool(false));
        put(&mut a, "end", Value::Bool(false));
        put(&mut a, "doc", Value::Bool(false));
        put(&mut a, "returnsRef", Value::Bool(false));
        put(&mut a, "generator", Value::Bool(false));
        put(&mut a, "variadic", Value::Bool(false));
        put(&mut a, "static", Value::Bool(true));
        put(&mut a, "class", Value::Str(cname.clone()));
        put(&mut a, "reflClass", Value::Str(cname.clone()));
        put(&mut a, "vis", Value::Int(1));
        put(&mut a, "modifiers", Value::Int(17));
        put(&mut a, "abstract", Value::Bool(false));
        put(&mut a, "final", Value::Bool(false));
        put(&mut a, "prototype", Value::Null);
        put(&mut a, "attrs", Value::Array(PhpArray::new()));
        put(&mut a, "tentative", Value::Null);
        let ret = match name {
            "cases" => named_desc(b"array", false, true),
            "from" => named_desc(b"static", false, false),
            _ => named_desc(b"static", true, false),
        };
        put(&mut a, "ret", ret);
        let mut params = PhpArray::new();
        if name != "cases" {
            let mut d = PhpArray::new();
            put(&mut d, "name", s(b"value"));
            put(&mut d, "pos", Value::Int(0));
            put(&mut d, "optional", Value::Bool(false));
            put(&mut d, "variadic", Value::Bool(false));
            put(&mut d, "byref", Value::Bool(false));
            put(&mut d, "promoted", Value::Bool(false));
            put(&mut d, "hasDefault", Value::Bool(false));
            put(&mut d, "defaultConst", Value::Null);
            put(&mut d, "type", self.stub_type_desc("string|int", false));
            put(&mut d, "attrs", Value::Array(PhpArray::new()));
            params.push(Value::Array(d));
        }
        let n = params.len();
        put(&mut a, "params", Value::Array(params));
        put(&mut a, "nreq", Value::Int(n as i64));
        let mut loc = PhpArray::new();
        loc.push(Value::Str(cname));
        loc.push(s(name.as_bytes()));
        put(&mut a, "loc", Value::Array(loc));
        Value::Array(a)
    }

    /// Parameter descriptors for a stub row, and the required count.
    fn stub_params_desc(&self, ps: &'static [sigs::Param]) -> (PhpArray, usize) {
        let nreq = ps
            .iter()
            .rposition(|p| p.default.is_empty() && p.flags & (sigs::V | sigs::U) == 0)
            .map_or(0, |i| i + 1);
        let mut params = PhpArray::new();
        for (i, p) in ps.iter().enumerate() {
            let mut d = PhpArray::new();
            put(&mut d, "name", s(p.name.as_bytes()));
            put(&mut d, "pos", Value::Int(i as i64));
            put(&mut d, "optional", Value::Bool(i >= nreq));
            put(&mut d, "variadic", Value::Bool(p.variadic()));
            put(&mut d, "byref", Value::Bool(p.by_ref()));
            put(&mut d, "preferRef", Value::Bool(p.prefer_ref()));
            put(&mut d, "promoted", Value::Bool(false));
            put(&mut d, "hasDefault", Value::Bool(p.default().is_some()));
            put(
                &mut d,
                "defaultConst",
                match p.default().and_then(crate::stub_default::constant_name) {
                    Some(c) => s(c.as_bytes()),
                    None => Value::Null,
                },
            );
            // Internal parameters print their arginfo default text.
            put(
                &mut d,
                "defaultText",
                match p.default() {
                    Some(t) => s(t.as_bytes()),
                    None => Value::Null,
                },
            );
            put(
                &mut d,
                "type",
                match p.ty {
                    "" => Value::Null,
                    t => self.stub_type_desc(t, false),
                },
            );
            put(&mut d, "attrs", Value::Array(PhpArray::new()));
            params.push(Value::Array(d));
        }
        (params, nreq)
    }

    /// A native builtin's descriptor, from its stub row when there is one.
    fn builtin_fn_info(&self, name: &'static str) -> Value {
        let b = rhp_builtins::registry().get(name);
        let sig = sigs::function(name.as_bytes());
        let mut a = PhpArray::new();
        put(
            &mut a,
            "ext",
            s(sig.map_or("standard", |f| f.ext).as_bytes()),
        );
        put(&mut a, "name", s(sig.map_or(name, |f| f.name).as_bytes()));
        if let Some(f) = sig {
            put(&mut a, "short", s(name.as_bytes()));
            put(&mut a, "ns", s(b""));
            put(&mut a, "internal", Value::Bool(true));
            put(&mut a, "file", Value::Bool(false));
            put(&mut a, "start", Value::Bool(false));
            put(&mut a, "end", Value::Bool(false));
            put(&mut a, "doc", Value::Bool(false));
            put(&mut a, "returnsRef", Value::Bool(false));
            put(&mut a, "generator", Value::Bool(false));
            put(&mut a, "static", Value::Bool(false));
            put(&mut a, "deprecated", Value::Bool(f.deprecated()));
            put(
                &mut a,
                "ret",
                match f.ret {
                    "" => Value::Null,
                    r => self.stub_type_desc(r, false),
                },
            );
            put(&mut a, "tentative", Value::Null);
            put(&mut a, "attrs", Value::Array(PhpArray::new()));
            put(&mut a, "variadic", Value::Bool(f.max_args().is_none()));
            let loc = s(name.as_bytes());
            let (params, nreq) = self.stub_params_desc(f.params);
            put(&mut a, "params", Value::Array(params));
            put(&mut a, "nreq", Value::Int(nreq as i64));
            put(&mut a, "loc", loc);
            return Value::Array(a);
        }
        put(&mut a, "short", s(name.as_bytes()));
        put(&mut a, "ns", s(b""));
        put(&mut a, "internal", Value::Bool(true));
        put(&mut a, "file", Value::Bool(false));
        put(&mut a, "start", Value::Bool(false));
        put(&mut a, "end", Value::Bool(false));
        put(&mut a, "doc", Value::Bool(false));
        put(&mut a, "returnsRef", Value::Bool(false));
        put(&mut a, "generator", Value::Bool(false));
        put(&mut a, "static", Value::Bool(false));
        put(&mut a, "ret", Value::Null);
        put(&mut a, "attrs", Value::Array(PhpArray::new()));
        // An rhp-only builtin: the registration's arity, unnamed parameters.
        let min = b.map(|b| b.min_args).unwrap_or(0);
        let max = b.and_then(|b| b.max_args);
        let n = max.unwrap_or(min + 1);
        let mut params = PhpArray::new();
        let mut variadic = false;
        for i in 0..n {
            let pv = max.is_none() && i + 1 == n;
            variadic |= pv;
            let mut d = PhpArray::new();
            put(&mut d, "name", s(format!("arg{i}").as_bytes()));
            put(&mut d, "pos", Value::Int(i as i64));
            put(&mut d, "optional", Value::Bool(i >= min || pv));
            put(&mut d, "variadic", Value::Bool(pv));
            put(&mut d, "byref", Value::Bool(false));
            put(&mut d, "promoted", Value::Bool(false));
            put(&mut d, "hasDefault", Value::Bool(i >= min && !pv));
            put(&mut d, "defaultConst", Value::Null);
            put(&mut d, "type", Value::Null);
            put(&mut d, "attrs", Value::Array(PhpArray::new()));
            params.push(Value::Array(d));
        }
        put(&mut a, "variadic", Value::Bool(variadic));
        put(&mut a, "params", Value::Array(params));
        put(&mut a, "nreq", Value::Int(min as i64));
        put(&mut a, "loc", s(name.as_bytes()));
        Value::Array(a)
    }

    /// A type descriptor from a stub type string (`int|string`, `?array`).
    fn stub_type_desc(&self, ty: &str, nullable: bool) -> Value {
        let t = ty.trim_start_matches('?');
        let parts: Vec<&str> = t.split('|').collect();
        let nullable = nullable || ty.starts_with('?') || parts.contains(&"null");
        let parts: Vec<&str> = parts.into_iter().filter(|p| *p != "null").collect();
        // php counts `static` as a class type for isBuiltin().
        let builtin = |p: &str| {
            matches!(
                p,
                "int"
                    | "float"
                    | "string"
                    | "bool"
                    | "false"
                    | "true"
                    | "array"
                    | "callable"
                    | "iterable"
                    | "object"
                    | "mixed"
                    | "void"
                    | "never"
            )
        };
        if parts.len() == 1 {
            let p = parts[0];
            return named_desc(p.as_bytes(), nullable || p == "mixed", builtin(p));
        }
        let mut types = PhpArray::new();
        for p in &parts {
            types.push(named_desc(p.as_bytes(), false, builtin(p)));
        }
        if nullable {
            types.push(named_desc(b"null", true, true));
        }
        let mut d = PhpArray::new();
        put(&mut d, "kind", s(b"union"));
        put(&mut d, "types", Value::Array(types));
        put(&mut d, "nullable", Value::Bool(nullable));
        Value::Array(d)
    }

    /// A Reflection type descriptor (`kind` = named / union / intersection)
    /// following php's `reflection_type_factory` rules.
    /// `cid` when php resolves `self`/`parent` types against it at compile
    /// time (a class or interface or enum, not a trait / anonymous class).
    fn known_scope(&self, cid: ClassId) -> Option<ClassId> {
        let e = &self.classes[cid as usize];
        let anon = e.name.as_bytes().windows(10).any(|w| w == b"@anonymous");
        (e.kind != ClassKind::Trait && !anon).then_some(cid)
    }

    pub(crate) fn type_desc(&self, ty: &TypeHint, ctx: Option<ClassId>) -> Value {
        use TypeKind as K;
        let has = |k: &K| ty.kinds.contains(k);
        if has(&K::Mixed) {
            return named_desc(b"mixed", true, true);
        }
        if ty.intersection && ty.groups.is_empty() {
            let mut types = PhpArray::new();
            for k in &ty.kinds {
                if let K::Named(n) = k {
                    types.push(named_desc(n.as_bytes(), false, false));
                }
            }
            let mut d = PhpArray::new();
            put(&mut d, "kind", s(b"intersection"));
            put(&mut d, "types", Value::Array(types));
            put(&mut d, "nullable", Value::Bool(false));
            return Value::Array(d);
        }
        // Members in php's getTypes() order: class-likes (declaration order),
        // then static, callable, object, array, string, int, float, bool /
        // false / true, void, never.
        let mut members: Vec<Value> = Vec::new();
        for g in &ty.groups {
            let mut types = PhpArray::new();
            for n in g {
                types.push(named_desc(n.as_bytes(), false, false));
            }
            let mut d = PhpArray::new();
            put(&mut d, "kind", s(b"intersection"));
            put(&mut d, "types", Value::Array(types));
            put(&mut d, "nullable", Value::Bool(false));
            members.push(Value::Array(d));
        }
        let iterable_alone = has(&K::Iterable) && ty.kinds.len() == 1 && ty.groups.is_empty();
        let mut names: Vec<(Vec<u8>, bool)> = Vec::new();
        for k in &ty.kinds {
            match k {
                K::Named(n) => names.push((n.as_bytes().to_vec(), false)),
                K::SelfType => names.push((
                    match ctx {
                        Some(c) => self.classes[c as usize].name.as_bytes().to_vec(),
                        None => b"self".to_vec(),
                    },
                    false,
                )),
                K::Parent => names.push((
                    match ctx.and_then(|c| self.classes[c as usize].parent) {
                        Some(p) => self.classes[p as usize].name.as_bytes().to_vec(),
                        None => b"parent".to_vec(),
                    },
                    false,
                )),
                K::Iterable if !iterable_alone => names.push((b"Traversable".to_vec(), false)),
                _ => {}
            }
        }
        let bool_ = has(&K::Bool) || (has(&K::True) && has(&K::False));
        for (on, n) in [
            (has(&K::StaticType), &b"static"[..]),
            (iterable_alone, b"iterable"),
            (has(&K::Callable), b"callable"),
            (has(&K::Object), b"object"),
            (
                (has(&K::Array) || has(&K::Iterable)) && !iterable_alone,
                b"array",
            ),
            (has(&K::String), b"string"),
            (has(&K::Int), b"int"),
            (has(&K::Float), b"float"),
            (bool_, b"bool"),
            (!bool_ && has(&K::False), b"false"),
            (!bool_ && has(&K::True), b"true"),
            (has(&K::Void), b"void"),
            (has(&K::Never), b"never"),
        ] {
            if on {
                // php treats `static` as a class type for isBuiltin().
                names.push((n.to_vec(), n != b"static"));
            }
        }
        let nullable = ty.nullable || has(&K::Null);
        if members.is_empty() && names.is_empty() {
            return named_desc(b"null", true, true);
        }
        if members.is_empty() && names.len() == 1 {
            let (n, builtin) = &names[0];
            return named_desc(n, nullable, *builtin);
        }
        for (n, builtin) in names {
            members.push(named_desc(&n, false, builtin));
        }
        if nullable {
            members.push(named_desc(b"null", true, true));
        }
        let mut types = PhpArray::new();
        for m in members {
            types.push(m);
        }
        let mut d = PhpArray::new();
        put(&mut d, "kind", s(b"union"));
        put(&mut d, "types", Value::Array(types));
        put(&mut d, "nullable", Value::Bool(nullable));
        Value::Array(d)
    }
}

/// What Reflection takes from a stub row.
struct StubSig {
    ret: &'static str,
    tentative: bool,
    deprecated: bool,
    params: &'static [sigs::Param],
}

impl StubSig {
    fn of_fn(f: &'static sigs::Func) -> StubSig {
        StubSig {
            ret: f.ret,
            tentative: false,
            deprecated: f.deprecated(),
            params: f.params,
        }
    }
}

fn named_desc(name: &[u8], nullable: bool, builtin: bool) -> Value {
    let mut d = PhpArray::new();
    put(&mut d, "kind", s(b"named"));
    put(&mut d, "name", s(name));
    put(&mut d, "nullable", Value::Bool(nullable));
    put(&mut d, "builtin", Value::Bool(builtin));
    Value::Array(d)
}

/// A class's own instance + static property declarations in declaration
/// order (by source line; instance first on a tie).
fn own_props_ordered(p: &rhp_bytecode::ClassProto) -> Vec<(PhpStr, bool)> {
    let mut v: Vec<(u32, usize, PhpStr, bool)> = Vec::new();
    for (i, pp) in p.props.iter().enumerate() {
        v.push((pp.meta.offset, i, pp.name.clone(), false));
    }
    let n = p.props.len();
    for (i, sp) in p.static_props.iter().enumerate() {
        v.push((sp.meta.offset, n + i, sp.name.clone(), true));
    }
    v.sort_by_key(|(l, i, _, _)| (*l, *i));
    v.into_iter().map(|(_, _, n, st)| (n, st)).collect()
}
