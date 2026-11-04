//! Pure array builtins (no callbacks, no by-reference): `array_slice`,
//! `array_flip`, `array_fill`, `array_fill_keys`, `array_combine`,
//! `array_search`, `array_unique`, `array_pad`, `array_key_first`,
//! `array_key_last`, `array_is_list`, `array_diff`, `array_intersect`,
//! `array_column`, `array_chunk`, `array_count_values`.
//!
//! Key handling and var_dump/print_r output verified byte-for-byte against the
//! local php 8.5.8 binary (see the `#[cfg(test)]` module for the captured cases).

use std::collections::HashSet;

use rhp_semantics::{deref, ops};
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// `ArrayKey` -> `Value` (for returning keys, e.g. `array_search`).
fn key_to_value(k: &ArrayKey) -> Value {
    match k {
        ArrayKey::Int(n) => Value::Int(*n),
        ArrayKey::Str(s) => Value::Str(s.clone()),
    }
}

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

fn want_array(v: Value, func: &str) -> Result<PhpArray, Thrown> {
    match v {
        Value::Array(a) => Ok(a),
        other => Err(Thrown::type_error(format!(
            "{}(): Argument must be of type array, {} given",
            func,
            other.type_name_lower_()
        ))),
    }
}

trait TypeNameLower_ {
    fn type_name_lower_(&self) -> &'static str;
}
impl TypeNameLower_ for Value {
    fn type_name_lower_(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) | Value::Closure(_) => "object",
            Value::Resource(_) => "resource",
            Value::Ref(r) => r.borrow().type_name_lower_(),
        }
    }
}

// ---------------------------------------------------------------------------
// array_slice
// ---------------------------------------------------------------------------

