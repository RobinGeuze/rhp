//! By-reference array builtins: the sort family and the stack/queue operations.
//!
//! These take their array argument **by reference** (`array &$array`): the
//! compiler emits a `MakeRef` for arg 0, so the builtin receives a
//! `Value::Ref` and mutates the backing array in place. The `u*` variants
//! invoke a user comparator through the [`Host`] callback hook.

use std::cmp::Ordering;

use phrust_values::{conv, ArrayKey, PhpArray, Value};
use phrust_semantics::{deref, ops};

use crate::{Builtin, Host, Thrown};

// PHP sort-flag constants (low byte selects the comparison kind).
const SORT_NUMERIC: i64 = 1;
const SORT_STRING: i64 = 2;

fn key_to_value(k: &ArrayKey) -> Value {
    match k {
        ArrayKey::Int(n) => Value::Int(*n),
        ArrayKey::Str(s) => Value::Str(s.clone()),
    }
}

/// Compare two values honouring a `SORT_*` flag (default: PHP loose compare).
fn cmp_flagged(a: &Value, b: &Value, flags: i64) -> Ordering {
    match flags & 0xff {
        SORT_NUMERIC => conv::to_float(a)
            .partial_cmp(&conv::to_float(b))
            .unwrap_or(Ordering::Equal),
        SORT_STRING => conv::to_string_bytes(a).cmp(&conv::to_string_bytes(b)),
        _ => ops::loose_compare(a, b),
    }
}

/// Borrow the referenced array mutably and hand it to `f`. Returns `None` when
/// arg 0 is not a reference to an array (PHP raises a TypeError; we no-op).
fn snapshot(arg: &Value) -> Option<Vec<(ArrayKey, Value)>> {
    if let Value::Ref(rc) = arg {
        if let Value::Array(a) = &*rc.borrow() {
            return Some(a.iter().map(|(k, v)| (k.clone(), deref(v).clone())).collect());
        }
    }
    None
}

/// Replace the referenced array with a freshly built one.
fn store(arg: &Value, new: PhpArray) {
    if let Value::Ref(rc) = arg {
        *rc.borrow_mut() = Value::Array(new);
    }
}

fn reindex(vals: Vec<Value>) -> PhpArray {
    let mut out = PhpArray::new();
    for v in vals {
        out.push(v);
    }
    out
}

fn from_pairs(pairs: Vec<(ArrayKey, Value)>) -> PhpArray {
    let mut out = PhpArray::new();
    for (k, v) in pairs {
        out.insert(k, v);
    }
    out
}

fn flags_arg(args: &[Value], i: usize) -> i64 {
    args.get(i).map(|v| conv::to_int(&deref(v))).unwrap_or(0)
}

// -- value sorts (reindex keys) -------------------------------------------

fn sort_values(args: &[Value], reverse: bool) -> Value {
    let Some(entries) = snapshot(&args[0]) else { return Value::Bool(false) };
    let flags = flags_arg(args, 1);
    let mut vals: Vec<Value> = entries.into_iter().map(|(_, v)| v).collect();
    vals.sort_by(|a, b| {
        let o = cmp_flagged(a, b, flags);
        if reverse { o.reverse() } else { o }
    });
    store(&args[0], reindex(vals));
    Value::Bool(true)
}

fn fn_sort(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(sort_values(args, false))
}

fn fn_rsort(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(sort_values(args, true))
}

// -- associative sorts (preserve keys) ------------------------------------

fn asort_impl(args: &[Value], reverse: bool, by_key: bool) -> Value {
    let Some(mut pairs) = snapshot(&args[0]) else { return Value::Bool(false) };
    let flags = flags_arg(args, 1);
    pairs.sort_by(|a, b| {
        let (x, y) = if by_key { (key_to_value(&a.0), key_to_value(&b.0)) } else { (a.1.clone(), b.1.clone()) };
        let o = cmp_flagged(&x, &y, flags);
        if reverse { o.reverse() } else { o }
    });
    store(&args[0], from_pairs(pairs));
    Value::Bool(true)
}

fn fn_asort(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(asort_impl(args, false, false))
}
fn fn_arsort(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(asort_impl(args, true, false))
}
fn fn_ksort(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(asort_impl(args, false, true))
}
fn fn_krsort(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(asort_impl(args, true, true))
}

// -- user-comparator sorts ------------------------------------------------

