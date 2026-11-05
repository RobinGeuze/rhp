//! ext/standard/array.c: the array functions, `count`, `min`/`max`,
//! `range`, the sort family and the internal-pointer functions.
//!
//! Key handling and var_dump/print_r output verified byte-for-byte against the
//! local php 8.5.8 binary (see `array/tests.rs` for the captured cases).

use std::cmp::Ordering;
use std::collections::HashSet;

use rhp_values::ops::deref;
use rhp_values::{conv, ops, ArrayKey, PhpArray, PhpStr, Value};

use crate::ext::random::mt_range;
use crate::zend::zpp::{array_param, int_param, zval_value_name};
use crate::{ext, host_compare, host_loose_eq, warn, zend, Host, Thrown, E_WARNING};

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// The "string-cast" key coercion used by `array_fill_keys` / `array_combine`:
/// integers stay integers; everything else is `(string)`-cast and then run
/// through the canonical-integer-string rule. Notably floats become string
/// keys (e.g. `2.9` -> `"2.9"`), unlike normal `$a[$f]` coercion.
fn key_strcast(v: &Value) -> ArrayKey {
    match deref(v) {
        Value::Int(n) => ArrayKey::Int(n),
        other => ArrayKey::from_bytes(&conv::to_string_bytes(&other)),
    }
}

