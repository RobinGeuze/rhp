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
use rhp_values::{conv, fmt, ArrayKey, PhpArray, PhpStr, Thrown, Value};

use crate::zend::object_handlers::PropPurpose;
use crate::{zend, Host, E_WARNING};

fn sp(n: usize, out: &mut Vec<u8>) {
    out.extend(std::iter::repeat_n(b' ', n));
}

/// Split a php-mangled property name: `"\0Class\0name"` -> (name, Some(Class)),
/// `"\0*\0name"` -> (name, Some("*")), plain `name` -> (name, None).
pub fn unmangle_prop(key: &[u8]) -> (&[u8], Option<&[u8]>) {
    if key.first() == Some(&0) {
        if let Some(end) = key[1..].iter().position(|&b| b == 0) {
            return (&key[end + 2..], Some(&key[1..end + 1]));
        }
    }
    (key, None)
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
        _ => {}
    }
    Ok(())
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
            // php raises this mid-dump; rhp formats after resolving, so it
            // comes before the dump output.
            let cname = host.class_name(class).unwrap_or_default();
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                &format!(
                    "Returning null from {}::__debugInfo() is deprecated, return an empty array instead",
                    String::from_utf8_lossy(&cname)
                ),
            )?;
            Ok(rhp_values::PhpArray::new())
        }
        _ => Err(Thrown::error("Error", "__debuginfo() must return an array")),
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

/// `var_dump` of one value at the given indentation (spaces). `host` supplies
/// class names / enum-ness for object rendering.
pub fn var_dump(host: &dyn crate::Host, out: &mut Vec<u8>, v: &Value, indent: usize) {
    let mut seen = Seen::default();
    var_dump_value(host, out, &deref(v), false, indent, &mut seen);
}

