//! By-reference array builtins: the sort family and the stack/queue operations.
//!
//! These take their array argument **by reference** (`array &$array`): the
//! compiler emits a `MakeRef` for arg 0, so the builtin receives a
//! `Value::Ref` and mutates the backing array in place. The `u*` variants
//! invoke a user comparator through the [`Host`] callback hook.

use std::cmp::Ordering;

use rhp_values::ops::{self, deref};
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::{Host, Thrown};

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
        SORT_NATURAL => crate::strcmp_fns::strnatcmp_ex(
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
        crate::strcmp_fns::strnatcmp_ex(
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
            crate::argparse::zval_value_name(host, &other)
        ))),
    }
}

thread_local! {
    /// The internal pointer of each object's property table, by object id.
    static OBJECT_POS: std::cell::RefCell<std::collections::HashMap<u32, usize>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Forget a freed object's internal pointer.
pub(crate) fn object_freed(id: u32) {
    let _ = OBJECT_POS.try_with(|m| m.borrow_mut().remove(&id));
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
fn fn_array_splice(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
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
    let replacement: Vec<Value> = match args.get(3) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().map(|(_, v)| deref(v).clone()).collect(),
        Some(Value::Ref(r)) => match &*r.borrow() {
            Value::Array(a) => a.iter().map(|(_, v)| deref(v).clone()).collect(),
            other => vec![deref(other).clone()],
        },
        Some(v) => vec![deref(v).clone()],
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

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
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
        ],
    );
}
