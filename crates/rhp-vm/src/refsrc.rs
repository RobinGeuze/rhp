//! Typed references: the typed properties a reference is bound to (php's
//! ZEND_REF_TYPE_SOURCES, Zend/zend_execute.c).
//!
//! A `Value::Ref` cell stays a plain `Rc<RefCell<Value>>`; the sources live
//! in a side table keyed by the cell's address. An entry holds a `Weak` to
//! the cell, so the address cannot be reused while the entry exists. A
//! source names the object (weakly) or the static property whose slot holds
//! the reference; a source whose object is gone or whose slot no longer
//! holds this cell is stale and skipped (php deletes it on unset, rebind and
//! object free). Untyped references cost one `is_empty` branch while no
//! typed reference exists, and one hash lookup after.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use rhp_values::ops::{deref, strict_eq};
use rhp_values::{ClassId, ObjectData, ObjectHandle, PhpStr, Value};

use crate::{Thrown, Vm};

/// One typed property slot holding a reference.
pub(crate) struct RefSource {
    /// The object whose property it is; `None`: a static property.
    obj: Option<Weak<RefCell<ObjectData>>>,
    /// The object's class (a static property: its declaring class).
    cls: ClassId,
    /// The property's storage key.
    key: PhpStr,
}

struct TypedRef {
    cell: Weak<RefCell<Value>>,
    sources: Vec<RefSource>,
}

/// The side table of typed references.
#[derive(Default)]
pub(crate) struct TypedRefs {
    map: rustc_hash::FxHashMap<usize, TypedRef>,
    /// Table size at which dead entries are dropped next.
    purge_at: usize,
}

impl TypedRefs {
    #[inline(always)]
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Whether `rc` may have type sources (one hash lookup).
    #[inline]
    pub(crate) fn has(&self, rc: &Rc<RefCell<Value>>) -> bool {
        !self.map.is_empty() && self.map.contains_key(&addr(rc))
    }

    fn purge(&mut self) {
        self.map.retain(|_, t| {
            t.cell.strong_count() > 0
                && t.sources.iter().any(|s| {
                    s.obj
                        .as_ref()
                        .is_none_or(|w: &Weak<RefCell<ObjectData>>| w.strong_count() > 0)
                })
        });
        self.purge_at = (self.map.len() * 2).max(64);
    }
}

fn addr(rc: &Rc<RefCell<Value>>) -> usize {
    Rc::as_ptr(rc) as usize
}

/// A source resolved against the class table.
#[derive(Clone)]
pub(crate) struct SrcInfo {
    pub(crate) ty: Rc<rhp_bytecode::TypeHint>,
    pub(crate) decl: ClassId,
    pub(crate) cls: ClassId,
    pub(crate) name: PhpStr,
}

/// What a value is to one source type (i_zend_verify_type_assignable_zval
/// plus the weak coercion).
enum Fit {
    Exact,
    Coerced(Value),
    No,
}

impl Vm {
    /// Add an object's typed property `key` as a source of `rc` (once per
    /// slot).
    pub(crate) fn add_prop_ref_source(
        &mut self,
        rc: &Rc<RefCell<Value>>,
        obj: &ObjectHandle,
        key: &PhpStr,
    ) {
        let Ok(cls) = obj.try_borrow().map(|o| o.class) else {
            return;
        };
        if !self
            .classes
            .get(cls as usize)
            .and_then(|c| c.props.get(key))
            .is_some_and(|pe| pe.ty.is_some())
        {
            return;
        }
        let src = RefSource {
            obj: Some(Rc::downgrade(obj)),
            cls,
            key: key.clone(),
        };
        self.add_ref_source(rc, src);
    }

    /// Add typed static property `name` of `cls` as a source of `rc`.
    pub(crate) fn add_static_ref_source(
        &mut self,
        rc: &Rc<RefCell<Value>>,
        cls: ClassId,
        name: &[u8],
    ) {
        let Some(sp) = self.class(cls).static_props.get(name) else {
            return;
        };
        if sp.ty.is_none() {
            return;
        }
        let decl = sp.declaring_class;
        let src = RefSource {
            obj: None,
            cls: decl,
            key: PhpStr::from_bytes(name),
        };
        self.add_ref_source(rc, src);
    }

    fn add_ref_source(&mut self, rc: &Rc<RefCell<Value>>, src: RefSource) {
        let t = &mut self.typed_refs;
        if t.map.len() >= t.purge_at {
            t.purge();
        }
        let e = t.map.entry(addr(rc)).or_insert_with(|| TypedRef {
            cell: Rc::downgrade(rc),
            sources: Vec::new(),
        });
        if e.sources.iter().any(|s| same_source(s, &src)) {
            return;
        }
        e.sources.push(src);
    }