fn fn_array_slice(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(deref(&args[0]), "array_slice")?;
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

fn fn_array_flip(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(deref(&args[0]), "array_flip")?;
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        if let Some(nk) = key_intstr(v) {
            out.insert(nk, key_to_value(k));
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

fn fn_array_fill_keys(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let keys = want_array(deref(&args[0]), "array_fill_keys")?;
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

fn fn_array_combine(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let keys = want_array(deref(&args[0]), "array_combine")?;
    let values = want_array(deref(&args[1]), "array_combine")?;
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
    let a = want_array(deref(&args[1]), "array_search")?;
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
            return Ok(key_to_value(k));
        }
    }
    Ok(Value::Bool(false))
}

// ---------------------------------------------------------------------------
// array_unique (SORT_STRING semantics — the default)
// ---------------------------------------------------------------------------

fn fn_array_unique(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(deref(&args[0]), "array_unique")?;
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

fn fn_array_pad(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(deref(&args[0]), "array_pad")?;
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

fn fn_array_key_first(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(deref(&args[0]), "array_key_first")?;
    Ok(a.iter()
        .next()
        .map(|(k, _)| key_to_value(k))
        .unwrap_or(Value::Null))
}

fn fn_array_key_last(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(deref(&args[0]), "array_key_last")?;
    Ok(a.iter()
        .last()
        .map(|(k, _)| key_to_value(k))
        .unwrap_or(Value::Null))
}

// ---------------------------------------------------------------------------
// array_is_list
// ---------------------------------------------------------------------------

fn fn_array_is_list(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(deref(&args[0]), "array_is_list")?;
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

fn fn_array_diff(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = want_array(deref(&args[0]), "array_diff")?;
    // Union of the string forms of every subsequent array's values.
    let mut others: HashSet<Vec<u8>> = HashSet::new();
    for arg in &args[1..] {
        let o = want_array(deref(arg), "array_diff")?;
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

fn fn_array_intersect(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = want_array(deref(&args[0]), "array_intersect")?;
    let mut sets: Vec<HashSet<Vec<u8>>> = Vec::new();
    for arg in &args[1..] {
        let o = want_array(deref(arg), "array_intersect")?;
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
    let rows = want_array(deref(&args[0]), "array_column")?;
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

fn fn_array_chunk(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(deref(&args[0]), "array_chunk")?;
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

fn fn_array_count_values(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(deref(&args[0]), "array_count_values")?;
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

pub fn register(m: &mut std::collections::HashMap<&'static str, Builtin>) {
    let add = |m: &mut std::collections::HashMap<&'static str, Builtin>,
               name: &'static str,
               func: crate::NativeFn,
               min: usize,
               max: Option<usize>| {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    };
    add(m, "array_slice", fn_array_slice, 2, Some(4));
    add(m, "array_flip", fn_array_flip, 1, Some(1));
    add(m, "array_fill", fn_array_fill, 3, Some(3));
    add(m, "array_fill_keys", fn_array_fill_keys, 2, Some(2));
    add(m, "array_combine", fn_array_combine, 2, Some(2));
    add(m, "array_search", fn_array_search, 2, Some(3));
    add(m, "array_unique", fn_array_unique, 1, Some(2));
    add(m, "array_pad", fn_array_pad, 3, Some(3));
    add(m, "array_key_first", fn_array_key_first, 1, Some(1));
    add(m, "array_key_last", fn_array_key_last, 1, Some(1));
    add(m, "array_is_list", fn_array_is_list, 1, Some(1));
    add(m, "array_diff", fn_array_diff, 1, None);
    add(m, "array_intersect", fn_array_intersect, 1, None);
    add(m, "array_column", fn_array_column, 2, Some(3));
    add(m, "array_chunk", fn_array_chunk, 2, Some(3));
    add(m, "array_count_values", fn_array_count_values, 1, Some(1));
}

// ---------------------------------------------------------------------------
// tests — expected strings captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct NullHost;
    impl Host for NullHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    fn dumped(v: &Value) -> String {
        let mut out = Vec::new();
        crate::dump::var_dump(&NullHost, &mut out, v, 0);
        String::from_utf8_lossy(&out).into_owned()
    }

    fn call(f: crate::NativeFn, args: &[Value]) -> Value {
        f(&mut NullHost, args).expect("builtin returned Err")
    }

    fn arr(items: Vec<(ArrayKey, Value)>) -> Value {
        let mut a = PhpArray::new();
        for (k, v) in items {
            a.insert(k, v);
        }
        Value::Array(a)
    }

    fn list(items: Vec<Value>) -> Value {
        let mut a = PhpArray::new();
        for v in items {
            a.push(v);
        }
        Value::Array(a)
    }

    fn s(b: &str) -> Value {
        Value::str(b)
    }
    fn i(n: i64) -> Value {
        Value::Int(n)
    }
    fn ik(n: i64) -> ArrayKey {
        ArrayKey::Int(n)
    }
    fn sk(b: &str) -> ArrayKey {
        ArrayKey::from_bytes(b.as_bytes())
    }

    #[test]
    fn t_array_slice() {
        let base = list(vec![s("a"), s("b"), s("c"), s("d"), s("e")]);
        let r = call(fn_array_slice, &[base.clone(), i(2)]);
        assert_eq!(
            dumped(&r),
            "array(3) {\n  [0]=>\n  string(1) \"c\"\n  [1]=>\n  string(1) \"d\"\n  [2]=>\n  string(1) \"e\"\n}\n"
        );
        let r = call(fn_array_slice, &[base.clone(), i(-2), i(1)]);
        assert_eq!(dumped(&r), "array(1) {\n  [0]=>\n  string(1) \"d\"\n}\n");
        let r = call(fn_array_slice, &[base.clone(), i(1), i(-1)]);
        assert_eq!(
            dumped(&r),
            "array(3) {\n  [0]=>\n  string(1) \"b\"\n  [1]=>\n  string(1) \"c\"\n  [2]=>\n  string(1) \"d\"\n}\n"
        );
        let r = call(fn_array_slice, &[base, i(1), i(2), Value::Bool(true)]);
        assert_eq!(
            dumped(&r),
            "array(2) {\n  [1]=>\n  string(1) \"b\"\n  [2]=>\n  string(1) \"c\"\n}\n"
        );
    }

    #[test]
    fn t_array_flip() {
        let base = arr(vec![(sk("a"), i(1)), (sk("b"), i(2)), (sk("c"), i(1))]);
        let r = call(fn_array_flip, &[base]);
        assert_eq!(
            dumped(&r),
            "array(2) {\n  [1]=>\n  string(1) \"c\"\n  [2]=>\n  string(1) \"b\"\n}\n"
        );
    }

    #[test]
    fn t_array_fill() {
        let r = call(fn_array_fill, &[i(5), i(3), s("v")]);
        assert_eq!(
            dumped(&r),
            "array(3) {\n  [5]=>\n  string(1) \"v\"\n  [6]=>\n  string(1) \"v\"\n  [7]=>\n  string(1) \"v\"\n}\n"
        );
        let r = call(fn_array_fill, &[i(-3), i(3), s("v")]);
        assert_eq!(
            dumped(&r),
            "array(3) {\n  [-3]=>\n  string(1) \"v\"\n  [-2]=>\n  string(1) \"v\"\n  [-1]=>\n  string(1) \"v\"\n}\n"
        );
        assert!(fn_array_fill(&mut NullHost, &[i(0), i(-1), s("v")]).is_err());
    }

    #[test]
    fn t_array_fill_keys() {
        let keys = list(vec![s("a"), s("b"), i(7)]);
        let r = call(fn_array_fill_keys, &[keys, i(0)]);
        assert_eq!(
            dumped(&r),
            "array(3) {\n  [\"a\"]=>\n  int(0)\n  [\"b\"]=>\n  int(0)\n  [7]=>\n  int(0)\n}\n"
        );
        let keys = list(vec![
            i(1),
            s("1"),
            s("01"),
            Value::Float(2.9),
            Value::Bool(true),
        ]);
        let r = call(fn_array_fill_keys, &[keys, s("x")]);
        assert_eq!(
            dumped(&r),
            "array(3) {\n  [1]=>\n  string(1) \"x\"\n  [\"01\"]=>\n  string(1) \"x\"\n  [\"2.9\"]=>\n  string(1) \"x\"\n}\n"
        );
    }

    #[test]
    fn t_array_combine() {
        let keys = list(vec![s("a"), Value::Float(2.9), Value::Bool(true), s("123")]);
        let vals = list(vec![i(1), i(2), i(3), i(4)]);
        let r = call(fn_array_combine, &[keys, vals]);
        assert_eq!(
            dumped(&r),
            "array(4) {\n  [\"a\"]=>\n  int(1)\n  [\"2.9\"]=>\n  int(2)\n  [1]=>\n  int(3)\n  [123]=>\n  int(4)\n}\n"
        );
        assert!(
            fn_array_combine(&mut NullHost, &[list(vec![i(1), i(2)]), list(vec![i(1)])]).is_err()
        );
    }

    #[test]
    fn t_array_search() {
        let base = arr(vec![(sk("x"), s("a")), (sk("y"), s("b"))]);
        assert_eq!(
            dumped(&call(fn_array_search, &[s("b"), base])),
            "string(1) \"y\"\n"
        );
        let base = list(vec![s("a"), s("b")]);
        assert_eq!(
            dumped(&call(fn_array_search, &[i(0), base])),
            "bool(false)\n"
        );
        let base = list(vec![i(1), i(2), i(3)]);
        assert_eq!(
            dumped(&call(fn_array_search, &[s("1"), base.clone()])),
            "int(0)\n"
        );
        assert_eq!(
            dumped(&call(fn_array_search, &[s("1"), base, Value::Bool(true)])),
            "bool(false)\n"
        );
    }

    #[test]
    fn t_array_unique() {
        let base = list(vec![i(1), s("1"), Value::Float(1.0), s("a"), s("a"), i(2)]);
        let r = call(fn_array_unique, &[base]);
        assert_eq!(
            dumped(&r),
            "array(3) {\n  [0]=>\n  int(1)\n  [3]=>\n  string(1) \"a\"\n  [5]=>\n  int(2)\n}\n"
        );
    }

    #[test]
    fn t_array_pad() {
        let base = list(vec![i(1), i(2), i(3)]);
        let r = call(fn_array_pad, &[base.clone(), i(5), i(0)]);
        assert_eq!(
            dumped(&r),
            "array(5) {\n  [0]=>\n  int(1)\n  [1]=>\n  int(2)\n  [2]=>\n  int(3)\n  [3]=>\n  int(0)\n  [4]=>\n  int(0)\n}\n"
        );
        let r = call(fn_array_pad, &[base.clone(), i(-5), i(0)]);
        assert_eq!(
            dumped(&r),
            "array(5) {\n  [0]=>\n  int(0)\n  [1]=>\n  int(0)\n  [2]=>\n  int(1)\n  [3]=>\n  int(2)\n  [4]=>\n  int(3)\n}\n"
        );
        let assoc = arr(vec![(sk("a"), i(1)), (sk("b"), i(2))]);
        let r = call(fn_array_pad, &[assoc, i(4), i(9)]);
        assert_eq!(
            dumped(&r),
            "array(4) {\n  [\"a\"]=>\n  int(1)\n  [\"b\"]=>\n  int(2)\n  [0]=>\n  int(9)\n  [1]=>\n  int(9)\n}\n"
        );
        let sparse = arr(vec![(ik(5), s("a")), (ik(9), s("b"))]);
        let r = call(fn_array_pad, &[sparse, i(2), i(0)]);
        assert_eq!(
            dumped(&r),
            "array(2) {\n  [5]=>\n  string(1) \"a\"\n  [9]=>\n  string(1) \"b\"\n}\n"
        );
    }

    #[test]
    fn t_key_first_last() {
        let base = arr(vec![(sk("a"), i(1)), (sk("b"), i(2))]);
        assert_eq!(
            dumped(&call(fn_array_key_first, &[base])),
            "string(1) \"a\"\n"
        );
        let base = arr(vec![(ik(3), s("x")), (ik(7), s("y"))]);
        assert_eq!(dumped(&call(fn_array_key_last, &[base])), "int(7)\n");
        assert_eq!(dumped(&call(fn_array_key_first, &[list(vec![])])), "NULL\n");
        assert_eq!(dumped(&call(fn_array_key_last, &[list(vec![])])), "NULL\n");
    }

    #[test]
    fn t_array_is_list() {
        assert_eq!(
            dumped(&call(fn_array_is_list, &[list(vec![])])),
            "bool(true)\n"
        );
        assert_eq!(
            dumped(&call(fn_array_is_list, &[list(vec![i(1), i(2), i(3)])])),
            "bool(true)\n"
        );
        let swapped = arr(vec![(ik(1), s("a")), (ik(0), s("b"))]);
        assert_eq!(dumped(&call(fn_array_is_list, &[swapped])), "bool(false)\n");
        let gap = arr(vec![(ik(0), s("a")), (ik(2), s("b"))]);
        assert_eq!(dumped(&call(fn_array_is_list, &[gap])), "bool(false)\n");
    }

    #[test]
    fn t_array_diff() {
        let base = list(vec![i(1), i(2), i(3), i(4)]);
        let r = call(
            fn_array_diff,
            &[base, list(vec![i(2), i(4)]), list(vec![i(3)])],
        );
        assert_eq!(dumped(&r), "array(1) {\n  [0]=>\n  int(1)\n}\n");
        let base = list(vec![i(1), s("1"), i(2)]);
        let r = call(fn_array_diff, &[base, list(vec![s("1")])]);
        assert_eq!(dumped(&r), "array(1) {\n  [2]=>\n  int(2)\n}\n");
    }

    #[test]
    fn t_array_intersect() {
        let base = list(vec![i(1), i(2), i(3), i(4)]);
        let r = call(
            fn_array_intersect,
            &[base, list(vec![i(2), i(4), i(5)]), list(vec![i(4), i(2)])],
        );
        assert_eq!(
            dumped(&r),
            "array(2) {\n  [1]=>\n  int(2)\n  [3]=>\n  int(4)\n}\n"
        );
    }

    #[test]
    fn t_array_column() {
        let rows = list(vec![
            arr(vec![(sk("id"), i(1)), (sk("name"), s("a"))]),
            arr(vec![(sk("id"), i(2)), (sk("name"), s("b"))]),
        ]);
        let r = call(fn_array_column, &[rows.clone(), s("name")]);
        assert_eq!(
            dumped(&r),
            "array(2) {\n  [0]=>\n  string(1) \"a\"\n  [1]=>\n  string(1) \"b\"\n}\n"
        );
        let r = call(fn_array_column, &[rows, s("name"), s("id")]);
        assert_eq!(
            dumped(&r),
            "array(2) {\n  [1]=>\n  string(1) \"a\"\n  [2]=>\n  string(1) \"b\"\n}\n"
        );
        let rows = list(vec![
            arr(vec![(sk("id"), i(5)), (sk("name"), s("a"))]),
            arr(vec![(sk("id"), i(6)), (sk("name"), s("b"))]),
        ]);
        let r = call(fn_array_column, &[rows, Value::Null, s("id")]);
        assert_eq!(
            dumped(&r),
            "array(2) {\n  [5]=>\n  array(2) {\n    [\"id\"]=>\n    int(5)\n    [\"name\"]=>\n    string(1) \"a\"\n  }\n  [6]=>\n  array(2) {\n    [\"id\"]=>\n    int(6)\n    [\"name\"]=>\n    string(1) \"b\"\n  }\n}\n"
        );
    }

    #[test]
    fn t_array_column_missing_index() {
        let rows = list(vec![
            arr(vec![(sk("id"), i(1)), (sk("name"), s("a"))]),
            arr(vec![(sk("name"), s("b"))]),
            arr(vec![(sk("id"), i(3)), (sk("name"), s("c"))]),
        ]);
        let r = call(fn_array_column, &[rows, s("name"), s("id")]);
        assert_eq!(
            dumped(&r),
            "array(3) {\n  [1]=>\n  string(1) \"a\"\n  [2]=>\n  string(1) \"b\"\n  [3]=>\n  string(1) \"c\"\n}\n"
        );
    }

    #[test]
    fn t_array_chunk() {
        let base = list(vec![i(1), i(2), i(3), i(4), i(5)]);
        let r = call(fn_array_chunk, &[base, i(2)]);
        assert_eq!(
            dumped(&r),
            "array(3) {\n  [0]=>\n  array(2) {\n    [0]=>\n    int(1)\n    [1]=>\n    int(2)\n  }\n  [1]=>\n  array(2) {\n    [0]=>\n    int(3)\n    [1]=>\n    int(4)\n  }\n  [2]=>\n  array(1) {\n    [0]=>\n    int(5)\n  }\n}\n"
        );
        let assoc = arr(vec![(sk("a"), i(1)), (sk("b"), i(2)), (sk("c"), i(3))]);
        let r = call(fn_array_chunk, &[assoc, i(2), Value::Bool(true)]);
        assert_eq!(
            dumped(&r),
            "array(2) {\n  [0]=>\n  array(2) {\n    [\"a\"]=>\n    int(1)\n    [\"b\"]=>\n    int(2)\n  }\n  [1]=>\n  array(1) {\n    [\"c\"]=>\n    int(3)\n  }\n}\n"
        );
        assert!(fn_array_chunk(&mut NullHost, &[list(vec![i(1)]), i(0)]).is_err());
    }

    #[test]
    fn t_array_count_values() {
        let base = list(vec![
            i(1),
            i(1),
            s("1"),
            s("hello"),
            s("hello"),
            Value::Float(2.5),
            s("123"),
        ]);
        let r = call(fn_array_count_values, &[base]);
        assert_eq!(
            dumped(&r),
            "array(3) {\n  [1]=>\n  int(3)\n  [\"hello\"]=>\n  int(2)\n  [123]=>\n  int(1)\n}\n"
        );
    }
}
