//! ext/standard/var.c: `var_dump`, `var_export`, `serialize`/`unserialize`
//! and `memory_get_usage`, plus the byte-exact formatters behind them and
//! `print_r` (Zend/zend.c's `zend_print_zval_r`).
//!
//! Indentation and punctuation verified against php 8.5.8
//! (tests/golden/stdlib/var/{var_dump,print_r,var_export}_nested.out).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rhp_values::ops::deref;
use rhp_values::{conv, fmt, ArrayKey, ObjectHandle, PhpArray, PhpStr, Thrown, Value};

use crate::zend::object_handlers::PropPurpose;
use crate::{zend, Host, UnserProp, E_WARNING};

fn sp(n: usize, out: &mut Vec<u8>) {
    out.extend(std::iter::repeat_n(b' ', n));
}

/// Split a php-mangled property name: `"\0Class\0name"` -> (name, Some(Class)),
/// `"\0*\0name"` -> (name, Some("*")), plain `name` -> (name, None).
///
/// As zend_unmangle_property_name_ex, an anonymous owner's name keeps its
/// `\0file:line$n` tail out of the property name; the owner returned is cut
/// at the NUL (php prints it with `%s`).
pub fn unmangle_prop(key: &[u8]) -> (&[u8], Option<&[u8]>) {
    if key.first() == Some(&0) {
        if let Some(end) = key[1..].iter().position(|&b| b == 0) {
            let class = &key[1..end + 1];
            let mut prop_at = end + 2;
            // A second NUL: the class name had one (anonymous class).
            if let Some(more) = key[prop_at..].iter().position(|&b| b == 0) {
                prop_at += more + 1;
            }
            return (&key[prop_at..], Some(class));
        }
    }
    (key, None)
}

/// php_var_dump_object_prefix: "lazy ghost " / "lazy proxy " for a lazy
/// object.
pub fn lazy_prefix(h: &rhp_values::ObjectHandle) -> &'static str {
    let b = h.borrow();
    match b.lazy() {
        Some(l) if l.obj_flags & rhp_values::LazyInfo::PROXY != 0 => "lazy proxy ",
        Some(l) if l.obj_flags != 0 => "lazy ghost ",
        _ => "",
    }
}

/// An object's properties keyed by their php-mangled names (debug view).
/// Engine-internal slots of built-in classes are left out.
fn mangled_props(host: &dyn crate::Host, h: &rhp_values::ObjectHandle) -> Vec<(PhpStr, Value)> {
    let b = h.borrow();
    b.properties
        .iter()
        .filter(|(k, _)| !host.is_internal_slot(b.class, k))
        .map(|(k, v)| (host.mangled_prop_name(b.class, k), v.clone()))
        .collect()
}