fn usort_impl(
    host: &mut dyn Host,
    args: &[Value],
    preserve_keys: bool,
    by_key: bool,
) -> Result<Value, Thrown> {
    let Some(pairs) = snapshot(&args[0]) else { return Ok(Value::Bool(false)) };
    let cmp = args[1].clone();
    let mut pairs = pairs;
    let mut err: Option<Thrown> = None;
    pairs.sort_by(|a, b| {
        if err.is_some() {
            return Ordering::Equal;
        }
        let (x, y) = if by_key {
            (key_to_value(&a.0), key_to_value(&b.0))
        } else {
            (a.1.clone(), b.1.clone())
        };
        match host.call_callable(&cmp, &[x, y]) {
            Ok(v) => conv::to_int(&v).cmp(&0),
            Err(e) => {
                err = Some(e);
                Ordering::Equal
            }
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    let built = if preserve_keys {
        from_pairs(pairs)
    } else {
        reindex(pairs.into_iter().map(|(_, v)| v).collect())
    };
    store(&args[0], built);
    Ok(Value::Bool(true))
}

fn fn_usort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    usort_impl(host, args, false, false)
}
fn fn_uasort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    usort_impl(host, args, true, false)
}
fn fn_uksort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    usort_impl(host, args, true, true)
}

// -- stack / queue --------------------------------------------------------

fn fn_array_push(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Ref(rc) = &args[0] else { return Ok(Value::Bool(false)) };
    let mut g = rc.borrow_mut();
    let Value::Array(a) = &mut *g else { return Ok(Value::Bool(false)) };
    for v in &args[1..] {
        a.push(deref(v).clone());
    }
    Ok(Value::Int(a.len() as i64))
}

fn fn_array_pop(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(pairs) = snapshot(&args[0]) else { return Ok(Value::Null) };
    if pairs.is_empty() {
        return Ok(Value::Null);
    }
    let mut pairs = pairs;
    let (_, removed) = pairs.pop().unwrap();
    // Surviving keys are preserved (no reindex); `from_pairs` recomputes the
    // next free integer index, matching PHP's post-pop append behaviour.
    store(&args[0], from_pairs(pairs));
    Ok(removed)
}

fn fn_array_shift(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(pairs) = snapshot(&args[0]) else { return Ok(Value::Null) };
    if pairs.is_empty() {
        return Ok(Value::Null);
    }
    let mut it = pairs.into_iter();
    let (_, removed) = it.next().unwrap();
    store(&args[0], rebuild_reindex_ints(it));
    Ok(removed)
}

fn fn_array_unshift(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(pairs) = snapshot(&args[0]) else { return Ok(Value::Bool(false)) };
    let mut out = PhpArray::new();
    for v in &args[1..] {
        out.push(deref(v).clone());
    }
    for (k, v) in pairs {
        match k {
            ArrayKey::Int(_) => out.push(v),
            s => out.insert(s, v),
        }
    }
    let len = out.len() as i64;
    store(&args[0], out);
    Ok(Value::Int(len))
}

/// Rebuild from `pairs`, renumbering integer keys from 0 while preserving
/// string keys (the reindexing `array_shift`/`array_unshift` perform).
fn rebuild_reindex_ints(pairs: impl Iterator<Item = (ArrayKey, Value)>) -> PhpArray {
    let mut out = PhpArray::new();
    for (k, v) in pairs {
        match k {
            ArrayKey::Int(_) => out.push(v),
            s => out.insert(s, v),
        }
    }
    out
}

pub fn register(m: &mut std::collections::HashMap<&'static str, Builtin>) {
    let add = |m: &mut std::collections::HashMap<&'static str, Builtin>,
               name: &'static str,
               func: crate::NativeFn,
               min: usize,
               max: Option<usize>| {
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    };
    add(m, "sort", fn_sort, 1, Some(2));
    add(m, "rsort", fn_rsort, 1, Some(2));
    add(m, "asort", fn_asort, 1, Some(2));
    add(m, "arsort", fn_arsort, 1, Some(2));
    add(m, "ksort", fn_ksort, 1, Some(2));
    add(m, "krsort", fn_krsort, 1, Some(2));
    add(m, "usort", fn_usort, 2, Some(2));
    add(m, "uasort", fn_uasort, 2, Some(2));
    add(m, "uksort", fn_uksort, 2, Some(2));
    add(m, "array_push", fn_array_push, 1, None);
    add(m, "array_pop", fn_array_pop, 1, Some(1));
    add(m, "array_shift", fn_array_shift, 1, Some(1));
    add(m, "array_unshift", fn_array_unshift, 1, None);
}
