//! Pure array builtins (no callbacks, no by-reference): `array_slice`,
//! `array_flip`, `array_fill`, `array_fill_keys`, `array_combine`,
//! `array_search`, `array_unique`, `array_pad`, `array_key_first`,
//! `array_key_last`, `array_is_list`, `array_diff`, `array_intersect`,
//! `array_column`, `array_chunk`, `array_count_values`.
//!
//! Key handling and var_dump/print_r output verified byte-for-byte against the
//! local php 8.5.8 binary (see `arr_fns/tests.rs` for the captured cases).

use std::collections::HashSet;

use rhp_values::ops::{self, deref};
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::zend::zpp::array_param;
use crate::{Host, Thrown};

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

const HT_MAX_SIZE: i128 = 1073741824; // 0x40000000

fn fn_array_pad(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_pad")?;
    let size = conv::to_int(&deref(&args[1]));
    let val = deref(&args[2]);

    let abs = (size as i128).abs();
    if abs > HT_MAX_SIZE {
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

fn fn_array_is_list(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_is_list")?;
    let mut expected: i64 = 0;
    for (k, _) in a.iter() {
        match k {
            ArrayKey::Int(n) if *n == expected => expected += 1,
            _ => return Ok(Value::Bool(false)),
        }
    }
    Ok(Value::Bool(true))
}

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
            ("array_is_list", fn_array_is_list),
            ("array_diff", fn_array_diff),
            ("array_intersect", fn_array_intersect),
            ("array_column", fn_array_column),
            ("array_chunk", fn_array_chunk),
            ("array_count_values", fn_array_count_values),
        ],
    );
}

#[cfg(test)]
mod tests;