thread_local! {
    /// The views of the objects of the value being formatted, by object id
    /// (resolved up front by [`resolve_views`], since the formatters only
    /// see a `&dyn Host`).
    static VIEWS: std::cell::RefCell<std::collections::HashMap<u32, rhp_values::PhpArray>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// php's `zend_get_properties_for(purpose)` for every object reachable
/// from `v` (through views): the class's `get_properties_for` handler, and
/// for [`PropPurpose::Debug`] then `__debugInfo()`. The formatters print
/// such a view instead of the properties; objects without one keep their
/// properties. The views stay installed until [`clear_views`].
pub fn resolve_views(
    host: &mut dyn crate::Host,
    v: &Value,
    purpose: PropPurpose,
) -> Result<(), Thrown> {
    let mut seen_arrays = Vec::new();
    let mut seen_objects = Vec::new();
    resolve_in(host, v, purpose, &mut seen_arrays, &mut seen_objects)
}

fn resolve_in(
    host: &mut dyn crate::Host,
    v: &Value,
    purpose: PropPurpose,
    seen_arrays: &mut Vec<usize>,
    seen_objects: &mut Vec<u32>,
) -> Result<(), Thrown> {
    // Past the stack limit the dump itself stops ("nesting level too deep").
    if crate::zend::call_stack::overflowed() {
        return Ok(());
    }
    match deref(v) {
        Value::Array(a) => {
            if seen_arrays.contains(&a.data_ptr()) {
                return Ok(());
            }
            seen_arrays.push(a.data_ptr());
            for (_, val) in a.iter() {
                resolve_in(host, val, purpose, seen_arrays, seen_objects)?;
            }
        }
        Value::Object(h) => {
            let (id, class) = {
                let b = h.borrow();
                (b.id, b.class)
            };
            if seen_objects.contains(&id) {
                return Ok(());
            }
            seen_objects.push(id);
            let view = match crate::zend::object_handlers::properties_for(host, &h, purpose)? {
                Some(table) => Some(table),
                None if purpose == PropPurpose::Debug
                    && host.class_has_method(class, b"__debuginfo") =>
                {
                    Some(debug_info(host, &h, class)?)
                }
                // zend_lazy_object_debug_info: an initialized proxy shows
                // its instance.
                None if purpose == PropPurpose::Debug => {
                    let inst = h.borrow().lazy_instance();
                    inst.map(|i| {
                        let mut a = rhp_values::PhpArray::new();
                        a.insert(ArrayKey::from_bytes(b"instance"), Value::Object(i));
                        a
                    })
                }
                None => None,
            };
            let children: Vec<Value> = match view {
                Some(view) => {
                    let vals = view.iter().map(|(_, v)| v.clone()).collect();
                    VIEWS.with(|m| m.borrow_mut().insert(id, view));
                    vals
                }
                None => h
                    .borrow()
                    .properties
                    .iter()
                    .map(|(_, v)| v.clone())
                    .collect(),
            };
            for c in &children {
                resolve_in(host, c, purpose, seen_arrays, seen_objects)?;
            }
        }
        Value::Closure(c) if purpose == PropPurpose::Debug => {
            if seen_objects.contains(&c.id) {
                return Ok(());
            }
            seen_objects.push(c.id);
            if let Some(view) = host.closure_debug_info(&Value::Closure(c.clone())) {
                let vals: Vec<Value> = view.iter().map(|(_, v)| v.clone()).collect();
                VIEWS.with(|m| m.borrow_mut().insert(c.id, view));
                for v in &vals {
                    resolve_in(host, v, purpose, seen_arrays, seen_objects)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// A Closure's debug view (zend_closure_get_debug_info), resolved up front
/// or built now.
fn closure_view(host: &dyn crate::Host, v: &Value) -> Vec<(ArrayKey, Value)> {
    let Value::Closure(c) = v else {
        return Vec::new();
    };
    view_of(c.id).unwrap_or_else(|| {
        host.closure_debug_info(v)
            .map(|a| a.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    })
}

/// zend_std_get_debug_info: the object's `__debugInfo()` result.
fn debug_info(
    host: &mut dyn crate::Host,
    h: &rhp_values::ObjectHandle,
    class: u32,
) -> Result<rhp_values::PhpArray, Thrown> {
    let mut cb = rhp_values::PhpArray::new();
    cb.push(Value::Object(h.clone()));
    cb.push(Value::str("__debugInfo"));
    match deref(&host.call_callable(&Value::Array(cb), &[])?) {
        Value::Array(a) => Ok(a),
        Value::Null => {
            let cname = host.class_name(class).unwrap_or_default();
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                format!(
                    "Returning null from {}::__debugInfo() is deprecated, return an empty array instead",
                    String::from_utf8_lossy(&cname)
                ).as_bytes(),
            )?;
            Ok(rhp_values::PhpArray::new())
        }
        // zend_std_get_debug_info: E_ERROR, no exception.
        _ => Err(host.fatal_error(b"__debuginfo() must return an array")),
    }
}

/// Drop the views installed by [`resolve_views`].
pub fn clear_views() {
    VIEWS.with(|m| m.borrow_mut().clear());
}

/// The resolved view of object `id`, as (key, value) entries.
fn view_of(id: u32) -> Option<Vec<(ArrayKey, Value)>> {
    VIEWS.with(|m| {
        m.borrow()
            .get(&id)
            .map(|v| v.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    })
}

/// php's `get_debug_info` view (`var_dump`, `print_r`): the resolved view
/// (a handler's or `__debugInfo()`'s), else the properties.
fn debug_props(host: &dyn crate::Host, h: &rhp_values::ObjectHandle) -> Vec<(ArrayKey, Value)> {
    let id = h.borrow().id;
    if let Some(v) = view_of(id) {
        return v;
    }
    mangled_props(host, h)
        .into_iter()
        .map(|(k, v)| (ArrayKey::Str(k), v))
        .collect()
}

/// A debug-view key as raw bytes (an integer key as its digits).
fn key_bytes(k: &ArrayKey) -> Vec<u8> {
    match k {
        ArrayKey::Int(n) => n.to_string().into_bytes(),
        ArrayKey::Str(s) => s.as_bytes().to_vec(),
    }
}

/// The state every running var_dump / debug_zval_dump shares: php keeps
/// its recursion guards on the values, so a dump that `__debugInfo()`
/// starts sees them too.
#[derive(Default)]
struct Dumping {
    /// Arrays (by storage identity) and objects (by id) being printed.
    arrays: Vec<usize>,
    objects: Vec<u32>,
    /// Copies the dumpers hold, by storage identity: left out of the
    /// refcounts they print (php reads the values in place).
    held: HashMap<usize, usize>,
}

thread_local! {
    static DUMPING: RefCell<Dumping> = RefCell::new(Dumping::default());
}

fn guard<T>(f: impl FnOnce(&mut Dumping) -> T) -> T {
    DUMPING.with(|g| f(&mut g.borrow_mut()))
}

/// The storage `v` shares with its copies (`None` for a scalar), and how
/// many values share it.
fn storage(v: &Value) -> Option<(usize, usize)> {
    Some(match v {
        Value::Str(s) => (s.as_bytes().as_ptr() as usize, s.ref_count()),
        Value::Array(a) => (a.data_ptr(), a.ref_count()),
        Value::Object(h) => (Rc::as_ptr(h) as usize, Rc::strong_count(h)),
        Value::Closure(c) => (Rc::as_ptr(c) as usize, Rc::strong_count(c)),
        Value::Resource(r) => (Rc::as_ptr(r) as usize, Rc::strong_count(r)),
        Value::Ref(r) => (Rc::as_ptr(r) as usize, Rc::strong_count(r)),
        _ => return None,
    })
}

/// A copy of `v` the dumper holds while printing (see [`Dumping::held`]);
/// [`Held::release`] gives it back.
struct Held(Vec<Value>);

impl Held {
    fn new() -> Held {
        Held(Vec::new())
    }

    fn hold(&mut self, v: Value) -> &Value {
        if let Some((p, _)) = storage(&v) {
            guard(|g| *g.held.entry(p).or_insert(0) += 1);
        }
        self.0.push(v);
        self.0.last().unwrap()
    }

    fn release(self) {
        for v in &self.0 {
            if let Some((p, _)) = storage(v) {
                guard(|g| {
                    if let Some(n) = g.held.get_mut(&p) {
                        *n -= 1;
                        if *n == 0 {
                            g.held.remove(&p);
                        }
                    }
                });
            }
        }
    }
}

/// `v`'s refcount as php has it: the values sharing its storage, less the
/// copies dumpers hold.
fn refcount(v: &Value) -> usize {
    match storage(v) {
        Some((p, n)) => n.saturating_sub(guard(|g| g.held.get(&p).copied().unwrap_or(0))),
        None => 0,
    }
}

/// php_var_dump / php_debug_zval_dump: the output so far. With `stream`,
/// it is flushed before user code runs (`__debugInfo()`, a class's
/// handler), so what that code prints lands where php prints it.
struct Dumper<'h> {
    host: &'h mut dyn Host,
    out: Vec<u8>,
    stream: bool,
    /// debug_zval_dump: refcount / interned / packed annotations.
    zval: bool,
}

impl Dumper<'_> {
    fn new(host: &mut dyn Host, stream: bool, zval: bool) -> Dumper<'_> {
        Dumper {
            host,
            out: Vec::new(),
            stream,
            zval,
        }
    }

    fn flush(&mut self) {
        if self.stream && !self.out.is_empty() {
            self.host.write_out(&self.out);
            self.out.clear();
        }
    }

    fn put(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }

    fn pad(&mut self, level: usize) {
        if level > 1 {
            sp(level - 1, &mut self.out);
        }
    }

    /// `v`'s refcount, less what [`Host::shadow_holders`] reports (var_dump
    /// only reads a reference's).
    fn rc(&self, v: &Value) -> usize {
        let shadow = if self.zval || matches!(v, Value::Ref(_)) {
            self.host.shadow_holders(v)
        } else {
            0
        };
        refcount(v).saturating_sub(shadow)
    }

    /// One value at php's `level` (1 at the top).
    fn value(&mut self, v: &Value, level: usize) -> Result<(), Thrown> {
        self.pad(level);
        let Value::Ref(r) = v else {
            return self.plain(v, "", level);
        };
        let rc = self.rc(v);
        let mut held = Held::new();
        let inner = held.hold(r.borrow().clone());
        let res = if self.zval {
            self.put(format!("reference refcount({rc}) {{\n").as_bytes());
            let res = self.value(inner, level + 2);
            if res.is_ok() {
                self.pad(level);
                self.put(b"}\n");
            }
            res
        } else {
            // php hides a reference nothing else holds.
            self.plain(inner, if rc > 1 { "&" } else { "" }, level)
        };
        held.release();
        res
    }

    /// A value that is not a reference; `amp` is var_dump's marker.
    fn plain(&mut self, v: &Value, amp: &str, level: usize) -> Result<(), Thrown> {
        match v {
            Value::Null | Value::Undef => self.put(format!("{amp}NULL\n").as_bytes()),
            Value::Bool(b) => self.put(format!("{amp}bool({b})\n").as_bytes()),
            Value::Int(n) => self.put(format!("{amp}int({n})\n").as_bytes()),
            Value::Float(f) => {
                self.put(format!("{amp}float(").as_bytes());
                self.put(&fmt::float_serialize(*f));
                self.put(b")\n");
            }
            Value::Str(s) => {
                self.put(format!("{amp}string({}) \"", s.len()).as_bytes());
                self.put(s.as_bytes());
                if !self.zval {
                    self.put(b"\"\n");
                } else if s.len() <= 1 || self.host.is_interned_str(s) {
                    self.put(b"\" interned\n");
                } else {
                    let rc = self.rc(v);
                    self.put(format!("\" refcount({rc})\n").as_bytes());
                }
            }
            Value::Array(a) => {
                let id = a.data_ptr();
                if guard(|g| g.arrays.contains(&id)) {
                    self.put(b"*RECURSION*\n");
                    return Ok(());
                }
                if self.zval {
                    let rc = self.rc(v);
                    if a.is_empty() {
                        self.put(b"array(0) interned {\n");
                    } else {
                        let packed = if is_packed(a) { "packed " } else { "" };
                        self.put(
                            format!("array({}) {packed}refcount({rc}){{\n", a.len()).as_bytes(),
                        );
                    }
                } else {
                    self.put(format!("{amp}array({}) {{\n", a.len()).as_bytes());
                }
                // Held for the dump (php's GC_ADDREF), so user code that
                // drops the original cannot free what is being printed.
                let mut held = Held::new();
                let Value::Array(a) = held.hold(v.clone()) else {
                    unreachable!()
                };
                guard(|g| g.arrays.push(id));
                let mut res = Ok(());
                for (k, val) in a.iter() {
                    sp(level + 1, &mut self.out);
                    if crate::zend::call_stack::overflowed() {
                        self.put(b"nesting level too deep");
                        continue;
                    }
                    dump_key(&mut self.out, k);
                    self.put(b"=>\n");
                    res = self.value(val, level + 2);
                    if res.is_err() {
                        break;
                    }
                }
                guard(|g| g.arrays.pop());
                held.release();
                res?;
                self.pad(level);
                self.put(b"}\n");
            }
            Value::Object(h) => self.object(h, v, amp, level)?,
            Value::Closure(c) => {
                if guard(|g| g.objects.contains(&c.id)) {
                    self.put(b"*RECURSION*\n");
                    return Ok(());
                }
                let refcount = self.refcount_suffix(v);
                let view = self.host.closure_debug_info(v).unwrap_or_default();
                self.put(
                    format!(
                        "{amp}object(Closure)#{} ({}){refcount}{{\n",
                        c.id,
                        view.len()
                    )
                    .as_bytes(),
                );
                guard(|g| g.objects.push(c.id));
                let entries: Vec<_> = view.iter().map(|(k, v)| (k.clone(), v)).collect();
                let res = self.entries(&entries, &[], level);
                guard(|g| g.objects.pop());
                res?;
                self.pad(level);
                self.put(b"}\n");
            }
            Value::Resource(r) => {
                let ty = if self.host.resource_closed(v) {
                    "Unknown"
                } else {
                    r.type_name
                };
                self.put(format!("{amp}resource({}) of type ({ty})", r.id).as_bytes());
                if self.zval {
                    let rc = self.rc(v);
                    self.put(format!(" refcount({rc})").as_bytes());
                }
                self.put(b"\n");
            }
            Value::Ref(_) => unreachable!("a reference holds no reference"),
        }
        Ok(())
    }

    /// The ` refcount(N)` debug_zval_dump puts after an object's count
    /// (var_dump: a plain space).
    fn refcount_suffix(&self, v: &Value) -> String {
        if self.zval {
            format!(" refcount({})", self.rc(v))
        } else {
            " ".to_string()
        }
    }

    /// An object: `enum(...)` for an enum case (var_dump), else the header
    /// and the debug view (zend_get_properties_for(DEBUG)).
    fn object(
        &mut self,
        h: &rhp_values::ObjectHandle,
        v: &Value,
        amp: &str,
        level: usize,
    ) -> Result<(), Thrown> {
        let (class, id) = {
            let b = h.borrow();
            (b.class, b.id)
        };
        // php prints the name as a C string: an anonymous class's
        // `X@anonymous\0file…` shows as `X@anonymous`.
        let cname = c_str(
            self.host
                .class_name(class)
                .unwrap_or_else(|| b"class".to_vec()),
        );
        if !self.zval && self.host.is_enum_class(class) {
            let case = h
                .borrow()
                .properties
                .get(&PhpStr::from("name"))
                .map(conv::to_string_bytes)
                .unwrap_or_default();
            self.put(format!("{amp}enum(").as_bytes());
            self.put(&cname);
            self.put(b"::");
            self.put(&case);
            self.put(b")\n");
            return Ok(());
        }
        if guard(|g| g.objects.contains(&id)) {
            self.put(b"*RECURSION*\n");
            return Ok(());
        }
        guard(|g| g.objects.push(id));
        let res = self.object_body(h, v, class, id, &cname, amp, level);
        guard(|g| g.objects.pop());
        res
    }

    #[allow(clippy::too_many_arguments)]
    fn object_body(
        &mut self,
        h: &rhp_values::ObjectHandle,
        v: &Value,
        class: u32,
        id: u32,
        cname: &[u8],
        amp: &str,
        level: usize,
    ) -> Result<(), Thrown> {
        let user_code = self.host.class_has_method(class, b"__debuginfo")
            || self
                .host
                .object_handlers(class)
                .is_some_and(|t| t.get_properties_for.is_some());
        if user_code {
            self.flush();
        }
        let view =
            match zend::object_handlers::properties_for(&mut *self.host, h, PropPurpose::Debug)? {
                Some(table) => Some(table),
                None if self.host.class_has_method(class, b"__debuginfo") => {
                    Some(debug_info(&mut *self.host, h, class)?)
                }
                // zend_lazy_object_debug_info: an initialized proxy shows
                // its instance.
                None => h.borrow().lazy_instance().map(|i| {
                    let mut a = PhpArray::new();
                    a.insert(ArrayKey::from_bytes(b"instance"), Value::Object(i));
                    a
                }),
            };
        // The header's refcount is read before the properties are copied.
        let refcount = self.refcount_suffix(v);
        // A view lists only its own entries; the properties also show the
        // uninitialized typed ones. The properties are copied (user code
        // may change them while they print), and the copies held.
        let mut held = Held::new();
        let (keys, uninit) = match &view {
            Some(_) => (Vec::new(), Vec::new()),
            None => {
                let b = h.borrow();
                let host = &*self.host;
                let mut keys = Vec::new();
                for (k, v) in b.properties.iter() {
                    if !host.is_internal_slot(b.class, k) {
                        keys.push(ArrayKey::Str(host.mangled_prop_name(b.class, k)));
                        held.hold(v.clone());
                    }
                }
                (keys, host.uninit_props(h))
            }
        };
        let entries: Vec<(ArrayKey, &Value)> = match &view {
            // The view is a table of its own (php's too): read in place.
            Some(t) => t.iter().map(|(k, v)| (k.clone(), v)).collect(),
            None => keys.into_iter().zip(held.0.iter()).collect(),
        };
        self.put(format!("{amp}{}object(", lazy_prefix(h)).as_bytes());
        self.put(cname);
        self.put(format!(")#{id} ({}){refcount}{{\n", entries.len()).as_bytes());
        let res = self.entries(&entries, &uninit, level);
        drop(entries);
        held.release();
        res?;
        self.pad(level);
        self.put(b"}\n");
        Ok(())
    }

    /// An object's entries (php_object_property_dump), the uninitialized
    /// typed properties at their declaration positions.
    fn entries(
        &mut self,
        entries: &[(ArrayKey, &Value)],
        uninit: &[(PhpStr, String, usize)],
        level: usize,
    ) -> Result<(), Thrown> {
        let mut next = 0usize;
        for i in 0..=entries.len() {
            while let Some((k, ty, pos)) = uninit.get(next) {
                if *pos > i && i < entries.len() {
                    break;
                }
                self.prop_key(&ArrayKey::Str(k.clone()), level);
                sp(level + 1, &mut self.out);
                self.put(format!("uninitialized({ty})\n").as_bytes());
                next += 1;
            }
            let Some((k, val)) = entries.get(i) else {
                break;
            };
            if crate::zend::call_stack::overflowed() {
                sp(level + 1, &mut self.out);
                self.put(b"nesting level too deep");
                continue;
            }
            self.prop_key(k, level);
            self.value(val, level + 2)?;
        }
        Ok(())
    }

    /// `["name"]=>`, `["name":protected]=>`, `["name":"Class":private]=>`.
    fn prop_key(&mut self, k: &ArrayKey, level: usize) {
        sp(level + 1, &mut self.out);
        let k = match k {
            ArrayKey::Int(n) => {
                self.put(format!("[{n}]=>\n").as_bytes());
                return;
            }
            ArrayKey::Str(s) => s,
        };
        self.put(b"[\"");
        let (name, owner) = unmangle_prop(k.as_bytes());
        self.put(name);
        self.put(b"\"");
        match owner {
            None => {}
            Some(b"*") => self.put(b":protected"),
            Some(class) => {
                self.put(b":\"");
                self.put(class);
                self.put(b"\":private");
            }
        }
        self.put(b"]=>\n");
    }
}

/// Whether php would hold `a` as a packed array (HT_IS_PACKED): integer
/// keys in ascending order that a packed table of php's sizes (8, then
/// doubling while at least half full) can hold.
fn is_packed(a: &PhpArray) -> bool {
    let mut size: i64 = 8;
    let mut used: i64 = 0;
    for (i, (k, _)) in a.iter().enumerate() {
        let ArrayKey::Int(k) = *k else {
            return false;
        };
        if k < used {
            return false;
        }
        while k >= size {
            if (i as i64) <= size / 2 || k >= size * 2 {
                return false;
            }
            size *= 2;
        }
        used = k + 1;
    }
    true
}

/// `var_dump` of one value, without streaming: the output is returned in
/// `out` (user code a view runs prints straight to the host).
#[cfg(test)]
pub fn var_dump(host: &mut dyn Host, out: &mut Vec<u8>, v: &Value) -> Result<(), Thrown> {
    let mut d = Dumper::new(host, false, false);
    let r = d.value(&deref(v), 1);
    out.extend_from_slice(&d.out);
    r
}

/// The arrays (by storage identity) and objects (by id) currently being
/// printed: php's recursion protection.
#[derive(Default)]
struct Seen {
    arrays: Vec<usize>,
    objects: Vec<u32>,
    /// Cycles `var_export` cut short (each one is a php warning).
    circular: usize,
}

fn dump_key(out: &mut Vec<u8>, k: &ArrayKey) {
    match k {
        ArrayKey::Int(n) => out.extend_from_slice(format!("[{n}]").as_bytes()),
        ArrayKey::Str(s) => {
            out.extend_from_slice(b"[\"");
            out.extend_from_slice(s.as_bytes());
            out.extend_from_slice(b"\"]");
        }
    }
}

/// `print_r` of one value. `indent` = column of the opening `(` for arrays.
pub fn print_r(host: &dyn crate::Host, out: &mut Vec<u8>, v: &Value, indent: usize) {
    print_r_value(host, out, v, indent, &mut Seen::default());
}

/// `print_r` of one value; `printing` holds the arrays / objects currently
/// being printed, so a cycle prints ` *RECURSION*` like php instead of
/// recursing forever.
fn print_r_value(
    host: &dyn crate::Host,
    out: &mut Vec<u8>,
    v: &Value,
    indent: usize,
    printing: &mut Seen,
) {
    let v = deref(v);
    match &v {
        Value::Array(a) => {
            out.extend_from_slice(b"Array\n");
            let id = a.data_ptr();
            if printing.arrays.contains(&id) {
                out.extend_from_slice(b" *RECURSION*");
                return;
            }
            let entries: Vec<(Vec<u8>, Value)> = a
                .iter()
                .map(|(k, val)| (print_r_key(k), val.clone()))
                .collect();
            printing.arrays.push(id);
            print_r_block(host, out, &entries, indent, printing);
            printing.arrays.pop();
        }
        Value::Object(h) => {
            // Snapshot under a short borrow so recursion can't double-borrow.
            let entries: Vec<(Vec<u8>, Value)> = debug_props(host, h)
                .into_iter()
                .map(|(k, v)| {
                    let k = key_bytes(&k);
                    let label = match unmangle_prop(&k) {
                        (name, None) => name.to_vec(),
                        (name, Some(b"*")) => [name, b":protected"].concat(),
                        (name, Some(class)) => [name, b":", class, b":private"].concat(),
                    };
                    (label, v)
                })
                .collect();
            let (class_id, obj_id) = {
                let b = h.borrow();
                (b.class, b.id)
            };
            out.extend_from_slice(&c_str(
                host.class_name(class_id)
                    .unwrap_or_else(|| b"class".to_vec()),
            ));
            if host.is_enum_class(class_id) {
                out.extend_from_slice(b" Enum");
                match entries
                    .iter()
                    .find(|(k, _)| k == b"value")
                    .map(|(_, v)| deref(v))
                {
                    Some(Value::Int(_)) => out.extend_from_slice(b":int"),
                    Some(Value::Str(_)) => out.extend_from_slice(b":string"),
                    _ => {}
                }
                out.push(b'\n');
            } else {
                out.extend_from_slice(b" Object\n");
            }
            if printing.objects.contains(&obj_id) {
                out.extend_from_slice(b" *RECURSION*");
                return;
            }
            printing.objects.push(obj_id);
            print_r_block(host, out, &entries, indent, printing);
            printing.objects.pop();
        }
        Value::Closure(c) => {
            out.extend_from_slice(b"Closure Object\n");
            if printing.objects.contains(&c.id) {
                out.extend_from_slice(b" *RECURSION*");
                return;
            }
            let entries: Vec<(Vec<u8>, Value)> = closure_view(host, &v)
                .into_iter()
                .map(|(k, v)| (key_bytes(&k), v))
                .collect();
            printing.objects.push(c.id);
            print_r_block(host, out, &entries, indent, printing);
            printing.objects.pop();
        }
        scalar => out.extend_from_slice(&conv::to_string_bytes(scalar)),
    }
}

/// The `(\n    [key] => value\n ... )\n` body shared by arrays and objects.
fn print_r_block(
    host: &dyn crate::Host,
    out: &mut Vec<u8>,
    entries: &[(Vec<u8>, Value)],
    indent: usize,
    printing: &mut Seen,
) {
    sp(indent, out);
    out.extend_from_slice(b"(\n");
    for (k, val) in entries {
        sp(indent + 4, out);
        out.push(b'[');
        out.extend_from_slice(k);
        out.extend_from_slice(b"] => ");
        if matches!(
            deref(val),
            Value::Array(_) | Value::Object(_) | Value::Closure(_)
        ) {
            print_r_value(host, out, val, indent + 8, printing);
        } else {
            out.extend_from_slice(&conv::to_string_bytes(val));
        }
        out.push(b'\n');
    }
    sp(indent, out);
    out.extend_from_slice(b")\n");
}

fn print_r_key(k: &ArrayKey) -> Vec<u8> {
    match k {
        ArrayKey::Int(n) => n.to_string().into_bytes(),
        ArrayKey::Str(s) => s.as_bytes().to_vec(),
    }
}

/// `var_export` of one value — php's `php_var_export_ex`; `level` starts at 1.
/// Returns how many circular references were cut short (php warns "var_export
/// does not handle circular references" for each).
pub fn var_export(host: &dyn crate::Host, out: &mut Vec<u8>, v: &Value, level: usize) -> usize {
    let mut seen = Seen::default();
    var_export_value(host, out, v, level, &mut seen);
    seen.circular
}

/// One `var_export` value. A container already being exported (a cycle) is
/// `NULL` (php also warns "var_export does not handle circular references").
fn var_export_value(
    host: &dyn crate::Host,
    out: &mut Vec<u8>,
    v: &Value,
    level: usize,
    seen: &mut Seen,
) {
    let v = deref(v);
    let recursive = match &v {
        Value::Array(a) => seen.arrays.contains(&a.data_ptr()),
        Value::Object(h) => seen.objects.contains(&h.borrow().id),
        _ => false,
    };
    if recursive {
        seen.circular += 1;
        out.extend_from_slice(b"NULL");
        return;
    }
    match &v {
        Value::Null | Value::Undef => out.extend_from_slice(b"NULL"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        // i64::MIN as a literal would parse as a float.
        Value::Int(i64::MIN) => out.extend_from_slice(b"-9223372036854775807-1"),
        Value::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::Float(f) => out.extend_from_slice(&export_float(*f)),
        Value::Str(s) => export_string(out, s.as_bytes()),
        Value::Array(a) => {
            if level > 1 {
                out.push(b'\n');
                sp(level - 1, out);
            }
            out.extend_from_slice(b"array (\n");
            seen.arrays.push(a.data_ptr());
            for (k, val) in a.iter() {
                sp(level + 1, out);
                export_key(out, k);
                out.extend_from_slice(b" => ");
                var_export_value(host, out, val, level + 2, seen);
                out.extend_from_slice(b",\n");
            }
            seen.arrays.pop();
            if level > 1 {
                sp(level - 1, out);
            }
            out.push(b')');
        }
        Value::Object(h) => {
            let class_id = h.borrow().class;
            let cname = host
                .class_name(class_id)
                .unwrap_or_else(|| b"class".to_vec());
            let is_enum = host.is_enum_class(class_id);
            // get_properties_for(VAR_EXPORT), resolved by the caller (an
            // ArrayObject's storage keeps its integer keys).
            let props: Vec<(ArrayKey, Value)> = match view_of(h.borrow().id) {
                Some(v) => v,
                None => mangled_props(host, h)
                    .into_iter()
                    .map(|(k, v)| (ArrayKey::Str(k), v))
                    .collect(),
            };
            if level > 1 {
                out.push(b'\n');
                sp(level - 1, out);
            }
            let is_std = cname.eq_ignore_ascii_case(b"stdClass");
            if is_std {
                out.extend_from_slice(b"(object) array(\n");
            } else {
                out.push(b'\\');
                out.extend_from_slice(&cname);
                if is_enum {
                    let case = props
                        .iter()
                        .find(|(k, _)| matches!(k, ArrayKey::Str(s) if s.as_bytes() == b"name"))
                        .map(|(_, v)| conv::to_string_bytes(v))
                        .unwrap_or_default();
                    out.extend_from_slice(b"::");
                    out.extend_from_slice(&case);
                } else {
                    out.extend_from_slice(b"::__set_state(array(\n");
                }
            }
            if !is_enum {
                seen.objects.push(h.borrow().id);
                for (k, val) in &props {
                    sp(level + 2, out);
                    let k = match k {
                        ArrayKey::Int(n) => {
                            out.extend_from_slice(format!("{n} => ").as_bytes());
                            var_export_value(host, out, val, level + 2, seen);
                            out.extend_from_slice(b",\n");
                            continue;
                        }
                        ArrayKey::Str(s) => s,
                    };
                    let (name, _) = unmangle_prop(k.as_bytes());
                    // Property names are always string keys (even "0").
                    out.push(b'\'');
                    for &b in name {
                        if b == b'\\' || b == b'\'' {
                            out.push(b'\\');
                        }
                        out.push(b);
                    }
                    out.push(b'\'');
                    out.extend_from_slice(b" => ");
                    var_export_value(host, out, val, level + 2, seen);
                    out.extend_from_slice(b",\n");
                }
                seen.objects.pop();
                if level > 1 {
                    sp(level - 1, out);
                }
            }
            if is_std {
                out.push(b')');
            } else if !is_enum {
                out.extend_from_slice(b"))");
            }
        }
        Value::Closure(_) => {
            // A Closure exports as `\Closure::__set_state(array(\n))`.
            if level > 1 {
                out.push(b'\n');
                sp(level - 1, out);
            }
            out.extend_from_slice(b"\\Closure::__set_state(array(\n");
            if level > 1 {
                sp(level - 1, out);
            }
            out.extend_from_slice(b"))");
        }
        Value::Resource(_) => out.extend_from_slice(b"NULL"),
        Value::Ref(_) => unreachable!("deref'd above"),
    }
}

fn export_key(out: &mut Vec<u8>, k: &ArrayKey) {
    match k {
        ArrayKey::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        ArrayKey::Str(s) => export_string(out, s.as_bytes()),
    }
}

/// Single-quoted string literal with `\` and `'` escaped and NUL bytes spliced
/// as `' . "\0" . '` (var_export style).
fn export_string(out: &mut Vec<u8>, s: &[u8]) {
    out.push(b'\'');
    for &b in s {
        if b == 0 {
            out.extend_from_slice(b"' . \"\\0\" . '");
            continue;
        }
        if b == b'\\' || b == b'\'' {
            out.push(b'\\');
        }
        out.push(b);
    }
    out.push(b'\'');
}

/// var_export floats always look like floats (append `.0` if integer-valued).
fn export_float(f: f64) -> Vec<u8> {
    let s = fmt::float_serialize(f);
    if s.iter()
        .any(|&b| matches!(b, b'.' | b'e' | b'E' | b'N' | b'I'))
    {
        s
    } else {
        [s, b".0".to_vec()].concat()
    }
}

/// A name cut at its first NUL, as php's `%s` / `ZSTR_VAL` printing shows it.
pub fn c_str(mut name: Vec<u8>) -> Vec<u8> {
    if let Some(i) = name.iter().position(|&b| b == 0) {
        name.truncate(i);
    }
    name
}

const INCOMPLETE: &[u8] = b"__PHP_Incomplete_Class";

const INCOMPLETE_NAME: &[u8] = b"__PHP_Incomplete_Class_Name";

// ---------------------------------------------------------------------------
// serialize
// ---------------------------------------------------------------------------

/// What a `var_hash` slot was registered for.
#[derive(Hash, PartialEq, Eq, Clone, Copy)]
enum VarKey {
    Obj(u32),
    Ref(usize),
}

/// php's `php_serialize_data`: every serialized value takes the next slot
/// number; objects and references remember theirs for `r:` / `R:`.
#[derive(Default)]
struct SerData {
    n: i64,
    seen: HashMap<VarKey, i64>,
    /// Keeps every registered value alive, so its address is not reused.
    hold: Vec<Value>,
}

thread_local! {
    /// BG(serialize) / BG(unserialize): a serialize() or unserialize() called
    /// from Serializable::serialize() / ::unserialize() shares its caller's
    /// var_hash. `SERIALIZE_LOCK` (BG(serialize_lock)) is held around the
    /// magic calls that must get a fresh one (__sleep, __serialize, __wakeup,
    /// __unserialize, autoloading).
    static SER: RefCell<(u32, Option<Rc<RefCell<SerData>>>)> = const { RefCell::new((0, None)) };
    static UNSER: RefCell<(u32, Option<Rc<RefCell<UnserData>>>)> = const { RefCell::new((0, None)) };
    static SERIALIZE_LOCK: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Call a magic method of `obj` as php does (by its function handler:
/// visibility does not matter).
fn call_method(
    host: &mut dyn Host,
    obj: &Value,
    m: &str,
    args: Vec<Value>,
) -> Result<Value, Thrown> {
    let Value::Object(h) = obj else {
        return Ok(Value::Null);
    };
    let cid = h.borrow().class;
    let class = host.class_name(cid).unwrap_or_default();
    host.reflect_invoke(obj, &class, m.as_bytes(), args)
}

/// Forget the shared serialize state (a request that bailed out mid-call).
pub(crate) fn reset_state() {
    SER.with(|s| *s.borrow_mut() = (0, None));
    UNSER.with(|s| *s.borrow_mut() = (0, None));
    SERIALIZE_LOCK.with(|l| l.set(0));
}

/// Run `f` (user code) with BG(serialize_lock) held.
fn locked<T>(f: impl FnOnce() -> T) -> T {
    SERIALIZE_LOCK.with(|l| l.set(l.get() + 1));
    let r = f();
    SERIALIZE_LOCK.with(|l| l.set(l.get().saturating_sub(1)));
    r
}

fn lock_held() -> bool {
    SERIALIZE_LOCK.with(|l| l.get() > 0)
}

/// php_var_serialize_init / _destroy.
fn ser_init() -> Rc<RefCell<SerData>> {
    if lock_held() {
        return Rc::default();
    }
    SER.with(|s| {
        let mut s = s.borrow_mut();
        if s.0 == 0 || s.1.is_none() {
            let d: Rc<RefCell<SerData>> = Rc::default();
            *s = (1, Some(d.clone()));
            d
        } else {
            s.0 += 1;
            s.1.clone().unwrap_or_default()
        }
    })
}

fn ser_destroy() {
    if lock_held() {
        return;
    }
    SER.with(|s| {
        let mut s = s.borrow_mut();
        s.0 = s.0.saturating_sub(1);
        if s.0 == 0 {
            s.1 = None;
        }
    })
}

struct Serializer {
    d: Rc<RefCell<SerData>>,
}

impl Serializer {
    /// `php_add_var_hash`: 0 for a first sighting, else the earlier slot.
    fn add_var_hash(&mut self, v: &Value) -> i64 {
        let mut d = self.d.borrow_mut();
        d.n += 1;
        let (key, is_ref) = match v {
            Value::Ref(rc) => match &*rc.borrow() {
                // References to objects are treated as if the reference
                // didn't exist.
                Value::Object(h) => (VarKey::Obj(h.borrow().id), true),
                _ => (VarKey::Ref(Rc::as_ptr(rc) as usize), true),
            },
            Value::Object(h) => (VarKey::Obj(h.borrow().id), false),
            _ => return 0,
        };
        match d.seen.get(&key) {
            Some(&n) => {
                // References are only counted once.
                if is_ref && n != -1 {
                    d.n -= 1;
                }
                n
            }
            None => {
                let n = d.n;
                d.seen.insert(key, n);
                d.hold.push(v.clone());
                0
            }
        }
    }

    /// Mark object `id` as failed: later references to it serialize as `N;`.
    fn mark_failed(&mut self, id: u32) {
        if let Some(n) = self.d.borrow_mut().seen.get_mut(&VarKey::Obj(id)) {
            *n = -1;
        }
    }

    /// A nested element / property value: a reference nobody else holds is
    /// serialized as its plain value (`Z_REFCOUNT == 1`).
    fn nested(&mut self, host: &mut dyn Host, out: &mut Vec<u8>, v: &Value) -> Result<(), Thrown> {
        match v {
            Value::Ref(rc) if Rc::strong_count(rc) == 1 => {
                let inner = rc.borrow().clone();
                self.value(host, out, &inner)
            }
            _ => self.value(host, out, v),
        }
    }

    fn value(&mut self, host: &mut dyn Host, out: &mut Vec<u8>, v: &Value) -> Result<(), Thrown> {
        crate::zend::call_stack::check()?;
        match self.add_var_hash(v) {
            0 => {}
            -1 => {
                out.extend_from_slice(b"N;");
                return Ok(());
            }
            n => {
                let tag = if matches!(v, Value::Ref(_)) { "R" } else { "r" };
                out.extend_from_slice(format!("{tag}:{n};").as_bytes());
                return Ok(());
            }
        }
        match deref(v) {
            Value::Null | Value::Undef => out.extend_from_slice(b"N;"),
            Value::Bool(b) => out.extend_from_slice(if b { b"b:1;" } else { b"b:0;" }),
            Value::Int(n) => out.extend_from_slice(format!("i:{n};").as_bytes()),
            Value::Float(f) => {
                out.extend_from_slice(b"d:");
                out.extend_from_slice(&fmt::float_serialize_gcvt(f));
                out.push(b';');
            }
            Value::Str(s) => serialize_str(out, s.as_bytes()),
            Value::Array(a) => {
                out.extend_from_slice(format!("a:{}:{{", a.len()).as_bytes());
                for (k, val) in a.iter() {
                    serialize_key(out, k);
                    self.nested(host, out, val)?;
                }
                out.push(b'}');
            }
            Value::Object(h) => self.object(host, out, &h)?,
            Value::Closure(_) => {
                return Err(Thrown::error(
                    "Exception",
                    "Serialization of 'Closure' is not allowed",
                ))
            }
            Value::Resource(_) => out.extend_from_slice(b"i:0;"), // php serializes a resource as 0
            Value::Ref(_) => unreachable!("deref'd above"),
        }
        Ok(())
    }

    fn object(
        &mut self,
        host: &mut dyn Host,
        out: &mut Vec<u8>,
        h: &rhp_values::ObjectHandle,
    ) -> Result<(), Thrown> {
        let class_id = h.borrow().class;
        let mut cname = host.class_name(class_id).unwrap_or_default();
        if host.not_serializable(class_id) {
            // php's `%s`: an anonymous class's name ends at its NUL.
            let shown = cname.split(|&b| b == 0).next().unwrap_or_default();
            return Err(Thrown::error(
                "Exception",
                format!(
                    "Serialization of '{}' is not allowed",
                    String::from_utf8_lossy(shown)
                ),
            ));
        }
        if host.is_enum_class(class_id) {
            let case = h
                .borrow()
                .properties
                .get(&PhpStr::from("name"))
                .map(conv::to_string_bytes)
                .unwrap_or_default();
            let body = [&cname[..], b":", &case[..]].concat();
            out.extend_from_slice(format!("E:{}:\"", body.len()).as_bytes());
            out.extend_from_slice(&body);
            out.extend_from_slice(b"\";");
            return Ok(());
        }
        let obj = Value::Object(h.clone());
        let call = |host: &mut dyn Host, m: &str| locked(|| call_method(host, &obj, m, Vec::new()));
        let head = |out: &mut Vec<u8>, cname: &[u8], n: usize| {
            out.extend_from_slice(format!("O:{}:\"", cname.len()).as_bytes());
            out.extend_from_slice(cname);
            out.extend_from_slice(format!("\":{n}:{{").as_bytes());
        };
        if host.class_has_method(class_id, b"__serialize") {
            let data = match deref(&call(host, "__serialize")?) {
                Value::Array(a) => a,
                _ => {
                    return Err(Thrown::type_error(format!(
                        "{}::__serialize() must return an array",
                        String::from_utf8_lossy(&cname)
                    )))
                }
            };
            head(out, &cname, data.len());
            for (k, val) in data.iter() {
                serialize_key(out, k);
                self.nested(host, out, val)?;
            }
            out.push(b'}');
            return Ok(());
        }
        if host.reflect_is_a(&cname, b"Serializable") {
            // zend_user_serialize: `C:len:"Class":datalen:{data}`; NULL (or
            // a failure) writes `N;` and later references to the object too.
            let r = call_method(host, &obj, "serialize", Vec::new())?;
            match deref(&r) {
                Value::Str(data) => {
                    out.extend_from_slice(format!("C:{}:\"", cname.len()).as_bytes());
                    out.extend_from_slice(&cname);
                    out.extend_from_slice(format!("\":{}:{{", data.len()).as_bytes());
                    out.extend_from_slice(data.as_bytes());
                    out.push(b'}');
                }
                Value::Null => {
                    self.mark_failed(h.borrow().id);
                    out.extend_from_slice(b"N;");
                }
                _ => {
                    return Err(Thrown::error(
                        "Exception",
                        format!(
                            "{}::serialize() must return a string or NULL",
                            String::from_utf8_lossy(&cname)
                        ),
                    ))
                }
            }
            return Ok(());
        }
        // The object's slots under their php-mangled names, or an internal
        // class's get_properties_for(SERIALIZE).
        let view = crate::zend::object_handlers::properties_for(
            host,
            h,
            crate::zend::object_handlers::PropPurpose::Serialize,
        )?;
        let mut props: Vec<(PhpStr, Value)> = match view {
            Some(a) => a
                .iter()
                .map(|(k, v)| {
                    let k = match k {
                        ArrayKey::Int(n) => PhpStr::from(n.to_string().as_str()),
                        ArrayKey::Str(s) => s.clone(),
                    };
                    (k, v.clone())
                })
                .collect(),
            None => {
                let b = h.borrow();
                b.properties
                    .iter()
                    .filter(|(k, _)| !host.is_internal_slot(class_id, k))
                    .map(|(k, v)| (host.mangled_prop_name(class_id, k), v.clone()))
                    .collect()
            }
        };
        let incomplete = cname == INCOMPLETE;
        if !incomplete && host.class_has_method(class_id, b"__sleep") {
            let names = match deref(&call(host, "__sleep")?) {
                Value::Array(a) => a,
                _ => {
                    host.emit_diagnostic(
                        crate::E_WARNING,
                        format!(
                            "serialize(): {}::__sleep() should return an array only containing the names of instance-variables to serialize",
                            String::from_utf8_lossy(&cname)
                        ).as_bytes(),
                    )?;
                    out.extend_from_slice(b"N;");
                    return Ok(());
                }
            };
            // (__sleep may have changed the properties.) A lazy object
            // initializes unless SKIP_INITIALIZATION_ON_SERIALIZE.
            let skip = h.borrow().lazy().is_some_and(|l| {
                l.flags & rhp_values::LazyInfo::SKIP_INITIALIZATION_ON_SERIALIZE != 0
            });
            let src = if skip {
                let i = h.borrow().lazy_instance();
                i.unwrap_or_else(|| h.clone())
            } else {
                host.lazy_props_object(h)?
            };
            let uninit = host.uninit_props(&src);
            let props: Vec<(PhpStr, Value)> = {
                let b = src.borrow();
                b.properties
                    .iter()
                    .filter(|(k, _)| !host.is_internal_slot(class_id, k))
                    .map(|(k, v)| (host.mangled_prop_name(class_id, k), v.clone()))
                    .collect()
            };
            // php_var_serialize_get_sleep_props: the name as is, then this
            // class's private mangling, then the protected one; a name that
            // matches none is skipped ("returned as member variable from
            // __sleep() but does not exist"), a repeat is dropped ("is
            // returned from __sleep() multiple times").
            let mut picked: Vec<(PhpStr, Value)> = Vec::new();
            for (_, n) in names.iter() {
                let n = deref(n);
                if !matches!(n, Value::Str(_)) {
                    host.emit_diagnostic(
                        crate::E_WARNING,
                        format!(
                            "serialize(): {}::__sleep() should return an array only containing the names of instance-variables to serialize",
                            String::from_utf8_lossy(&cname)
                        ).as_bytes(),
                    )?;
                }
                let n = conv::to_string_bytes(&n);
                let priv_ = [&b"\0"[..], &cname[..], b"\0", &n[..]].concat();
                let prot = [&b"\0*\0"[..], &n[..]].concat();
                let hit = [&n[..], &priv_[..], &prot[..]]
                    .iter()
                    .find_map(|want| props.iter().find(|(k, _)| k.as_bytes() == *want));
                match hit {
                    Some((k, v)) => {
                        if !picked.iter().any(|(pk, _)| pk == k) {
                            picked.push((k.clone(), v.clone()));
                        } else {
                            host.emit_diagnostic(
                                crate::E_WARNING,
                                &[
                                    b"serialize(): \"".as_slice(),
                                    &n,
                                    b"\" is returned from __sleep() multiple times",
                                ]
                                .concat(),
                            )?;
                        }
                    }
                    // An uninitialized typed property is skipped silently.
                    None if uninit.iter().any(|(k, _, _)| {
                        [&n[..], &priv_[..], &prot[..]].contains(&k.as_bytes())
                    }) => {}
                    None => host.emit_diagnostic(
                        crate::E_WARNING,
                        &[
                            b"serialize(): \"".as_slice(),
                            &n,
                            b"\" returned as member variable from __sleep() but does not exist",
                        ]
                        .concat(),
                    )?,
                }
            }
            head(out, &cname, picked.len());
            for (k, v) in &picked {
                serialize_str(out, k.as_bytes());
                self.nested(host, out, v)?;
            }
            out.push(b'}');
            return Ok(());
        }
        if incomplete {
            // An incomplete object serializes under its original class name,
            // without the magic name member.
            if let Some(i) = props
                .iter()
                .position(|(k, _)| k.as_bytes() == INCOMPLETE_NAME)
            {
                if let Value::Str(orig) = deref(&props[i].1) {
                    cname = orig.as_bytes().to_vec();
                }
                props.remove(i);
            }
        }
        head(out, &cname, props.len());
        for (k, v) in &props {
            // Property names are always string keys (`s:1:"0";`).
            serialize_str(out, k.as_bytes());
            self.nested(host, out, v)?;
        }
        out.push(b'}');
        Ok(())
    }
}

fn serialize_str(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(format!("s:{}:\"", bytes.len()).as_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\";");
}

fn serialize_key(out: &mut Vec<u8>, k: &ArrayKey) {
    match k {
        ArrayKey::Int(n) => out.extend_from_slice(format!("i:{n};").as_bytes()),
        ArrayKey::Str(s) => serialize_str(out, s.as_bytes()),
    }
}

fn fn_serialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    let mut s = Serializer { d: ser_init() };
    let r = s.value(host, &mut out, &deref(&args[0]));
    ser_destroy();
    r?;
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// unserialize (ext/standard/var_unserializer.re)
// ---------------------------------------------------------------------------

/// Ok(false) is php's failure return; Err an exception.
type PResult<T> = Result<T, Thrown>;

/// The `allowed_classes` option: `None` allows every class, else only the
/// listed (lowercased) names.
type Allowed = Option<Vec<Vec<u8>>>;

type Cell = Rc<RefCell<Value>>;

/// A var_dtor entry: a value kept alive until the end, or a delayed
/// `__wakeup` / `__unserialize` call.
enum Dtor {
    Hold(#[allow(dead_code)] Cell),
    Wakeup(ObjectHandle),
    Unserialize(ObjectHandle, Cell),
}

/// php_unserialize_data. Every parsed value lives in a cell; containers
/// hold `Value::Ref(cell)` while parsing, so `R:` can turn a value already
/// stored into a reference. A cell holding `Value::Ref(other)` is an `R:`
/// slot that aliases `other`. Once parsing is done the cells are unwrapped:
/// only the ones an `R:` named (and that more than one place holds) stay
/// PHP references.
#[derive(Default)]
struct UnserData {
    /// var_entries: the slots `r:` / `R:` count (`None` after a failed
    /// nested php_var_unserialize).
    slots: Vec<Option<Cell>>,
    dtor: Vec<Dtor>,
    allowed: Allowed,
    cur_depth: i64,
    max_depth: i64,
    /// Objects this unserialize created from `O:` (their properties hold
    /// cells; an object a `C:` handler filled may hold real references).
    created: rustc_hash::FxHashSet<u32>,
    /// Cells an `R:` turned into references (kept alive, so the addresses
    /// stay unique).
    referenced: HashMap<usize, Cell>,
    /// ref_props: the typed property a (not yet referenced) cell is the
    /// value of.
    ref_props: HashMap<usize, (ObjectHandle, PhpStr)>,
    /// The first typed property bound to a referenced cell.
    sources: HashMap<usize, (ObjectHandle, PhpStr)>,
}

fn ptr(c: &Cell) -> usize {
    Rc::as_ptr(c) as usize
}

/// php_var_unserialize_init: a fresh context, or the caller's while an
/// unserialize() is running (unless BG(serialize_lock) is held).
fn unser_init() -> Rc<RefCell<UnserData>> {
    let fresh = || {
        let max_depth = crate::ini_value(b"unserialize_max_depth")
            .flatten()
            .map_or(4096, |v| conv::to_int(&Value::str(v)));
        Rc::new(RefCell::new(UnserData {
            max_depth,
            ..Default::default()
        }))
    };
    if lock_held() {
        return fresh();
    }
    UNSER.with(|s| {
        let mut s = s.borrow_mut();
        match (&s.1, s.0) {
            (Some(d), n) if n > 0 => {
                let d = d.clone();
                s.0 += 1;
                d
            }
            _ => {
                let d = fresh();
                *s = (1, Some(d.clone()));
                d
            }
        }
    })
}

/// BG(unserialize).level: 1 for an outermost unserialize().
fn unser_level() -> u32 {
    if lock_held() {
        return 1;
    }
    UNSER.with(|s| s.borrow().0)
}

/// php_var_unserialize_destroy: the outermost call runs the delayed
/// `__wakeup` / `__unserialize` calls (var_destroy). `failed`: an exception
/// is pending, so none runs.
fn unser_destroy(
    host: &mut dyn Host,
    d: &Rc<RefCell<UnserData>>,
    failed: bool,
) -> Result<(), Thrown> {
    let outer = lock_held() || unser_level() == 1;
    if !lock_held() {
        UNSER.with(|s| {
            let mut s = s.borrow_mut();
            s.0 = s.0.saturating_sub(1);
            if s.0 == 0 {
                s.1 = None;
            }
        });
    }
    if !outer {
        return Ok(());
    }
    let entries = std::mem::take(&mut d.borrow_mut().dtor);
    // Past memory_limit nothing more runs (the VM reports it).
    let mut failed = failed || rhp_values::mem::state().attention();
    let mut thrown = None;
    for e in entries {
        let (obj, data) = match e {
            Dtor::Hold(_) => continue,
            Dtor::Wakeup(o) => (o, None),
            Dtor::Unserialize(o, c) => (o, Some(deref(&c.borrow()))),
        };
        if failed {
            host.mark_destructor_called(&obj);
            continue;
        }
        let o = Value::Object(obj.clone());
        let r = match data {
            Some(data) => locked(|| call_method(host, &o, "__unserialize", vec![data])),
            None => locked(|| call_method(host, &o, "__wakeup", Vec::new())),
        };
        if let Err(t) = r {
            failed = true;
            host.mark_destructor_called(&obj);
            thrown.get_or_insert(t);
        }
    }
    let mut d = d.borrow_mut();
    d.slots.clear();
    d.created.clear();
    d.referenced.clear();
    d.ref_props.clear();
    d.sources.clear();
    thrown.map_or(Ok(()), Err)
}

/// Whether `n` elements cannot fit in `len` bytes (each takes at least 2).
fn fake_elem_count(n: i64, len: usize) -> bool {
    n > (len / 2) as i64
}

const HT_MAX_SIZE: i64 = 0x4000_0000;

/// Matched length of `[0-9]+` at `i`.
fn digits(b: &[u8], i: usize) -> usize {
    b.get(i..)
        .map_or(0, |s| s.iter().take_while(|c| c.is_ascii_digit()).count())
}

/// `[+-]? [0-9]+` at `i`.
fn iv_len(b: &[u8], i: usize) -> usize {
    let sign = matches!(b.get(i), Some(b'+' | b'-')) as usize;
    match digits(b, i + sign) {
        0 => 0,
        n => sign + n,
    }
}

/// php's `d:` token (`iv | nv | nvexp`) at `i`: its length, or 0.
fn double_len(b: &[u8], i: usize) -> usize {
    let sign = matches!(b.get(i), Some(b'+' | b'-')) as usize;
    let int = digits(b, i + sign);
    let mut n = sign + int;
    let mut ok = int > 0;
    if b.get(i + n) == Some(&b'.') {
        let frac = digits(b, i + n + 1);
        if int > 0 || frac > 0 {
            n += 1 + frac;
            ok = true;
        }
    }
    if !ok {
        return 0;
    }
    if matches!(b.get(i + n), Some(b'e' | b'E')) {
        let e = iv_len(b, i + n + 1);
        if e > 0 {
            n += 1 + e;
        }
    }
    n
}

/// parse_uiv: digits as a size_t (wrapping, as php's does).
fn parse_uiv(b: &[u8], i: usize) -> u64 {
    let mut r: u64 = 0;
    for &c in b[i..].iter().take_while(|c| c.is_ascii_digit()) {
        r = r.wrapping_mul(10).wrapping_add((c - b'0') as u64);
    }
    r
}

struct Parser<'a, 'h> {
    host: &'h mut dyn Host,
    b: &'a [u8],
    /// php's `max`: the end of the data.
    max: usize,
    /// php's `*p`: where an error is reported.
    p: usize,
    d: Rc<RefCell<UnserData>>,
    /// The function php_error_docref names.
    fname: String,
}

impl Parser<'_, '_> {
    /// The byte at `i`; past the end the NUL php's strings end with.
    fn at(&self, i: usize) -> u8 {
        self.b.get(i).copied().unwrap_or(0)
    }

    fn warn(&mut self, msg: &str) -> Result<(), Thrown> {
        crate::warn(self.host, &self.fname, msg)
    }

    fn new_cell(&self) -> Cell {
        Rc::new(RefCell::new(Value::Null))
    }

    /// The cell an existing container entry holds, to parse into again
    /// (php overwrites the zval in place).
    fn reuse(&self, v: Option<&Value>) -> Option<Cell> {
        match v {
            // (Containers being built hold only cells.) A cell an `R:` made
            // a reference is shared: the slot gets a fresh one.
            Some(Value::Ref(c)) if !self.d.borrow().referenced.contains_key(&ptr(c)) => {
                Some(c.clone())
            }
            _ => None,
        }
    }

    /// The cell `c` stands for: itself, or the one its `R:` aliases.
    fn resolve(&self, c: &Cell) -> Cell {
        let inner = match &*c.borrow() {
            Value::Ref(t) => Some(t.clone()),
            _ => None,
        };
        inner.unwrap_or_else(|| c.clone())
    }

    /// Whether `c` is (now) a PHP reference, and the cell that holds it.
    fn as_ref(&self, c: &Cell) -> Option<Cell> {
        let t = self.resolve(c);
        (!Rc::ptr_eq(&t, c) || self.d.borrow().referenced.contains_key(&ptr(&t))).then_some(t)
    }

    fn value_of(&self, c: &Cell) -> Value {
        deref(&self.resolve(c).borrow())
    }

    fn oom(&self) -> bool {
        rhp_values::mem::state().attention()
    }

    /// php_var_unserialize: on failure the slots it added are unusable.
    fn unserialize(&mut self, rval: &Cell) -> PResult<bool> {
        let from = self.d.borrow().slots.len();
        let r = self.parse(rval, true);
        if !matches!(r, Ok(true)) {
            for s in self.d.borrow_mut().slots.iter_mut().skip(from) {
                *s = None;
            }
        }
        r
    }

    /// php_var_unserialize_internal. `vh`: values get var_hash slots (keys
    /// do not). Ok(false) is php's failure return.
    fn parse(&mut self, rval: &Cell, vh: bool) -> PResult<bool> {
        let start = self.p;
        if start >= self.max {
            return Ok(false);
        }
        let c0 = self.at(start);
        if vh && c0 != b'R' {
            self.d.borrow_mut().slots.push(Some(rval.clone()));
        }
        let c1 = self.at(start + 1);
        match (c0, c1) {
            (b'N', b';') => {
                self.p = start + 2;
                *rval.borrow_mut() = Value::Null;
                Ok(true)
            }
            (b'b', b':')
                if matches!(self.at(start + 2), b'0' | b'1') && self.at(start + 3) == b';' =>
            {
                self.p = start + 4;
                *rval.borrow_mut() = Value::Bool(self.at(start + 2) == b'1');
                Ok(true)
            }
            (b'R' | b'r', b':') => {
                let n = digits(self.b, start + 2);
                if n == 0 || self.at(start + 2 + n) != b';' {
                    return Ok(false);
                }
                self.p = start + 3 + n;
                if !vh {
                    return Ok(false);
                }
                let id = parse_uiv(self.b, start + 2);
                let target = id
                    .checked_sub(1)
                    .and_then(|i| self.d.borrow().slots.get(i as usize).cloned().flatten());
                let Some(target) = target else {
                    return Ok(false);
                };
                if c0 == b'r' {
                    if Rc::ptr_eq(&target, rval) {
                        return Ok(false);
                    }
                    let v = self.value_of(&target);
                    if !matches!(v, Value::Object(_)) {
                        return Ok(false);
                    }
                    *rval.borrow_mut() = v;
                    return Ok(true);
                }
                let t = self.resolve(&target);
                if Rc::ptr_eq(&target, rval) || Rc::ptr_eq(&t, rval) {
                    return Ok(false);
                }
                {
                    let mut d = self.d.borrow_mut();
                    if d.referenced.insert(ptr(&t), t.clone()).is_none() {
                        if let Some(info) = d.ref_props.remove(&ptr(&t)) {
                            d.sources.insert(ptr(&t), info);
                        }
                    }
                }
                *rval.borrow_mut() = Value::Ref(t);
                Ok(true)
            }
            (b'i', b':') => {
                let n = iv_len(self.b, start + 2);
                if n == 0 || self.at(start + 2 + n) != b';' {
                    return Ok(false);
                }
                self.p = start + 3 + n;
                let v = self.parse_iv2(start + 2).0?;
                *rval.borrow_mut() = Value::Int(v);
                Ok(true)
            }
            (b'd', b':') => {
                let rest = &self.b[(start + 2).min(self.b.len())..];
                let special = [&b"NAN;"[..], b"INF;", b"-INF;"]
                    .into_iter()
                    .find(|s| rest.starts_with(s));
                let (len, v) = match special {
                    Some(s) => (
                        s.len() - 1,
                        match s {
                            b"NAN;" => f64::NAN,
                            b"INF;" => f64::INFINITY,
                            _ => f64::NEG_INFINITY,
                        },
                    ),
                    None => {
                        let n = double_len(self.b, start + 2);
                        if n == 0 || self.at(start + 2 + n) != b';' {
                            return Ok(false);
                        }
                        let tok = &self.b[start + 2..start + 2 + n];
                        (
                            n,
                            std::str::from_utf8(tok)
                                .ok()
                                .and_then(|t| t.parse().ok())
                                .unwrap_or(0.0),
                        )
                    }
                };
                self.p = start + 3 + len;
                *rval.borrow_mut() = Value::Float(v);
                Ok(true)
            }
            (b's' | b'S' | b'O' | b'C' | b'E', b':') => {
                let n = digits(self.b, start + 2);
                if n == 0 || self.at(start + 2 + n) != b':' || self.at(start + 3 + n) != b'"' {
                    return Ok(false);
                }
                let cursor = start + 4 + n;
                let len = parse_uiv(self.b, start + 2);
                match c0 {
                    b's' => self.string(rval, start, cursor, len),
                    b'S' => self.escaped_string(rval, start, cursor, len),
                    b'E' => self.enum_case(rval, start, cursor, len, vh),
                    _ => {
                        if !vh {
                            return Ok(false);
                        }
                        self.object(rval, start, cursor, len)
                    }
                }
            }
            (b'a', b':') => {
                let n = digits(self.b, start + 2);
                if n == 0 || self.at(start + 2 + n) != b':' || self.at(start + 3 + n) != b'{' {
                    return Ok(false);
                }
                let cursor = start + 4 + n;
                // parse_iv, not uiv, to check the range.
                let elements = self.parse_iv2(start + 2).0?;
                self.p = cursor;
                if !vh {
                    return Ok(false);
                }
                if !(0..HT_MAX_SIZE).contains(&elements)
                    || fake_elem_count(elements, self.max - cursor)
                {
                    return Ok(false);
                }
                let mut arr = PhpArray::new();
                if elements > 0 {
                    let ok = self.nested_array(&mut arr, elements);
                    *rval.borrow_mut() = Value::Array(arr);
                    if !ok? {
                        return Ok(false);
                    }
                } else {
                    *rval.borrow_mut() = Value::Array(arr);
                }
                Ok(self.finish_nested())
            }
            (b'}', _) => {
                // "this is the case where we have less data than planned"
                self.warn("Unexpected end of serialized data")?;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// parse_iv2 at `i`: an optionally signed integer (too long warns and
    /// saturates); also where it ends.
    fn parse_iv2(&mut self, mut i: usize) -> (Result<i64, Thrown>, usize) {
        let mut neg = false;
        match self.at(i) {
            b'-' => {
                neg = true;
                i += 1;
            }
            b'+' => i += 1,
            _ => {}
        }
        while self.at(i) == b'0' {
            i += 1;
        }
        let start = i;
        let mut v: u64 = 0;
        while self.at(i).is_ascii_digit() {
            v = v.wrapping_mul(10).wrapping_add((self.at(i) - b'0') as u64);
            i += 1;
        }
        // MAX_LENGTH_OF_LONG - 1 digits
        if i - start > 19 || v > i64::MAX as u64 + neg as u64 {
            let r = self.warn("Numerical result out of range").map(|_| {
                if neg {
                    i64::MIN
                } else {
                    i64::MAX
                }
            });
            return (r, i);
        }
        (
            Ok(if neg {
                (v as i64).wrapping_neg()
            } else {
                v as i64
            }),
            i,
        )
    }

    /// `s:len:"…";`
    fn string(&mut self, rval: &Cell, start: usize, cursor: usize, len: u64) -> PResult<bool> {
        if ((self.max - cursor.min(self.max)) as u64) < len {
            self.p = start + 2;
            return Ok(false);
        }
        let end = cursor + len as usize;
        if self.at(end) != b'"' {
            self.p = end;
            return Ok(false);
        }
        if self.at(end + 1) != b';' {
            self.p = end + 1;
            return Ok(false);
        }
        self.p = end + 2;
        *rval.borrow_mut() = Value::str(self.b[cursor..end].to_vec());
        Ok(true)
    }

    /// `S:len:"…";`: `\xx` hex escapes (deprecated).
    fn escaped_string(
        &mut self,
        rval: &Cell,
        start: usize,
        cursor: usize,
        len: u64,
    ) -> PResult<bool> {
        let maxlen = self.max - cursor.min(self.max);
        if (maxlen as u64) < len {
            self.p = start + 2;
            return Ok(false);
        }
        let end = cursor + maxlen;
        let mut out = Vec::with_capacity(len as usize);
        let mut c = cursor;
        for _ in 0..len {
            if c >= end {
                return Ok(false);
            }
            if self.at(c) != b'\\' {
                out.push(self.at(c));
            } else {
                let mut ch: u8 = 0;
                for _ in 0..2 {
                    c += 1;
                    let d = match self.at(c) {
                        x @ b'0'..=b'9' => x - b'0',
                        x @ b'a'..=b'f' => x - b'a' + 10,
                        x @ b'A'..=b'F' => x - b'A' + 10,
                        _ => return Ok(false),
                    };
                    ch = (ch << 4).wrapping_add(d);
                }
                out.push(ch);
            }
            c += 1;
        }
        if self.at(c) != b'"' {
            self.p = c;
            return Ok(false);
        }
        if self.at(c + 1) != b';' {
            self.p = c + 1;
            return Ok(false);
        }
        self.p = c + 2;
        *rval.borrow_mut() = Value::str(out);
        let msg = format!(
            "{}(): Unserializing the 'S' format is deprecated",
            self.fname
        );
        self.host
            .emit_diagnostic(crate::E_DEPRECATED, msg.as_bytes())?;
        Ok(true)
    }

    /// The max_depth check of process_nested_*_data.
    fn enter(&mut self) -> PResult<bool> {
        let (cur, max) = {
            let d = self.d.borrow();
            (d.cur_depth, d.max_depth)
        };
        if max > 0 && cur >= max {
            self.warn(&format!(
                "Maximum depth of {max} exceeded. The depth limit can be changed using the max_depth unserialize() option or the unserialize_max_depth ini setting"
            ))?;
            return Ok(false);
        }
        self.d.borrow_mut().cur_depth += 1;
        Ok(true)
    }

    fn leave(&mut self) {
        self.d.borrow_mut().cur_depth -= 1;
    }

    /// A key (array key / property name): parsed without a var_hash slot.
    fn key(&mut self) -> PResult<Option<Value>> {
        let cell: Cell = Rc::new(RefCell::new(Value::Null));
        if !self.parse(&cell, false)? {
            return Ok(None);
        }
        let v = cell.borrow().clone();
        Ok(Some(v))
    }

    /// The check after each element: the value must end in `;` or `}`.
    fn element_end(&mut self, more: bool) -> bool {
        if more && !matches!(self.at(self.p.wrapping_sub(1)), b';' | b'}') {
            self.p -= 1;
            return false;
        }
        !self.oom()
    }

    /// process_nested_array_data.
    fn nested_array(&mut self, arr: &mut PhpArray, elements: i64) -> PResult<bool> {
        if !self.enter()? {
            return Ok(false);
        }
        let r = self.nested_array_inner(arr, elements);
        self.leave();
        r
    }

    fn nested_array_inner(&mut self, arr: &mut PhpArray, mut elements: i64) -> PResult<bool> {
        while elements > 0 {
            elements -= 1;
            let key = match self.key()? {
                Some(Value::Int(n)) => ArrayKey::Int(n),
                Some(Value::Str(s)) => ArrayKey::from_bytes(s.as_bytes()),
                _ => return Ok(false),
            };
            // (Containers being built hold only cells.)
            let slot = arr.get_or_insert_mut(key);
            let cell = match slot {
                Value::Ref(c) if !self.d.borrow().referenced.contains_key(&ptr(c)) => {
                    *c.borrow_mut() = Value::Null;
                    c.clone()
                }
                _ => {
                    let c = self.new_cell();
                    *slot = Value::Ref(c.clone());
                    c
                }
            };
            if !self.parse(&cell, true)? {
                return Ok(false);
            }
            if !self.element_end(elements > 0) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// process_nested_object_data.
    fn nested_object(&mut self, obj: &ObjectHandle, elements: i64) -> PResult<bool> {
        if !self.enter()? {
            return Ok(false);
        }
        let r = self.nested_object_inner(obj, elements);
        self.leave();
        r
    }

    fn nested_object_inner(&mut self, obj: &ObjectHandle, mut elements: i64) -> PResult<bool> {
        while elements > 0 {
            elements -= 1;
            let key = match self.key()? {
                Some(Value::Int(n)) => n.to_string().into_bytes(),
                Some(Value::Str(s)) => s.as_bytes().to_vec(),
                _ => return Ok(false),
            };
            let (storage, typed) = match self.host.unserialize_prop(obj, &key)? {
                UnserProp::Declared { storage, typed } => (storage, typed),
                UnserProp::Dynamic(k) => (k, false),
                UnserProp::Fail => return Ok(false),
            };
            let existing = obj.borrow().properties.get(&storage).cloned();
            let cell = match self.reuse(existing.as_ref()) {
                Some(c) => {
                    if typed {
                        let mut d = self.d.borrow_mut();
                        d.ref_props.remove(&ptr(&c));
                    }
                    *c.borrow_mut() = Value::Null;
                    c
                }
                None => self.new_cell(),
            };
            self.host
                .prop_store(obj, &storage, Some(Value::Ref(cell.clone())));
            let ok = self.parse(&cell, true);
            if typed {
                if !matches!(ok, Ok(true)) {
                    // The type source is added even when parsing failed.
                    if let Some(t) = self.as_ref(&cell) {
                        self.d
                            .borrow_mut()
                            .sources
                            .entry(ptr(&t))
                            .or_insert((obj.clone(), storage.clone()));
                    }
                    ok?;
                    return Ok(false);
                }
                let r = self.as_ref(&cell);
                let source = r
                    .as_ref()
                    .and_then(|t| self.d.borrow().sources.get(&ptr(t)).cloned());
                let val = self.value_of(&cell);
                match self.host.unserialize_prop_type(
                    obj,
                    &storage,
                    &val,
                    source.as_ref().map(|(o, k)| (o, k)),
                ) {
                    Err(t) => {
                        self.host.prop_store(obj, &storage, None);
                        return Err(t);
                    }
                    Ok(Some(conv)) => *cell.borrow_mut() = conv,
                    Ok(None) => {}
                }
                let mut d = self.d.borrow_mut();
                match r {
                    Some(t) => {
                        d.sources
                            .entry(ptr(&t))
                            .or_insert((obj.clone(), storage.clone()));
                    }
                    None => {
                        d.ref_props
                            .insert(ptr(&cell), (obj.clone(), storage.clone()));
                    }
                }
            } else if !ok? {
                return Ok(false);
            }
            if !self.element_end(elements > 0) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn finish_nested(&mut self) -> bool {
        if self.p >= self.max || self.at(self.p) != b'}' {
            return false;
        }
        self.p += 1;
        true
    }

    fn class_allowed(&self, lc: &[u8]) -> bool {
        match &self.d.borrow().allowed {
            None => true,
            Some(list) => list.iter().any(|n| n[..] == lc[..]),
        }
    }

    /// The `object ":" uiv ":" ["]` rule: `O:` and `C:`.
    fn object(&mut self, rval: &Cell, start: usize, cursor: usize, len: u64) -> PResult<bool> {
        let custom = self.at(start) == b'C';
        let maxlen = self.max - cursor.min(self.max);
        if (maxlen as u64) < len || len == 0 {
            self.p = start + 2;
            return Ok(false);
        }
        let end = cursor + len as usize;
        if self.at(end) != b'"' {
            self.p = end;
            return Ok(false);
        }
        if self.at(end + 1) != b':' {
            self.p = end + 1;
            return Ok(false);
        }
        let cname = self.b[cursor..end].to_vec();
        if cname[0] == 0 || cname[0] == b'\\' {
            return Ok(false);
        }
        let lc = cname.to_ascii_lowercase();
        // (Only an anonymous class has a name that is not valid.)
        if !valid_class_name(&cname) {
            return Ok(false);
        }
        let ce = if !self.class_allowed(&lc) {
            None
        } else if let Some(cid) = self.host.resolve_class(&cname) {
            Some(cid)
        } else {
            match locked(|| self.host.resolve_class_autoload(&cname))? {
                Some(cid) => Some(cid),
                None => self.callback_class(&cname)?,
            }
        };
        self.p = end;
        let name = match ce {
            Some(cid) => self.host.class_name(cid).unwrap_or_else(|| cname.clone()),
            None => INCOMPLETE.to_vec(),
        };
        if ce.is_some_and(|cid| self.host.not_serializable(cid)) {
            return Err(Thrown::error(
                "Exception",
                format!(
                    "Unserialization of '{}' is not allowed",
                    String::from_utf8_lossy(&name)
                ),
            ));
        }
        if custom {
            return self.object_custom(rval, ce, &name, &cname);
        }
        if self.p >= self.max.saturating_sub(2) {
            self.host
                .emit_diagnostic(crate::E_WARNING, b"Bad unserialize data")?;
            return Ok(false);
        }
        let (elements, after) = self.parse_iv2(self.p + 2);
        let elements = elements?;
        self.p = after;
        if elements < 0 || fake_elem_count(elements, self.max - end) {
            return Ok(false);
        }
        if self.at(self.p) != b':' {
            return Ok(false);
        }
        if self.at(self.p + 1) != b'{' {
            self.p += 1;
            return Ok(false);
        }
        self.p += 2;
        let has_unserialize =
            ce.is_some_and(|cid| self.host.class_has_method(cid, b"__unserialize"));
        // A Serializable class reads `C:` data, unless it also has
        // __unserialize.
        if ce.is_some_and(|cid| !has_unserialize && self.host.class_has_method(cid, b"serialize"))
            && self.host.reflect_is_a(&name, b"Serializable")
        {
            let msg = format!(
                "Erroneous data format for unserializing '{}'",
                String::from_utf8_lossy(&name)
            );
            self.host
                .emit_diagnostic(crate::E_WARNING, msg.as_bytes())?;
            return Ok(false);
        }
        let obj = match ce {
            Some(_) => self.host.reflect_new_instance(&name, Vec::new(), false)?,
            None => self.incomplete(&cname)?,
        };
        if self.oom() {
            return Ok(false);
        }
        let Value::Object(h) = obj.clone() else {
            return Ok(false);
        };
        self.d.borrow_mut().created.insert(h.borrow().id);
        *rval.borrow_mut() = obj;
        self.object_common(&h, elements, has_unserialize, ce.is_some())
    }

    /// An `__PHP_Incomplete_Class` remembering class `name`.
    fn incomplete(&mut self, name: &[u8]) -> PResult<Value> {
        self.host
            .instantiate(
                INCOMPLETE,
                vec![(INCOMPLETE_NAME.to_vec(), Value::str(name.to_vec()))],
            )
            .ok_or_else(|| Thrown::error("Error", "no incomplete class"))
    }

    /// unserialize_callback_func for a class nothing could load.
    fn callback_class(&mut self, cname: &[u8]) -> PResult<Option<u32>> {
        let func = crate::ini_value(b"unserialize_callback_func")
            .flatten()
            .unwrap_or_default();
        if func.is_empty() {
            return Ok(None);
        }
        let f = Value::str(func.clone());
        let arg = Value::str(cname.to_vec());
        locked(|| -> Result<(), Thrown> {
            // zend_call_function: "Invalid callback".
            if let Some(why) = self.host.check_callable(&f)? {
                return Err(Thrown::error(
                    "Error",
                    format!("Invalid callback {}, {why}", String::from_utf8_lossy(&func)),
                ));
            }
            self.host.call_callable(&f, &[arg])?;
            Ok(())
        })?;
        match locked(|| self.host.resolve_class_autoload(cname))? {
            Some(cid) => Ok(Some(cid)),
            None => {
                self.warn(&format!(
                    "Function {}() hasn't defined the class it was called for",
                    String::from_utf8_lossy(&func)
                ))?;
                Ok(None)
            }
        }
    }

    /// object_common: the properties (or the `__unserialize` data); the
    /// `__wakeup` / `__unserialize` call is delayed to the end.
    fn object_common(
        &mut self,
        obj: &ObjectHandle,
        elements: i64,
        has_unserialize: bool,
        complete: bool,
    ) -> PResult<bool> {
        if has_unserialize {
            if elements >= HT_MAX_SIZE {
                return Ok(false);
            }
            let mut arr = PhpArray::new();
            let r = self.nested_array(&mut arr, elements);
            if !matches!(r, Ok(true)) {
                self.host.mark_destructor_called(obj);
                return r;
            }
            let data: Cell = Rc::new(RefCell::new(Value::Array(arr)));
            self.d
                .borrow_mut()
                .dtor
                .push(Dtor::Unserialize(obj.clone(), data));
            return Ok(self.finish_nested());
        }
        let cid = obj.borrow().class;
        let has_wakeup = complete && self.host.class_has_method(cid, b"__wakeup");
        let r = self.nested_object(obj, elements);
        if !matches!(r, Ok(true)) {
            if has_wakeup {
                self.host.mark_destructor_called(obj);
            }
            return r;
        }
        if has_wakeup {
            self.d.borrow_mut().dtor.push(Dtor::Wakeup(obj.clone()));
        }
        Ok(self.finish_nested())
    }

    /// object_custom: `C:len:"Class":datalen:{data}`, read by the class's
    /// `ce->unserialize`.
    fn object_custom(
        &mut self,
        rval: &Cell,
        ce: Option<u32>,
        name: &[u8],
        cname: &[u8],
    ) -> PResult<bool> {
        let (datalen, after) = self.parse_iv2(self.p + 2);
        let datalen = datalen?;
        self.p = after;
        if self.max.saturating_sub(self.p) < 2 {
            return Ok(false);
        }
        if self.at(self.p) != b':' {
            return Ok(false);
        }
        if self.at(self.p + 1) != b'{' {
            self.p += 1;
            return Ok(false);
        }
        self.p += 2;
        let present = (self.max - self.p) as i64;
        if datalen < 0 || present <= datalen {
            let msg = format!(
                "Insufficient data for unserializing - {datalen} required, {present} present"
            );
            self.host
                .emit_diagnostic(crate::E_WARNING, msg.as_bytes())?;
            return Ok(false);
        }
        let end = self.p + datalen as usize;
        if self.at(end) != b'}' {
            self.p = end;
            return Ok(false);
        }
        let data = self.b[self.p..end].to_vec();
        match ce {
            Some(cid) if crate::ext::gmp::is_gmp_class(self.host, cid) => {
                let obj = crate::ext::gmp::unserialize_create(self.host)?;
                *rval.borrow_mut() = obj.clone();
                // The callback reads only its data.
                let (whole_max, at) = (self.max, self.p);
                self.max = end;
                let r = self.gmp_unserialize(&obj);
                self.max = whole_max;
                self.p = at;
                r?;
            }
            Some(cid)
                if self.host.reflect_is_a(name, b"Serializable") || self.stub_serializable(cid) =>
            {
                // zend_user_unserialize: the object's unserialize($data).
                let obj = self.host.reflect_new_instance(name, Vec::new(), false)?;
                *rval.borrow_mut() = obj.clone();
                let Value::Object(h) = &obj else {
                    return Ok(false);
                };
                if !self.host.class_has_method(h.borrow().class, b"unserialize") {
                    return Err(Thrown::unsupported(
                        format!(
                            "unserialize() of a C: {} payload",
                            String::from_utf8_lossy(name)
                        ),
                        "the class does not implement Serializable yet",
                    ));
                }
                call_method(self.host, &obj, "unserialize", vec![Value::str(data)])?;
            }
            _ => {
                let msg = format!(
                    "Class {} has no unserializer",
                    String::from_utf8_lossy(name)
                );
                self.host
                    .emit_diagnostic(crate::E_WARNING, msg.as_bytes())?;
                let obj = match ce {
                    Some(_) => self.host.reflect_new_instance(name, Vec::new(), false)?,
                    None => self.incomplete(cname)?,
                };
                *rval.borrow_mut() = obj;
            }
        }
        if self.oom() {
            return Ok(false);
        }
        self.p = end + 1;
        Ok(true)
    }

    /// Whether php's stubs declare that class `cid` or an ancestor
    /// implements `Serializable`.
    fn stub_serializable(&self, cid: u32) -> bool {
        let mut cur = Some(cid);
        while let Some(id) = cur {
            let name = self.host.class_name(id).unwrap_or_default();
            if rhp_bytecode::signatures::class(&name)
                .is_some_and(|c| c.interfaces.contains(&"Serializable"))
            {
                return true;
            }
            cur = self
                .host
                .class_parent_name(id)
                .and_then(|p| self.host.resolve_class(&p));
        }
        false
    }

    /// gmp_unserialize: the number as a decimal string, then the properties.
    fn gmp_unserialize(&mut self, obj: &Value) -> PResult<()> {
        let fail = |p: &mut Self, what: &str, prev: Option<Thrown>| -> Thrown {
            let e = Thrown::error("Exception", format!("Could not unserialize {what}"));
            match prev {
                Some(prev) => p.host.chain_thrown(e, prev),
                None => e,
            }
        };
        let next = |p: &mut Self, what: &str| -> PResult<Value> {
            let cell = p.new_cell();
            match p.unserialize(&cell) {
                Ok(true) => Ok(p.value_of(&cell)),
                Ok(false) => Err(fail(p, what, None)),
                Err(t) => Err(fail(p, what, Some(t))),
            }
        };
        let Value::Str(num) = next(self, "number")? else {
            return Err(fail(self, "number", None));
        };
        if let Err(e) = crate::ext::gmp::unserialize_number(obj, num.as_bytes()) {
            return Err(fail(self, "number", Some(e)));
        }
        let Value::Array(props) = next(self, "properties")? else {
            return Err(fail(self, "properties", None));
        };
        // zend_hash_copy into the property table (cells, unwrapped later).
        if let Value::Object(h) = obj {
            self.d.borrow_mut().created.insert(h.borrow().id);
            let mut o = h.borrow_mut();
            for (k, v) in props.iter() {
                let key = match k {
                    ArrayKey::Int(n) => PhpStr::from_bytes(n.to_string().into_bytes()),
                    ArrayKey::Str(s) => s.clone(),
                };
                o.properties.insert(key, v.clone());
            }
        }
        Ok(())
    }

    /// `E:len:"Enum:Case";`
    fn enum_case(
        &mut self,
        rval: &Cell,
        start: usize,
        cursor: usize,
        len: u64,
        vh: bool,
    ) -> PResult<bool> {
        if !vh {
            return Ok(false);
        }
        let maxlen = self.max - cursor.min(self.max);
        if (maxlen as u64) < len || len == 0 {
            self.p = start + 2;
            return Ok(false);
        }
        let end = cursor + len as usize;
        if self.at(end) != b'"' {
            self.p = end;
            return Ok(false);
        }
        if self.at(end + 1) != b';' {
            self.p = end + 1;
            return Ok(false);
        }
        let body = self.b[cursor..end].to_vec();
        let Some(colon) = body.iter().position(|&c| c == b':') else {
            self.warn(&format!(
                "Invalid enum name '{}' (missing colon)",
                String::from_utf8_lossy(&body)
            ))?;
            return Ok(false);
        };
        let (class, case) = (&body[..colon], &body[colon + 1..]);
        if !valid_class_name(class) {
            return Ok(false);
        }
        let shown = |b: &[u8]| String::from_utf8_lossy(c_str(b.to_vec()).as_slice()).into_owned();
        let Some(cid) = self.host.resolve_class_autoload(class)? else {
            self.warn(&format!("Class '{}' not found", shown(class)))?;
            return Ok(false);
        };
        if !self.host.is_enum_class(cid) {
            self.warn(&format!("Class '{}' is not an enum", shown(class)))?;
            return Ok(false);
        }
        self.p = end + 2;
        match self.host.class_constant(class, case) {
            None => {
                self.warn(&format!(
                    "Undefined constant {}::{}",
                    shown(class),
                    shown(case)
                ))?;
                Ok(false)
            }
            Some(Value::Object(h))
                if h.borrow().class == cid
                    && h.borrow()
                        .properties
                        .get(&PhpStr::from("name"))
                        .is_some_and(|n| conv::to_string_bytes(n) == case) =>
            {
                *rval.borrow_mut() = Value::Object(h);
                Ok(true)
            }
            Some(_) => {
                self.warn(&format!(
                    "{}::{} is not an enum case",
                    shown(class),
                    shown(case)
                ))?;
                Ok(false)
            }
        }
    }
}

/// Replacing the parser's cells by plain values once parsing is done: a cell
/// an `R:` named stays a reference where more than one place holds it
/// (`counts`; without counts, wherever it was named). Every `Value::Ref`
/// in the parsed arrays and in the objects this unserialize created is a
/// cell.
struct Unwrap<'a> {
    d: &'a UnserData,
    /// Where each referenced cell is held.
    counts: Option<HashMap<usize, usize>>,
    objs: rustc_hash::FxHashSet<u32>,
    /// Referenced cells already unwrapped.
    done: HashSet<usize>,
}

impl Unwrap<'_> {
    fn resolve(c: &Cell) -> Cell {
        match &*c.borrow() {
            Value::Ref(t) => t.clone(),
            _ => c.clone(),
        }
    }

    /// Objects whose properties are parser cells: the ones created here,
    /// visited once.
    fn visit(&mut self, h: &ObjectHandle) -> bool {
        let id = h.borrow().id;
        self.d.created.contains(&id) && self.objs.insert(id)
    }

    fn count(&mut self, v: &Value) {
        match v {
            Value::Ref(c) => {
                let t = Self::resolve(c);
                let p = ptr(&t);
                let first = if self.d.referenced.contains_key(&p) {
                    let n = self
                        .counts
                        .get_or_insert_with(HashMap::new)
                        .entry(p)
                        .or_insert(0);
                    *n += 1;
                    *n == 1
                } else {
                    true
                };
                if first {
                    let inner = t.borrow().clone();
                    self.count(&inner);
                }
            }
            Value::Array(a) => {
                for (_, e) in a.iter() {
                    if matches!(e, Value::Ref(_) | Value::Array(_) | Value::Object(_)) {
                        self.count(e);
                    }
                }
            }
            Value::Object(h) if self.visit(h) => {
                let props: Vec<Value> = h.borrow().properties.values().cloned().collect();
                for p in &props {
                    self.count(p);
                }
            }
            _ => {}
        }
    }

    fn value(&mut self, v: Value) -> Value {
        match v {
            Value::Ref(c) => {
                let t = Self::resolve(&c);
                let p = ptr(&t);
                if !self.d.referenced.contains_key(&p) {
                    let inner = t.borrow().clone();
                    let inner = self.value(inner);
                    *t.borrow_mut() = inner.clone();
                    return inner;
                }
                let keep = self
                    .counts
                    .as_ref()
                    .is_none_or(|m| m.get(&p).copied().unwrap_or(0) > 1);
                if self.done.insert(p) {
                    let inner = std::mem::replace(&mut *t.borrow_mut(), Value::Null);
                    let inner = self.value(inner);
                    *t.borrow_mut() = inner;
                }
                if keep {
                    Value::Ref(t)
                } else {
                    t.borrow().clone()
                }
            }
            Value::Array(a) => {
                if !a
                    .iter()
                    .any(|(_, e)| matches!(e, Value::Ref(_) | Value::Array(_) | Value::Object(_)))
                {
                    return Value::Array(a);
                }
                let mut out = PhpArray::new();
                for (k, e) in a.iter() {
                    out.insert(k.clone(), self.value(e.clone()));
                }
                Value::Array(out)
            }
            Value::Object(h) => {
                if self.visit(&h) {
                    let mut i = 0;
                    loop {
                        let Some(val) = h.borrow().properties.get_index(i).map(|(_, v)| v.clone())
                        else {
                            break;
                        };
                        if matches!(val, Value::Ref(_) | Value::Array(_) | Value::Object(_)) {
                            let u = self.value(val);
                            if let Some((_, slot)) = h.borrow_mut().properties.get_index_mut(i) {
                                *slot = u;
                            }
                        }
                        i += 1;
                    }
                }
                Value::Object(h)
            }
            other => other,
        }
    }
}

/// Unwrap the result `root` and the pending `__unserialize` data. `count`:
/// the outermost call (references are counted).
fn unwrap_all(d: &Rc<RefCell<UnserData>>, root: &Cell, count: bool) {
    let data: Vec<Cell> = d
        .borrow()
        .dtor
        .iter()
        .filter_map(|e| match e {
            Dtor::Unserialize(_, c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    let dref = d.borrow();
    let mut u = Unwrap {
        d: &dref,
        counts: None,
        objs: Default::default(),
        done: HashSet::new(),
    };
    let rootv = Value::Ref(root.clone());
    if count && !dref.referenced.is_empty() {
        u.count(&rootv);
        for c in &data {
            let v = c.borrow().clone();
            u.count(&v);
        }
        u.counts.get_or_insert_with(HashMap::new);
        u.objs.clear();
    }
    let r = root.borrow().clone();
    let r = u.value(r);
    *root.borrow_mut() = r;
    for c in &data {
        let v = c.borrow().clone();
        let v = u.value(v);
        *c.borrow_mut() = v;
    }
}

/// php's `zend_is_valid_class_name`: name chars plus `\`.
fn valid_class_name(name: &[u8]) -> bool {
    name.iter()
        .all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'\\' || c >= 0x80)
}

/// unserialize()'s `$options`: allowed_classes and max_depth.
fn unserialize_options(
    host: &mut dyn Host,
    opts: Option<&Value>,
) -> Result<(Option<Allowed>, Option<i64>), Thrown> {
    let Some(Value::Array(opts)) = opts.map(deref) else {
        return Ok((None, None));
    };
    let allowed = match opts
        .get(&ArrayKey::from_bytes(b"allowed_classes"))
        .map(deref)
    {
        None => None,
        Some(Value::Bool(true)) => Some(None),
        Some(Value::Bool(false)) => Some(Some(Vec::new())),
        Some(Value::Array(list)) => {
            let mut names = Vec::new();
            for (_, v) in list.iter() {
                let v = deref(v);
                // php: a string, or an object through its string cast.
                if !matches!(v, Value::Str(_) | Value::Object(_)) {
                    return Err(Thrown::type_error(format!(
                        "unserialize(): Option \"allowed_classes\" must be an array of class names, {} given",
                        crate::zend::zpp::zval_value_name(host, &v)
                    )));
                }
                let s = crate::zend::zpp::zval_get_string(host, &v)?;
                if !valid_class_name(&s) {
                    return Err(Thrown::error(
                        "ValueError",
                        format!(
                            "unserialize(): Option \"allowed_classes\" must be an array of class names, \"{}\" given",
                            String::from_utf8_lossy(&c_str(s))
                        ),
                    ));
                }
                names.push(s.to_ascii_lowercase())
            }
            Some(Some(names))
        }
        Some(other) => {
            return Err(Thrown::type_error(format!(
                "unserialize(): Option \"allowed_classes\" must be of type array|bool, {} given",
                crate::zend::zpp::zval_value_name(host, &other)
            )))
        }
    };
    let depth = match opts.get(&ArrayKey::from_bytes(b"max_depth")).map(deref) {
        None => None,
        Some(Value::Int(n)) if n >= 0 => Some(n),
        Some(Value::Int(_)) => {
            return Err(Thrown::error(
                "ValueError",
                "unserialize(): Option \"max_depth\" must be greater than or equal to 0",
            ))
        }
        Some(other) => {
            return Err(Thrown::type_error(format!(
                "unserialize(): Option \"max_depth\" must be of type int, {} given",
                crate::zend::zpp::zval_value_name(host, &other)
            )))
        }
    };
    Ok((allowed, depth))
}

fn fn_unserialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let bytes = conv::to_string_bytes(&deref(&args[0]));
    if bytes.is_empty() {
        return Ok(Value::Bool(false));
    }
    let (allowed, depth) = unserialize_options(host, args.get(1))?;
    let d = unser_init();
    let nested = unser_level() > 1;
    let prev = {
        let mut dm = d.borrow_mut();
        let prev = (dm.allowed.clone(), dm.max_depth, dm.cur_depth);
        if let Some(a) = allowed {
            dm.allowed = a;
        }
        if let Some(n) = depth {
            dm.max_depth = n;
            // A nested call with its own max_depth counts from zero.
            dm.cur_depth = 0;
        }
        prev
    };
    let total = bytes.len();
    let mut p = Parser {
        host,
        b: &bytes,
        max: total,
        p: 0,
        d: d.clone(),
        fname: "unserialize".to_string(),
    };
    let root = p.new_cell();
    if nested {
        // var_tmp_var
        d.borrow_mut().dtor.push(Dtor::Hold(root.clone()));
    }
    let r = p.unserialize(&root);
    let at = p.p;
    let host = p.host;
    let restore = |d: &Rc<RefCell<UnserData>>| {
        let mut dm = d.borrow_mut();
        (dm.allowed, dm.max_depth, dm.cur_depth) = prev.clone();
    };
    let ok = match r {
        Ok(ok) => ok,
        Err(t) => {
            unwrap_all(&d, &root, !nested);
            restore(&d);
            unser_destroy(host, &d, true)?;
            return Err(t);
        }
    };
    let warned = if !ok {
        crate::warn(
            host,
            "unserialize",
            &format!("Error at offset {at} of {total} bytes"),
        )
    } else if at < total {
        crate::warn(
            host,
            "unserialize",
            &format!("Extra data starting at offset {at} of {total} bytes"),
        )
    } else {
        Ok(())
    };
    unwrap_all(&d, &root, !nested);
    restore(&d);
    if let Err(t) = warned {
        unser_destroy(host, &d, true)?;
        return Err(t);
    }
    unser_destroy(host, &d, false)?;
    if !ok {
        return Ok(Value::Bool(false));
    }
    // Per calling convention the result is never a reference.
    let v = deref(&root.borrow());
    Ok(v)
}

/// `__unserialize_value($data, $offset)`: one value of an SPL class's
/// `unserialize()` payload, read through the running unserialize()'s
/// var_hash. Returns `[$ok, $value, $offset_after]`.
fn fn_unserialize_value(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let bytes = conv::to_string_bytes(&deref(&args[0]));
    let at = args.get(1).map_or(0, |v| conv::to_int(&deref(v))).max(0) as usize;
    let fname = args.get(2).map_or(b"unserialize".to_vec(), |v| {
        conv::to_string_bytes(&deref(v))
    });
    let d = unser_init();
    let nested = unser_level() > 1;
    let mut p = Parser {
        host,
        b: &bytes,
        max: bytes.len(),
        p: at.min(bytes.len()),
        d: d.clone(),
        fname: String::from_utf8_lossy(&fname).into_owned(),
    };
    let cell = p.new_cell();
    d.borrow_mut().dtor.push(Dtor::Hold(cell.clone()));
    let r = p.unserialize(&cell);
    let end = p.p;
    let host = p.host;
    unwrap_all(&d, &cell, !nested);
    let ok = match r {
        Ok(ok) => ok,
        Err(t) => {
            unser_destroy(host, &d, true)?;
            return Err(t);
        }
    };
    unser_destroy(host, &d, false)?;
    let mut out = PhpArray::new();
    out.push(Value::Bool(ok));
    out.push(deref(&cell.borrow()));
    out.push(Value::Int(end as i64));
    Ok(Value::Array(out))
}

/// `__unserialize_init()` / `__unserialize_destroy()`: PHP_VAR_UNSERIALIZE_INIT
/// / _DESTROY around an SPL `unserialize()` method, so its values share one
/// var_hash (and the delayed calls run at the end, when outermost).
fn fn_unserialize_init(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    unser_init();
    Ok(Value::Null)
}

fn fn_unserialize_destroy(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    if lock_held() {
        return Ok(Value::Null);
    }
    let d = UNSER.with(|s| s.borrow().1.clone());
    if let Some(d) = d {
        unser_destroy(host, &d, false)?;
    }
    Ok(Value::Null)
}

/// `__object_properties_load($object, $members)`: php's
/// object_properties_load, as SPL's unserialize() methods restore their
/// members (keys are mangled names; a new dynamic property is deprecated).
fn fn_object_properties_load(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (Value::Object(h), Value::Array(m)) = (deref(&args[0]), deref(&args[1])) else {
        return Ok(Value::Null);
    };
    for (k, v) in m.iter() {
        let key = match k {
            ArrayKey::Int(n) => n.to_string().into_bytes(),
            ArrayKey::Str(s) => s.as_bytes().to_vec(),
        };
        let storage = match host.unserialize_prop(&h, &key)? {
            UnserProp::Declared { storage, .. } | UnserProp::Dynamic(storage) => storage,
            UnserProp::Fail => continue,
        };
        host.prop_store(&h, &storage, Some(rhp_values::ops::copy_elem(v)));
    }
    Ok(Value::Null)
}

/// `__spl_array_unserialize($data, $fn)`: ArrayObject::unserialize()'s
/// payload (`x:i:flags;storage;m:members`), as `[flags, storage, members]`
/// (`storage` null with SPL_ARRAY_IS_SELF; all null for no data). A bad
/// payload throws UnexpectedValueException "Error at offset".
fn fn_spl_array_unserialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let bytes = conv::to_string_bytes(&deref(&args[0]));
    let fname = args.get(1).map_or(b"unserialize".to_vec(), |v| {
        conv::to_string_bytes(&deref(v))
    });
    let n = bytes.len();
    let mut out = PhpArray::new();
    if n == 0 {
        for _ in 0..3 {
            out.push(Value::Null);
        }
        return Ok(Value::Array(out));
    }
    let d = unser_init();
    let nested = unser_level() > 1;
    let mut p = Parser {
        host,
        b: &bytes,
        max: n,
        p: 0,
        d: d.clone(),
        fname: String::from_utf8_lossy(&fname).into_owned(),
    };
    let r = spl_array_payload(&mut p);
    let at = p.p;
    let host = p.host;
    let r = match r {
        Ok(v) => v,
        Err(t) => {
            unser_destroy(host, &d, true)?;
            return Err(t);
        }
    };
    let cells: Vec<Cell> = r.iter().flatten().flatten().cloned().collect();
    for c in &cells {
        unwrap_all(&d, c, !nested);
    }
    unser_destroy(host, &d, false)?;
    let Some([flags, storage, members]) = r else {
        return Err(Thrown::error(
            "UnexpectedValueException",
            format!("Error at offset {at} of {n} bytes"),
        ));
    };
    let val = |c: &Option<Cell>| c.as_ref().map_or(Value::Null, |c| deref(&c.borrow()));
    out.push(Value::Int(conv::to_int(&val(&flags)) & 0x0100_ffff));
    out.push(val(&storage));
    out.push(val(&members));
    Ok(Value::Array(out))
}

/// The parse of [`fn_spl_array_unserialize`]: `None` on a bad payload
/// (`p.p` is the offset to report).
fn spl_array_payload(p: &mut Parser) -> PResult<Option<[Option<Cell>; 3]>> {
    let value = |p: &mut Parser| -> PResult<Option<Cell>> {
        let c = p.new_cell();
        p.d.borrow_mut().dtor.push(Dtor::Hold(c.clone()));
        Ok(p.unserialize(&c)?.then_some(c))
    };
    if p.at(0) != b'x' {
        return Ok(None);
    }
    p.p = 1;
    if p.at(1) != b':' {
        return Ok(None);
    }
    p.p = 2;
    let Some(flags) = value(p)?.filter(|c| matches!(deref(&c.borrow()), Value::Int(_))) else {
        return Ok(None);
    };
    p.p -= 1;
    if p.at(p.p) != b';' {
        return Ok(None);
    }
    p.p += 1;
    let is_self = conv::to_int(&deref(&flags.borrow())) & 0x0100_0000 != 0;
    let mut storage = None;
    if !is_self {
        if !matches!(p.at(p.p), b'a' | b'O' | b'C' | b'r') {
            return Ok(None);
        }
        let Some(s) =
            value(p)?.filter(|c| matches!(p.value_of(c), Value::Array(_) | Value::Object(_)))
        else {
            return Ok(None);
        };
        storage = Some(s);
        if p.at(p.p) != b';' {
            return Ok(None);
        }
        p.p += 1;
    }
    if p.at(p.p) != b'm' {
        return Ok(None);
    }
    p.p += 1;
    if p.at(p.p) != b':' {
        return Ok(None);
    }
    p.p += 1;
    let Some(members) = value(p)?.filter(|c| matches!(p.value_of(c), Value::Array(_))) else {
        return Ok(None);
    };
    Ok(Some([Some(flags), storage, Some(members)]))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("serialize", fn_serialize),
            ("unserialize", fn_unserialize),
            ("__unserialize_value", fn_unserialize_value),
            ("__spl_array_unserialize", fn_spl_array_unserialize),
            ("__object_properties_load", fn_object_properties_load),
            ("__unserialize_init", fn_unserialize_init),
            ("__unserialize_destroy", fn_unserialize_destroy),
            ("var_dump", fn_var_dump),
            ("debug_zval_dump", fn_debug_zval_dump),
            ("var_export", fn_var_export),
            ("memory_get_usage", fn_memory_get_usage),
            ("memory_get_peak_usage", fn_memory_get_peak_usage),
            ("memory_reset_peak_usage", fn_memory_reset_peak_usage),
        ],
    );
}

#[cfg(test)]
mod tests;

fn fn_var_dump(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    dump_args(host, args, false)
}

/// `debug_zval_dump(mixed $value, mixed ...$values)`.
fn fn_debug_zval_dump(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    dump_args(host, args, true)
}

/// php_var_dump / php_debug_zval_dump of each argument, streamed.
fn dump_args(host: &mut dyn Host, args: &[Value], zval: bool) -> Result<Value, Thrown> {
    let mut d = Dumper::new(host, true, zval);
    let mut r = Ok(());
    for a in args {
        // Arguments come by value (a reference's value, held once more).
        r = match a {
            Value::Ref(_) => {
                let mut held = Held::new();
                let r = d.value(held.hold(deref(a)), 1);
                held.release();
                r
            }
            _ => d.value(a, 1),
        };
        if r.is_err() {
            break;
        }
    }
    d.flush();
    r.map(|_| Value::Null)
}

fn fn_var_export(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    let r = resolve_views(
        host,
        &args[0],
        zend::object_handlers::PropPurpose::VarExport,
    );
    let circular = if r.is_ok() {
        var_export(host, &mut out, &args[0], 1)
    } else {
        0
    };
    clear_views();
    r?;
    for _ in 0..circular {
        host.emit_diagnostic(E_WARNING, b"var_export does not handle circular references")?;
    }
    let return_mode = args.get(1).map(conv::to_bool).unwrap_or(false);
    if return_mode {
        Ok(Value::str(out))
    } else {
        host.write_out(&out);
        Ok(Value::Null)
    }
}

/// `memory_get_usage(bool $real_usage = false)`: zend_mm's `size`, or with
/// `$real_usage` the chunks and huge blocks taken from the system.
fn fn_memory_get_usage(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let real =
        crate::zend::zpp::bool_param(host, args, 0, "memory_get_usage", "real_usage", false)?;
    Ok(Value::Int(rhp_values::mem::usage(real) as i64))
}

/// `memory_get_peak_usage(bool $real_usage = false)`.
fn fn_memory_get_peak_usage(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let real =
        crate::zend::zpp::bool_param(host, args, 0, "memory_get_peak_usage", "real_usage", false)?;
    Ok(Value::Int(rhp_values::mem::peak_usage(real) as i64))
}

fn fn_memory_reset_peak_usage(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    rhp_values::mem::reset_peak();
    Ok(Value::Null)
}