/// Whether a slot holds a PHP reference that php's `var_dump` marks with `&`.
/// Not rendered yet: many builtins still copy reference wrappers out of
/// arrays where php copies the plain value, so the mark would be spurious.
fn is_shared_ref(_host: &dyn crate::Host, _v: &Value) -> bool {
    false
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

/// One `var_dump` value; `shared` marks a nested PHP reference with `&`.
fn var_dump_value(
    host: &dyn crate::Host,
    out: &mut Vec<u8>,
    v: &Value,
    shared: bool,
    indent: usize,
    seen: &mut Seen,
) {
    let amp = if shared { "&" } else { "" };
    let v = deref(v);
    sp(indent, out);
    match &v {
        Value::Null => out.extend_from_slice(format!("{amp}NULL\n").as_bytes()),
        Value::Bool(b) => out.extend_from_slice(format!("{amp}bool({b})\n").as_bytes()),
        Value::Int(n) => out.extend_from_slice(format!("{amp}int({n})\n").as_bytes()),
        Value::Float(f) => {
            out.extend_from_slice(format!("{amp}float(").as_bytes());
            out.extend_from_slice(&fmt::float_serialize(*f));
            out.extend_from_slice(b")\n");
        }
        Value::Str(s) => {
            out.extend_from_slice(format!("{amp}string({}) \"", s.len()).as_bytes());
            out.extend_from_slice(s.as_bytes());
            out.extend_from_slice(b"\"\n");
        }
        Value::Array(a) => {
            let id = a.data_ptr();
            if seen.arrays.contains(&id) {
                out.extend_from_slice(b"*RECURSION*\n");
                return;
            }
            seen.arrays.push(id);
            out.extend_from_slice(format!("{amp}array({}) {{\n", a.len()).as_bytes());
            for (k, val) in a.iter() {
                sp(indent + 2, out);
                dump_key(out, k);
                out.extend_from_slice(b"=>\n");
                let shared = is_shared_ref(host, val);
                var_dump_value(host, out, val, shared, indent + 2, seen);
            }
            sp(indent, out);
            out.extend_from_slice(b"}\n");
            seen.arrays.pop();
        }
        Value::Object(h) => {
            // Snapshot under a short borrow so recursion can't double-borrow
            // (reference-ness is read before the snapshot adds a holder).
            let (class_id, obj_id, shared) = {
                let b = h.borrow();
                let shared: Vec<bool> = b
                    .properties
                    .iter()
                    .filter(|(k, _)| !host.is_internal_slot(b.class, k))
                    .map(|(_, v)| is_shared_ref(host, v))
                    .collect();
                (b.class, b.id, shared)
            };
            let props = debug_props(host, h);
            // php prints the name as a C string: an anonymous class's
            // `X@anonymous\0file…` shows as `X@anonymous`.
            let cname = c_str(
                host.class_name(class_id)
                    .unwrap_or_else(|| b"class".to_vec()),
            );
            if host.is_enum_class(class_id) {
                // `enum(Name::Case)` — the case is the object's `name` property.
                let case = props
                    .iter()
                    .find(|(k, _)| matches!(k, ArrayKey::Str(s) if s.as_bytes() == b"name"))
                    .map(|(_, v)| conv::to_string_bytes(v))
                    .unwrap_or_default();
                out.extend_from_slice(format!("{amp}enum(").as_bytes());
                out.extend_from_slice(&cname);
                out.extend_from_slice(b"::");
                out.extend_from_slice(&case);
                out.extend_from_slice(b")\n");
            } else {
                if seen.objects.contains(&obj_id) {
                    out.extend_from_slice(b"*RECURSION*\n");
                    return;
                }
                seen.objects.push(obj_id);
                out.extend_from_slice(
                    format!(
                        "{amp}object({})#{} ({}) {{\n",
                        String::from_utf8_lossy(&cname),
                        obj_id,
                        props.len()
                    )
                    .as_bytes(),
                );
                // A view (a handler's or __debugInfo()'s) lists only its own
                // entries.
                let uninit = if VIEWS.with(|m| m.borrow().contains_key(&obj_id)) {
                    Vec::new()
                } else {
                    host.uninit_props(h)
                };
                let mut next_uninit = 0usize;
                let dump_uninit = |out: &mut Vec<u8>, upto: usize, next: &mut usize| {
                    while let Some((k, ty, pos)) = uninit.get(*next) {
                        if *pos > upto {
                            break;
                        }
                        sp(indent + 2, out);
                        out.extend_from_slice(b"[\"");
                        let (name, owner) = unmangle_prop(k.as_bytes());
                        out.extend_from_slice(name);
                        out.push(b'"');
                        match owner {
                            None => {}
                            Some(b"*") => out.extend_from_slice(b":protected"),
                            Some(class) => {
                                out.extend_from_slice(b":\"");
                                out.extend_from_slice(class);
                                out.extend_from_slice(b"\":private");
                            }
                        }
                        out.extend_from_slice(b"]=>\n");
                        sp(indent + 2, out);
                        out.extend_from_slice(format!("uninitialized({ty})\n").as_bytes());
                        *next += 1;
                    }
                };
                for (i, (k, val)) in props.iter().enumerate() {
                    dump_uninit(out, i, &mut next_uninit);
                    sp(indent + 2, out);
                    let k = match k {
                        ArrayKey::Int(n) => {
                            out.extend_from_slice(format!("[{n}]=>\n").as_bytes());
                            var_dump_value(host, out, val, false, indent + 2, seen);
                            continue;
                        }
                        ArrayKey::Str(s) => s,
                    };
                    out.extend_from_slice(b"[\"");
                    let (name, owner) = unmangle_prop(k.as_bytes());
                    out.extend_from_slice(name);
                    out.push(b'"');
                    match owner {
                        None => {}
                        Some(b"*") => out.extend_from_slice(b":protected"),
                        Some(class) => {
                            out.extend_from_slice(b":\"");
                            out.extend_from_slice(class);
                            out.extend_from_slice(b"\":private");
                        }
                    }
                    out.extend_from_slice(b"]=>\n");
                    let shared = shared.get(i).copied().unwrap_or(false);
                    var_dump_value(host, out, val, shared, indent + 2, seen);
                }
                dump_uninit(out, usize::MAX, &mut next_uninit);
                sp(indent, out);
                out.extend_from_slice(b"}\n");
                seen.objects.pop();
            }
        }
        Value::Closure(_) => {
            out.extend_from_slice(format!("{amp}object(Closure)#0 (0) {{\n").as_bytes());
            sp(indent, out);
            out.extend_from_slice(b"}\n");
        }
        Value::Resource(r) => {
            let ty = if host.resource_closed(&v) {
                "Unknown"
            } else {
                r.type_name
            };
            out.extend_from_slice(format!("{amp}resource({}) of type ({})\n", r.id, ty).as_bytes());
        }
        Value::Ref(_) => unreachable!("deref'd above"),
    }
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
        if matches!(deref(val), Value::Array(_) | Value::Object(_)) {
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
        Value::Null => out.extend_from_slice(b"NULL"),
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
struct Serializer {
    n: i64,
    seen: HashMap<VarKey, i64>,
}

impl Serializer {
    /// `php_add_var_hash`: 0 for a first sighting, else the earlier slot.
    fn add_var_hash(&mut self, v: &Value) -> i64 {
        self.n += 1;
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
        match self.seen.get(&key) {
            Some(&n) => {
                // References are only counted once.
                if is_ref && n != -1 {
                    self.n -= 1;
                }
                n
            }
            None => {
                self.seen.insert(key, self.n);
                0
            }
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
            Value::Null => out.extend_from_slice(b"N;"),
            Value::Bool(b) => out.extend_from_slice(if b { b"b:1;" } else { b"b:0;" }),
            Value::Int(n) => out.extend_from_slice(format!("i:{n};").as_bytes()),
            Value::Float(f) => {
                out.extend_from_slice(b"d:");
                out.extend_from_slice(&fmt::float_serialize(f));
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
        let call = |host: &mut dyn Host, m: &str| {
            let mut cb = PhpArray::new();
            cb.push(obj.clone());
            cb.push(Value::str(m));
            host.call_callable(&Value::Array(cb), &[])
        };
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
                        &format!(
                            "serialize(): {}::__sleep() should return an array only containing the names of instance-variables to serialize",
                            String::from_utf8_lossy(&cname)
                        ),
                    )?;
                    out.extend_from_slice(b"N;");
                    return Ok(());
                }
            };
            // (__sleep may have changed the properties.)
            let props: Vec<(PhpStr, Value)> = {
                let b = h.borrow();
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
                let n = conv::to_string_bytes(&deref(n));
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
                                &format!(
                                    "serialize(): \"{}\" is returned from __sleep() multiple times",
                                    String::from_utf8_lossy(&n)
                                ),
                            )?;
                        }
                    }
                    None => host.emit_diagnostic(
                        crate::E_WARNING,
                        &format!(
                            "serialize(): \"{}\" returned as member variable from __sleep() but does not exist",
                            String::from_utf8_lossy(&n)
                        ),
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
    let mut s = Serializer {
        n: 0,
        seen: HashMap::new(),
    };
    s.value(host, &mut out, &deref(&args[0]))?;
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// unserialize
// ---------------------------------------------------------------------------

/// Why parsing stopped: malformed input (`false`) or a thrown error.
enum Fail {
    Malformed,
    Thrown(Thrown),
}

impl From<Thrown> for Fail {
    fn from(t: Thrown) -> Self {
        Fail::Thrown(t)
    }
}

type PResult<T> = Result<T, Fail>;

/// The `allowed_classes` option.
enum Allowed {
    All,
    None,
    /// Lowercased class names.
    Only(Vec<Vec<u8>>),
}

type Cell = Rc<RefCell<Value>>;

struct Parser<'a, 'h> {
    pub(crate) host: &'h mut dyn Host,
    b: &'a [u8],
    pos: usize,
    /// php's `var_hash` slots: every parsed value (but `R:`) in order. A
    /// container holds `Value::Ref(slot)` while parsing; slots no `R:` names
    /// are unwrapped back to plain values afterwards.
    slots: Vec<Cell>,
    /// Slots an `R:` targets: they stay PHP references.
    referenced: HashSet<usize>,
    /// Objects awaiting `__unserialize` (with its data) / `__wakeup`, in
    /// php's call order (they run once the whole payload has been parsed).
    pending: Vec<(Value, Option<Value>)>,
    allowed: Allowed,
    /// Where php's "Error at offset" points: the innermost failure.
    err_pos: Option<usize>,
    /// A value was expected but the container ended (`}`).
    unexpected_end: bool,
}

/// php's var_unserializer gives up once memory_limit was crossed (the VM
/// then reports it).
fn oom_stop() -> PResult<()> {
    if rhp_values::mem::state().attention() {
        return malformed();
    }
    Ok(())
}

fn malformed<T>() -> PResult<T> {
    Err(Fail::Malformed)
}

impl<'a> Parser<'a, '_> {
    fn expect(&mut self, c: u8) -> PResult<()> {
        if self.pos < self.b.len() && self.b[self.pos] == c {
            self.pos += 1;
            Ok(())
        } else {
            malformed()
        }
    }

    /// Read bytes up to (not including) `term`, consuming `term`.
    fn read_until(&mut self, term: u8) -> PResult<&'a [u8]> {
        let start = self.pos;
        while self.pos < self.b.len() && self.b[self.pos] != term {
            self.pos += 1;
        }
        if self.pos >= self.b.len() {
            return malformed();
        }
        let slice = &self.b[start..self.pos];
        self.pos += 1; // consume terminator
        Ok(slice)
    }

    fn read_len(&mut self, term: u8) -> PResult<usize> {
        parse_usize(self.read_until(term)?).ok_or(Fail::Malformed)
    }

    /// `"<len bytes>"` after a length prefix.
    fn read_quoted(&mut self, len: usize) -> PResult<Vec<u8>> {
        self.expect(b'"')?;
        let Some(bytes) = self.b.get(self.pos..self.pos + len) else {
            // php points just past the type letter and colon (`start + 2`).
            let at = self.b[..self.pos]
                .iter()
                .rposition(|&c| c == b':')
                .unwrap_or(0);
            let at = self.b[..at]
                .iter()
                .rposition(|&c| c == b':')
                .map_or(0, |i| i + 1);
            return self.fail_at(at);
        };
        let bytes = bytes.to_vec();
        self.pos += len;
        if self.expect(b'"').is_err() {
            return self.fail_at(self.pos);
        }
        Ok(bytes)
    }

    /// Fail with php's error offset at `pos` (unless an inner failure set it).
    fn fail_at<T>(&mut self, pos: usize) -> PResult<T> {
        self.err_pos.get_or_insert(pos);
        malformed()
    }

    /// A key (array key / property name): parsed without a var_hash slot.
    fn parse_key(&mut self) -> PResult<Value> {
        match self.b.get(self.pos) {
            Some(b'i') | Some(b's') => {
                let cell = Rc::new(RefCell::new(Value::Null));
                self.parse_into(&cell)?;
                let v = cell.borrow().clone();
                Ok(v)
            }
            Some(b'}') => {
                self.unexpected_end = true;
                self.fail_at(self.pos)
            }
            _ => self.fail_at(self.pos),
        }
    }

    /// One value as stored in its container: `Value::Ref(slot)`.
    fn parse_slot(&mut self) -> PResult<Value> {
        if self.b.get(self.pos) == Some(&b'R') {
            self.pos += 1;
            self.expect(b':')?;
            let id = self.read_len(b';')?;
            let cell = match id.checked_sub(1).and_then(|i| self.slots.get(i)) {
                Some(c) => c.clone(),
                None => return malformed(),
            };
            self.referenced.insert(Rc::as_ptr(&cell) as usize);
            return Ok(Value::Ref(cell));
        }
        let cell = Rc::new(RefCell::new(Value::Null));
        self.slots.push(cell.clone());
        self.parse_into(&cell)?;
        Ok(Value::Ref(cell))
    }

    fn parse_into(&mut self, cell: &Cell) -> PResult<()> {
        let start = self.pos;
        let r = self.parse_into_inner(cell);
        if matches!(r, Err(Fail::Malformed)) {
            self.err_pos.get_or_insert(start);
        }
        r
    }

    fn parse_into_inner(&mut self, cell: &Cell) -> PResult<()> {
        let v = match self.b.get(self.pos).copied() {
            Some(b'N') => {
                self.pos += 1;
                self.expect(b';')?;
                Value::Null
            }
            Some(b'b') => {
                self.pos += 1;
                self.expect(b':')?;
                match self.read_until(b';')? {
                    b"0" => Value::Bool(false),
                    b"1" => Value::Bool(true),
                    _ => return malformed(),
                }
            }
            Some(b'i') => {
                self.pos += 1;
                self.expect(b':')?;
                let tok = self.read_until(b';')?;
                Value::Int(parse_i64(tok).ok_or(Fail::Malformed)?)
            }
            Some(b'd') => {
                self.pos += 1;
                self.expect(b':')?;
                let tok = self.read_until(b';')?;
                Value::Float(parse_f64(tok).ok_or(Fail::Malformed)?)
            }
            Some(b's') => {
                self.pos += 1;
                self.expect(b':')?;
                let len = self.read_len(b':')?;
                let content = self.read_quoted(len)?;
                if self.expect(b';').is_err() {
                    return self.fail_at(self.pos);
                }
                Value::str(content)
            }
            Some(b'r') => {
                self.pos += 1;
                self.expect(b':')?;
                let id = self.read_len(b';')?;
                let target = match id.checked_sub(1).and_then(|i| self.slots.get(i)) {
                    Some(c) if !Rc::ptr_eq(c, cell) => deref(&c.borrow()),
                    _ => return malformed(),
                };
                match target {
                    Value::Object(_) => target,
                    _ => return malformed(),
                }
            }
            Some(b'a') => {
                self.pos += 1;
                self.expect(b':')?;
                let count = self.read_len(b':')?;
                self.expect(b'{')?;
                let mut arr = PhpArray::new();
                for _ in 0..count {
                    let key = ArrayKey::from_value(&self.parse_key()?).ok_or(Fail::Malformed)?;
                    let val = self.parse_slot()?;
                    arr.insert(key, val);
                    oom_stop()?;
                }
                if self.expect(b'}').is_err() {
                    return self.fail_at(self.pos);
                }
                Value::Array(arr)
            }
            Some(b'O') => return self.parse_object(cell),
            Some(b'C') => return self.parse_custom(cell),
            Some(b'}') => {
                // php: "this is the case where we have less data than planned"
                self.unexpected_end = true;
                return self.fail_at(self.pos);
            }
            Some(b'E') => {
                self.pos += 1;
                self.expect(b':')?;
                let len = self.read_len(b':')?;
                let body = self.read_quoted(len)?;
                self.expect(b';')?;
                self.enum_case(&body)?
            }
            _ => return malformed(),
        };
        *cell.borrow_mut() = v;
        Ok(())
    }

    /// `E:len:"Enum:Case";`. php warns (and fails) on a missing colon, an
    /// unknown class, a non-enum class, an undefined constant or a constant
    /// that is not a case.
    fn enum_case(&mut self, body: &[u8]) -> PResult<Value> {
        let colon = body
            .iter()
            .position(|&c| c == b':')
            .ok_or(Fail::Malformed)?;
        let (class, case) = (&body[..colon], &body[colon + 1..]);
        let Some(cid) = self.host.resolve_class_autoload(class)? else {
            let msg = format!(
                "unserialize(): Class '{}' not found",
                String::from_utf8_lossy(class)
            );
            self.host.emit_diagnostic(crate::E_WARNING, &msg)?;
            return malformed();
        };
        if !self.host.is_enum_class(cid) {
            return malformed();
        }
        match self.host.class_constant(class, case) {
            Some(Value::Object(h)) if h.borrow().class == cid => Ok(Value::Object(h)),
            _ => malformed(),
        }
    }

    fn class_allowed(&self, name: &[u8]) -> bool {
        match &self.allowed {
            Allowed::All => true,
            Allowed::None => false,
            Allowed::Only(list) => list.iter().any(|n| n[..] == name.to_ascii_lowercase()[..]),
        }
    }

    /// `O:len:"Class":count:{props}`: the object is created (and takes its
    /// slot) before its properties are read, so `r:` inside can name it.
    fn parse_object(&mut self, cell: &Cell) -> PResult<()> {
        self.pos += 1;
        self.expect(b':')?;
        let len = self.read_len(b':')?;
        let cname = self.read_quoted(len)?;
        if cname.is_empty() || cname[0] == 0 || cname[0] == b'\\' || !valid_class_name(&cname) {
            return malformed();
        }
        self.expect(b':')?;
        let count = self.read_len(b':')?;
        self.expect(b'{')?;

        let cid = if self.class_allowed(&cname) {
            self.host.resolve_class_autoload(&cname)?
        } else {
            None
        };
        let (obj, has_unserialize, has_wakeup) = match cid {
            Some(cid) => {
                let name = self.host.class_name(cid).unwrap_or_else(|| cname.clone());
                if self.host.not_serializable(cid) {
                    return Err(Thrown::error(
                        "Exception",
                        format!(
                            "Unserialization of '{}' is not allowed",
                            String::from_utf8_lossy(&name)
                        ),
                    )
                    .into());
                }
                // Enums / interfaces / abstract classes throw "Cannot
                // instantiate ...".
                let obj = self.host.reflect_new_instance(&name, Vec::new(), false)?;
                let has_unserialize = self.host.class_has_method(cid, b"__unserialize");
                let has_wakeup = self.host.class_has_method(cid, b"__wakeup");
                (obj, has_unserialize, has_wakeup)
            }
            None => {
                // Unknown or disallowed class: an incomplete object that
                // remembers the name.
                let obj = self
                    .host
                    .instantiate(
                        INCOMPLETE,
                        vec![(INCOMPLETE_NAME.to_vec(), Value::str(cname))],
                    )
                    .ok_or(Fail::Malformed)?;
                (obj, false, false)
            }
        };
        *cell.borrow_mut() = obj.clone();

        if has_unserialize {
            let mut data = PhpArray::new();
            for _ in 0..count {
                let key = ArrayKey::from_value(&self.parse_key()?).ok_or(Fail::Malformed)?;
                let val = self.parse_slot()?;
                data.insert(key, val);
                oom_stop()?;
            }
            self.expect(b'}')?;
            self.pending.push((obj, Some(Value::Array(data))));
            return Ok(());
        }
        for _ in 0..count {
            let key = match self.parse_key()? {
                Value::Int(n) => n.to_string().into_bytes(),
                Value::Str(s) => s.as_bytes().to_vec(),
                _ => return malformed(),
            };
            let val = self.parse_slot()?;
            if !self.host.unserialize_set_prop(&obj, &key, val) {
                return malformed();
            }
            oom_stop()?;
        }
        self.expect(b'}')?;
        if has_wakeup {
            self.pending.push((obj, None));
        }
        Ok(())
    }

    /// `C:len:"Class":datalen:{data}` (php's `object_custom`): the class's
    /// `ce->unserialize` reads `data`, parsing its values through the same
    /// var_hash.
    fn parse_custom(&mut self, cell: &Cell) -> PResult<()> {
        self.pos += 1;
        self.expect(b':')?;
        let len = self.read_len(b':')?;
        let cname = self.read_quoted(len)?;
        if cname.is_empty() || cname[0] == 0 || cname[0] == b'\\' || !valid_class_name(&cname) {
            return malformed();
        }
        if self.b.get(self.pos) != Some(&b':') {
            return self.fail_at(self.pos);
        }
        self.pos += 1;
        let cid = if self.class_allowed(&cname) {
            self.host.resolve_class_autoload(&cname)?
        } else {
            None
        };
        let name = match cid {
            Some(cid) => self.host.class_name(cid).unwrap_or_else(|| cname.clone()),
            None => INCOMPLETE.to_vec(),
        };
        if cid.is_some_and(|cid| self.host.not_serializable(cid)) {
            return Err(Thrown::error(
                "Exception",
                format!(
                    "Unserialization of '{}' is not allowed",
                    String::from_utf8_lossy(&name)
                ),
            )
            .into());
        }
        let datalen = self.parse_iv2()?;
        if self.b.len() - self.pos < 2 || self.b[self.pos] != b':' {
            return self.fail_at(self.pos);
        }
        if self.b[self.pos + 1] != b'{' {
            return self.fail_at(self.pos + 1);
        }
        self.pos += 2;
        let present = (self.b.len() - self.pos) as i64;
        if datalen < 0 || present <= datalen {
            let msg = format!(
                "Insufficient data for unserializing - {datalen} required, {present} present"
            );
            self.host.emit_diagnostic(crate::E_WARNING, &msg)?;
            return self.fail_at(self.pos);
        }
        let end = self.pos + datalen as usize;
        if self.b[end] != b'}' {
            return self.fail_at(end);
        }
        let data = &self.b[self.pos..end];
        match cid {
            Some(cid) if crate::ext::gmp::is_gmp_class(self.host, cid) => {
                let obj = crate::ext::gmp::unserialize_create(self.host)?;
                *cell.borrow_mut() = obj.clone();
                // The callback reads only its data.
                let whole = self.b;
                self.b = &whole[..end];
                let r = self.gmp_unserialize(&obj);
                self.b = whole;
                r?;
            }
            Some(_) if self.host.reflect_is_a(&name, b"Serializable") => {
                // zend_user_unserialize
                let obj = self.host.reflect_new_instance(&name, Vec::new(), false)?;
                *cell.borrow_mut() = obj.clone();
                let mut cb = PhpArray::new();
                cb.push(obj);
                cb.push(Value::str("unserialize"));
                self.host
                    .call_callable(&Value::Array(cb), &[Value::str(data.to_vec())])?;
            }
            // php's own Serializable classes (ArrayObject, SplObjectStorage…)
            // that rhp's do not implement yet.
            Some(cid) if self.stub_serializable(cid) => {
                return Err(Thrown::unsupported(
                    format!(
                        "unserialize() of a C: {} payload",
                        String::from_utf8_lossy(&name)
                    ),
                    "the class does not implement Serializable yet",
                )
                .into());
            }
            _ => {
                let msg = format!(
                    "Class {} has no unserializer",
                    String::from_utf8_lossy(&name)
                );
                self.host.emit_diagnostic(crate::E_WARNING, &msg)?;
                let obj = match cid {
                    Some(_) => self.host.reflect_new_instance(&name, Vec::new(), false)?,
                    None => self
                        .host
                        .instantiate(
                            INCOMPLETE,
                            vec![(INCOMPLETE_NAME.to_vec(), Value::str(cname))],
                        )
                        .ok_or(Fail::Malformed)?,
                };
                *cell.borrow_mut() = obj;
            }
        }
        oom_stop()?;
        self.pos = end + 1;
        Ok(())
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
        let fail = |p: &mut Self, what: &str, prev: Option<Thrown>| -> Fail {
            let e = Thrown::error("Exception", format!("Could not unserialize {what}"));
            Fail::Thrown(match prev {
                Some(prev) => p.host.chain_thrown(e, prev),
                None => e,
            })
        };
        let num = match self.parse_slot() {
            Ok(v) => deref(&v),
            Err(Fail::Malformed) => return Err(fail(self, "number", None)),
            Err(Fail::Thrown(t)) => return Err(fail(self, "number", Some(t))),
        };
        let Value::Str(num) = num else {
            return Err(fail(self, "number", None));
        };
        if let Err(e) = crate::ext::gmp::unserialize_number(obj, num.as_bytes()) {
            return Err(fail(self, "number", Some(e)));
        }
        let props = match self.parse_slot() {
            Ok(v) => deref(&v),
            Err(Fail::Malformed) => return Err(fail(self, "properties", None)),
            Err(Fail::Thrown(t)) => return Err(fail(self, "properties", Some(t))),
        };
        let Value::Array(props) = props else {
            return Err(fail(self, "properties", None));
        };
        // zend_hash_copy into the property table.
        if let Value::Object(h) = obj {
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

    /// php's `parse_iv2`: an optionally signed integer (a too long one warns
    /// and saturates).
    fn parse_iv2(&mut self) -> PResult<i64> {
        let mut neg = false;
        match self.b.get(self.pos) {
            Some(b'-') => {
                neg = true;
                self.pos += 1;
            }
            Some(b'+') => self.pos += 1,
            _ => {}
        }
        while self.b.get(self.pos) == Some(&b'0') {
            self.pos += 1;
        }
        let start = self.pos;
        let mut v: u64 = 0;
        while let Some(&c) = self.b.get(self.pos).filter(|c| c.is_ascii_digit()) {
            v = v.wrapping_mul(10).wrapping_add((c - b'0') as u64);
            self.pos += 1;
        }
        // MAX_LENGTH_OF_LONG - 1 digits
        if self.pos - start > 19 || v > i64::MAX as u64 + neg as u64 {
            crate::warn(self.host, "unserialize", "Numerical result out of range")?;
            return Ok(if neg { i64::MIN } else { i64::MAX });
        }
        Ok(if neg {
            (v as i64).wrapping_neg()
        } else {
            v as i64
        })
    }

    /// Replace every slot wrapper no `R:` named by its plain value (php only
    /// has references where the payload asked for them).
    fn unwrap(&self, v: Value, objs: &mut HashSet<u32>, cells: &mut HashSet<usize>) -> Value {
        match v {
            Value::Ref(cell) => {
                let ptr = Rc::as_ptr(&cell) as usize;
                if self.referenced.contains(&ptr) {
                    if cells.insert(ptr) {
                        let inner = std::mem::replace(&mut *cell.borrow_mut(), Value::Null);
                        let inner = self.unwrap(inner, objs, cells);
                        *cell.borrow_mut() = inner;
                    }
                    Value::Ref(cell)
                } else {
                    let inner = std::mem::replace(&mut *cell.borrow_mut(), Value::Null);
                    let inner = self.unwrap(inner, objs, cells);
                    // Keep the slot readable for any later visitor.
                    *cell.borrow_mut() = inner.clone();
                    inner
                }
            }
            Value::Array(a) => {
                let mut out = PhpArray::new();
                for (k, val) in a.iter() {
                    out.insert(k.clone(), self.unwrap(val.clone(), objs, cells));
                }
                Value::Array(out)
            }
            Value::Object(h) => {
                let id = h.borrow().id;
                if objs.insert(id) {
                    let props: Vec<(PhpStr, Value)> = h
                        .borrow()
                        .properties
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    for (k, val) in props {
                        let u = self.unwrap(val, objs, cells);
                        if let Some(slot) = h.borrow_mut().properties.get_mut(&k) {
                            *slot = u;
                        }
                    }
                }
                Value::Object(h)
            }
            other => other,
        }
    }
}

/// php's `zend_is_valid_class_name`: name chars plus `\`.
fn valid_class_name(name: &[u8]) -> bool {
    name.iter()
        .all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'\\' || c >= 0x80)
}

/// Parse an optionally-signed integer token, saturating on overflow (php
/// clamps out-of-range ints to `PHP_INT_MIN`/`PHP_INT_MAX`).
fn parse_i64(tok: &[u8]) -> Option<i64> {
    if tok.is_empty() {
        return None;
    }
    let (neg, digits) = match tok[0] {
        b'+' => (false, &tok[1..]),
        b'-' => (true, &tok[1..]),
        _ => (false, tok),
    };
    if digits.is_empty() || !digits.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let s = std::str::from_utf8(tok).ok()?;
    match s.parse::<i64>() {
        Ok(n) => Some(n),
        Err(_) => Some(if neg { i64::MIN } else { i64::MAX }),
    }
}

fn parse_usize(tok: &[u8]) -> Option<usize> {
    if tok.is_empty() || !tok.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    std::str::from_utf8(tok).ok()?.parse::<usize>().ok()
}

/// Parse a `d:` float token. Handles php's `INF` / `-INF` / `NAN` spellings and
/// otherwise delegates to Rust's `f64` parser (which accepts `1.0E+20` etc.).
fn parse_f64(tok: &[u8]) -> Option<f64> {
    match tok {
        b"INF" => return Some(f64::INFINITY),
        b"-INF" => return Some(f64::NEG_INFINITY),
        b"NAN" => return Some(f64::NAN),
        _ => {}
    }
    let s = std::str::from_utf8(tok).ok()?;
    s.parse::<f64>().ok()
}

/// The `allowed_classes` entry of unserialize()'s `$options`.
fn allowed_option(host: &mut dyn Host, opts: Option<&Value>) -> Result<Allowed, Thrown> {
    let Some(Value::Array(opts)) = opts.map(deref) else {
        return Ok(Allowed::All);
    };
    match opts
        .get(&ArrayKey::from_bytes(b"allowed_classes"))
        .map(deref)
    {
        None => Ok(Allowed::All),
        Some(Value::Bool(true)) => Ok(Allowed::All),
        Some(Value::Bool(false)) => Ok(Allowed::None),
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
                            String::from_utf8_lossy(&s)
                        ),
                    ));
                }
                names.push(s.to_ascii_lowercase())
            }
            Ok(Allowed::Only(names))
        }
        Some(other) => Err(Thrown::type_error(format!(
            "unserialize(): Option \"allowed_classes\" must be of type array|bool, {} given",
            crate::zend::zpp::zval_value_name(host, &other)
        ))),
    }
}

