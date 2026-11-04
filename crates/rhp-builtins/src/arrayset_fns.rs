//! Array set-operation and aggregate extras (no by-reference args):
//! `array_replace_recursive`, `array_diff_assoc`, `array_intersect_assoc`,
//! `array_find`, `array_find_key`, `array_any`, `array_all`, and the
//! `key_exists` alias of `array_key_exists`.
//!
//! The `array_find` family (PHP 8.4+) is callback-based: the callback is
//! invoked with `(value, key)` in that order — verified against php 8.5.8.
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see `arrayset_fns/tests.rs` — expected values hardcoded from
//! `php -r '...'` output). Collisions with existing modules are skipped:
//! `array_sum`, `array_product`, `in_array`, `array_key_exists` live in
//! `lib.rs`; `array_replace` and `array_merge_recursive` live in
//! `strarr2_fns.rs`.

use std::collections::HashMap;

use rhp_semantics::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Require an array argument (after deref), or a `TypeError` naming the position.
fn want_array(v: &Value, func: &str, argn: usize) -> Result<PhpArray, Thrown> {
    match deref(v) {
        Value::Array(a) => Ok(a),
        other => Err(Thrown::type_error(format!(
            "{func}(): Argument #{argn} must be of type array, {} given",
            other.type_name_lower()
        ))),
    }
}

/// `ArrayKey` -> `Value` (for returning keys from the find family).
fn key_to_value(k: &ArrayKey) -> Value {
    match k {
        ArrayKey::Int(n) => Value::Int(*n),
        ArrayKey::Str(s) => Value::Str(s.clone()),
    }
}

// ---------------------------------------------------------------------------
// array_diff_assoc / array_intersect_assoc
//
// Both compare keys AND the `(string)` form of values (php casts both sides to
// string: two elements are equal iff `(string)$a === (string)$b` and the keys
// match).
// ---------------------------------------------------------------------------

fn fn_array_diff_assoc(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = want_array(&args[0], "array_diff_assoc", 1)?;
    let mut others = Vec::with_capacity(args.len() - 1);
    for (i, arg) in args[1..].iter().enumerate() {
        others.push(want_array(arg, "array_diff_assoc", i + 2)?);
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        let s = conv::to_string_bytes(v);
        // Keep when no other array holds this key with a matching string value.
        let present = others
            .iter()
            .any(|o| o.get(k).is_some_and(|ov| conv::to_string_bytes(ov) == s));
        if !present {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_intersect_assoc(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = want_array(&args[0], "array_intersect_assoc", 1)?;
    let mut others = Vec::with_capacity(args.len() - 1);
    for (i, arg) in args[1..].iter().enumerate() {
        others.push(want_array(arg, "array_intersect_assoc", i + 2)?);
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        let s = conv::to_string_bytes(v);
        // Keep when every other array holds this key with a matching string value.
        let in_all = others
            .iter()
            .all(|o| o.get(k).is_some_and(|ov| conv::to_string_bytes(ov) == s));
        if in_all {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_replace_recursive
//
// Like `array_replace`, but when both the existing and replacement values are
// arrays, merge them recursively instead of overwriting. Any non-array on
// either side replaces outright.
// ---------------------------------------------------------------------------

/// Fold `src` into `dest` per php's `php_array_replace_recursive`.
fn replace_recursive_into(dest: &mut PhpArray, src: &PhpArray) {
    for (k, v) in src.iter() {
        let recurse = match (dest.get(k), deref(v)) {
            (Some(Value::Array(cur)), Value::Array(src_arr)) => {
                let mut merged = cur.clone();
                replace_recursive_into(&mut merged, &src_arr);
                Some(merged)
            }
            _ => None,
        };
        match recurse {
            Some(merged) => {
                dest.insert(k.clone(), Value::Array(merged));
            }
            None => {
                dest.insert(k.clone(), v.clone());
            }
        }
    }
}

fn fn_array_replace_recursive(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = want_array(&args[0], "array_replace_recursive", 1)?;
    for (i, arg) in args[1..].iter().enumerate() {
        let a = want_array(arg, "array_replace_recursive", i + 2)?;
        replace_recursive_into(&mut out, &a);
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_find / array_find_key / array_any / array_all  (PHP 8.4+)
//
// The callback is invoked with `(value, key)` in that order (verified vs
// php 8.5.8). Iteration stops at the first short-circuit, matching php.
// ---------------------------------------------------------------------------

fn fn_array_find(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(&args[0], "array_find", 1)?;
    let cb = &args[1];
    for (k, v) in a.iter() {
        if conv::to_bool(&host.call_callable(cb, &[v.clone(), key_to_value(k)])?) {
            return Ok(v.clone());
        }
    }
    Ok(Value::Null)
}

fn fn_array_find_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(&args[0], "array_find_key", 1)?;
    let cb = &args[1];
    for (k, v) in a.iter() {
        if conv::to_bool(&host.call_callable(cb, &[v.clone(), key_to_value(k)])?) {
            return Ok(key_to_value(k));
        }
    }
    Ok(Value::Null)
}

fn fn_array_any(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(&args[0], "array_any", 1)?;
    let cb = &args[1];
    for (k, v) in a.iter() {
        if conv::to_bool(&host.call_callable(cb, &[v.clone(), key_to_value(k)])?) {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn fn_array_all(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(&args[0], "array_all", 1)?;
    let cb = &args[1];
    for (k, v) in a.iter() {
        if !conv::to_bool(&host.call_callable(cb, &[v.clone(), key_to_value(k)])?) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

// ---------------------------------------------------------------------------
// key_exists  (alias of array_key_exists; the canonical name lives in lib.rs)
// ---------------------------------------------------------------------------

fn fn_key_exists(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[1]) else {
        return Err(Thrown::type_error(
            "key_exists(): Argument #2 ($array) must be of type array",
        ));
    };
    let Some(key) = ArrayKey::from_value(&args[0]) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(a.get(&key).is_some()))
}

// ---------------------------------------------------------------------------
// small helper on Value for error messages (mirrors lib.rs)
// ---------------------------------------------------------------------------

trait TypeNameLower {
    fn type_name_lower(&self) -> &'static str;
}
impl TypeNameLower for Value {
    fn type_name_lower(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) | Value::Closure(_) => "object",
            Value::Resource(_) => "resource",
            Value::Ref(r) => r.borrow().type_name_lower(),
        }
    }
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "array_diff_assoc", fn_array_diff_assoc, 1, None);
    put(
        m,
        "array_intersect_assoc",
        fn_array_intersect_assoc,
        1,
        None,
    );
    put(
        m,
        "array_replace_recursive",
        fn_array_replace_recursive,
        1,
        None,
    );
    put(m, "array_find", fn_array_find, 2, Some(2));
    put(m, "array_find_key", fn_array_find_key, 2, Some(2));
    put(m, "array_any", fn_array_any, 2, Some(2));
    put(m, "array_all", fn_array_all, 2, Some(2));
    put(m, "key_exists", fn_key_exists, 2, Some(2));
}

#[cfg(test)]
mod tests;