/// The "int-or-string only" key coercion used by `array_flip` /
/// `array_count_values`: integers and strings become keys (strings via the
/// canonical rule); any other type is skipped (php emits a warning to stderr).
fn key_intstr(v: &Value) -> Option<ArrayKey> {
    match deref(v) {
        Value::Int(n) => Some(ArrayKey::Int(n)),
        Value::Str(s) => Some(ArrayKey::from_bytes(s.as_bytes())),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// array_slice
// ---------------------------------------------------------------------------

fn fn_array_slice(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_slice")?;
    let offset = conv::to_int(&deref(&args[1]));
    let length = match args.get(2) {
        None => None,
        Some(v) => match deref(v) {
            Value::Null => None,
            o => Some(conv::to_int(&o)),
        },
    };
    let preserve = args
        .get(3)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);

    let num = a.len() as i64;
    let mut start = offset;
    if start > num {
        start = num;
    } else if start < 0 {
        start += num;
        if start < 0 {
            start = 0;
        }
    }
    let cnt = match length {
        None => num - start,
        Some(l) if l < 0 => {
            let c = num - start + l;
            if c < 0 {
                0
            } else {
                c
            }
        }
        Some(l) => {
            if l > num - start {
                num - start
            } else {
                l
            }
        }
    };

    let entries: Vec<(ArrayKey, Value)> = a.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut out = PhpArray::new();
    let begin = start as usize;
    let end = (start + cnt) as usize;
    for (k, v) in &entries[begin..end] {
        match k {
            ArrayKey::Str(_) => out.insert(k.clone(), v.clone()),
            ArrayKey::Int(_) if preserve => out.insert(k.clone(), v.clone()),
            ArrayKey::Int(_) => out.push(v.clone()),
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_flip
// ---------------------------------------------------------------------------

fn fn_array_flip(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_flip")?;
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        if let Some(nk) = key_intstr(v) {
            out.insert(nk, k.to_value());
        }
        // non int/string values are skipped (php: warning to stderr)
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_fill
// ---------------------------------------------------------------------------

fn fn_array_fill(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let start = conv::to_int(&deref(&args[0]));
    let count = conv::to_int(&deref(&args[1]));
    if count < 0 {
        return Err(Thrown::error(
            "ValueError",
            "array_fill(): Argument #2 ($count) must be greater than or equal to 0",
        ));
    }
    if count > i32::MAX as i64 {
        return Err(Thrown::error(
            "ValueError",
            "array_fill(): Argument #2 ($count) is too large",
        ));
    }
    // php sizes the array up front, fallibly: packed when the keys start in
    // 0..count, else a hash.
    let reserved = if (0..count).contains(&start) {
        rhp_values::mem::reserve_packed_array((start + count) as usize)
    } else {
        count == 0 || rhp_values::mem::reserve_hash_array(count as usize)
    };
    if !reserved {
        return Ok(Value::Null);
    }
    let val = deref(&args[2]);
    let mut out = PhpArray::new();
    // PHP 8: keys are consecutive starting at `start` (incl. negative starts).
    for i in 0..count {
        out.insert(ArrayKey::Int(start + i), val.clone());
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_fill_keys
// ---------------------------------------------------------------------------

fn fn_array_fill_keys(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let keys = array_param(host, args, 0, "array_fill_keys")?;
    let val = deref(&args[1]);
    let mut out = PhpArray::new();
    for (_, kv) in keys.iter() {
        out.insert(key_strcast(kv), val.clone());
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_combine
// ---------------------------------------------------------------------------

fn fn_array_combine(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let keys = array_param(host, args, 0, "array_combine")?;
    let values = array_param(host, args, 1, "array_combine")?;
    if keys.len() != values.len() {
        return Err(Thrown::error(
            "ValueError",
            "array_combine(): Argument #1 ($keys) and argument #2 ($values) must have the same number of elements",
        ));
    }
    let mut out = PhpArray::new();
    for ((_, kv), (_, vv)) in keys.iter().zip(values.iter()) {
        out.insert(key_strcast(kv), vv.clone());
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_search
// ---------------------------------------------------------------------------

fn fn_array_search(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let needle = deref(&args[0]);
    let a = array_param(host, args, 1, "array_search")?;
    let strict = args
        .get(2)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    for (k, v) in a.iter() {
        let hit = if strict {
            ops::strict_eq(&needle, v)
        } else {
            crate::host_loose_eq(host, &needle, v)?
        };
        if hit {
            return Ok(k.to_value());
        }
    }
    Ok(Value::Bool(false))
}

// ---------------------------------------------------------------------------
// array_unique (SORT_STRING semantics — the default)
// ---------------------------------------------------------------------------

fn fn_array_unique(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_unique")?;
    let flags = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(2);
    if flags != 2 && a.len() > 1 {
        // SORT_REGULAR / SORT_NUMERIC / SORT_LOCALE_STRING: php sorts and drops
        // neighbours that compare equal, keeping the earliest; for a consistent
        // comparator that is "first occurrence of each equal group".
        let mut kept: Vec<(ArrayKey, Value)> = Vec::new();
        for (k, v) in a.iter() {
            let v = deref(v);
            let mut dup = false;
            for (_, kv) in &kept {
                let eq = match flags & !8 {
                    1 => conv::to_float(kv) == conv::to_float(&v),
                    5 => conv::to_string_bytes(kv) == conv::to_string_bytes(&v),
                    _ => crate::host_loose_eq(host, kv, &v)?,
                };
                if eq {
                    dup = true;
                    break;
                }
            }
            if !dup {
                kept.push((k.clone(), v));
            }
        }
        let mut out = PhpArray::new();
        for (k, v) in kept {
            out.insert(k, v);
        }
        return Ok(Value::Array(out));
    }
    // SORT_STRING (the default): values are compared by their string form.
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let s = conv::to_string_bytes(v);
        if seen.insert(s) {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_pad
// ---------------------------------------------------------------------------

const PAD_MAX_SIZE: i128 = 1073741824; // 0x40000000

fn fn_array_pad(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_pad")?;
    let size = conv::to_int(&deref(&args[1]));
    let val = deref(&args[2]);

    let abs = (size as i128).abs();
    if abs > PAD_MAX_SIZE {
        return Err(Thrown::error(
            "ValueError",
            "array_pad(): Argument #2 ($length) must not exceed the maximum allowed array size",
        ));
    }
    let n = a.len();
    let abs = abs as usize;
    if abs <= n {
        // No padding needed: original is returned with keys preserved.
        return Ok(Value::Array(a));
    }
    let pad = abs - n;
    // php sizes the result up front, fallibly: packed when the first key
    // it inserts is an int (a list input, or pads going first).
    let packed = size < 0
        || a.iter()
            .enumerate()
            .all(|(i, (k, _))| matches!(k, ArrayKey::Int(n) if *n == i as i64));
    let reserved = if packed {
        rhp_values::mem::reserve_packed_array(abs)
    } else {
        rhp_values::mem::reserve_hash_array(abs)
    };
    if !reserved {
        return Ok(Value::Null);
    }
    let mut out = PhpArray::new();
    let append_original = |out: &mut PhpArray| {
        for (k, v) in a.iter() {
            match k {
                ArrayKey::Str(_) => out.insert(k.clone(), v.clone()),
                ArrayKey::Int(_) => out.push(v.clone()),
            }
        }
    };
    if size < 0 {
        for _ in 0..pad {
            out.push(val.clone());
        }
        append_original(&mut out);
    } else {
        append_original(&mut out);
        for _ in 0..pad {
            out.push(val.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_key_first / array_key_last
// ---------------------------------------------------------------------------

fn fn_array_key_first(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_key_first")?;
    Ok(a.iter()
        .next()
        .map(|(k, _)| k.to_value())
        .unwrap_or(Value::Null))
}

fn fn_array_key_last(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_key_last")?;
    Ok(a.iter()
        .last()
        .map(|(k, _)| k.to_value())
        .unwrap_or(Value::Null))
}

// ---------------------------------------------------------------------------
// array_is_list
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// array_diff / array_intersect (string comparison)
// ---------------------------------------------------------------------------

fn fn_array_diff(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = array_param(host, args, 0, "array_diff")?;
    // Union of the string forms of every subsequent array's values.
    let mut others: HashSet<Vec<u8>> = HashSet::new();
    for i in 1..args.len() {
        let o = array_param(host, args, i, "array_diff")?;
        for (_, v) in o.iter() {
            others.insert(conv::to_string_bytes(v));
        }
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        if !others.contains(&conv::to_string_bytes(v)) {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_intersect(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = array_param(host, args, 0, "array_intersect")?;
    let mut sets: Vec<HashSet<Vec<u8>>> = Vec::new();
    for i in 1..args.len() {
        let o = array_param(host, args, i, "array_intersect")?;
        sets.push(o.iter().map(|(_, v)| conv::to_string_bytes(v)).collect());
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        let s = conv::to_string_bytes(v);
        if sets.iter().all(|set| set.contains(&s)) {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_column
// ---------------------------------------------------------------------------

/// array_column_fetch_prop for an object row: the property if it exists
/// as seen from the caller (has_property's "exists" mode), else what
/// `__isset` admits (its "isset" mode), read through `__get`.
fn column_prop(host: &mut dyn Host, row: &Value, key: &Value) -> Result<Option<Value>, Thrown> {
    let name = conv::to_string_bytes(key);
    if host.object_prop_exists(row, &name) {
        return Ok(host.object_prop(row, &name).map(|v| deref(&v)));
    }
    let Value::Object(h) = row else {
        return Ok(None);
    };
    let cls = h.borrow().class;
    if !host.class_has_method(cls, b"__isset") {
        return Ok(None);
    }
    let call = |host: &mut dyn Host, m: &str| {
        let mut cb = PhpArray::new();
        cb.push(row.clone());
        cb.push(Value::str(m));
        host.call_callable(&Value::Array(cb), &[Value::str(name.clone())])
    };
    if !conv::to_bool(&call(host, "__isset")?) {
        return Ok(None);
    }
    if !host.class_has_method(cls, b"__get") {
        return Ok(Some(Value::Null));
    }
    Ok(Some(call(host, "__get")?))
}

fn fn_array_column(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let rows = array_param(host, args, 0, "array_column")?;
    let column_key = deref(&args[1]);
    let column_null = matches!(column_key, Value::Null);
    let column_ak = ArrayKey::from_value(&column_key);
    // index_key: None if omitted or explicitly null; otherwise its normal key.
    let index_ak: Option<Option<ArrayKey>> = match args.get(2).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(ArrayKey::from_value(&v)),
    };

    let mut out = PhpArray::new();
    for (_, row) in rows.iter() {
        let row = deref(row);

        // Extracted value.
        let value = if column_null {
            row.clone()
        } else if let Value::Object(_) = row {
            match column_prop(host, &row, &column_key)? {
                Some(v) => v,
                None => continue,
            }
        } else {
            let Value::Array(ref ra) = row else { continue };
            match column_ak.as_ref().and_then(|k| ra.get(k)) {
                Some(v) => v.clone(),
                None => continue,
            }
        };

        // Index key, if requested and present in this (array) row.
        let idx = match &index_ak {
            None => None,
            Some(ak) => match &row {
                Value::Array(ra) => ak.as_ref().and_then(|k| ra.get(k)).cloned(),
                Value::Object(_) => {
                    let key = deref(&args[2]);
                    column_prop(host, &row, &key)?
                }
                _ => None,
            },
        };

        match idx {
            Some(iv) => match ArrayKey::from_value(&iv) {
                Some(k) => out.insert(k, value),
                None => out.push(value),
            },
            None => out.push(value),
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_chunk
// ---------------------------------------------------------------------------

fn fn_array_chunk(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_chunk")?;
    let size = conv::to_int(&deref(&args[1]));
    if size < 1 {
        return Err(Thrown::error(
            "ValueError",
            "array_chunk(): Argument #2 ($length) must be greater than 0",
        ));
    }
    let preserve = args
        .get(2)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    let size = size as usize;

    let mut out = PhpArray::new();
    let mut chunk = PhpArray::new();
    let mut i = 0usize;
    for (k, v) in a.iter() {
        if preserve {
            chunk.insert(k.clone(), v.clone());
        } else {
            chunk.push(v.clone());
        }
        i += 1;
        if i == size {
            out.push(Value::Array(std::mem::take(&mut chunk)));
            i = 0;
        }
    }
    if !chunk.is_empty() {
        out.push(Value::Array(chunk));
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_count_values
// ---------------------------------------------------------------------------

fn fn_array_count_values(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_count_values")?;
    let mut out = PhpArray::new();
    for (_, v) in a.iter() {
        let Some(k) = key_intstr(v) else { continue };
        let cur = out.get(&k).map(conv::to_int).unwrap_or(0);
        out.insert(k, Value::Int(cur + 1));
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// php_count_recursive: an array met again through a reference warns
/// "Recursion detected" and counts 0.
fn count_recursive(
    host: &mut dyn Host,
    fname: &str,
    a: &rhp_values::PhpArray,
    seen: &mut Vec<usize>,
) -> Result<i64, Thrown> {
    if seen.contains(&a.data_ptr()) {
        host.emit_diagnostic(
            E_WARNING,
            format!("{fname}(): Recursion detected").as_bytes(),
        )?;
        return Ok(0);
    }
    seen.push(a.data_ptr());
    let mut n = a.len() as i64;
    for (_, v) in a.iter() {
        if let Value::Array(sub) = deref(v) {
            n += count_recursive(host, fname, &sub, seen)?;
        }
    }
    seen.pop();
    Ok(n)
}

fn fn_count(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    count_impl(host, args, "count")
}

/// sizeof() is count() under its own name, which its errors carry.
fn fn_sizeof(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    count_impl(host, args, "sizeof")
}

fn count_impl(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let mode = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if mode != 0 && mode != 1 {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #2 ($mode) must be either COUNT_NORMAL or COUNT_RECURSIVE"
            ),
        ));
    }
    match deref(&args[0]) {
        Value::Array(a) if mode == 1 => Ok(Value::Int(count_recursive(
            host,
            fname,
            &a,
            &mut Vec::new(),
        )?)),
        Value::Array(a) => Ok(Value::Int(a.len() as i64)),
        // A Countable object: count($obj) calls $obj->count().
        obj @ Value::Object(_) => {
            let callable = Value::Array({
                let mut a = rhp_values::PhpArray::new();
                a.push(obj.clone());
                a.push(Value::str("count"));
                a
            });
            if host.is_callable_value(&callable) {
                let r = host.call_callable(&callable, &[])?;
                return Ok(Value::Int(conv::to_int(&deref(&r))));
            }
            Err(Thrown::type_error(format!(
                "{fname}(): Argument #1 ($value) must be of type Countable|array, {} given",
                zend::zpp::zval_value_name(host, &obj)
            )))
        }
        other => Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($value) must be of type Countable|array, {} given",
            zend::zpp::zval_value_name(host, &other)
        ))),
    }
}

fn fn_max(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    extremum(host, args, true)
}
fn fn_min(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    extremum(host, args, false)
}

/// `(double)l` converts back to `l` exactly (`zend_dval_to_lval_silent`).
fn long_is_exact_double(l: i64) -> bool {
    let d = l as f64;
    (-(2f64.powi(63))..2f64.powi(63)).contains(&d) && d as i64 == l
}

/// php's `min`/`max`: the array form (`zend_hash_minmax`), the two-argument
/// frameless form, and the variadic loop with its long/double fast paths.
fn extremum(host: &mut dyn Host, args: &[Value], want_max: bool) -> Result<Value, Thrown> {
    use std::cmp::Ordering;
    let name = if want_max { "max" } else { "min" };
    let args: Vec<Value> = args.iter().map(deref).collect();
    if args.len() == 1 {
        let Value::Array(a) = &args[0] else {
            return Err(Thrown::type_error(format!(
                "{name}(): Argument #1 ($value) must be of type array, {} given",
                zend::zpp::zval_value_name(host, &args[0])
            )));
        };
        let mut it = a.iter().map(|(_, v)| deref(v));
        let Some(mut res) = it.next() else {
            return Err(Thrown::error(
                "ValueError",
                format!("{name}(): Argument #1 ($value) must contain at least one element"),
            ));
        };
        for zv in it {
            let c = host_compare(host, &res, &zv)?;
            if (want_max && c == Ordering::Less) || (!want_max && c == Ordering::Greater) {
                res = zv;
            }
        }
        return Ok(res);
    }
    if args.len() == 2 {
        // ZEND_FRAMELESS_FUNCTION(min|max, 2)
        let (lhs, rhs) = (&args[0], &args[1]);
        let pick = |keep_lhs: bool| if keep_lhs { lhs.clone() } else { rhs.clone() };
        let dbl = |l: f64, r: f64| if want_max { l >= r } else { l < r };
        return Ok(match (lhs, rhs) {
            (Value::Int(l), Value::Int(r)) => pick(if want_max { l >= r } else { l < r }),
            (Value::Int(l), Value::Float(r)) if long_is_exact_double(*l) => {
                pick(dbl(*l as f64, *r))
            }
            (Value::Float(l), Value::Float(r)) => pick(dbl(*l, *r)),
            (Value::Float(l), Value::Int(r)) if long_is_exact_double(*r) => {
                pick(dbl(*l, *r as f64))
            }
            _ => {
                let c = host_compare(host, lhs, rhs)?;
                pick(if want_max {
                    c != Ordering::Less
                } else {
                    c == Ordering::Less
                })
            }
        });
    }
    #[derive(PartialEq)]
    enum Mode {
        Long(i64),
        Double(f64),
        Generic,
    }
    let mut best = 0usize;
    let mut mode = match &args[0] {
        Value::Int(l) => Mode::Long(*l),
        Value::Float(d) => Mode::Double(*d),
        _ => Mode::Generic,
    };
    let better_d = |cur: f64, v: f64| if want_max { cur < v } else { cur > v };
    for i in 1..args.len() {
        if let Mode::Long(cur) = mode {
            match &args[i] {
                Value::Int(v) => {
                    if (want_max && cur < *v) || (!want_max && cur > *v) {
                        mode = Mode::Long(*v);
                        best = i;
                    }
                    continue;
                }
                Value::Float(_) if long_is_exact_double(cur) => mode = Mode::Double(cur as f64),
                _ => mode = Mode::Generic,
            }
        }
        if let Mode::Double(cur) = mode {
            match &args[i] {
                Value::Float(v) => {
                    if better_d(cur, *v) {
                        mode = Mode::Double(*v);
                        best = i;
                    }
                    continue;
                }
                Value::Int(v) if long_is_exact_double(*v) => {
                    if better_d(cur, *v as f64) {
                        mode = Mode::Double(*v as f64);
                        best = i;
                    }
                    continue;
                }
                _ => mode = Mode::Generic,
            }
        }
        let c = host_compare(host, &args[i], &args[best])?;
        if (want_max && c == Ordering::Greater) || (!want_max && c == Ordering::Less) {
            best = i;
        }
    }
    Ok(args[best].clone())
}

fn fn_array_sum(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    array_binop(host, args, "array_sum", "Addition", ops::add, 0)
}

fn fn_array_product(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    array_binop(host, args, "array_product", "Multiplication", ops::mul, 1)
}

/// php_array_binop: `op` over the entries, objects by their
/// `cast_object(_IS_NUMBER)`. An entry the operator refuses is skipped
/// with a warning (a resource counts as its id, a non-numeric string as 0).
fn array_binop(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    op_name: &str,
    op: fn(&Value, &Value) -> Result<Value, Thrown>,
    initial: i64,
) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($array) must be of type array"
        )));
    };
    let mut acc = Value::Int(initial);
    for (_, v) in a.iter() {
        let v = deref(v);
        let operand = match &v {
            Value::Object(_) | Value::Closure(_) => {
                match zend::object_handlers::cast(
                    host,
                    &v,
                    zend::object_handlers::CastTarget::Number,
                )? {
                    Some(n @ (Value::Int(_) | Value::Float(_))) => Some(n),
                    _ => None,
                }
            }
            Value::Array(_) => None,
            Value::Resource(r) => {
                acc = op(&acc, &Value::Int(r.id as i64))?;
                None
            }
            Value::Str(s) => match conv::scan_number(s.as_bytes()) {
                None => {
                    acc = op(&acc, &Value::Int(0))?;
                    None
                }
                Some((num, _)) => {
                    if conv::numeric_string(s.as_bytes()).is_none() {
                        host.emit_diagnostic(E_WARNING, b"A non-numeric value encountered")?;
                    }
                    Some(num.to_value())
                }
            },
            _ => Some(v.clone()),
        };
        match operand {
            Some(n) => acc = op(&acc, &n)?,
            None => {
                let ty = zend::zpp::zval_type_name(host, &v);
                warn(
                    host,
                    fname,
                    &format!("{op_name} is not supported on type {ty}"),
                )?
            }
        }
    }
    Ok(acc)
}

fn fn_array_keys(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error(
            "array_keys(): Argument #1 ($array) must be of type array",
        ));
    };
    // Optional search value: keep only keys whose value matches (loose by
    // default, strict `===` when the third argument is true).
    let search = args.get(1).map(deref);
    let strict = args
        .get(2)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let keep = match &search {
            None => true,
            Some(needle) => {
                if strict {
                    ops::strict_eq(&deref(v), needle)
                } else {
                    host_loose_eq(host, &deref(v), needle)?
                }
            }
        };
        if keep {
            out.push(k.to_value());
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_values(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error(
            "array_values(): Argument #1 ($array) must be of type array",
        ));
    };
    let mut out = PhpArray::new();
    for (_, v) in a.iter() {
        out.push(v.clone());
    }
    Ok(Value::Array(out))
}

fn fn_array_merge(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = PhpArray::new();
    for arg in args {
        let Value::Array(a) = deref(arg) else {
            return Err(Thrown::type_error(
                "array_merge(): Arguments must be of type array",
            ));
        };
        for (k, v) in a.iter() {
            match k {
                // integer keys are renumbered; string keys overwrite
                rhp_values::ArrayKey::Int(_) => out.push(v.clone()),
                rhp_values::ArrayKey::Str(_) => out.insert(k.clone(), v.clone()),
            }
        }
    }
    Ok(Value::Array(out))
}

pub(crate) fn as_array(v: &Value, who: &str) -> Result<PhpArray, Thrown> {
    match deref(v) {
        Value::Array(a) => Ok(a),
        _ => Err(Thrown::type_error(format!(
            "{who}(): argument must be of type array"
        ))),
    }
}

fn fn_array_map(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cb = &args[0];
    let cb_null = matches!(deref(cb), Value::Null);
    let arrays: Vec<PhpArray> = args[1..]
        .iter()
        .map(|a| as_array(a, "array_map"))
        .collect::<Result<_, _>>()?;
    if arrays.len() == 1 {
        // single array: preserve keys
        let mut out = PhpArray::new();
        for (k, v) in arrays[0].iter() {
            let r = if cb_null {
                v.clone()
            } else {
                host.call_callable(cb, std::slice::from_ref(v))?
            };
            out.insert(k.clone(), r);
        }
        return Ok(Value::Array(out));
    }
    // multiple arrays: zip by position, reindex
    let max_len = arrays.iter().map(|a| a.len()).max().unwrap_or(0);
    let mut out = PhpArray::new();
    for i in 0..max_len {
        let tuple: Vec<Value> = arrays
            .iter()
            .map(|a| {
                a.get_index(i)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null)
            })
            .collect();
        if cb_null {
            let mut t = PhpArray::new();
            for v in tuple {
                t.push(v);
            }
            out.push(Value::Array(t));
        } else {
            out.push(host.call_callable(cb, &tuple)?);
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_filter(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = as_array(&args[0], "array_filter")?;
    let cb = args.get(1).filter(|c| !matches!(deref(c), Value::Null));
    let mode = args.get(2).map(conv::to_int).unwrap_or(0);
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let keep = match cb {
            None => zend::object_handlers::is_true(host, v)?,
            Some(cb) => {
                let call_args: Vec<Value> = match mode {
                    2 => vec![k.to_value()],            // ARRAY_FILTER_USE_KEY
                    1 => vec![v.clone(), k.to_value()], // ARRAY_FILTER_USE_BOTH
                    _ => vec![v.clone()],
                };
                let r = host.call_callable(cb, &call_args)?;
                zend::object_handlers::is_true(host, &r)?
            }
        };
        if keep {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_reduce(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = as_array(&args[0], "array_reduce")?;
    let cb = &args[1];
    let mut acc = args.get(2).map(deref).unwrap_or(Value::Null);
    for (_, v) in a.iter() {
        acc = host.call_callable(cb, &[acc, v.clone()])?;
    }
    Ok(acc)
}

fn fn_array_reverse(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error(
            "array_reverse(): Argument #1 ($array) must be of type array",
        ));
    };
    let preserve = args.get(1).map(conv::to_bool).unwrap_or(false);
    let mut out = PhpArray::new();
    let entries: Vec<_> = a.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (k, v) in entries.into_iter().rev() {
        match k {
            rhp_values::ArrayKey::Int(_) if !preserve => out.push(v),
            _ => out.insert(k, v),
        }
    }
    Ok(Value::Array(out))
}

/// What php_range_process_input made of a range() bound, ordered as php's
/// type codes (IS_LONG < IS_DOUBLE < IS_STRING < IS_ARRAY).
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum RangeIn {
    Long,
    Double,
    /// A non-numeric string: its first byte.
    Char,
    /// A one-digit string: a char or an int, whichever the other bound is.
    Digit,
}

fn range_value_error(n: usize, name: &str, msg: &str) -> Thrown {
    Thrown::error(
        "ValueError",
        format!("range(): Argument #{n} (${name}) {msg}"),
    )
}

/// php_range_process_input: the bound's kind with its int and float values.
fn range_input(
    host: &mut dyn Host,
    v: &Value,
    n: usize,
    name: &str,
) -> Result<(RangeIn, i64, f64), Thrown> {
    let finite = |d: f64| {
        if d.is_infinite() {
            Err(range_value_error(
                n,
                name,
                "must be a finite number, INF provided",
            ))
        } else if d.is_nan() {
            Err(range_value_error(
                n,
                name,
                "must be a finite number, NAN provided",
            ))
        } else {
            Ok((RangeIn::Double, 0, d))
        }
    };
    let s = match v {
        Value::Str(s) => s.as_bytes(),
        Value::Float(d) => return finite(*d),
        // The VM's zpp leaves an int or a float, or a string.
        other => {
            let l = conv::to_int(other);
            return Ok((RangeIn::Long, l, l as f64));
        }
    };
    if s.is_empty() {
        let msg = format!("Argument #{n} (${name}) must not be empty, casted to 0");
        host.emit_diagnostic(E_WARNING, format!("range(): {msg}").as_bytes())?;
        return Ok((RangeIn::Long, 0, 0.0));
    }
    match conv::numeric_string(s) {
        Some(conv::Num::Float(d)) => finite(d),
        Some(conv::Num::Int(l)) if s.len() == 1 => Ok((RangeIn::Digit, l, l as f64)),
        Some(conv::Num::Int(l)) => Ok((RangeIn::Long, l, l as f64)),
        None => {
            if s.len() != 1 {
                let msg = format!(
                    "Argument #{n} (${name}) must be a single byte, subsequent bytes are ignored"
                );
                host.emit_diagnostic(E_WARNING, format!("range(): {msg}").as_bytes())?;
            }
            Ok((RangeIn::Char, 0, 0.0))
        }
    }
}

const HT_MAX_SIZE: u64 = 0x4000_0000;

/// RANGE_CHECK_DOUBLE_INIT_ARRAY for `hi > lo`: the element count, or the
/// ValueError for a range past HT_MAX_SIZE.
fn range_size_double(hi: f64, lo: f64, step: f64) -> Result<usize, Thrown> {
    let size = (hi - lo) / step + 1.0;
    if size >= HT_MAX_SIZE as f64 {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "The supplied range exceeds the maximum array size by {:.1} elements: start={lo:.1}, end={hi:.1}, step={step:.1}. Max size: {HT_MAX_SIZE}",
                size - HT_MAX_SIZE as f64
            ),
        ));
    }
    Ok(ext::standard::math::php_math_round(size, 0, 1) as usize)
}

/// RANGE_CHECK_LONG_INIT_ARRAY for `hi > lo`.
fn range_size_long(hi: i64, lo: i64, step: u64) -> Result<usize, Thrown> {
    let size = (hi as u64).wrapping_sub(lo as u64) / step;
    if size >= HT_MAX_SIZE - 1 {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "The supplied range exceeds the maximum array size by {} elements: start={lo}, end={hi}, step={}. Calculated size: {size}. Maximum size: {HT_MAX_SIZE}.",
                size - (HT_MAX_SIZE - 1),
                step as i64
            ),
        ));
    }
    Ok(size as usize + 1)
}

/// php's range(): an int, float or character sequence from `$start` to
/// `$end` by `abs($step)`, with php_range's argument checks.
fn fn_range(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let boundary_error = || {
        range_value_error(
            3,
            "step",
            "must be less than the range spanned by argument #1 ($start) and argument #2 ($end)",
        )
    };
    let negative_step_error =
        || range_value_error(3, "step", "must be greater than 0 for increasing ranges");
    let mut is_step_double = false;
    let mut is_step_negative = false;
    let mut step_double = 1.0;
    let mut step: i64 = 1;
    if let Some(user_step) = args.get(2).map(deref) {
        if let Value::Float(d) = user_step {
            if d.is_infinite() {
                return Err(range_value_error(
                    3,
                    "step",
                    "must be a finite number, INF provided",
                ));
            }
            if d.is_nan() {
                return Err(range_value_error(
                    3,
                    "step",
                    "must be a finite number, NAN provided",
                ));
            }
            step_double = d;
            if step_double < 0.0 {
                is_step_negative = true;
                step_double = -step_double;
            }
            // zend_dval_to_lval_silent, then zend_is_long_compatible
            step = rhp_values::float_to_int(step_double);
            is_step_double = step as f64 != step_double;
        } else {
            step = conv::to_int(&user_step);
            if step < 0 {
                if step == i64::MIN {
                    return Err(range_value_error(
                        3,
                        "step",
                        &format!("must be greater than {step}"),
                    ));
                }
                is_step_negative = true;
                step = -step;
            }
            step_double = step as f64;
        }
        if step_double == 0.0 {
            return Err(range_value_error(3, "step", "cannot be 0"));
        }
    }
    let start = deref(&args[0]);
    let end = deref(&args[1]);
    let (mut start_type, start_long, start_double) = range_input(host, &start, 1, "start")?;
    let (mut end_type, end_long, end_double) = range_input(host, &end, 2, "end")?;
    let mut out = PhpArray::new();

    if start_type >= RangeIn::Char || end_type >= RangeIn::Char {
        if start_type < RangeIn::Char || end_type < RangeIn::Char {
            if start_type < RangeIn::Char {
                if end_type != RangeIn::Digit {
                    host.emit_diagnostic(
                        E_WARNING,
                        b"range(): Argument #1 ($start) must be a single byte string if argument #2 ($end) is a single byte string, argument #2 ($end) converted to 0",
                    )?;
                }
                end_type = RangeIn::Long;
            } else {
                if start_type != RangeIn::Digit {
                    host.emit_diagnostic(
                        E_WARNING,
                        b"range(): Argument #2 ($end) must be a single byte string if argument #1 ($start) is a single byte string, argument #1 ($start) converted to 0",
                    )?;
                }
                start_type = RangeIn::Long;
            }
        } else if is_step_double {
            if start_type == RangeIn::Char || end_type == RangeIn::Char {
                host.emit_diagnostic(
                    E_WARNING,
                    b"range(): Argument #3 ($step) must be of type int when generating an array of characters, inputs converted to 0",
                )?;
            }
            start_type = RangeIn::Long;
            end_type = RangeIn::Long;
        } else {
            // Both bounds are strings: a range of characters.
            let first = |v: &Value| match v {
                Value::Str(s) => s.as_bytes().first().copied().unwrap_or(0) as i64,
                _ => 0,
            };
            let (mut low, high) = (first(&start), first(&end));
            let char_value = |c: i64| Value::str(vec![c as u8]);
            if low > high {
                if low - high < step {
                    return Err(boundary_error());
                }
                while low >= high {
                    out.push(char_value(low));
                    if low - step < 0 {
                        break;
                    }
                    low -= step;
                }
            } else if high > low {
                if is_step_negative {
                    return Err(negative_step_error());
                }
                if high - low < step {
                    return Err(boundary_error());
                }
                while low <= high {
                    out.push(char_value(low));
                    if low + step > 255 {
                        break;
                    }
                    low += step;
                }
            } else {
                out.push(char_value(low));
            }
            return Ok(Value::Array(out));
        }
    }

    // php sizes the result up front, fallibly.
    let reserve = |n: usize| rhp_values::mem::reserve_packed_array(n);
    if start_type == RangeIn::Double || end_type == RangeIn::Double || is_step_double {
        let (s, e) = (start_double, end_double);
        if s > e {
            if s - e < step_double {
                return Err(boundary_error());
            }
            let size = range_size_double(s, e, step_double)?;
            if !reserve(size) {
                return Ok(Value::Null);
            }
            let mut el = s;
            let mut i = 0;
            while i < size && el >= e {
                out.push(Value::Float(el));
                i += 1;
                el = s - i as f64 * step_double;
            }
        } else if e > s {
            if is_step_negative {
                return Err(negative_step_error());
            }
            if e - s < step_double {
                return Err(boundary_error());
            }
            let size = range_size_double(e, s, step_double)?;
            if !reserve(size) {
                return Ok(Value::Null);
            }
            let mut el = s;
            let mut i = 0;
            while i < size && el <= e {
                out.push(Value::Float(el));
                i += 1;
                el = s + i as f64 * step_double;
            }
        } else {
            out.push(Value::Float(s));
        }
    } else {
        let (s, e) = (start_long, end_long);
        let ustep = step as u64;
        if s > e {
            if (s as u64).wrapping_sub(e as u64) < ustep {
                return Err(boundary_error());
            }
            let size = range_size_long(s, e, ustep)?;
            if !reserve(size) {
                return Ok(Value::Null);
            }
            for i in 0..size as u64 {
                out.push(Value::Int(s.wrapping_sub(i.wrapping_mul(ustep) as i64)));
            }
        } else if e > s {
            if is_step_negative {
                return Err(negative_step_error());
            }
            if (e as u64).wrapping_sub(s as u64) < ustep {
                return Err(boundary_error());
            }
            let size = range_size_long(e, s, ustep)?;
            if !reserve(size) {
                return Ok(Value::Null);
            }
            for i in 0..size as u64 {
                out.push(Value::Int(s.wrapping_add(i.wrapping_mul(ustep) as i64)));
            }
        } else {
            out.push(Value::Int(s));
        }
    }
    Ok(Value::Array(out))
}

fn fn_in_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let needle = deref(&args[0]);
    let Value::Array(a) = deref(&args[1]) else {
        return Err(Thrown::type_error(
            "in_array(): Argument #2 ($haystack) must be of type array",
        ));
    };
    let strict = args.get(2).map(conv::to_bool).unwrap_or(false);
    for (_, v) in a.iter() {
        let hit = if strict {
            ops::strict_eq(&needle, v)
        } else {
            host_loose_eq(host, &needle, v)?
        };
        if hit {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn fn_array_key_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[1]) else {
        return Err(Thrown::type_error(
            "array_key_exists(): Argument #2 ($array) must be of type array",
        ));
    };
    let k = deref(&args[0]);
    match &k {
        Value::Null => host.emit_diagnostic(
            crate::E_DEPRECATED,
            b"Using null as the key parameter for array_key_exists() is deprecated, use an empty string instead",
        )?,
        Value::Float(f) => {
            crate::zend::operators::dval_to_lval_safe(host, *f)?;
        }
        Value::Resource(r) => host.emit_diagnostic(
            E_WARNING,
            format!(
                "Resource ID#{} used as offset, casting to integer ({})",
                r.id, r.id
            )
            .as_bytes(),
        )?,
        _ => {}
    }
    let Some(key) = rhp_values::ArrayKey::from_value(&k) else {
        return Err(Thrown::type_error(
            "array_key_exists(): Argument #1 ($key) must be a valid array offset type",
        ));
    };
    Ok(Value::Bool(a.get(&key).is_some()))
}

// The user-comparator members of the `array_(u)diff` / `array_(u)intersect`
// family: `array_udiff`, `array_uintersect`, `array_diff_ukey`,
// `array_intersect_ukey`, `array_diff_uassoc`, `array_intersect_uassoc`,
// `array_udiff_uassoc`, `array_uintersect_uassoc` (php's `php_array_diff` /
// `php_array_intersect`: sort every array with the comparator, then merge),
// and `array_udiff_assoc`, `array_uintersect_assoc` (php's
// `php_array_diff_key` / `php_array_intersect_key`: hash lookup by key, then
// the user value comparator).
//
// The sort-and-merge algorithm is ported step for step (including
// `zend_sort`), because both the result for inconsistent comparators and the
// sequence of callback invocations are observable.

// ---------------------------------------------------------------------------
// zend_sort (Zend/zend_sort.c): hybrid insertion sort / quicksort, derived
// from LLVM's libc++ std::sort (NCSA OR MIT, Copyright (c) 2009-2012 by the
// libc++ contributors); see THIRD-PARTY-NOTICES, section 22.
// ---------------------------------------------------------------------------

type CmpFn<'a, T> = dyn FnMut(&T, &T) -> i32 + 'a;

fn sort2<T>(a: &mut [T], x: usize, y: usize, cmp: &mut CmpFn<T>) {
    if cmp(&a[x], &a[y]) > 0 {
        a.swap(x, y);
    }
}

fn sort3<T>(a: &mut [T], x: usize, y: usize, z: usize, cmp: &mut CmpFn<T>) {
    if cmp(&a[x], &a[y]) <= 0 {
        if cmp(&a[y], &a[z]) <= 0 {
            return;
        }
        a.swap(y, z);
        if cmp(&a[x], &a[y]) > 0 {
            a.swap(x, y);
        }
        return;
    }
    if cmp(&a[z], &a[y]) <= 0 {
        a.swap(x, z);
        return;
    }
    a.swap(x, y);
    if cmp(&a[y], &a[z]) > 0 {
        a.swap(y, z);
    }
}

fn sort4<T>(a: &mut [T], p: [usize; 4], cmp: &mut CmpFn<T>) {
    sort3(a, p[0], p[1], p[2], cmp);
    if cmp(&a[p[2]], &a[p[3]]) > 0 {
        a.swap(p[2], p[3]);
        if cmp(&a[p[1]], &a[p[2]]) > 0 {
            a.swap(p[1], p[2]);
            if cmp(&a[p[0]], &a[p[1]]) > 0 {
                a.swap(p[0], p[1]);
            }
        }
    }
}

fn sort5<T>(a: &mut [T], p: [usize; 5], cmp: &mut CmpFn<T>) {
    sort4(a, [p[0], p[1], p[2], p[3]], cmp);
    if cmp(&a[p[3]], &a[p[4]]) > 0 {
        a.swap(p[3], p[4]);
        if cmp(&a[p[2]], &a[p[3]]) > 0 {
            a.swap(p[2], p[3]);
            if cmp(&a[p[1]], &a[p[2]]) > 0 {
                a.swap(p[1], p[2]);
                if cmp(&a[p[0]], &a[p[1]]) > 0 {
                    a.swap(p[0], p[1]);
                }
            }
        }
    }
}

/// Move the element at `i` down to position `j` (`j <= i`), shifting the
/// ones in between up — the `swp(k, k - 1)` loop of `zend_insert_sort`.
fn shift_down<T>(a: &mut [T], j: usize, i: usize) {
    let mut k = i;
    while k > j {
        a.swap(k, k - 1);
        k -= 1;
    }
}

/// `zend_insert_sort` over `a[lo..lo + n]`.
fn insert_sort<T>(a: &mut [T], lo: usize, n: usize, cmp: &mut CmpFn<T>) {
    match n {
        0 | 1 => {}
        2 => sort2(a, lo, lo + 1, cmp),
        3 => sort3(a, lo, lo + 1, lo + 2, cmp),
        4 => sort4(a, [lo, lo + 1, lo + 2, lo + 3], cmp),
        5 => sort5(a, [lo, lo + 1, lo + 2, lo + 3, lo + 4], cmp),
        _ => {
            let start = lo;
            let end = lo + n;
            let sentry = start + 6;
            for i in start + 1..sentry {
                let mut j = i - 1;
                if cmp(&a[j], &a[i]) <= 0 {
                    continue;
                }
                while j != start {
                    j -= 1;
                    if cmp(&a[j], &a[i]) <= 0 {
                        j += 1;
                        break;
                    }
                }
                shift_down(a, j, i);
            }
            for i in sentry..end {
                let mut j = i - 1;
                if cmp(&a[j], &a[i]) <= 0 {
                    continue;
                }
                loop {
                    j -= 2;
                    if cmp(&a[j], &a[i]) <= 0 {
                        j += 1;
                        if cmp(&a[j], &a[i]) <= 0 {
                            j += 1;
                        }
                        break;
                    }
                    if j == start {
                        break;
                    }
                    if j == start + 1 {
                        j -= 1;
                        if cmp(&a[i], &a[j]) > 0 {
                            j += 1;
                        }
                        break;
                    }
                }
                shift_down(a, j, i);
            }
        }
    }
}

/// `zend_sort` over `a[lo..lo + n]`.
fn zend_sort_range<T>(a: &mut [T], mut lo: usize, mut n: usize, cmp: &mut CmpFn<T>) {
    loop {
        if n <= 16 {
            insert_sort(a, lo, n, cmp);
            return;
        }
        let start = lo;
        let end = lo + n;
        let offset = n >> 1;
        let mut pivot = start + offset;
        if n >> 10 != 0 {
            let delta = offset >> 1;
            sort5(
                a,
                [start, start + delta, pivot, pivot + delta, end - 1],
                cmp,
            );
        } else {
            sort3(a, start, pivot, end - 1, cmp);
        }
        a.swap(start + 1, pivot);
        pivot = start + 1;
        let mut i = pivot + 1;
        let mut j = end - 1;
        'part: loop {
            while cmp(&a[pivot], &a[i]) > 0 {
                i += 1;
                if i == j {
                    break 'part;
                }
            }
            j -= 1;
            if j == i {
                break 'part;
            }
            while cmp(&a[j], &a[pivot]) > 0 {
                j -= 1;
                if j == i {
                    break 'part;
                }
            }
            a.swap(i, j);
            i += 1;
            if i == j {
                break 'part;
            }
        }
        a.swap(pivot, i - 1);
        if (i - 1) - start < end - i {
            zend_sort_range(a, start, i - start - 1, cmp);
            lo = i;
            n = end - i;
        } else {
            zend_sort_range(a, i, end - i, cmp);
            n = i - start - 1;
        }
    }
}

/// php's `zend_sort` (hybrid insertion sort, unstable).
pub(crate) fn zend_sort<T>(a: &mut [T], cmp: &mut CmpFn<T>) {
    let n = a.len();
    zend_sort_range(a, 0, n, cmp);
}

// ---------------------------------------------------------------------------
// comparators
// ---------------------------------------------------------------------------

type Elem = (ArrayKey, Value);

fn normalize(n: i64) -> i32 {
    n.signum() as i32
}

/// Comparator state: the host, the callbacks, and the first exception thrown
/// by a callback (after which no more callbacks run and every comparison is
/// 0, as with php's `zend_call_function` under a pending exception).
struct Ctx<'h> {
    host: &'h mut dyn Host,
    data_cb: Option<Value>,
    key_cb: Option<Value>,
    err: Option<Thrown>,
}

impl Ctx<'_> {
    fn call(&mut self, cb: &Value, a: Value, b: Value) -> Option<Value> {
        if self.err.is_some() {
            return None;
        }
        match self.host.call_callable(cb, &[a, b]) {
            Ok(v) => Some(v),
            Err(e) => {
                self.err = Some(e);
                None
            }
        }
    }

    /// `php_array_user_compare_unstable` / `php_array_user_key_compare_unstable`:
    /// a `false` result is retried with the operands swapped (php also emits a
    /// one-time "Returning bool from comparison function is deprecated").
    fn user_sort_cmp(&mut self, cb: &Value, a: Value, b: Value) -> i32 {
        match self.call(cb, a.clone(), b.clone()) {
            None => 0,
            Some(Value::Bool(false)) => match self.call(cb, b, a) {
                None => 0,
                Some(v) => -normalize(conv::to_int(&v)),
            },
            Some(v) => normalize(conv::to_int(&v)),
        }
    }

    fn key_cmp(&mut self, x: &Elem, y: &Elem) -> i32 {
        let cb = self.key_cb.clone().expect("key callback");
        self.user_sort_cmp(&cb, x.0.to_value(), y.0.to_value())
    }

    /// The data comparator: the user callback, or php's
    /// `string_compare_function` (binary compare of the string forms).
    fn data_cmp(&mut self, x: &Elem, y: &Elem) -> i32 {
        match self.data_cb.clone() {
            Some(cb) => self.user_sort_cmp(&cb, x.1.clone(), y.1.clone()),
            None => {
                let a = conv::to_string_bytes(&x.1);
                let b = conv::to_string_bytes(&y.1);
                match a.cmp(&b) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// argument parsing (zpp "+f" / "+ff")
// ---------------------------------------------------------------------------

/// Validate the trailing `ncb` callbacks, then the arrays; returns the arrays
/// and callbacks (in argument order).
fn parse_args(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    ncb: usize,
) -> Result<(Vec<PhpArray>, Vec<Value>), Thrown> {
    if args.len() < ncb + 1 {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!(
                "{func}() expects at least {} arguments, {} given",
                ncb + 1,
                args.len()
            ),
        ));
    }
    let narr = args.len() - ncb;
    let cbs: Vec<Value> = args[narr..].iter().map(deref).collect();
    for (i, cb) in cbs.iter().enumerate() {
        if !host.is_callable_value(cb) {
            return Err(Thrown::type_error(format!(
                "{func}(): Argument #{} must be a valid callback, {}",
                narr + i + 1,
                match cb {
                    Value::Str(s) => format!(
                        "function \"{}\" not found or invalid function name",
                        String::from_utf8_lossy(s.as_bytes())
                    ),
                    other => format!(
                        "no array or string given ({} given)",
                        zval_value_name(host, other)
                    ),
                }
            )));
        }
    }
    let mut arrays = Vec::with_capacity(narr);
    for (i, a) in args[..narr].iter().enumerate() {
        match deref(a) {
            Value::Array(arr) => arrays.push(arr),
            other => {
                // Only the first parameter is named (`array $array, ...$rest`).
                return Err(Thrown::type_error(format!(
                    "{func}(): Argument #{}{} must be of type array, {} given",
                    i + 1,
                    if i == 0 { " ($array)" } else { "" },
                    zval_value_name(host, &other)
                )));
            }
        }
    }
    Ok((arrays, cbs))
}

// ---------------------------------------------------------------------------
// php_array_intersect / php_array_diff
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Behavior {
    /// Compare values only (`array_udiff`, `array_uintersect`).
    Normal,
    /// Compare keys only (`array_diff_ukey`, `array_intersect_ukey`).
    Key,
    /// Compare keys, then values (`*_uassoc`).
    Assoc,
}

/// Copy each array into a list of (key, value) and sort it with the
/// behavior's comparator.
fn sorted_lists(ctx: &mut Ctx, arrays: &[PhpArray], behavior: Behavior) -> Vec<Vec<Elem>> {
    let mut lists = Vec::with_capacity(arrays.len());
    for arr in arrays {
        let mut list: Vec<Elem> = arr.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        if list.len() > 1 {
            if behavior == Behavior::Normal {
                zend_sort(&mut list, &mut |x, y| ctx.data_cmp(x, y));
            } else {
                zend_sort(&mut list, &mut |x, y| ctx.key_cmp(x, y));
            }
        }
        lists.push(list);
    }
    lists
}

fn finish(ctx: Ctx, base: &PhpArray, deleted: HashSet<ArrayKey>) -> Result<Value, Thrown> {
    if let Some(e) = ctx.err {
        return Err(e);
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        if !deleted.contains(k) {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

fn php_array_intersect(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    behavior: Behavior,
    data_user: bool,
    key_user: bool,
) -> Result<Value, Thrown> {
    let ncb = data_user as usize + key_user as usize;
    let (arrays, cbs) = parse_args(host, args, func, ncb)?;
    let mut ctx = Ctx {
        host,
        data_cb: if data_user {
            Some(cbs[0].clone())
        } else {
            None
        },
        key_cb: if key_user { cbs.last().cloned() } else { None },
        err: None,
    };
    let lists = sorted_lists(&mut ctx, &arrays, behavior);
    let n = lists.len();
    let mut ptrs = vec![0usize; n];
    let mut deleted = HashSet::new();
    let l0 = &lists[0];
    let mut c: i32 = 0;
    'out: while ptrs[0] < l0.len() {
        let mut i = 1;
        while i < n {
            let li = &lists[i];
            if behavior == Behavior::Normal {
                while ptrs[i] < li.len() {
                    c = ctx.data_cmp(&l0[ptrs[0]], &li[ptrs[i]]);
                    if c <= 0 {
                        break;
                    }
                    ptrs[i] += 1;
                }
            } else {
                while ptrs[i] < li.len() {
                    c = ctx.key_cmp(&l0[ptrs[0]], &li[ptrs[i]]);
                    if c <= 0 {
                        break;
                    }
                    ptrs[i] += 1;
                }
                if c == 0
                    && ptrs[i] < li.len()
                    && behavior == Behavior::Assoc
                    && ctx.data_cmp(&l0[ptrs[0]], &li[ptrs[i]]) != 0
                {
                    c = 1;
                }
            }
            if ptrs[i] == li.len() {
                // The rest of the first list is missing from this array.
                for e in &l0[ptrs[0]..] {
                    deleted.insert(e.0.clone());
                }
                break 'out;
            }
            if c != 0 {
                break;
            }
            ptrs[i] += 1;
            i += 1;
        }
        if c != 0 {
            // Not in all arrays: drop it (and, for values, every entry that
            // still sorts below the other array's current one).
            loop {
                deleted.insert(l0[ptrs[0]].0.clone());
                ptrs[0] += 1;
                if ptrs[0] == l0.len() {
                    break 'out;
                }
                if behavior == Behavior::Normal {
                    if ctx.data_cmp(&l0[ptrs[0]], &lists[i][ptrs[i]]) >= 0 {
                        break;
                    }
                } else {
                    break;
                }
            }
        } else {
            // In all arrays: keep it and every equal entry after it.
            loop {
                ptrs[0] += 1;
                if ptrs[0] == l0.len() {
                    break 'out;
                }
                if behavior == Behavior::Normal {
                    if ctx.data_cmp(&l0[ptrs[0] - 1], &l0[ptrs[0]]) != 0 {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
    }
    finish(ctx, &arrays[0], deleted)
}

fn php_array_diff(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    behavior: Behavior,
    data_user: bool,
    key_user: bool,
) -> Result<Value, Thrown> {
    let ncb = data_user as usize + key_user as usize;
    let (arrays, cbs) = parse_args(host, args, func, ncb)?;
    let mut ctx = Ctx {
        host,
        data_cb: if data_user {
            Some(cbs[0].clone())
        } else {
            None
        },
        key_cb: if key_user { cbs.last().cloned() } else { None },
        err: None,
    };
    let lists = sorted_lists(&mut ctx, &arrays, behavior);
    let n = lists.len();
    let mut ptrs = vec![0usize; n];
    let mut deleted = HashSet::new();
    let l0 = &lists[0];
    'out: while ptrs[0] < l0.len() {
        let mut c: i32 = 1;
        for i in 1..n {
            let li = &lists[i];
            let mut ptr = ptrs[i];
            if behavior == Behavior::Normal {
                while ptrs[i] < li.len() {
                    c = ctx.data_cmp(&l0[ptrs[0]], &li[ptrs[i]]);
                    if c <= 0 {
                        break;
                    }
                    ptrs[i] += 1;
                }
            } else {
                // Keys: a linear scan of the whole list from its start.
                while ptr < li.len() {
                    c = ctx.key_cmp(&l0[ptrs[0]], &li[ptr]);
                    if c == 0 {
                        break;
                    }
                    ptr += 1;
                }
            }
            if c == 0 {
                match behavior {
                    Behavior::Normal => {
                        if ptrs[i] < li.len() {
                            ptrs[i] += 1;
                        }
                        break;
                    }
                    Behavior::Assoc => {
                        if ptr < li.len() {
                            if ctx.data_cmp(&l0[ptrs[0]], &li[ptr]) != 0 {
                                c = -1;
                            } else {
                                break;
                            }
                        }
                    }
                    Behavior::Key => break,
                }
            }
        }
        if c == 0 {
            // Found in another array: drop it and every equal entry after it.
            loop {
                deleted.insert(l0[ptrs[0]].0.clone());
                ptrs[0] += 1;
                if ptrs[0] == l0.len() {
                    break 'out;
                }
                if behavior == Behavior::Normal {
                    if ctx.data_cmp(&l0[ptrs[0] - 1], &l0[ptrs[0]]) != 0 {
                        break;
                    }
                } else {
                    break;
                }
            }
        } else {
            // In none of the others: keep it and every equal entry after it.
            loop {
                ptrs[0] += 1;
                if ptrs[0] == l0.len() {
                    break 'out;
                }
                if behavior == Behavior::Normal {
                    if ctx.data_cmp(&l0[ptrs[0] - 1], &l0[ptrs[0]]) != 0 {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
    }
    finish(ctx, &arrays[0], deleted)
}

// ---------------------------------------------------------------------------
// php_array_intersect_key / php_array_diff_key with a user value comparator
// ---------------------------------------------------------------------------

/// `zval_user_compare`: the callback result, normalised (no bool retry).
fn user_eq(host: &mut dyn Host, cb: &Value, a: &Value, b: &Value) -> Result<bool, Thrown> {
    Ok(conv::to_int(&host.call_callable(cb, &[a.clone(), b.clone()])?) == 0)
}

fn assoc_by_key(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    intersect: bool,
) -> Result<Value, Thrown> {
    let (arrays, cbs) = parse_args(host, args, func, 1)?;
    let cb = &cbs[0];
    let mut out = PhpArray::new();
    for (k, v) in arrays[0].iter() {
        let mut ok = true;
        for other in &arrays[1..] {
            let hit = match other.get(k) {
                Some(ov) => user_eq(host, cb, v, ov)?,
                None => false,
            };
            if hit != intersect {
                ok = false;
                break;
            }
        }
        if ok {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// the builtins
// ---------------------------------------------------------------------------

pub(crate) fn fn_array_udiff(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_array_diff(host, args, "array_udiff", Behavior::Normal, true, false)
}

pub(crate) fn fn_array_uintersect(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_array_intersect(
        host,
        args,
        "array_uintersect",
        Behavior::Normal,
        true,
        false,
    )
}

pub(crate) fn fn_array_diff_ukey(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_array_diff(host, args, "array_diff_ukey", Behavior::Key, false, true)
}

fn fn_array_intersect_ukey(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_array_intersect(
        host,
        args,
        "array_intersect_ukey",
        Behavior::Key,
        false,
        true,
    )
}

fn fn_array_diff_uassoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_array_diff(
        host,
        args,
        "array_diff_uassoc",
        Behavior::Assoc,
        false,
        true,
    )
}

fn fn_array_intersect_uassoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_array_intersect(
        host,
        args,
        "array_intersect_uassoc",
        Behavior::Assoc,
        false,
        true,
    )
}

fn fn_array_udiff_uassoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_array_diff(
        host,
        args,
        "array_udiff_uassoc",
        Behavior::Assoc,
        true,
        true,
    )
}

fn fn_array_uintersect_uassoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_array_intersect(
        host,
        args,
        "array_uintersect_uassoc",
        Behavior::Assoc,
        true,
        true,
    )
}

fn fn_array_udiff_assoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    assoc_by_key(host, args, "array_udiff_assoc", false)
}

fn fn_array_uintersect_assoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    assoc_by_key(host, args, "array_uintersect_assoc", true)
}

// Array set-operation and aggregate extras (no by-reference args):
// `array_replace_recursive`, `array_diff_assoc`, `array_intersect_assoc`,
// `array_find`, `array_find_key`, `array_any`, `array_all`, and the
// `key_exists` alias of `array_key_exists`.
//
// The `array_find` family (PHP 8.4+) is callback-based: the callback is
// invoked with `(value, key)` in that order — verified against php 8.5.8.
//
// Every function here is verified byte-for-byte against the local php 8.5.8
// binary (expected values hardcoded from `php -r '...'` output).

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// array_diff_assoc / array_intersect_assoc
//
// Both compare keys AND the `(string)` form of values (php casts both sides to
// string: two elements are equal iff `(string)$a === (string)$b` and the keys
// match).
// ---------------------------------------------------------------------------

fn fn_array_diff_assoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = array_param(host, args, 0, "array_diff_assoc")?;
    let mut others = Vec::with_capacity(args.len() - 1);
    for i in 1..args.len() {
        others.push(array_param(host, args, i, "array_diff_assoc")?);
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

fn fn_array_intersect_assoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = array_param(host, args, 0, "array_intersect_assoc")?;
    let mut others = Vec::with_capacity(args.len() - 1);
    for i in 1..args.len() {
        others.push(array_param(host, args, i, "array_intersect_assoc")?);
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

fn fn_array_replace_recursive(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = array_param(host, args, 0, "array_replace_recursive")?;
    for i in 1..args.len() {
        let a = array_param(host, args, i, "array_replace_recursive")?;
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
    let a = array_param(host, args, 0, "array_find")?;
    let cb = &args[1];
    for (k, v) in a.iter() {
        if conv::to_bool(&host.call_callable(cb, &[v.clone(), k.to_value()])?) {
            return Ok(v.clone());
        }
    }
    Ok(Value::Null)
}

fn fn_array_find_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_find_key")?;
    let cb = &args[1];
    for (k, v) in a.iter() {
        if conv::to_bool(&host.call_callable(cb, &[v.clone(), k.to_value()])?) {
            return Ok(k.to_value());
        }
    }
    Ok(Value::Null)
}

fn fn_array_any(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_any")?;
    let cb = &args[1];
    for (k, v) in a.iter() {
        if conv::to_bool(&host.call_callable(cb, &[v.clone(), k.to_value()])?) {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn fn_array_all(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_all")?;
    let cb = &args[1];
    for (k, v) in a.iter() {
        if !conv::to_bool(&host.call_callable(cb, &[v.clone(), k.to_value()])?) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

// ---------------------------------------------------------------------------
// key_exists  (alias of array_key_exists)
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

// By-reference array builtins: the sort family and the stack/queue operations.
//
// These take their array argument **by reference** (`array &$array`): the
// compiler emits a `MakeRef` for arg 0, so the builtin receives a
// `Value::Ref` and mutates the backing array in place. The `u*` variants
// invoke a user comparator through the [`Host`] callback hook.

// PHP sort-flag constants. The low byte selects the comparison kind; the
// `SORT_FLAG_CASE` bit (8) can be OR'd into `SORT_STRING` / `SORT_NATURAL` to
// fold ASCII case.
const SORT_NUMERIC: i64 = 1;
const SORT_STRING: i64 = 2;
const SORT_NATURAL: i64 = 6;
const SORT_FLAG_CASE: i64 = 8;

/// Compare two values honouring a `SORT_*` flag (default: PHP loose compare).
/// The `SORT_FLAG_CASE` bit folds ASCII case for the string/natural kinds.
/// [`cmp_flagged`] with the engine's comparison context (notices such as
/// "Object of class X could not be converted to int", `__toString`): the
/// first exception (e.g. from a throwing error handler) is kept in `err` and
/// the remaining comparisons are skipped.
pub(crate) fn cmp_flagged_cx(
    host: &mut dyn Host,
    a: &Value,
    b: &Value,
    flags: i64,
    err: &mut Option<Thrown>,
) -> Ordering {
    if err.is_some() {
        return Ordering::Equal;
    }
    if flags & !SORT_FLAG_CASE & 0xff != 0 {
        return cmp_flagged(a, b, flags);
    }
    match host.compare_cx() {
        Some(cx) => match ops::compare(a, b, cx) {
            Ok(o) => o,
            Err(t) => {
                *err = Some(t);
                Ordering::Equal
            }
        },
        None => cmp_flagged(a, b, flags),
    }
}

fn cmp_flagged(a: &Value, b: &Value, flags: i64) -> Ordering {
    let fold_case = flags & SORT_FLAG_CASE != 0;
    // Strip the case bit before selecting the base comparison kind.
    match flags & !SORT_FLAG_CASE & 0xff {
        SORT_NUMERIC => conv::to_float(a)
            .partial_cmp(&conv::to_float(b))
            .unwrap_or(Ordering::Equal),
        SORT_STRING => {
            let (mut sa, mut sb) = (conv::to_string_bytes(a), conv::to_string_bytes(b));
            if fold_case {
                sa.make_ascii_lowercase();
                sb.make_ascii_lowercase();
            }
            sa.cmp(&sb)
        }
        SORT_NATURAL => crate::ext::standard::string::strnatcmp_ex(
            &conv::to_string_bytes(a),
            &conv::to_string_bytes(b),
            fold_case,
        )
        .cmp(&0),
        _ => ops::loose_compare(a, b),
    }
}

/// Borrow the referenced array mutably and hand it to `f`. Returns `None` when
/// arg 0 is not a reference to an array (PHP raises a TypeError; we no-op).
fn snapshot(arg: &Value) -> Option<Vec<(ArrayKey, Value)>> {
    if let Value::Ref(rc) = arg {
        if let Value::Array(a) = &*rc.borrow() {
            return Some(
                a.iter()
                    .map(|(k, v)| (k.clone(), deref(v).clone()))
                    .collect(),
            );
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

fn sort_values(host: &mut dyn Host, args: &[Value], reverse: bool) -> Result<Value, Thrown> {
    let Some(entries) = snapshot(&args[0]) else {
        return Ok(Value::Bool(false));
    };
    let flags = flags_arg(args, 1);
    let mut vals: Vec<Value> = entries.into_iter().map(|(_, v)| v).collect();
    let mut err = None;
    vals.sort_by(|a, b| {
        let o = cmp_flagged_cx(host, a, b, flags, &mut err);
        if reverse {
            o.reverse()
        } else {
            o
        }
    });
    if let Some(t) = err.or_else(|| host.take_pending_throw()) {
        return Err(t);
    }
    store(&args[0], reindex(vals));
    Ok(Value::Bool(true))
}

fn fn_sort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    sort_values(host, args, false)
}

fn fn_rsort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    sort_values(host, args, true)
}

// -- associative sorts (preserve keys) ------------------------------------

fn asort_impl(
    host: &mut dyn Host,
    args: &[Value],
    reverse: bool,
    by_key: bool,
) -> Result<Value, Thrown> {
    let Some(mut pairs) = snapshot(&args[0]) else {
        return Ok(Value::Bool(false));
    };
    let flags = flags_arg(args, 1);
    let mut err = None;
    pairs.sort_by(|a, b| {
        let o = if by_key {
            cmp_flagged(&a.0.to_value(), &b.0.to_value(), flags)
        } else {
            cmp_flagged_cx(host, &a.1, &b.1, flags, &mut err)
        };
        if reverse {
            o.reverse()
        } else {
            o
        }
    });
    if let Some(t) = err.or_else(|| host.take_pending_throw()) {
        return Err(t);
    }
    store(&args[0], from_pairs(pairs));
    Ok(Value::Bool(true))
}

fn fn_asort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    asort_impl(host, args, false, false)
}
fn fn_arsort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    asort_impl(host, args, true, false)
}
fn fn_ksort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    asort_impl(host, args, false, true)
}
fn fn_krsort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    asort_impl(host, args, true, true)
}

// -- user-comparator sorts ------------------------------------------------

fn usort_impl(
    host: &mut dyn Host,
    args: &[Value],
    preserve_keys: bool,
    by_key: bool,
) -> Result<Value, Thrown> {
    let Some(pairs) = snapshot(&args[0]) else {
        return Ok(Value::Bool(false));
    };
    let cmp = args[1].clone();
    let mut pairs = pairs;
    let mut err: Option<Thrown> = None;
    pairs.sort_by(|a, b| {
        if err.is_some() {
            return Ordering::Equal;
        }
        let (x, y) = if by_key {
            (a.0.to_value(), b.0.to_value())
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

// -- natural-order sorts (preserve keys) ----------------------------------

/// `natsort(&$array)` / `natcasesort(&$array)`: sort by value using the natural
/// order algorithm (reusing `strnatcmp`/`strnatcasecmp`), preserving keys.
fn natsort_impl(args: &[Value], fold_case: bool) -> Value {
    let Some(mut pairs) = snapshot(&args[0]) else {
        return Value::Bool(false);
    };
    pairs.sort_by(|a, b| {
        crate::ext::standard::string::strnatcmp_ex(
            &conv::to_string_bytes(&a.1),
            &conv::to_string_bytes(&b.1),
            fold_case,
        )
        .cmp(&0)
    });
    store(&args[0], from_pairs(pairs));
    Value::Bool(true)
}

fn fn_natsort(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(natsort_impl(args, false))
}

fn fn_natcasesort(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(natsort_impl(args, true))
}

// -- array_walk -----------------------------------------------------------

/// `array_walk(&$array, $callback, $extra = null)`: invoke
/// `$callback($value, $key, $extra)` for each element. The callback's first
/// parameter is passed BY REFERENCE: a fresh `Value::Ref` is handed to the
/// callback, so a callback declaring `&$value` mutates the element in place
/// (the possibly-modified value is written back under the original key). A
/// callback taking `$value` by value leaves the element unchanged, matching PHP.
fn fn_array_walk(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(pairs) = snapshot(&args[0]) else {
        return Ok(Value::Bool(false));
    };
    let cb = args[1].clone();
    let extra = args.get(2).map(deref).unwrap_or(Value::Null);
    let mut out = PhpArray::new();
    for (k, v) in pairs {
        // Pass the element by reference so `&$value` callbacks write back.
        let cell = std::rc::Rc::new(std::cell::RefCell::new(v));
        let value_ref = Value::Ref(cell.clone());
        host.call_callable(&cb, &[value_ref, k.to_value(), extra.clone()])?;
        let new_val = cell.borrow().clone();
        out.insert(k, new_val);
    }
    store(&args[0], out);
    Ok(Value::Bool(true))
}

/// `array_walk_recursive(&$array, $callback, $extra = null)`: like `array_walk`
/// but descends into sub-arrays, invoking the callback only on leaf (non-array)
/// values (by reference, written back in place).
fn fn_array_walk_recursive(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(pairs) = snapshot(&args[0]) else {
        return Ok(Value::Bool(false));
    };
    let cb = args[1].clone();
    let extra = args.get(2).map(deref).unwrap_or(Value::Null);
    let out = walk_rec_pairs(host, &cb, &extra, pairs)?;
    store(&args[0], out);
    Ok(Value::Bool(true))
}

fn walk_rec_pairs(
    host: &mut dyn Host,
    cb: &Value,
    extra: &Value,
    pairs: Vec<(rhp_values::ArrayKey, Value)>,
) -> Result<PhpArray, Thrown> {
    let mut out = PhpArray::new();
    for (k, v) in pairs {
        match deref(&v) {
            Value::Array(sub) => {
                let subpairs: Vec<_> = sub.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let walked = walk_rec_pairs(host, cb, extra, subpairs)?;
                out.insert(k, Value::Array(walked));
            }
            leaf => {
                let cell = std::rc::Rc::new(std::cell::RefCell::new(leaf));
                host.call_callable(cb, &[Value::Ref(cell.clone()), k.to_value(), extra.clone()])?;
                let nv = cell.borrow().clone();
                out.insert(k, nv);
            }
        }
    }
    Ok(out)
}

// -- stack / queue --------------------------------------------------------

/// Shared access for the internal-pointer functions: run `f` with a mutable
/// `ArrayData` (through the by-ref arg). Returns `None` if the arg isn't an array.
fn with_array_data<R>(arg: &Value, f: impl FnOnce(&mut rhp_values::ArrayData) -> R) -> Option<R> {
    match arg {
        Value::Ref(rc) => {
            let mut g = rc.borrow_mut();
            if let Value::Array(a) = &mut *g {
                Some(f(a.make_mut()))
            } else {
                None
            }
        }
        // A by-value (temp) array: php warns but still operates on it; the moved
        // pointer just isn't observable afterwards. We work on a local copy.
        Value::Array(a) => {
            let mut a = a.clone();
            Some(f(a.make_mut()))
        }
        _ => None,
    }
}

/// Z_PARAM_ARRAY_OR_OBJECT of the internal-pointer functions: an array
/// (`None`), or an object (deprecated) whose property table they walk;
/// anything else is a TypeError.
fn ptr_arg(
    host: &mut dyn Host,
    fname: &str,
    arg: &Value,
) -> Result<Option<rhp_values::ObjectHandle>, Thrown> {
    match deref(arg) {
        Value::Array(_) => Ok(None),
        Value::Object(h) => {
            crate::deprecated(
                host,
                fname,
                &format!("Calling {fname}() on an object is deprecated"),
            )?;
            Ok(Some(h))
        }
        other => Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($array) must be of type array, {} given",
            crate::zend::zpp::zval_value_name(host, &other)
        ))),
    }
}

thread_local! {
    /// The internal pointer of each object's property table, by object id.
    static OBJECT_POS: std::cell::RefCell<std::collections::HashMap<u32, usize>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// A new request: object ids start over, so no pointer may survive.
pub(crate) fn reset_state() {
    OBJECT_POS.with(|m| m.borrow_mut().clear());
}

/// Forget a freed object's internal pointer.
pub(crate) fn object_freed(id: u32) {
    // Runs for every object freed: an empty table is not hashed into.
    let _ = OBJECT_POS.try_with(|m| {
        let mut m = m.borrow_mut();
        if !m.is_empty() {
            m.remove(&id);
        }
    });
}

/// The table php's Z_OBJPROP gives the internal-pointer functions: the
/// property table by mangled name (an ArrayObject's storage is not in it).
fn object_table(host: &mut dyn Host, h: &rhp_values::ObjectHandle) -> Vec<(Value, Value)> {
    let b = h.borrow();
    b.properties
        .iter()
        .filter(|(k, _)| !host.is_internal_slot(b.class, k))
        .map(|(k, v)| (Value::Str(host.mangled_prop_name(b.class, k)), deref(v)))
        .collect()
}

/// Which internal-pointer operation to run.
#[derive(Clone, Copy)]
enum PtrOp {
    Reset,
    End,
    Next,
    Prev,
    Current,
    Key,
}

/// An internal-pointer function on an object's property table.
fn object_ptr(host: &mut dyn Host, h: &rhp_values::ObjectHandle, op: PtrOp) -> Value {
    let table = object_table(host, h);
    let id = h.borrow().id;
    let pos = OBJECT_POS.with(|m| m.borrow().get(&id).copied().unwrap_or(0));
    let pos = match op {
        PtrOp::Reset => 0,
        PtrOp::End => table.len().saturating_sub(1),
        PtrOp::Next => pos.saturating_add(1),
        PtrOp::Prev if pos == 0 || pos == usize::MAX => usize::MAX,
        PtrOp::Prev => pos - 1,
        PtrOp::Current | PtrOp::Key => pos,
    };
    OBJECT_POS.with(|m| m.borrow_mut().insert(id, pos));
    match (op, table.get(pos)) {
        (PtrOp::Key, Some((k, _))) => k.clone(),
        (PtrOp::Key, None) => Value::Null,
        (_, Some((_, v))) => v.clone(),
        (_, None) => Value::Bool(false),
    }
}

fn elem_at(d: &rhp_values::ArrayData, pos: usize) -> Value {
    d.entries
        .get_index(pos)
        .map(|(_, v)| deref(v).clone())
        .unwrap_or(Value::Bool(false))
}

fn fn_reset(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(h) = ptr_arg(host, "reset", &args[0])? {
        return Ok(object_ptr(host, &h, PtrOp::Reset));
    }
    Ok(with_array_data(&args[0], |d| {
        d.pos = 0;
        elem_at(d, 0)
    })
    .unwrap_or(Value::Bool(false)))
}

fn fn_end(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(h) = ptr_arg(host, "end", &args[0])? {
        return Ok(object_ptr(host, &h, PtrOp::End));
    }
    Ok(with_array_data(&args[0], |d| {
        let n = d.entries.len();
        d.pos = n.saturating_sub(1);
        if n == 0 {
            Value::Bool(false)
        } else {
            elem_at(d, d.pos)
        }
    })
    .unwrap_or(Value::Bool(false)))
}

fn fn_next(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(h) = ptr_arg(host, "next", &args[0])? {
        return Ok(object_ptr(host, &h, PtrOp::Next));
    }
    Ok(with_array_data(&args[0], |d| {
        d.pos = d.pos.saturating_add(1);
        elem_at(d, d.pos)
    })
    .unwrap_or(Value::Bool(false)))
}

fn fn_prev(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(h) = ptr_arg(host, "prev", &args[0])? {
        return Ok(object_ptr(host, &h, PtrOp::Prev));
    }
    Ok(with_array_data(&args[0], |d| {
        if d.pos == 0 {
            d.pos = usize::MAX; // past-begin
            Value::Bool(false)
        } else {
            d.pos -= 1;
            elem_at(d, d.pos)
        }
    })
    .unwrap_or(Value::Bool(false)))
}

fn fn_current(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(h) = ptr_arg(host, "current", &args[0])? {
        return Ok(object_ptr(host, &h, PtrOp::Current));
    }
    Ok(with_array_data(&args[0], |d| elem_at(d, d.pos)).unwrap_or(Value::Bool(false)))
}

fn fn_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(h) = ptr_arg(host, "key", &args[0])? {
        return Ok(object_ptr(host, &h, PtrOp::Key));
    }
    Ok(with_array_data(&args[0], |d| {
        d.entries
            .get_index(d.pos)
            .map(|(k, _)| k.to_value())
            .unwrap_or(Value::Null)
    })
    .unwrap_or(Value::Null))
}

fn fn_array_push(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Ref(rc) = &args[0] else {
        return Ok(Value::Bool(false));
    };
    let mut g = rc.borrow_mut();
    let Value::Array(a) = &mut *g else {
        return Ok(Value::Bool(false));
    };
    for v in &args[1..] {
        a.push(deref(v).clone());
    }
    Ok(Value::Int(a.len() as i64))
}

fn fn_array_pop(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(pairs) = snapshot(&args[0]) else {
        return Ok(Value::Null);
    };
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
    let Some(pairs) = snapshot(&args[0]) else {
        return Ok(Value::Null);
    };
    if pairs.is_empty() {
        return Ok(Value::Null);
    }
    let mut it = pairs.into_iter();
    let (_, removed) = it.next().unwrap();
    store(&args[0], rebuild_reindex_ints(it));
    Ok(removed)
}

fn fn_array_unshift(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(pairs) = snapshot(&args[0]) else {
        return Ok(Value::Bool(false));
    };
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

/// `array_splice(&$array, $offset, $length?, $replacement?)`: remove a slice
/// (reindexed), optionally splicing in `$replacement`, and return the removed
/// elements (reindexed). The array is passed by reference (arg 0).
fn fn_array_splice(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(pairs) = snapshot(&args[0]) else {
        return Ok(Value::Array(PhpArray::new()));
    };
    let len = pairs.len() as i64;

    // offset: negative counts from the end; clamped to [0, len].
    let mut offset = conv::to_int(&deref(&args[1]));
    if offset < 0 {
        offset = (len + offset).max(0);
    } else {
        offset = offset.min(len);
    }
    let offset = offset as usize;

    // length: default to the rest; negative stops that many from the end.
    let remove = match args.get(2) {
        None | Some(Value::Null) => (len as usize).saturating_sub(offset),
        Some(v) => {
            let l = conv::to_int(&deref(v));
            if l < 0 {
                ((len + l) as usize).saturating_sub(offset)
            } else {
                (l as usize).min((len as usize).saturating_sub(offset))
            }
        }
    };

    // replacement: an array splices its values; a scalar splices as one element.
    let replacement: Vec<Value> = match args.get(3).map(deref) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().map(|(_, v)| deref(v).clone()).collect(),
        // convert_to_array: the object's properties (ARRAY_CAST).
        Some(Value::Object(h)) => {
            let purpose = crate::zend::object_handlers::PropPurpose::ArrayCast;
            let a = match crate::zend::object_handlers::properties_for(host, &h, purpose)? {
                Some(a) => a,
                None => host.object_props_mangled(&h),
            };
            a.iter().map(|(_, v)| deref(v).clone()).collect()
        }
        Some(v) => vec![v],
    };

    let end = (offset + remove).min(pairs.len());
    let mut removed = PhpArray::new();
    let mut result = PhpArray::new();
    // Entries before the cut (reindex ints, preserve string keys).
    for (k, v) in &pairs[..offset] {
        match k {
            ArrayKey::Int(_) => result.push(v.clone()),
            s => result.insert(s.clone(), v.clone()),
        }
    }
    // Spliced-in replacement.
    for rv in &replacement {
        result.push(rv.clone());
    }
    // Removed slice (always reindexed in the return value).
    for (_, v) in &pairs[offset..end] {
        removed.push(v.clone());
    }
    // Entries after the cut.
    for (k, v) in &pairs[end..] {
        match k {
            ArrayKey::Int(_) => result.push(v.clone()),
            s => result.insert(s.clone(), v.clone()),
        }
    }
    store(&args[0], result);
    Ok(Value::Array(removed))
}

// ---------------------------------------------------------------------------
// array_diff_key / array_intersect_key
// ---------------------------------------------------------------------------

fn fn_array_diff_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = array_param(host, args, 0, "array_diff_key")?;
    let mut others = Vec::new();
    for i in 1..args.len() {
        others.push(array_param(host, args, i, "array_diff_key")?);
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        if !others.iter().any(|o| o.get(k).is_some()) {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_intersect_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = array_param(host, args, 0, "array_intersect_key")?;
    let mut others = Vec::new();
    for i in 1..args.len() {
        others.push(array_param(host, args, i, "array_intersect_key")?);
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        if others.iter().all(|o| o.get(k).is_some()) {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_replace
// ---------------------------------------------------------------------------

fn fn_array_replace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = array_param(host, args, 0, "array_replace")?;
    for i in 1..args.len() {
        let a = array_param(host, args, i, "array_replace")?;
        for (k, v) in a.iter() {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_merge_recursive
// ---------------------------------------------------------------------------

/// Merge `src` into `dest` per php's `php_array_merge_recursive`.
fn merge_recursive_into(dest: &mut PhpArray, src: &PhpArray) {
    for (k, v) in src.iter() {
        match k {
            // Integer keys are always appended (renumbered), never merged.
            ArrayKey::Int(_) => dest.push(v.clone()),
            ArrayKey::Str(_) => {
                let existing = dest.get(k).cloned();
                match existing {
                    None => {
                        dest.insert(k.clone(), v.clone());
                    }
                    Some(cur) => {
                        // Coerce the existing value to an array (scalars wrap as
                        // a single-element list), then fold in the new value.
                        let mut cur_arr = match cur {
                            Value::Array(a) => a,
                            other => {
                                let mut a = PhpArray::new();
                                a.push(other);
                                a
                            }
                        };
                        match deref(v) {
                            Value::Array(src_arr) => merge_recursive_into(&mut cur_arr, &src_arr),
                            other => cur_arr.push(other),
                        }
                        dest.insert(k.clone(), Value::Array(cur_arr));
                    }
                }
            }
        }
    }
}

fn fn_array_merge_recursive(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = PhpArray::new();
    for i in 0..args.len() {
        let a = array_param(host, args, i, "array_merge_recursive")?;
        merge_recursive_into(&mut out, &a);
    }
    Ok(Value::Array(out))
}

/// `shuffle(array &$array)`: Fisher-Yates over the values, keys dropped.
fn fn_shuffle(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Ref(r) = &args[0] else {
        return Ok(Value::Bool(true));
    };
    let arr = match &*r.borrow() {
        Value::Array(a) => a.clone(),
        _ => return Ok(Value::Bool(true)),
    };
    let mut vals: Vec<Value> = arr.iter().map(|(_, v)| v.clone()).collect();
    let n = vals.len();
    if n > 1 {
        let mut left = n - 1;
        while left > 0 {
            let j = mt_range(0, left as i64) as usize;
            vals.swap(left, j);
            left -= 1;
        }
    }
    let mut out = PhpArray::new();
    for v in vals {
        out.push(v);
    }
    *r.borrow_mut() = Value::Array(out);
    Ok(Value::Bool(true))
}

/// `array_rand(array $array, int $num = 1)` (php_array_pick_keys).
fn fn_array_rand(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arr = crate::zend::zpp::array_param(host, args, 0, "array_rand")?;
    let num = int_param(host, args, 1, "array_rand", "num", 1)?;
    let avail = arr.len() as i64;
    if avail == 0 {
        return Err(Thrown::error(
            "ValueError",
            "array_rand(): Argument #1 ($array) must not be empty",
        ));
    }
    if num == 1 {
        let i = mt_range(0, avail - 1) as usize;
        return Ok(arr.get_index(i).map(|(k, _)| k.to_value()).unwrap());
    }
    if num <= 0 || num > avail {
        return Err(Thrown::error(
            "ValueError",
            "array_rand(): Argument #2 ($num) must be between 1 and the number of elements in argument #1 ($array)",
        ));
    }
    let (mut want, negative) = if num > (avail >> 1) {
        (avail - num, true)
    } else {
        (num, false)
    };
    let mut bits = vec![false; avail as usize];
    while want > 0 {
        let r = mt_range(0, avail - 1) as usize;
        if !bits[r] {
            bits[r] = true;
            want -= 1;
        }
    }
    let mut out = PhpArray::new();
    for (i, (k, _)) in arr.iter().enumerate() {
        if bits[i] ^ negative {
            out.push(k.to_value());
        }
    }
    Ok(Value::Array(out))
}

// --- arrays ------------------------------------------------------------------

fn fn_array_change_key_case(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arr = crate::zend::zpp::array_param(host, args, 0, "array_change_key_case")?;
    let upper = int_param(host, args, 1, "array_change_key_case", "case", 0)? != 0;
    let mut out = PhpArray::new();
    for (k, v) in arr.iter() {
        let nk = match k {
            ArrayKey::Int(n) => ArrayKey::Int(*n),
            ArrayKey::Str(s) => {
                let b = if upper {
                    s.as_bytes().to_ascii_uppercase()
                } else {
                    s.as_bytes().to_ascii_lowercase()
                };
                ArrayKey::Str(PhpStr::from_bytes(b))
            }
        };
        out.insert(nk, v.clone());
    }
    Ok(Value::Array(out))
}

const SORT_ASC: i64 = 4;
const SORT_DESC: i64 = 3;

/// php names only the declared parameter (`$array`); variadic `$rest`
/// arguments are reported by position alone.
fn multisort_arg(i: usize) -> String {
    if i == 0 {
        "1 ($array)".to_string()
    } else {
        (i + 1).to_string()
    }
}

/// `array_multisort(array &$array, mixed &...$rest)`.
fn fn_array_multisort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // (array slot, sort type, descending)
    let mut arrays: Vec<(usize, i64, bool)> = Vec::new();
    let (mut order_ok, mut type_ok) = (false, false);
    for (i, a) in args.iter().enumerate() {
        match deref(a) {
            Value::Array(_) => {
                arrays.push((i, 0, false));
                order_ok = true;
                type_ok = true;
            }
            Value::Int(f) => match f & !SORT_FLAG_CASE {
                SORT_ASC | SORT_DESC => {
                    if !order_ok {
                        return Err(Thrown::type_error(format!(
                            "array_multisort(): Argument #{} must be an array or a sort flag that has not already been specified",
                            multisort_arg(i)
                        )));
                    }
                    arrays.last_mut().unwrap().2 = f == SORT_DESC;
                    order_ok = false;
                }
                0 | 1 | 2 | 5 | 6 => {
                    if !type_ok {
                        return Err(Thrown::type_error(format!(
                            "array_multisort(): Argument #{} must be an array or a sort flag that has not already been specified",
                            multisort_arg(i)
                        )));
                    }
                    arrays.last_mut().unwrap().1 = f;
                    type_ok = false;
                }
                _ => {
                    return Err(Thrown::error(
                        "ValueError",
                        format!(
                            "array_multisort(): Argument #{} must be a valid sort flag",
                            multisort_arg(i)
                        ),
                    ))
                }
            },
            _ => {
                return Err(Thrown::type_error(format!(
                    "array_multisort(): Argument #{} must be an array or a sort flag",
                    multisort_arg(i)
                )))
            }
        }
    }
    let cols: Vec<Vec<(ArrayKey, Value)>> = arrays
        .iter()
        .map(|&(slot, _, _)| match deref(&args[slot]) {
            Value::Array(a) => a.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => Vec::new(),
        })
        .collect();
    let size = cols[0].len();
    if cols.iter().any(|c| c.len() != size) {
        return Err(Thrown::error("ValueError", "Array sizes are inconsistent"));
    }
    if size == 0 {
        return Ok(Value::Bool(true));
    }
    let mut order: Vec<usize> = (0..size).collect();
    let mut err = None;
    order.sort_by(|&x, &y| {
        for (c, &(_, ty, desc)) in arrays.iter().enumerate() {
            let (a, b) = (&cols[c][x].1, &cols[c][y].1);
            let o = if desc {
                cmp_flagged_cx(host, b, a, ty, &mut err)
            } else {
                cmp_flagged_cx(host, a, b, ty, &mut err)
            };
            if o != Ordering::Equal {
                return o;
            }
        }
        x.cmp(&y)
    });
    if let Some(t) = err.or_else(|| host.take_pending_throw()) {
        return Err(t);
    }
    for (c, &(slot, _, _)) in arrays.iter().enumerate() {
        let mut out = PhpArray::new();
        for &i in &order {
            let (k, v) = &cols[c][i];
            match k {
                ArrayKey::Int(_) => out.push(v.clone()),
                ArrayKey::Str(_) => out.insert(k.clone(), v.clone()),
            }
        }
        if let Value::Ref(r) = &args[slot] {
            *r.borrow_mut() = Value::Array(out);
        }
    }
    Ok(Value::Bool(true))
}

fn fn_array_first(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_first")?;
    Ok(a.iter()
        .next()
        .map(|(_, v)| v.clone())
        .unwrap_or(Value::Null))
}

fn fn_array_last(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_last")?;
    Ok(a.iter()
        .last()
        .map(|(_, v)| v.clone())
        .unwrap_or(Value::Null))
}

/// array.c's constants: `COUNT_*`, `SORT_*`, `CASE_*`, `ARRAY_FILTER_*`.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("COUNT_NORMAL", Value::Int(0)),
        ("COUNT_RECURSIVE", Value::Int(1)),
        ("SORT_REGULAR", Value::Int(0)),
        ("SORT_NUMERIC", Value::Int(SORT_NUMERIC)),
        ("SORT_STRING", Value::Int(SORT_STRING)),
        ("SORT_DESC", Value::Int(SORT_DESC)),
        ("SORT_ASC", Value::Int(SORT_ASC)),
        ("SORT_LOCALE_STRING", Value::Int(5)),
        ("SORT_NATURAL", Value::Int(SORT_NATURAL)),
        ("SORT_FLAG_CASE", Value::Int(SORT_FLAG_CASE)),
        ("CASE_LOWER", Value::Int(0)),
        ("CASE_UPPER", Value::Int(1)),
        ("ARRAY_FILTER_USE_KEY", Value::Int(2)),
        ("ARRAY_FILTER_USE_BOTH", Value::Int(1)),
    ]
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("array_slice", fn_array_slice),
            ("array_flip", fn_array_flip),
            ("array_fill", fn_array_fill),
            ("array_fill_keys", fn_array_fill_keys),
            ("array_combine", fn_array_combine),
            ("array_search", fn_array_search),
            ("array_unique", fn_array_unique),
            ("array_pad", fn_array_pad),
            ("array_key_first", fn_array_key_first),
            ("array_key_last", fn_array_key_last),
            ("array_diff", fn_array_diff),
            ("array_intersect", fn_array_intersect),
            ("array_column", fn_array_column),
            ("array_chunk", fn_array_chunk),
            ("array_count_values", fn_array_count_values),
            ("count", fn_count),
            ("sizeof", fn_sizeof),
            ("max", fn_max),
            ("min", fn_min),
            ("array_sum", fn_array_sum),
            ("array_product", fn_array_product),
            ("array_keys", fn_array_keys),
            ("array_values", fn_array_values),
            ("array_merge", fn_array_merge),
            ("array_reverse", fn_array_reverse),
            ("range", fn_range),
            ("array_map", fn_array_map),
            ("array_filter", fn_array_filter),
            ("array_reduce", fn_array_reduce),
            ("in_array", fn_in_array),
            ("array_key_exists", fn_array_key_exists),
            ("array_udiff", fn_array_udiff),
            ("array_uintersect", fn_array_uintersect),
            ("array_diff_ukey", fn_array_diff_ukey),
            ("array_intersect_ukey", fn_array_intersect_ukey),
            ("array_diff_uassoc", fn_array_diff_uassoc),
            ("array_intersect_uassoc", fn_array_intersect_uassoc),
            ("array_udiff_uassoc", fn_array_udiff_uassoc),
            ("array_uintersect_uassoc", fn_array_uintersect_uassoc),
            ("array_udiff_assoc", fn_array_udiff_assoc),
            ("array_uintersect_assoc", fn_array_uintersect_assoc),
            ("array_diff_assoc", fn_array_diff_assoc),
            ("array_intersect_assoc", fn_array_intersect_assoc),
            ("array_replace_recursive", fn_array_replace_recursive),
            ("array_find", fn_array_find),
            ("array_find_key", fn_array_find_key),
            ("array_any", fn_array_any),
            ("array_all", fn_array_all),
            ("key_exists", fn_key_exists),
            ("sort", fn_sort),
            ("rsort", fn_rsort),
            ("asort", fn_asort),
            ("arsort", fn_arsort),
            ("ksort", fn_ksort),
            ("krsort", fn_krsort),
            ("usort", fn_usort),
            ("uasort", fn_uasort),
            ("uksort", fn_uksort),
            ("natsort", fn_natsort),
            ("natcasesort", fn_natcasesort),
            ("array_walk", fn_array_walk),
            ("array_walk_recursive", fn_array_walk_recursive),
            ("array_push", fn_array_push),
            ("array_pop", fn_array_pop),
            ("array_shift", fn_array_shift),
            ("array_unshift", fn_array_unshift),
            ("array_splice", fn_array_splice),
            ("reset", fn_reset),
            ("end", fn_end),
            ("next", fn_next),
            ("prev", fn_prev),
            ("current", fn_current),
            ("key", fn_key),
            ("array_diff_key", fn_array_diff_key),
            ("array_intersect_key", fn_array_intersect_key),
            ("array_replace", fn_array_replace),
            ("array_merge_recursive", fn_array_merge_recursive),
            ("shuffle", fn_shuffle),
            ("array_rand", fn_array_rand),
            ("array_change_key_case", fn_array_change_key_case),
            ("array_multisort", fn_array_multisort),
            ("array_first", fn_array_first),
            ("array_last", fn_array_last),
        ],
    );
}

#[cfg(test)]
mod tests;