    /// Remove object `obj`'s property `key` from the sources of `rc`
    /// (zend_ref_del_type_source: the last source moves into its place).
    pub(crate) fn del_prop_ref_source(
        &mut self,
        rc: &Rc<RefCell<Value>>,
        obj: &ObjectHandle,
        key: &PhpStr,
    ) {
        if self.typed_refs.is_empty() {
            return;
        }
        let p = Rc::as_ptr(obj);
        self.del_ref_source(rc, |s| {
            s.obj.as_ref().is_some_and(|w| w.as_ptr() == p) && s.key == *key
        });
    }

    /// Remove static property `name` (declared by `decl`) from the sources
    /// of `rc`.
    pub(crate) fn del_static_ref_source(
        &mut self,
        rc: &Rc<RefCell<Value>>,
        decl: ClassId,
        name: &[u8],
    ) {
        if self.typed_refs.is_empty() {
            return;
        }
        self.del_ref_source(rc, |s| {
            s.obj.is_none() && s.cls == decl && s.key.as_bytes() == name
        });
    }

    fn del_ref_source(&mut self, rc: &Rc<RefCell<Value>>, f: impl Fn(&RefSource) -> bool) {
        let a = addr(rc);
        let Some(e) = self.typed_refs.map.get_mut(&a) else {
            return;
        };
        if let Some(i) = e.sources.iter().position(f) {
            e.sources.swap_remove(i);
        }
        if e.sources.is_empty() {
            self.typed_refs.map.remove(&a);
        }
    }