fn fn_unserialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let bytes = conv::to_string_bytes(&deref(&args[0]));
    let allowed = allowed_option(host, args.get(1))?;
    let mut p = Parser {
        host,
        b: &bytes,
        pos: 0,
        slots: Vec::new(),
        referenced: HashSet::new(),
        pending: Vec::new(),
        allowed,
        err_pos: None,
        unexpected_end: false,
    };
    // php tolerates trailing "extra data" at the top level: it still returns
    // the parsed value (only emits a warning). Malformed/incomplete input
    // yields `false`.
    let total = bytes.len();
    if total == 0 {
        return Ok(Value::Bool(false));
    }
    let v = match p.parse_slot() {
        Ok(v) => v,
        Err(Fail::Malformed) => {
            let at = p.err_pos.unwrap_or(0);
            if p.unexpected_end {
                crate::warn(p.host, "unserialize", "Unexpected end of serialized data")?;
            }
            crate::warn(
                p.host,
                "unserialize",
                &format!("Error at offset {at} of {total} bytes"),
            )?;
            return Ok(Value::Bool(false));
        }
        Err(Fail::Thrown(t)) => return Err(t),
    };
    if p.pos < total {
        let at = p.pos;
        crate::warn(
            p.host,
            "unserialize",
            &format!("Extra data starting at offset {at} of {total} bytes"),
        )?;
    }
    let (mut objs, mut cells) = (HashSet::new(), HashSet::new());
    let v = deref(&p.unwrap(v, &mut objs, &mut cells));
    let pending: Vec<(Value, Option<Value>)> = std::mem::take(&mut p.pending)
        .into_iter()
        .map(|(o, d)| (o, d.map(|d| p.unwrap(d, &mut objs, &mut cells))))
        .collect();
    for (obj, data) in pending {
        let mut cb = PhpArray::new();
        cb.push(obj);
        match data {
            Some(d) => {
                cb.push(Value::str("__unserialize"));
                p.host.call_callable(&Value::Array(cb), &[d])?;
            }
            None => {
                cb.push(Value::str("__wakeup"));
                p.host.call_callable(&Value::Array(cb), &[])?;
            }
        }
    }
    Ok(v)
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
            ("var_dump", fn_var_dump),
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
    let mut out = Vec::new();
    for a in args {
        let r = resolve_views(host, a, zend::object_handlers::PropPurpose::Debug);
        if r.is_ok() {
            var_dump(host, &mut out, a, 0);
        }
        clear_views();
        if let Err(t) = r {
            host.write_out(&out);
            return Err(t);
        }
    }
    host.write_out(&out);
    Ok(Value::Null)
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
        host.emit_diagnostic(E_WARNING, "var_export does not handle circular references")?;
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
