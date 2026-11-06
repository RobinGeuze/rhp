//! ext/standard/array.c: the array functions, `count`, `min`/`max`,
//! `range`, the sort family and the internal-pointer functions.
//!
//! Key handling and var_dump/print_r output verified byte-for-byte against the
//! local php 8.5.8 binary (see `array/tests.rs` for the captured cases).

use std::cmp::Ordering;
use std::collections::HashSet;

use rhp_values::ops::{copy_elem, deref};
use rhp_values::{conv, ops, ArrayKey, PhpArray, PhpStr, Value};

use crate::ext::random::mt_range;
use crate::zend::zpp::{array_param, int_param, zval_value_name};
use crate::{ext, host_compare, host_loose_eq, warn, zend, Host, Thrown, E_WARNING};

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// The key `array_fill_keys` / `array_combine` make of a value: an int
/// stays an int, anything else goes through zval_get_string (Array to
/// string conversion, `__toString`) and zend_symtable_update's numeric
/// string rule. Floats become string keys (`2.9` -> `"2.9"`).
fn key_strcast(host: &mut dyn Host, v: &Value) -> Result<ArrayKey, Thrown> {
    Ok(match deref(v) {
        Value::Int(n) => ArrayKey::Int(n),
        other => ArrayKey::from_bytes(&crate::zend::zpp::zval_get_string(host, &other)?),
    })
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

/// zend_hash_next_index_insert failing: the next key is PHP_INT_MAX and taken.
fn cannot_add_element() -> Thrown {
    Thrown::error(
        "Error",
        "Cannot add element to the array as the next element is already occupied",
    )
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

    let mut out = PhpArray::new();
    for (k, v) in a.iter().skip(start as usize).take(cnt as usize) {
        let v = copy_elem(v);
        match k {
            ArrayKey::Str(_) => out.insert(k.clone(), v),
            ArrayKey::Int(_) if preserve => out.insert(k.clone(), v),
            ArrayKey::Int(_) => out.push(v),
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
        match key_intstr(v) {
            Some(nk) => out.insert(nk, k.to_value()),
            None => warn(
                host,
                "array_flip",
                "Can only flip string and integer values, entry skipped",
            )?,
        }
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
    if count > 0 && start > i64::MAX - count + 1 {
        return Err(cannot_add_element());
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
        out.insert(key_strcast(host, kv)?, val.clone());
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
        out.insert(key_strcast(host, kv)?, copy_elem(vv));
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
    let flags = args
        .get(1)
        .map(|v| conv::to_int(&deref(v)))
        .unwrap_or(SORT_STRING);
    if flags == SORT_STRING {
        // The first occurrence of each string form.
        let mut seen: HashSet<Vec<u8>> = HashSet::new();
        let mut out = PhpArray::new();
        for (k, v) in a.iter() {
            if seen.insert(tmp_string(host, v)?) {
                out.insert(k.clone(), copy_elem(v));
            }
        }
        return Ok(Value::Array(out));
    }
    // Sort a list of the elements with the unstable comparator, then drop
    // each neighbour equal to the last kept one, keeping the earliest.
    let mut items: Vec<(usize, ArrayKey, Value)> = a
        .iter()
        .enumerate()
        .map(|(i, (k, v))| (i, k.clone(), v.clone()))
        .collect();
    let mut err = None;
    zend_sort(&mut items, &mut |x, y| {
        data_compare(host, &x.2, &y.2, flags, &mut err)
    });
    if let Some(t) = err.take().or_else(|| host.take_pending_throw()) {
        return Err(t);
    }
    let mut out = a.clone();
    out.make_mut();
    let mut last = 0usize;
    for c in 1..items.len() {
        if data_compare(host, &items[last].2, &items[c].2, flags, &mut err) != 0 {
            last = c;
        } else {
            let del = if items[last].0 > items[c].0 {
                let d = last;
                last = c;
                d
            } else {
                c
            };
            out.remove(&items[del].1);
        }
    }
    if let Some(t) = err.or_else(|| host.take_pending_throw()) {
        return Err(t);
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

/// string_compare_function: two strings compare directly, anything else
/// through zval_get_string (Array to string conversion, `__toString`).
fn string_compare(host: &mut dyn Host, a: &Value, b: &Value) -> Result<i32, Thrown> {
    if let (Value::Str(x), Value::Str(y)) = (a, b) {
        return Ok(ord_i32(x.as_bytes().cmp(y.as_bytes())));
    }
    let x = crate::zend::zpp::zval_get_string(host, a)?;
    let y = crate::zend::zpp::zval_get_string(host, b)?;
    Ok(ord_i32(x.cmp(&y)))
}

/// zval_get_tmp_string, borrowing a string value.
fn tmp_string(host: &mut dyn Host, v: &Value) -> Result<Vec<u8>, Thrown> {
    match v {
        Value::Str(s) => Ok(s.as_bytes().to_vec()),
        other => crate::zend::zpp::zval_get_string(host, other),
    }
}

fn fn_array_diff(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = array_param(host, args, 0, "array_diff")?;
    let mut others = Vec::with_capacity(args.len().saturating_sub(1));
    for i in 1..args.len() {
        others.push(array_param(host, args, i, "array_diff")?);
    }
    match base.len() {
        0 => return Ok(Value::Array(PhpArray::new())),
        // One element: searched for in the others, stopping at a match.
        1 => {
            let search = tmp_string(host, base.get_index(0).expect("one").1)?;
            for o in &others {
                for (_, v) in o.iter() {
                    if tmp_string(host, v)? == search {
                        return Ok(Value::Array(PhpArray::new()));
                    }
                }
            }
            return Ok(Value::Array(base));
        }
        _ => {}
    }
    if others.iter().all(|o| o.is_empty()) {
        return Ok(Value::Array(base));
    }
    // The exclude set: the string forms of every other array's values.
    let mut exclude: HashSet<Vec<u8>> = HashSet::new();
    for o in &others {
        for (_, v) in o.iter() {
            exclude.insert(tmp_string(host, v)?);
        }
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        if !exclude.contains(&tmp_string(host, v)?) {
            out.insert(k.clone(), copy_elem(v));
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_intersect(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_array_intersect(
        host,
        args,
        "array_intersect",
        Behavior::Normal,
        false,
        false,
    )
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
            // array_column_fetch_prop: ZVAL_DEREF.
            match column_ak.as_ref().and_then(|k| ra.get(k)) {
                Some(v) => deref(v),
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
            Some(iv) => {
                let k = set_zval_key(host, &deref(&iv))?;
                out.insert(k, value);
            }
            None => out.push(value),
        }
    }
    Ok(Value::Array(out))
}

/// array_set_zval_key: the key a value makes, with php's diagnostics
/// (resource and null offsets, lossy floats) and the TypeError for an
/// array or object.
fn set_zval_key(host: &mut dyn Host, v: &Value) -> Result<ArrayKey, Thrown> {
    Ok(match v {
        Value::Str(s) => ArrayKey::from_bytes(s.as_bytes()),
        Value::Resource(r) => {
            let id = r.id as i64;
            host.emit_diagnostic(
                E_WARNING,
                format!("Resource ID#{id} used as offset, casting to integer ({id})").as_bytes(),
            )?;
            ArrayKey::Int(id)
        }
        Value::Bool(b) => ArrayKey::Int(*b as i64),
        Value::Int(n) => ArrayKey::Int(*n),
        Value::Float(f) => ArrayKey::Int(zend::operators::dval_to_lval_safe(host, *f)?),
        Value::Null => {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                b"Using null as an array offset is deprecated, use an empty string instead",
            )?;
            ArrayKey::from_bytes(b"")
        }
        other => {
            return Err(Thrown::type_error(format!(
                "Cannot access offset of type {} on array",
                zval_value_name(host, other)
            )))
        }
    })
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
        let Some(k) = key_intstr(v) else {
            warn(
                host,
                "array_count_values",
                "Can only count string and integer values, entry skipped",
            )?;
            continue;
        };
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
    crate::zend::call_stack::check()?;
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
        out.push(copy_elem(v));
    }
    Ok(Value::Array(out))
}

fn fn_array_merge(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arrays: Vec<PhpArray> = args
        .iter()
        .filter_map(|a| match deref(a) {
            Value::Array(a) => Some(a),
            _ => None,
        })
        .collect();
    merge_total_check(&arrays)?;
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
                rhp_values::ArrayKey::Int(_) => out.push(copy_elem(v)),
                rhp_values::ArrayKey::Str(_) => out.insert(k.clone(), copy_elem(v)),
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
        // No callback, or nothing to map: the array itself.
        if cb_null || arrays[0].is_empty() {
            return Ok(Value::Array(arrays[0].clone()));
        }
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
    let entries: Vec<_> = a.iter().map(|(k, v)| (k.clone(), copy_elem(v))).collect();
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
                if self.err.is_some() {
                    return 0;
                }
                match string_compare(self.host, &x.1, &y.1) {
                    Ok(r) => r,
                    Err(e) => {
                        self.err = Some(e);
                        0
                    }
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
        if let Some(why) = host.check_callable(cb)? {
            return Err(Thrown::type_error(format!(
                "{func}(): Argument #{} must be a valid callback, {why}",
                narr + i + 1,
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
        // Values, not references: the result copies from the arrays.
        let mut list: Vec<Elem> = arr.iter().map(|(k, v)| (k.clone(), deref(v))).collect();
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
    // php copies the first array (zend_array_dup) and deletes from it.
    for (k, v) in base.iter() {
        if !deleted.contains(k) {
            out.insert(k.clone(), copy_elem(v));
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
            out.insert(k.clone(), copy_elem(v));
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

/// php_array_diff_key / php_array_intersect_key with zval_compare
/// (string_compare_function) on the values of matching keys.
fn assoc_string(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    diff: bool,
) -> Result<Value, Thrown> {
    let base = array_param(host, args, 0, func)?;
    let mut others = Vec::with_capacity(args.len() - 1);
    for i in 1..args.len() {
        others.push(array_param(host, args, i, func)?);
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        let val = copy_elem(v);
        let mut keep = true;
        for o in &others {
            let matched = match o.get(k) {
                Some(d) => string_compare(host, &deref(&val), &deref(d))? == 0,
                None => false,
            };
            if diff && matched {
                keep = false;
                break;
            }
            if !diff && !matched {
                keep = false;
                break;
            }
        }
        if keep {
            out.insert(k.clone(), val);
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_diff_assoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    assoc_string(host, args, "array_diff_assoc", true)
}

fn fn_array_intersect_assoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    assoc_string(host, args, "array_intersect_assoc", false)
}

// ---------------------------------------------------------------------------
// array_replace_recursive
//
// Like `array_replace`, but when both the existing and replacement values are
// arrays, merge them recursively instead of overwriting. Any non-array on
// either side replaces outright.
// ---------------------------------------------------------------------------

/// php_array_replace_recursive. `guard` holds the arrays (either side)
/// being replaced further up (Z_PROTECT_RECURSION).
fn replace_recursive_into(
    dest: &mut PhpArray,
    src: &PhpArray,
    guard: &mut Vec<usize>,
) -> Result<(), Thrown> {
    crate::zend::call_stack::check()?;
    for (k, src_entry) in src.iter() {
        let src_val = deref(src_entry);
        let dest_entry = match (&src_val, dest.get(k)) {
            (Value::Array(_), Some(d)) if matches!(deref(d), Value::Array(_)) => d.clone(),
            _ => {
                dest.insert(k.clone(), copy_elem(src_entry));
                continue;
            }
        };
        let (Value::Array(cur), Value::Array(sa)) = (deref(&dest_entry), src_val) else {
            unreachable!("both arrays")
        };
        let same_ref = matches!((&dest_entry, src_entry), (Value::Ref(a), Value::Ref(b)) if std::rc::Rc::ptr_eq(a, b));
        if guard.contains(&cur.data_ptr()) || guard.contains(&sa.data_ptr()) || same_ref {
            return Err(recursion_detected());
        }
        let (dp, sp) = (cur.data_ptr(), sa.data_ptr());
        // SEPARATE_ZVAL: a reference gives up its value.
        let mut cur = cur;
        guard.push(dp);
        guard.push(sp);
        let r = replace_recursive_into(&mut cur, &sa, guard);
        guard.truncate(guard.len() - 2);
        r?;
        dest.insert(k.clone(), Value::Array(cur));
    }
    Ok(())
}

fn fn_array_replace_recursive(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arrays = (0..args.len())
        .map(|i| array_param(host, args, i, "array_replace_recursive"))
        .collect::<Result<Vec<_>, _>>()?;
    // zend_array_dup of the first array.
    let mut out = arrays[0].clone();
    out.make_mut();
    let mut guard = Vec::new();
    for a in &arrays[1..] {
        replace_recursive_into(&mut out, a, &mut guard)?;
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

// PHP sort-flag constants. The `SORT_FLAG_CASE` bit (8) can be OR'd into
// `SORT_STRING` / `SORT_NATURAL` to fold ASCII case.
const SORT_REGULAR: i64 = 0;
const SORT_NUMERIC: i64 = 1;
const SORT_STRING: i64 = 2;
const SORT_LOCALE_STRING: i64 = 5;
const SORT_NATURAL: i64 = 6;
const SORT_FLAG_CASE: i64 = 8;

fn ord_i32(o: Ordering) -> i32 {
    o as i32
}

/// ZEND_THREEWAY_COMPARE on doubles (NAN compares greater).
fn three_way(a: f64, b: f64) -> i32 {
    if a == b {
        0
    } else if a < b {
        -1
    } else {
        1
    }
}

/// The unstable data comparators of php_get_data_compare_func: zend_compare,
/// numeric_compare_function, string_(case_)compare_function and strnatcmp_ex,
/// with their conversions ("Array to string conversion", `__toString`,
/// "could not be converted" warnings and Errors). The first exception is
/// kept in `err`; every later comparison is 0.
pub(crate) fn data_compare(
    host: &mut dyn Host,
    a: &Value,
    b: &Value,
    flags: i64,
    err: &mut Option<Thrown>,
) -> i32 {
    if err.is_some() {
        return 0;
    }
    match data_compare_res(host, a, b, flags) {
        Ok(n) => n,
        Err(t) => {
            *err = Some(t);
            0
        }
    }
}

fn data_compare_res(host: &mut dyn Host, a: &Value, b: &Value, flags: i64) -> Result<i32, Thrown> {
    use crate::zend::zpp::zval_get_string;
    let fold = flags & SORT_FLAG_CASE != 0;
    Ok(match flags & !SORT_FLAG_CASE {
        SORT_NUMERIC => {
            let x = ext::standard::formatted_print::zval_get_double(host, a)?;
            let y = ext::standard::formatted_print::zval_get_double(host, b)?;
            three_way(x, y)
        }
        SORT_STRING => {
            let (mut x, mut y) = (zval_get_string(host, a)?, zval_get_string(host, b)?);
            if fold {
                x.make_ascii_lowercase();
                y.make_ascii_lowercase();
            }
            ord_i32(x.cmp(&y))
        }
        SORT_LOCALE_STRING => {
            let (x, y) = (zval_get_string(host, a)?, zval_get_string(host, b)?);
            ext::standard::string::strcoll(&x, &y).signum()
        }
        SORT_NATURAL => {
            let (x, y) = (zval_get_string(host, a)?, zval_get_string(host, b)?);
            ext::standard::string::strnatcmp_ex(&x, &y, fold).signum() as i32
        }
        _ => regular_compare(host, a, b)?,
    })
}

/// php_array_data_compare_unstable_i: zend_compare, with enums (which are
/// uncomparable) grouped by identity and moved after everything else.
fn regular_compare(host: &mut dyn Host, a: &Value, b: &Value) -> Result<i32, Thrown> {
    // The common scalar pairs, without the engine context.
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => return Ok(ord_i32(x.cmp(y))),
        (Value::Float(x), Value::Float(y)) => return Ok(three_way(*x, *y)),
        _ => {}
    }
    let r = match host.compare_cx() {
        Some(cx) => ord_i32(ops::compare(a, b, cx)?),
        None => ord_i32(ops::loose_compare(a, b)),
    };
    if r == 1 {
        let enum_id = |host: &dyn Host, v: &Value| match deref(v) {
            Value::Object(h) => {
                let o = h.borrow();
                host.is_enum_class(o.class).then_some(o.id)
            }
            _ => None,
        };
        if let Some(rid) = enum_id(host, b) {
            return Ok(match enum_id(host, a) {
                Some(lid) => ord_i32(lid.cmp(&rid)),
                None => -1,
            });
        }
    }
    Ok(r)
}

/// zend_strtod of a string key: the leading decimal number, else 0.
fn key_strtod(s: &[u8]) -> f64 {
    if s.first().is_some_and(|c| c.is_ascii_whitespace()) {
        return 0.0;
    }
    conv::to_float(&Value::Str(PhpStr::from_bytes(s.to_vec())))
}

/// The unstable key comparators of php_get_key_compare_func.
fn key_compare(a: &ArrayKey, b: &ArrayKey, flags: i64) -> i32 {
    let fold = flags & SORT_FLAG_CASE != 0;
    let kind = flags & !SORT_FLAG_CASE;
    if let (ArrayKey::Int(x), ArrayKey::Int(y)) = (a, b) {
        if kind == SORT_NUMERIC || !matches!(kind, SORT_STRING | SORT_LOCALE_STRING | SORT_NATURAL)
        {
            return if x > y { 1 } else { -1 };
        }
    }
    let bytes = |k: &ArrayKey| match k {
        ArrayKey::Int(n) => n.to_string().into_bytes(),
        ArrayKey::Str(s) => s.as_bytes().to_vec(),
    };
    match kind {
        SORT_NUMERIC => {
            let d = |k: &ArrayKey| match k {
                ArrayKey::Int(n) => *n as f64,
                ArrayKey::Str(s) => key_strtod(s.as_bytes()),
            };
            three_way(d(a), d(b))
        }
        SORT_STRING => {
            let (mut x, mut y) = (bytes(a), bytes(b));
            if fold {
                x.make_ascii_lowercase();
                y.make_ascii_lowercase();
            }
            ord_i32(x.cmp(&y))
        }
        SORT_LOCALE_STRING => ext::standard::string::strcoll(&bytes(a), &bytes(b)).signum(),
        SORT_NATURAL => {
            ext::standard::string::strnatcmp_ex(&bytes(a), &bytes(b), fold).signum() as i32
        }
        _ => ord_i32(ops::loose_compare(&a.to_value(), &b.to_value())),
    }
}

fn flags_arg(args: &[Value], i: usize) -> i64 {
    args.get(i).map(|v| conv::to_int(&deref(v))).unwrap_or(0)
}

/// zend_hash_sort over the by-reference array in `arg`: entries keep their
/// stored values (references stay references), ties keep their original
/// order (RETURN_STABLE_SORT), the internal pointer is reset, and
/// `renumber` gives keys 0..n. With `dup` the entries are taken from a
/// zend_array_dup copy, as php_usort sorts one. False when `arg` holds no
/// array.
fn sort_by_ref(
    host: &mut dyn Host,
    arg: &Value,
    renumber: bool,
    dup: bool,
    cmp: &mut dyn FnMut(&mut dyn Host, &Elem, &Elem) -> i32,
) -> bool {
    let Value::Ref(rc) = arg else {
        return false;
    };
    // Separated first (Z_PARAM_ARRAY_EX), so what the comparisons reach
    // through references is the array being sorted.
    let arr = match &mut *rc.borrow_mut() {
        Value::Array(a) => {
            a.make_mut();
            a.clone()
        }
        _ => return false,
    };
    // zend_hash_sort leaves a table that needs no sorting alone (keys,
    // pointer and next free key); php_usort still swaps in its copy.
    if arr.is_empty() || (arr.len() == 1 && !renumber) {
        if dup && !arr.is_empty() {
            let mut copy = arr.clone();
            copy.make_mut();
            drop(arr);
            *rc.borrow_mut() = Value::Array(copy);
        }
        return true;
    }
    let next_free = arr.next_free();
    let mut items: Vec<(usize, Elem)> = if dup {
        let mut copy = arr.clone();
        std::mem::take(&mut copy.make_mut().entries)
            .into_iter()
            .enumerate()
            .collect()
    } else {
        arr.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .enumerate()
            .collect()
    };
    zend_sort(&mut items, &mut |a, b| match cmp(host, &a.1, &b.1) {
        0 => ord_i32(a.0.cmp(&b.0)),
        r => r,
    });
    let mut out = PhpArray::new();
    if renumber {
        for (_, (_, v)) in items {
            out.push(v);
        }
    } else {
        for (_, (k, v)) in items {
            out.insert(k, v);
        }
        out.make_mut().set_next_free(next_free);
    }
    drop(arr);
    let mut g = rc.borrow_mut();
    // An array_walk over it goes on from the same position (zend_hash_sort
    // keeps iterator positions; php_usort's zend_array_dup copies them).
    if let Value::Array(old) = &mut *g {
        if old.has_iters() {
            old.make_mut().move_iters_to(out.make_mut());
        }
    }
    *g = Value::Array(out);
    true
}

/// sort/rsort/asort/arsort (by value) and ksort/krsort (by key).
fn flag_sort(
    host: &mut dyn Host,
    args: &[Value],
    reverse: bool,
    by_key: bool,
    renumber: bool,
) -> Result<Value, Thrown> {
    let flags = flags_arg(args, 1);
    let mut err = None;
    let sign = if reverse { -1 } else { 1 };
    sort_by_ref(host, &args[0], renumber, false, &mut |host, a, b| {
        sign * if by_key {
            key_compare(&a.0, &b.0, flags)
        } else {
            data_compare(host, &a.1, &b.1, flags, &mut err)
        }
    });
    if let Some(t) = err.or_else(|| host.take_pending_throw()) {
        return Err(t);
    }
    Ok(Value::Bool(true))
}

fn fn_sort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    flag_sort(host, args, false, false, true)
}
fn fn_rsort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    flag_sort(host, args, true, false, true)
}
fn fn_asort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    flag_sort(host, args, false, false, false)
}
fn fn_arsort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    flag_sort(host, args, true, false, false)
}
fn fn_ksort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    flag_sort(host, args, false, true, false)
}
fn fn_krsort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    flag_sort(host, args, true, true, false)
}

// -- user-comparator sorts ------------------------------------------------

/// php_usort with php_array_user_compare / php_array_user_key_compare: the
/// user's result, a bool deprecated once and `false` retried with the
/// operands swapped.
fn usort_impl(
    host: &mut dyn Host,
    args: &[Value],
    renumber: bool,
    by_key: bool,
) -> Result<Value, Thrown> {
    let cmp = args[1].clone();
    let fname = if by_key {
        "uksort"
    } else if renumber {
        "usort"
    } else {
        "uasort"
    };
    let mut err: Option<Thrown> = None;
    let mut warned = false;
    sort_by_ref(host, &args[0], renumber, true, &mut |host, a, b| {
        let operand = |e: &Elem| {
            if by_key {
                e.0.to_value()
            } else {
                deref(&e.1)
            }
        };
        let call = |host: &mut dyn Host, x: Value, y: Value, err: &mut Option<Thrown>| {
            if err.is_some() {
                return None;
            }
            match host.call_callable(&cmp, &[x, y]) {
                Ok(v) => Some(v),
                Err(e) => {
                    *err = Some(e);
                    None
                }
            }
        };
        match call(host, operand(a), operand(b), &mut err) {
            None => 0,
            Some(Value::Bool(v)) => {
                if !warned {
                    warned = true;
                    if let Err(e) = host.emit_diagnostic(
                        crate::E_DEPRECATED,
                        format!(
                            "{fname}(): Returning bool from comparison function is deprecated, return an integer less than, equal to, or greater than zero"
                        )
                        .as_bytes(),
                    ) {
                        err.get_or_insert(e);
                    }
                }
                if v {
                    1
                } else {
                    match call(host, operand(b), operand(a), &mut err) {
                        None => 0,
                        Some(v) => -normalize(conv::to_int(&v)),
                    }
                }
            }
            Some(v) => normalize(conv::to_int(&v)),
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    Ok(Value::Bool(true))
}

fn fn_usort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    usort_impl(host, args, true, false)
}
fn fn_uasort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    usort_impl(host, args, false, false)
}
fn fn_uksort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    usort_impl(host, args, false, true)
}

// -- natural-order sorts (preserve keys) ----------------------------------

/// `natsort(&$array)` / `natcasesort(&$array)`: php_array_natural_compare
/// over the string forms, keys kept.
fn natsort_impl(host: &mut dyn Host, args: &[Value], fold_case: bool) -> Result<Value, Thrown> {
    let flags = SORT_NATURAL | if fold_case { SORT_FLAG_CASE } else { 0 };
    let mut err = None;
    sort_by_ref(host, &args[0], false, false, &mut |host, a, b| {
        data_compare(host, &a.1, &b.1, flags, &mut err)
    });
    if let Some(t) = err.or_else(|| host.take_pending_throw()) {
        return Err(t);
    }
    Ok(Value::Bool(true))
}

fn fn_natsort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    natsort_impl(host, args, false)
}

fn fn_natcasesort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    natsort_impl(host, args, true)
}

// -- array_walk -----------------------------------------------------------

type Cell = std::rc::Rc<std::cell::RefCell<Value>>;

thread_local! {
    /// Ids for array_walk's HashTable iterators.
    static WALK_ITER_ID: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// `array_walk(array|object &$array, callable $callback, mixed $arg)`.
fn fn_array_walk(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    walk_entry(host, args, false)
}

/// `array_walk_recursive(array|object &$array, callable $callback, mixed $arg)`.
fn fn_array_walk_recursive(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    walk_entry(host, args, true)
}

fn walk_entry(host: &mut dyn Host, args: &[Value], recursive: bool) -> Result<Value, Thrown> {
    let cell = match &args[0] {
        Value::Ref(rc) => rc.clone(),
        other => std::rc::Rc::new(std::cell::RefCell::new(other.clone())),
    };
    let v = deref(&args[0]);
    if !matches!(v, Value::Array(_) | Value::Object(_)) {
        return Err(Thrown::type_error(format!(
            "{}(): Argument #1 ($array) must be of type array, {} given",
            if recursive {
                "array_walk_recursive"
            } else {
                "array_walk"
            },
            zval_value_name(host, &v)
        )));
    }
    // Not held during the walk: a second handle would make the walk
    // separate the array.
    drop(v);
    // The third argument is passed on only when given.
    let userdata = args.get(2).map(deref);
    let mut active = Vec::new();
    php_array_walk(
        host,
        &cell,
        &args[1],
        userdata.as_ref(),
        recursive,
        &mut active,
    )?;
    Ok(Value::Bool(true))
}

/// Make the slot a reference (ZVAL_MAKE_REF) and return its cell.
fn make_ref(slot: &mut Value) -> Cell {
    if let Value::Ref(rc) = slot {
        return rc.clone();
    }
    let rc = std::rc::Rc::new(std::cell::RefCell::new(std::mem::replace(
        slot,
        Value::Null,
    )));
    *slot = Value::Ref(rc.clone());
    rc
}

/// The next element of an array walk: its cell and key. The array keeps
/// iterator `id` on the following element, so removals and insertions by
/// the callback are seen (php's HashTable iterator); a different array
/// (the variable was reassigned) is walked from its internal pointer.
fn walk_next_array(a: &mut PhpArray, id: u32) -> Option<(Cell, Value)> {
    let d = a.make_mut();
    let pos = match d.iter_pos(id) {
        Some(p) => p,
        None => d.pos.min(d.entries.len()),
    };
    if pos >= d.entries.len() {
        d.iter_add(id, pos);
        return None;
    }
    d.iter_add(id, pos + 1);
    let (k, slot) = d.entries.get_index_mut(pos).expect("in range");
    Some((make_ref(slot), k.to_value()))
}

/// The next element of an object walk: the property table by mangled name
/// from position `pos` on, skipping engine-internal slots.
fn walk_next_object(
    host: &mut dyn Host,
    h: &rhp_values::ObjectHandle,
    pos: &mut usize,
) -> Option<(Cell, Value)> {
    let mut b = h.borrow_mut();
    let class = b.class;
    while *pos < b.properties.len() {
        let i = *pos;
        *pos += 1;
        let key = b.properties.get_index(i).expect("in range").0.clone();
        if host.is_internal_slot(class, &key) {
            continue;
        }
        let name = host.mangled_prop_name(class, &key);
        let slot = b.properties.get_index_mut(i).expect("in range").1;
        return Some((make_ref(slot), Value::Str(name)));
    }
    None
}

/// php_array_walk over the array or object held in `container`.
fn php_array_walk(
    host: &mut dyn Host,
    container: &Cell,
    cb: &Value,
    userdata: Option<&Value>,
    recursive: bool,
    active: &mut Vec<*const std::cell::RefCell<Value>>,
) -> Result<(), Thrown> {
    crate::zend::call_stack::check()?;
    let id = WALK_ITER_ID.with(|c| {
        let n = c.get().wrapping_add(1);
        c.set(n);
        n
    });
    let mut obj_pos = 0usize;
    // An object walks its own property table (a lazy object initializes).
    // `held`: the object in the container; `obj`: the one whose properties
    // are walked (a lazy object's real instance).
    let mut held = match &*container.borrow() {
        Value::Array(a) if a.is_empty() => return Ok(()),
        Value::Array(_) => None,
        Value::Object(h) => Some(h.clone()),
        _ => return Ok(()),
    };
    let mut obj = match &held {
        Some(h) => Some(host.lazy_props_object(h)?),
        None => {
            if let Value::Array(a) = &mut *container.borrow_mut() {
                a.make_mut().iter_add(id, 0);
            }
            None
        }
    };
    let result = (|| -> Result<(), Thrown> {
        loop {
            let next = match &obj {
                Some(h) => walk_next_object(host, h, &mut obj_pos),
                None => match &mut *container.borrow_mut() {
                    Value::Array(a) => walk_next_array(a, id),
                    _ => None,
                },
            };
            let Some((cell, key)) = next else {
                return Ok(());
            };
            let is_array = matches!(&*cell.borrow(), Value::Array(_));
            if recursive && is_array {
                // GC_PROTECT_RECURSION on the arrays recursed into (not
                // the one the walk started from).
                let p = std::rc::Rc::as_ptr(&cell);
                if active.contains(&p) {
                    return Err(Thrown::error("Error", "Recursion detected"));
                }
                active.push(p);
                let r = php_array_walk(host, &cell, cb, userdata, recursive, active);
                active.pop();
                r?;
            } else {
                let mut call_args = vec![Value::Ref(cell), key];
                if let Some(u) = userdata {
                    call_args.push(u.clone());
                }
                host.call_callable(cb, &call_args)?;
            }
            if let Some(t) = host.take_pending_throw() {
                return Err(t);
            }
            // Reload: the callback may have replaced the variable.
            let now = container.borrow().clone();
            match now {
                Value::Array(_) => {
                    held = None;
                    obj = None;
                }
                Value::Object(h) => {
                    if !held.as_ref().is_some_and(|o| std::rc::Rc::ptr_eq(o, &h)) {
                        obj = Some(host.lazy_props_object(&h)?);
                        held = Some(h);
                        obj_pos = 0;
                    }
                }
                _ => {
                    return Err(Thrown::type_error(
                        "Iterated value is no longer an array or object",
                    ))
                }
            }
        }
    })();
    if let Value::Array(a) = &mut *container.borrow_mut() {
        if a.ref_count() == 1 {
            a.make_mut().iter_del(id);
        }
    }
    result
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

/// Read the array in `arg` without separating it (current/key take it by
/// value).
fn with_array_read<R>(arg: &Value, f: impl FnOnce(&PhpArray) -> R) -> Option<R> {
    match arg {
        Value::Ref(rc) => match &*rc.borrow() {
            Value::Array(a) => Some(f(a)),
            _ => None,
        },
        Value::Array(a) => Some(f(a)),
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
/// property table by mangled name (an ArrayObject's storage is not in it),
/// with `None` for the slot of an uninitialized typed property.
fn object_table(host: &mut dyn Host, h: &rhp_values::ObjectHandle) -> Vec<(Value, Option<Value>)> {
    let mut uninit = host.uninit_props(h).into_iter().peekable();
    let b = h.borrow();
    let mut out = Vec::new();
    let mut seen = 0usize;
    for (k, v) in b.properties.iter() {
        if host.is_internal_slot(b.class, k) {
            continue;
        }
        while let Some((name, _, _)) = uninit.next_if(|u| u.2 == seen) {
            out.push((Value::Str(name), None));
        }
        out.push((
            Value::Str(host.mangled_prop_name(b.class, k)),
            Some(deref(v)),
        ));
        seen += 1;
    }
    for (name, _, _) in uninit {
        out.push((Value::Str(name), None));
    }
    out
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

/// An internal-pointer function on an object's property table
/// (php_array_iter_seek_current: an uninitialized slot is skipped in the
/// direction of travel, moving the pointer).
fn object_ptr(host: &mut dyn Host, h: &rhp_values::ObjectHandle, op: PtrOp) -> Value {
    let table = object_table(host, h);
    let n = table.len();
    let id = h.borrow().id;
    let mut pos = OBJECT_POS
        .with(|m| m.borrow().get(&id).copied().unwrap_or(0))
        .min(n);
    let moves = matches!(op, PtrOp::Reset | PtrOp::End | PtrOp::Next | PtrOp::Prev);
    if moves && n == 0 {
        return Value::Bool(false);
    }
    // zend_hash_move_backwards: from the first slot to past the end.
    let back = |p: usize| if p == 0 || p >= n { n } else { p - 1 };
    match op {
        PtrOp::Reset => pos = 0,
        PtrOp::End => pos = n - 1,
        PtrOp::Next => pos = (pos + 1).min(n),
        PtrOp::Prev => pos = back(pos),
        PtrOp::Current | PtrOp::Key => {}
    }
    let forward = !matches!(op, PtrOp::End | PtrOp::Prev);
    while pos < n && table[pos].1.is_none() {
        pos = if forward { pos + 1 } else { back(pos) };
    }
    OBJECT_POS.with(|m| m.borrow_mut().insert(id, pos));
    match (op, table.get(pos)) {
        (PtrOp::Key, Some((k, _))) => k.clone(),
        (PtrOp::Key, None) => Value::Null,
        (_, Some((_, Some(v)))) => v.clone(),
        (_, _) => Value::Bool(false),
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
        // php stops at the end (nNumUsed), so a later append is current.
        d.pos = d.pos.saturating_add(1).min(d.entries.len());
        elem_at(d, d.pos)
    })
    .unwrap_or(Value::Bool(false)))
}

fn fn_prev(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(h) = ptr_arg(host, "prev", &args[0])? {
        return Ok(object_ptr(host, &h, PtrOp::Prev));
    }
    Ok(with_array_data(&args[0], |d| {
        if d.pos == 0 || d.pos >= d.entries.len() {
            // Before the first element php's pointer is past the end too.
            d.pos = d.entries.len();
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
    Ok(with_array_read(&args[0], |a| {
        a.get_index(a.pos())
            .map(|(_, v)| deref(v).clone())
            .unwrap_or(Value::Bool(false))
    })
    .unwrap_or(Value::Bool(false)))
}

fn fn_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(h) = ptr_arg(host, "key", &args[0])? {
        return Ok(object_ptr(host, &h, PtrOp::Key));
    }
    Ok(with_array_read(&args[0], |a| {
        a.get_index(a.pos())
            .map(|(k, _)| k.to_value())
            .unwrap_or(Value::Null)
    })
    .unwrap_or(Value::Null))
}

fn fn_array_push(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Ref(rc) = &args[0] else {
        return Ok(Value::Bool(false));
    };
    let vals: Vec<Value> = args[1..].iter().map(deref).collect();
    let mut g = rc.borrow_mut();
    let Value::Array(a) = &mut *g else {
        return Ok(Value::Bool(false));
    };
    for v in vals {
        if !a.try_push(v) {
            return Err(cannot_add_element());
        }
    }
    Ok(Value::Int(a.len() as i64))
}

/// Remove the first or last element of the by-reference array in place
/// (other elements keep their references), then run `fix` on what is left.
/// The removed value is dereferenced only afterwards, as php unwraps the
/// returned reference last (it may be the array itself).
fn take_end(
    arg: &Value,
    first: bool,
    fix: impl FnOnce(&mut rhp_values::ArrayData, &ArrayKey),
) -> Value {
    let Value::Ref(rc) = arg else {
        return Value::Null;
    };
    let removed = {
        let mut g = rc.borrow_mut();
        let Value::Array(a) = &mut *g else {
            return Value::Null;
        };
        if a.is_empty() {
            return Value::Null;
        }
        let d = a.make_mut();
        let i = if first { 0 } else { d.entries.len() - 1 };
        let (k, v) = d.remove_index(i).expect("non-empty");
        fix(d, &k);
        d.pos = 0;
        v
    };
    deref(&removed)
}

fn fn_array_pop(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(take_end(&args[0], false, |d, k| {
        if let ArrayKey::Int(h) = *k {
            if d.next_free().checked_sub(1) == Some(h) {
                d.set_next_free(h);
            }
        }
    }))
}

fn fn_array_shift(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(take_end(&args[0], true, |d, _| {
        let old = std::mem::take(&mut d.entries);
        let mut k = 0i64;
        for (key, v) in old {
            match key {
                ArrayKey::Int(_) => {
                    d.entries.insert(ArrayKey::Int(k), v);
                    k += 1;
                }
                s => {
                    d.entries.insert(s, v);
                }
            }
        }
        d.set_next_free(k);
    }))
}

fn fn_array_unshift(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Ref(rc) = &args[0] else {
        return Ok(Value::Bool(false));
    };
    let vals: Vec<Value> = args[1..].iter().map(deref).collect();
    let mut g = rc.borrow_mut();
    let Value::Array(a) = &mut *g else {
        return Ok(Value::Bool(false));
    };
    let argc = vals.len();
    let d = a.make_mut();
    let old = std::mem::take(&mut d.entries);
    let mut out = PhpArray::new();
    for v in vals {
        out.push(v);
    }
    for (k, v) in old {
        match k {
            ArrayKey::Int(_) => out.push(v),
            s => out.insert(s, v),
        }
    }
    let nd = out.make_mut();
    d.entries = std::mem::take(&mut nd.entries);
    d.set_next_free(nd.next_free());
    d.iters_advance(argc);
    d.pos = 0;
    Ok(Value::Int(d.entries.len() as i64))
}

/// `array_splice(&$array, $offset, $length?, $replacement?)`: remove a slice
/// (reindexed), optionally splicing in `$replacement`, and return the removed
/// elements (reindexed). The array is passed by reference (arg 0).
fn fn_array_splice(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Ref(rc) = &args[0] else {
        return Ok(Value::Array(PhpArray::new()));
    };
    // Separated first (Z_PARAM_ARRAY_EX), so in-place removal keeps its identity.
    let arr = match &mut *rc.borrow_mut() {
        Value::Array(a) => {
            a.make_mut();
            a.clone()
        }
        _ => return Ok(Value::Array(PhpArray::new())),
    };
    let num_in = arr.len() as i64;
    let mut offset = conv::to_int(&deref(&args[1]));
    let mut length = match args.get(2).map(deref) {
        None | Some(Value::Null) => num_in,
        Some(v) => conv::to_int(&v),
    };
    // convert_to_array of the replacement (when given).
    let replacement: Vec<Value> = match args.get(3).map(deref) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().map(|(_, v)| v.clone()).collect(),
        // The object's properties (ARRAY_CAST).
        Some(Value::Object(h)) => {
            let purpose = crate::zend::object_handlers::PropPurpose::ArrayCast;
            let a = match crate::zend::object_handlers::properties_for(host, &h, purpose)? {
                Some(a) => a,
                None => host.object_props_mangled(&h),
            };
            a.iter().map(|(_, v)| v.clone()).collect()
        }
        Some(v) => vec![v],
    };
    // php_splice's clamping (its unsigned check is 32-bit).
    if offset > num_in {
        offset = num_in;
    } else if offset < 0 {
        offset = (num_in + offset).max(0);
    }
    if length < 0 {
        length += num_in - offset;
    } else if (offset as u32).wrapping_add(length as u32) > num_in as u32 {
        length = num_in - offset;
    }
    // Without a used result the removed elements are destroyed in place, one
    // by one; a destructor that changes the array makes php throw.
    let unused = host.result_unused();
    let mut out = PhpArray::new();
    let mut removed = PhpArray::new();
    let mut dropped: Vec<ArrayKey> = Vec::new();
    let add = |a: &mut PhpArray, k: &ArrayKey, v: &Value| match k {
        ArrayKey::Int(_) => a.push(v.clone()),
        s => a.insert(s.clone(), v.clone()),
    };
    let mut pos = 0i64;
    let mut it = arr.iter().peekable();
    while pos < offset {
        let Some((k, v)) = it.next() else { break };
        add(&mut out, k, v);
        pos += 1;
    }
    while pos - offset < length {
        let Some((k, v)) = it.next() else { break };
        if unused {
            dropped.push(k.clone());
        } else {
            add(&mut removed, k, v);
        }
        pos += 1;
    }
    for v in replacement {
        out.push(v);
    }
    for (k, v) in it {
        add(&mut out, k, v);
    }
    // HashTable iterators (array_walk) stay on the table, moved the way
    // php_splice moves them.
    let mut iter_moves = arr.iters();
    if !iter_moves.is_empty() {
        let packed = arr
            .iter()
            .try_fold(-1i64, |prev, (k, _)| match k {
                ArrayKey::Int(n) if *n > prev => Some(*n),
                _ => None,
            })
            .is_some();
        let cut = removed.len() + dropped.len();
        splice_iterators(
            &mut iter_moves,
            arr.len(),
            offset as usize,
            cut,
            length,
            out.len() + cut - arr.len(),
            packed,
            !unused,
        );
    }
    let ptr = arr.data_ptr();
    let mut expect_len = arr.len();
    drop(arr);
    let same = |v: &Value, len: usize| matches!(v, Value::Array(a) if a.data_ptr() == ptr && a.len() == len);
    // php holds an extra reference to the table while destructors run
    // (GC_ADDREF), so any write by a destructor separates the variable;
    // `held` is that reference. Once the variable has moved on, the rest
    // are still destroyed from the old table.
    let mut held: Option<PhpArray> = None;
    let mut modified = false;
    for k in dropped {
        let v = {
            let mut g = rc.borrow_mut();
            if !modified && same(&g, expect_len) {
                let Value::Array(a) = &mut *g else {
                    unreachable!()
                };
                drop(held.take());
                expect_len -= 1;
                let v = a.remove(&k);
                held = Some(a.clone());
                v
            } else {
                modified = true;
                held.as_mut().and_then(|h| h.remove(&k))
            }
        };
        drop(v);
        host.run_released_destructors()?;
    }
    if modified || !same(&rc.borrow(), expect_len) {
        return Err(Thrown::error(
            "Error",
            "Array was modified during array_splice operation",
        ));
    }
    if !iter_moves.is_empty() {
        let d = out.make_mut();
        for (id, p) in iter_moves {
            d.iter_add(id, p);
        }
    }
    *rc.borrow_mut() = Value::Array(out);
    Ok(Value::Array(removed))
}

/// php_splice's HashTable iterator updates, on positions: an rhp table has
/// no holes, so a bucket index is taken to be the position (php tables
/// with holes, such as packed arrays with sparse keys, can differ). `n`
/// elements, `cut`
/// removed at `off`, `repl` inserted; `length` is php's clamped length.
/// Deleting the last element shrinks the used part and clamps iterators
/// to it.
/// The packed loop also moves one met in the removed part to
/// `offset + length`; the rest moves each iterator met to its output
/// position, where it can be met again (php's loop does the same).
#[allow(clippy::too_many_arguments)]
fn splice_iterators(
    iters: &mut [(u32, usize)],
    n: usize,
    off: usize,
    cut: usize,
    length: i64,
    repl: usize,
    packed: bool,
    used: bool,
) {
    // zend_hash_iterators_lower_pos: the lowest position >= start, else
    // the used size.
    let lower = |its: &[(u32, usize)], start: usize, num_used: usize| {
        its.iter()
            .map(|e| e.1)
            .filter(|&p| p >= start && p < num_used)
            .min()
            .unwrap_or(num_used)
    };
    let update = |its: &mut [(u32, usize)], from: usize, to: usize| {
        for e in its.iter_mut() {
            if e.1 == from {
                e.1 = to;
            }
        }
    };
    let mut num_used = n;
    let mut iter_pos = lower(iters, 0, num_used);
    let (mut pos, mut idx) = (0usize, 0usize);
    while idx < off {
        if idx == iter_pos {
            iter_pos = lower(iters, iter_pos + 1, num_used);
        }
        pos += 1;
        idx += 1;
    }
    while idx < off + cut && idx < num_used {
        if used {
            pos += 1;
        }
        if idx + 1 == num_used {
            // Everything from `off` on is deleted now.
            num_used = off;
            for e in iters.iter_mut() {
                e.1 = e.1.min(num_used);
            }
        }
        if packed && idx == iter_pos {
            update(iters, idx, (off as i64 + length) as usize);
            iter_pos = lower(iters, iter_pos + 1, num_used);
        }
        idx += 1;
    }
    if !packed {
        iter_pos = lower(iters, iter_pos, num_used);
    }
    pos += repl;
    while idx < num_used {
        if idx == iter_pos {
            if idx != pos {
                update(iters, idx, pos);
            }
            iter_pos = lower(iters, iter_pos + 1, num_used);
        }
        pos += 1;
        idx += 1;
    }
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
            out.insert(k.clone(), copy_elem(v));
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
            out.insert(k.clone(), copy_elem(v));
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

fn recursion_detected() -> Thrown {
    Thrown::error("Error", "Recursion detected")
}

/// convert_to_array of an element php_array_merge_recursive merges into or
/// from: null gives `[null]` (dest only), an object its properties, a
/// scalar a one-element list.
fn merge_to_array(host: &mut dyn Host, v: Value, null_elem: bool) -> Result<PhpArray, Thrown> {
    Ok(match v {
        Value::Array(a) => a,
        Value::Null => {
            let mut a = PhpArray::new();
            if null_elem {
                a.push(Value::Null);
            }
            a
        }
        Value::Object(h) => {
            let purpose = crate::zend::object_handlers::PropPurpose::ArrayCast;
            match crate::zend::object_handlers::properties_for(host, &h, purpose)? {
                Some(a) => a,
                None => host.object_props_mangled(&h),
            }
        }
        other => {
            let mut a = PhpArray::new();
            a.push(other);
            a
        }
    })
}

/// php_array_merge_recursive. `guard` holds the arrays being merged into
/// further up (GC_PROTECT_RECURSION).
fn merge_recursive_into(
    host: &mut dyn Host,
    dest: &mut PhpArray,
    src: &PhpArray,
    guard: &mut Vec<usize>,
) -> Result<(), Thrown> {
    crate::zend::call_stack::check()?;
    for (k, src_entry) in src.iter() {
        if let ArrayKey::Int(_) = k {
            if !dest.try_push(copy_elem(src_entry)) {
                return Err(cannot_add_element());
            }
            continue;
        }
        let Some(dest_entry) = dest.get(k).cloned() else {
            dest.insert(k.clone(), copy_elem(src_entry));
            continue;
        };
        let dest_val = deref(&dest_entry);
        let thash = match &dest_val {
            Value::Array(a) => Some(a.data_ptr()),
            _ => None,
        };
        if thash.is_some_and(|p| guard.contains(&p)) {
            return Err(recursion_detected());
        }
        // convert_to_array + SEPARATE_ZVAL: a reference gives up its value.
        let mut cur = merge_to_array(host, dest_val, true)?;
        let src_val = match deref(src_entry) {
            o @ Value::Object(_) => Value::Array(merge_to_array(host, o, false)?),
            v => v,
        };
        match src_val {
            Value::Array(sa) => {
                if let Some(p) = thash {
                    guard.push(p);
                }
                let r = merge_recursive_into(host, &mut cur, &sa, guard);
                if thash.is_some() {
                    guard.pop();
                }
                r?;
            }
            v => {
                if !cur.try_push(v) {
                    return Err(cannot_add_element());
                }
            }
        }
        dest.insert(k.clone(), Value::Array(cur));
    }
    Ok(())
}

/// The first array of array_merge(_recursive): refcount-1 references
/// unwrapped, integer keys renumbered.
fn merge_first(a: &PhpArray) -> PhpArray {
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        match k {
            ArrayKey::Int(_) => out.push(copy_elem(v)),
            ArrayKey::Str(_) => out.insert(k.clone(), copy_elem(v)),
        }
    }
    out
}

fn merge_total_check(arrays: &[PhpArray]) -> Result<(), Thrown> {
    let count: u64 = arrays.iter().map(|a| a.len() as u64).sum();
    if count >= HT_MAX_SIZE {
        return Err(Thrown::error(
            "Error",
            format!("The total number of elements must be lower than {HT_MAX_SIZE}"),
        ));
    }
    Ok(())
}

fn fn_array_merge_recursive(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arrays = (0..args.len())
        .map(|i| array_param(host, args, i, "array_merge_recursive"))
        .collect::<Result<Vec<_>, _>>()?;
    let Some(first) = arrays.first() else {
        return Ok(Value::Array(PhpArray::new()));
    };
    merge_total_check(&arrays)?;
    let mut out = merge_first(first);
    let mut guard = Vec::new();
    for a in &arrays[1..] {
        merge_recursive_into(host, &mut out, a, &mut guard)?;
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
    crate::ext::random::shuffle(&mut vals, &mut |lo, hi| Ok(mt_range(lo, hi)))?;
    let mut out = PhpArray::new();
    for v in vals {
        out.push(v);
    }
    // Shuffled in place: iterators (array_walk) keep their positions.
    for (id, p) in arr.iters() {
        out.make_mut().iter_add(id, p);
    }
    *r.borrow_mut() = Value::Array(out);
    Ok(Value::Bool(true))
}

/// `array_rand(array $array, int $num = 1)` (php_array_pick_keys).
fn fn_array_rand(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arr = crate::zend::zpp::array_param(host, args, 0, "array_rand")?;
    let num = int_param(host, args, 1, "array_rand", "num", 1)?;
    match crate::ext::random::pick_keys(&arr, num, &mut |lo, hi| Ok(mt_range(lo, hi)))? {
        Ok(v) => Ok(v),
        Err(crate::ext::random::PickError::Empty) => Err(Thrown::error(
            "ValueError",
            "array_rand(): Argument #1 ($array) must not be empty",
        )),
        Err(crate::ext::random::PickError::Num) => Err(Thrown::error(
            "ValueError",
            "array_rand(): Argument #2 ($num) must be between 1 and the number of elements in argument #1 ($array)",
        )),
    }
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
    // The arrays as they are now: php sorts these hash tables and writes
    // the result into them, even if a variable is reassigned meanwhile.
    let tables: Vec<PhpArray> = arrays
        .iter()
        .map(|&(slot, _, _)| match deref(&args[slot]) {
            Value::Array(a) => a,
            _ => PhpArray::new(),
        })
        .collect();
    let size = tables[0].len();
    if tables.iter().any(|c| c.len() != size) {
        return Err(Thrown::error("ValueError", "Array sizes are inconsistent"));
    }
    if size == 0 {
        return Ok(Value::Bool(true));
    }
    let cols: Vec<Vec<(ArrayKey, Value)>> = tables
        .iter()
        .map(|a| a.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .collect();
    let mut order: Vec<usize> = (0..size).collect();
    let mut err = None;
    // php_multisort_compare with the unstable comparators.
    zend_sort(&mut order, &mut |&x, &y| {
        for (c, &(_, ty, desc)) in arrays.iter().enumerate() {
            let r = data_compare(host, &cols[c][x].1, &cols[c][y].1, ty, &mut err);
            let r = if desc { -r } else { r };
            if r != 0 {
                return r.signum();
            }
        }
        ord_i32(x.cmp(&y))
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
        out.make_mut().set_next_free(size as i64);
        if let Value::Ref(r) = &args[slot] {
            let mut g = r.borrow_mut();
            if let Value::Array(a) = &*g {
                if a.data_ptr() == tables[c].data_ptr() {
                    // Sorted in place: iterators (array_walk) keep their
                    // positions.
                    for (id, p) in a.iters() {
                        out.make_mut().iter_add(id, p);
                    }
                    *g = Value::Array(out);
                }
            }
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
        ("EXTR_OVERWRITE", Value::Int(0)),
        ("EXTR_SKIP", Value::Int(1)),
        ("EXTR_PREFIX_SAME", Value::Int(2)),
        ("EXTR_PREFIX_ALL", Value::Int(3)),
        ("EXTR_PREFIX_INVALID", Value::Int(4)),
        ("EXTR_PREFIX_IF_EXISTS", Value::Int(5)),
        ("EXTR_IF_EXISTS", Value::Int(6)),
        ("EXTR_REFS", Value::Int(0x100)),
        ("COUNT_NORMAL", Value::Int(0)),
        ("COUNT_RECURSIVE", Value::Int(1)),
        ("SORT_REGULAR", Value::Int(SORT_REGULAR)),
        ("SORT_NUMERIC", Value::Int(SORT_NUMERIC)),
        ("SORT_STRING", Value::Int(SORT_STRING)),
        ("SORT_DESC", Value::Int(SORT_DESC)),
        ("SORT_ASC", Value::Int(SORT_ASC)),
        ("SORT_LOCALE_STRING", Value::Int(SORT_LOCALE_STRING)),
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
            ("extract", fn_extract),
            ("compact", fn_compact),
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

// ---------------------------------------------------------------------------
// extract / compact
// ---------------------------------------------------------------------------

const EXTR_OVERWRITE: i64 = 0;
const EXTR_SKIP: i64 = 1;
const EXTR_PREFIX_SAME: i64 = 2;
const EXTR_PREFIX_ALL: i64 = 3;
const EXTR_PREFIX_INVALID: i64 = 4;
const EXTR_PREFIX_IF_EXISTS: i64 = 5;
const EXTR_IF_EXISTS: i64 = 6;
const EXTR_REFS: i64 = 0x100;

/// php_valid_var_name: `[a-zA-Z_\x7f-\xff][a-zA-Z0-9_\x7f-\xff]*`.
fn valid_var_name(name: &[u8]) -> bool {
    let first = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c >= 0x7f;
    match name.split_first() {
        Some((&c, rest)) => first(c) && rest.iter().all(|&c| first(c) || c.is_ascii_digit()),
        None => false,
    }
}

/// php_prefix_varname with the underscore.
fn prefix_varname(prefix: &[u8], name: &[u8]) -> Vec<u8> {
    [prefix, b"_", name].concat()
}

fn cannot_reassign_this() -> Thrown {
    Thrown::error("Error", "Cannot re-assign $this")
}

/// How extract() finds a name in the scope's symbol table: not there, a
/// compiled variable that is undefined, or a defined variable.
#[derive(PartialEq)]
enum Found {
    Missing,
    Undef,
    Set,
}

fn find_var(host: &mut dyn Host, name: &[u8]) -> Found {
    match host.scope_var(name) {
        crate::ScopeVar::Missing => Found::Missing,
        crate::ScopeVar::Undef => Found::Undef,
        crate::ScopeVar::Set(_) => Found::Set,
    }
}

/// The array extract() reads, and with EXTR_REFS the caller's variable
/// holding it, whose elements become references as they are bound.
struct ExtractSource {
    arr: PhpArray,
    cell: Option<std::rc::Rc<std::cell::RefCell<Value>>>,
}

impl ExtractSource {
    /// Element `key` as a reference (ZVAL_MAKE_REF on the array element).
    fn element_ref(&mut self, key: &ArrayKey) -> std::rc::Rc<std::cell::RefCell<Value>> {
        let make = |slot: &mut Value| match slot {
            Value::Ref(rc) => rc.clone(),
            v => {
                let rc =
                    std::rc::Rc::new(std::cell::RefCell::new(std::mem::replace(v, Value::Null)));
                *v = Value::Ref(rc.clone());
                rc
            }
        };
        if let Some(cell) = &self.cell {
            if let Value::Array(a) = &mut *cell.borrow_mut() {
                if let Some(slot) = a.get_mut(key) {
                    return make(slot);
                }
            }
        }
        match self.arr.get_mut(key) {
            Some(slot) => make(slot),
            None => std::rc::Rc::new(std::cell::RefCell::new(Value::Null)),
        }
    }
}

/// Bind or assign `name` from element `key`: a reference to the element
/// with EXTR_REFS, else a copy of its value.
fn extract_put(
    host: &mut dyn Host,
    src: &mut ExtractSource,
    refs: bool,
    name: &[u8],
    key: &ArrayKey,
    entry: &Value,
) -> Result<(), Thrown> {
    if refs {
        let rc = src.element_ref(key);
        host.scope_bind(name, rc);
        Ok(())
    } else {
        host.scope_assign(name, deref(entry))
    }
}

/// `extract(array &$array, int $flags = EXTR_OVERWRITE, string $prefix = "")`.
fn fn_extract(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let flags = args
        .get(1)
        .map_or(EXTR_OVERWRITE, |v| conv::to_int(&deref(v)));
    let prefix = args.get(2).map(|v| conv::to_string_bytes(&deref(v)));
    let refs = flags & EXTR_REFS != 0;
    let ty = flags & 0xff;
    if !(EXTR_OVERWRITE..=EXTR_IF_EXISTS).contains(&ty) {
        return Err(Thrown::error(
            "ValueError",
            "extract(): Argument #2 ($flags) must be a valid extract type",
        ));
    }
    if ty > EXTR_SKIP && ty <= EXTR_PREFIX_IF_EXISTS && prefix.is_none() {
        return Err(Thrown::error(
            "ValueError",
            "extract(): Argument #3 ($prefix) is required when using this extract type",
        ));
    }
    let prefix = prefix.unwrap_or_default();
    if !prefix.is_empty() && !valid_var_name(&prefix) {
        return Err(Thrown::error(
            "ValueError",
            "extract(): Argument #3 ($prefix) must be a valid identifier",
        ));
    }
    zend::builtin_functions::forbid_dynamic_call(host, "extract")?;
    let cell = match &args[0] {
        Value::Ref(rc) if refs => Some(rc.clone()),
        _ => None,
    };
    let arr = match deref(&args[0]) {
        Value::Array(a) => a,
        _ => PhpArray::new(),
    };
    let entries: Vec<(ArrayKey, Value)> = arr.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut src = ExtractSource { arr, cell };
    let mut count = 0i64;
    for (key, entry) in &entries {
        let name: Option<&[u8]> = match key {
            ArrayKey::Str(s) => Some(s.as_bytes()),
            ArrayKey::Int(_) => None,
        };
        // The name a prefixed mode binds, when valid.
        let prefixed = |n: &[u8]| Some(prefix_varname(&prefix, n)).filter(|f| valid_var_name(f));
        let target: Option<Vec<u8>> = match ty {
            EXTR_OVERWRITE | EXTR_IF_EXISTS => {
                let Some(n) = name else { continue };
                let found = find_var(host, n);
                if ty == EXTR_IF_EXISTS && found != Found::Set {
                    continue;
                }
                if !valid_var_name(n) {
                    continue;
                }
                if ty == EXTR_IF_EXISTS && n == b"GLOBALS" {
                    continue;
                }
                if n == b"this" {
                    return Err(cannot_reassign_this());
                }
                if found != Found::Missing && n == b"GLOBALS" {
                    continue;
                }
                Some(n.to_vec())
            }
            EXTR_SKIP => {
                let Some(n) = name else { continue };
                if !valid_var_name(n) || n == b"this" {
                    continue;
                }
                if find_var(host, n) == Found::Set {
                    continue;
                }
                Some(n.to_vec())
            }
            EXTR_PREFIX_SAME | EXTR_PREFIX_IF_EXISTS => {
                let Some(n) = name else { continue };
                if ty == EXTR_PREFIX_SAME && n.is_empty() {
                    continue;
                }
                match find_var(host, n) {
                    Found::Undef => Some(n.to_vec()),
                    Found::Set => match prefixed(n) {
                        Some(f) if f == b"this" => return Err(cannot_reassign_this()),
                        f => f,
                    },
                    Found::Missing if ty == EXTR_PREFIX_IF_EXISTS => continue,
                    Found::Missing => {
                        if !valid_var_name(n) {
                            continue;
                        }
                        if n == b"this" {
                            match prefixed(n) {
                                Some(f) if f == b"this" => return Err(cannot_reassign_this()),
                                f => f,
                            }
                        } else {
                            Some(n.to_vec())
                        }
                    }
                }
            }
            EXTR_PREFIX_ALL => {
                let n = match key {
                    ArrayKey::Str(s) if s.is_empty() => continue,
                    ArrayKey::Str(s) => s.as_bytes().to_vec(),
                    ArrayKey::Int(i) => i.to_string().into_bytes(),
                };
                match prefixed(&n) {
                    Some(f) if f == b"this" => return Err(cannot_reassign_this()),
                    f => f,
                }
            }
            EXTR_PREFIX_INVALID => {
                let f = match key {
                    ArrayKey::Str(s)
                        if !valid_var_name(s.as_bytes()) || s.as_bytes() == b"this" =>
                    {
                        prefixed(s.as_bytes())
                    }
                    ArrayKey::Str(s) => Some(s.as_bytes().to_vec()),
                    ArrayKey::Int(i) => prefixed(i.to_string().as_bytes()),
                };
                match f {
                    Some(f) if f == b"this" => return Err(cannot_reassign_this()),
                    f => f,
                }
            }
            _ => unreachable!("extract type checked above"),
        };
        let Some(target) = target else { continue };
        extract_put(host, &mut src, refs, &target, key, entry)?;
        count += 1;
    }
    Ok(Value::Int(count))
}

/// `compact(array|string $var_name, array|string ...$var_names)`.
fn fn_compact(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    zend::builtin_functions::forbid_dynamic_call(host, "compact")?;
    let mut out = PhpArray::new();
    let mut active = Vec::new();
    for (i, a) in args.iter().enumerate() {
        compact_var(host, &mut out, a, i + 1, &mut active)?;
    }
    Ok(Value::Array(out))
}

/// php_compact_var: a name, or an array of names (nested), into `out`.
fn compact_var(
    host: &mut dyn Host,
    out: &mut PhpArray,
    entry: &Value,
    pos: usize,
    active: &mut Vec<usize>,
) -> Result<(), Thrown> {
    match deref(entry) {
        Value::Str(name) => match host.scope_var(name.as_bytes()) {
            crate::ScopeVar::Set(v) => out.insert(ArrayKey::from_bytes(name.as_bytes()), v),
            _ if name.as_bytes() == b"this" => {
                if let Some(this) = host.scope_this() {
                    out.insert(ArrayKey::from_bytes(b"this"), this);
                }
            }
            _ => {
                let msg = [b"Undefined variable $".as_slice(), name.as_bytes()].concat();
                warn(host, "compact", &msg)?;
            }
        },
        Value::Array(a) => {
            crate::zend::call_stack::check()?;
            let p = a.data_ptr();
            if !a.is_empty() {
                if active.contains(&p) {
                    return Err(Thrown::error("Error", "Recursion detected"));
                }
                active.push(p);
            }
            let r = a
                .iter()
                .try_for_each(|(_, v)| compact_var(host, out, v, pos, active));
            if !a.is_empty() {
                active.pop();
            }
            r?;
        }
        v => {
            let msg = format!(
                "Argument #{pos} must be string or array of strings, {} given",
                zval_value_name(host, &v)
            );
            warn(host, "compact", &msg)?;
        }
    }
    Ok(())
}