    /// The live type sources of `rc`, in php's order.
    pub(crate) fn ref_sources(&self, rc: &Rc<RefCell<Value>>) -> Vec<SrcInfo> {
        let Some(e) = self.typed_refs.map.get(&addr(rc)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for s in &e.sources {
            let info = match &s.obj {
                Some(w) => {
                    let Some(h) = w.upgrade() else { continue };
                    // A slot that no longer holds this reference.
                    if let Ok(od) = h.try_borrow() {
                        match od.properties.get(&s.key) {
                            Some(Value::Ref(r)) if Rc::ptr_eq(r, rc) => {}
                            _ => continue,
                        }
                    }
                    let Some(pe) = self
                        .classes
                        .get(s.cls as usize)
                        .and_then(|c| c.props.get(&s.key))
                    else {
                        continue;
                    };
                    let Some(ty) = pe.ty.clone() else { continue };
                    SrcInfo {
                        ty,
                        decl: pe.declaring_class,
                        cls: s.cls,
                        name: pe.name.clone(),
                    }
                }
                None => {
                    let Some(sp) = self.class(s.cls).static_props.get(s.key.as_bytes()) else {
                        continue;
                    };
                    if let Ok(v) = sp.cell.try_borrow() {
                        match &*v {
                            Value::Ref(r) if Rc::ptr_eq(r, rc) => {}
                            _ => continue,
                        }
                    }
                    let Some(ty) = sp.ty.clone() else { continue };
                    SrcInfo {
                        ty,
                        decl: sp.declaring_class,
                        cls: s.cls,
                        name: s.key.clone(),
                    }
                }
            };
            out.push(info);
        }
        out
    }

    /// The first live source of `rc` (ZEND_REF_FIRST_SOURCE).
    pub(crate) fn first_ref_source(&self, rc: &Rc<RefCell<Value>>) -> Option<SrcInfo> {
        if !self.typed_refs.has(rc) {
            return None;
        }
        self.ref_sources(rc).into_iter().next()
    }

    fn fit(&mut self, s: &SrcInfo, v: &Value, strict: bool) -> Result<Fit, Thrown> {
        use rhp_bytecode::TypeKind as K;
        let ctx = Some(s.decl);
        if matches!(self.check_one(v, &s.ty, true, ctx), Ok(None)) {
            return Ok(Fit::Exact);
        }
        if strict {
            if let Value::Int(n) = v {
                if s.ty.kinds.contains(&K::Float) {
                    return Ok(Fit::Coerced(Value::Float(*n as f64)));
                }
            }
            return Ok(Fit::No);
        }
        let c = self.coerce_stringable(v.clone(), &s.ty, false, ctx)?;
        Ok(match self.check_one(&c, &s.ty, false, ctx) {
            Ok(None) => Fit::Coerced(c),
            Ok(Some(c2)) => {
                self.coercion_diag(&c, &c2)?;
                Fit::Coerced(c2)
            }
            Err(()) => Fit::No,
        })
    }

    /// zend_verify_ref_assignable_zval: `v` checked against every source
    /// of `rc`; the coerced value, the same for every source.
    pub(crate) fn verify_ref_assignable(
        &mut self,
        rc: &Rc<RefCell<Value>>,
        v: Value,
        strict: bool,
    ) -> Result<Value, Thrown> {
        let srcs = self.ref_sources(rc);
        let mut first: Option<&SrcInfo> = None;
        let mut coerced: Option<Value> = None;
        for s in &srcs {
            match self.fit(s, &v, strict)? {
                Fit::No => return Err(self.ref_type_error(s, &v)),
                Fit::Coerced(c) => match (first, &coerced) {
                    (None, _) => {
                        first = Some(s);
                        coerced = Some(c);
                    }
                    (Some(f), None) => return Err(self.conflicting_coercion_error(f, s, &v)),
                    (Some(f), Some(prev)) => {
                        if !strict_eq(prev, &c) {
                            return Err(self.conflicting_coercion_error(f, s, &v));
                        }
                    }
                },
                Fit::Exact => match (first, &coerced) {
                    (None, _) => first = Some(s),
                    (Some(f), Some(_)) => return Err(self.conflicting_coercion_error(f, s, &v)),
                    _ => {}
                },
            }
        }
        Ok(coerced.unwrap_or(v))
    }

    /// Write `v` through reference `rc` (zend_assign_to_variable on a
    /// reference): checked against its type sources. The stored value and
    /// the old one.
    pub(crate) fn assign_to_ref(
        &mut self,
        rc: &Rc<RefCell<Value>>,
        v: Value,
        strict: bool,
    ) -> Result<(Value, Value), Thrown> {
        let v = if self.typed_refs.has(rc) {
            self.verify_ref_assignable(rc, deref(&v), strict)?
        } else {
            v
        };
        let old = std::mem::replace(&mut *rc.borrow_mut(), v.clone());
        Ok((v, old))
    }

    /// zend_verify_ref_array_assignable: auto-vivifying an array inside a
    /// typed reference.
    pub(crate) fn verify_ref_array_assignable(
        &self,
        rc: &Rc<RefCell<Value>>,
    ) -> Result<(), Thrown> {
        use rhp_bytecode::TypeKind as K;
        if !self.typed_refs.has(rc) {
            return Ok(());
        }
        for s in self.ref_sources(rc) {
            let ok =
                s.ty.kinds
                    .iter()
                    .any(|k| matches!(k, K::Array | K::Iterable | K::Mixed));
            if !ok {
                return Err(Thrown::type_error(format!(
                    "Cannot auto-initialize an array inside a reference held by property {}::${} of type {}",
                    self.cname(s.decl),
                    String::from_utf8_lossy(s.name.as_bytes()),
                    self.prop_type_string(&s.ty, s.decl, Some(s.cls))
                )));
            }
        }
        Ok(())
    }

    /// zend_incdec_typed_ref: `++`/`--` (`delta`) of the value in typed
    /// reference `rc`. The old value.
    pub(crate) fn incdec_typed_ref(
        &mut self,
        rc: &Rc<RefCell<Value>>,
        delta: i64,
        strict: bool,
    ) -> Result<(Value, Value), Thrown> {
        use rhp_bytecode::TypeKind as K;
        let old = deref(&rc.borrow());
        let new = self.inc_dec_vm(&old, delta)?;
        if let (Value::Int(_), Value::Float(_)) = (&old, &new) {
            let srcs = self.ref_sources(rc);
            if let Some(s) = srcs
                .iter()
                .find(|s| !s.ty.kinds.iter().any(|k| matches!(k, K::Float | K::Mixed)))
            {
                let (what, end) = if delta > 0 {
                    ("increment", "maximal")
                } else {
                    ("decrement", "minimal")
                };
                return Err(Thrown::type_error(format!(
                    "Cannot {what} a reference held by property {}::${} of type {} past its {end} value",
                    self.cname(s.decl),
                    String::from_utf8_lossy(s.name.as_bytes()),
                    self.prop_type_string(&s.ty, s.decl, Some(s.cls))
                )));
            }
            *rc.borrow_mut() = new.clone();
            return Ok((old, new));
        }
        let new = self.verify_ref_assignable(rc, new, strict)?;
        *rc.borrow_mut() = new.clone();
        Ok((old, new))
    }

    /// zend_verify_prop_assignable_by_ref: binding the reference `v` (or a
    /// value) to typed property `s`. A plain reference's value is coerced
    /// in place.
    pub(crate) fn verify_prop_assignable_by_ref(
        &mut self,
        s: &SrcInfo,
        v: &Value,
        strict: bool,
    ) -> Result<(), Thrown> {
        let rc = match v {
            Value::Ref(rc) => rc.clone(),
            _ => return Ok(()),
        };
        let val = deref(&rc.borrow());
        if let Some(first) = self.first_ref_source(&rc) {
            match self.fit(s, &val, strict)? {
                Fit::Exact => return Ok(()),
                Fit::Coerced(_) => {
                    return Err(Thrown::type_error(format!(
                        "Reference with value of type {} held by property {}::${} of type {} is not compatible with property {}::${} of type {}",
                        rhp_builtins::zval_type_name(self, &val),
                        self.cname(first.decl),
                        String::from_utf8_lossy(first.name.as_bytes()),
                        self.prop_type_string(&first.ty, first.decl, Some(first.cls)),
                        self.cname(s.decl),
                        String::from_utf8_lossy(s.name.as_bytes()),
                        self.prop_type_string(&s.ty, s.decl, Some(s.cls)),
                    )));
                }
                Fit::No => {}
            }
        } else {
            match self.verify_type_for_prop(&s.ty, s.decl, s.cls, s.name.as_bytes(), val, strict) {
                Ok(c) => {
                    *rc.borrow_mut() = c;
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        }
        Err(self.prop_type_error(s, &val))
    }

    /// The source info of object property `key` of class `cls`, if typed.
    pub(crate) fn prop_src_info(&self, cls: ClassId, key: &PhpStr) -> Option<SrcInfo> {
        let pe = self.classes.get(cls as usize)?.props.get(key)?;
        Some(SrcInfo {
            ty: pe.ty.clone()?,
            decl: pe.declaring_class,
            cls,
            name: pe.name.clone(),
        })
    }

    /// The source info of static property `name` of `cls`, if typed.
    pub(crate) fn static_src_info(&self, cls: ClassId, name: &[u8]) -> Option<SrcInfo> {
        let sp = self.classes.get(cls as usize)?.static_props.get(name)?;
        Some(SrcInfo {
            ty: sp.ty.clone()?,
            decl: sp.declaring_class,
            cls,
            name: PhpStr::from_bytes(name),
        })
    }

    fn cname(&self, c: ClassId) -> String {
        String::from_utf8_lossy(self.class(c).name.as_bytes()).into_owned()
    }

    fn prop_type_error(&self, s: &SrcInfo, v: &Value) -> Thrown {
        Thrown::type_error(format!(
            "Cannot assign {} to property {}::${} of type {}",
            rhp_builtins::zval_value_name(self, v),
            self.cname(s.decl),
            String::from_utf8_lossy(s.name.as_bytes()),
            self.prop_type_string(&s.ty, s.decl, Some(s.cls))
        ))
    }

    fn ref_type_error(&self, s: &SrcInfo, v: &Value) -> Thrown {
        Thrown::type_error(format!(
            "Cannot assign {} to reference held by property {}::${} of type {}",
            rhp_builtins::zval_value_name(self, v),
            self.cname(s.decl),
            String::from_utf8_lossy(s.name.as_bytes()),
            self.prop_type_string(&s.ty, s.decl, Some(s.cls))
        ))
    }

    fn conflicting_coercion_error(&self, a: &SrcInfo, b: &SrcInfo, v: &Value) -> Thrown {
        Thrown::type_error(format!(
            "Cannot assign {} to reference held by property {}::${} of type {} and property {}::${} of type {}, as this would result in an inconsistent type conversion",
            rhp_builtins::zval_value_name(self, v),
            self.cname(a.decl),
            String::from_utf8_lossy(a.name.as_bytes()),
            self.prop_type_string(&a.ty, a.decl, Some(a.cls)),
            self.cname(b.decl),
            String::from_utf8_lossy(b.name.as_bytes()),
            self.prop_type_string(&b.ty, b.decl, Some(b.cls)),
        ))
    }
}

fn same_source(a: &RefSource, b: &RefSource) -> bool {
    a.cls == b.cls
        && a.key == b.key
        && match (&a.obj, &b.obj) {
            (Some(x), Some(y)) => Weak::ptr_eq(x, y),
            (None, None) => true,
            _ => false,
        }
}
