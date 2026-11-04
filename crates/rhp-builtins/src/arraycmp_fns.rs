//! The user-comparator members of the `array_(u)diff` / `array_(u)intersect`
//! family: `array_udiff`, `array_uintersect`, `array_diff_ukey`,
//! `array_intersect_ukey`, `array_diff_uassoc`, `array_intersect_uassoc`,
//! `array_udiff_uassoc`, `array_uintersect_uassoc` (php's `php_array_diff` /
//! `php_array_intersect`: sort every array with the comparator, then merge),
//! and `array_udiff_assoc`, `array_uintersect_assoc` (php's
//! `php_array_diff_key` / `php_array_intersect_key`: hash lookup by key, then
//! the user value comparator).
//!
//! The sort-and-merge algorithm is ported step for step (including
//! `zend_sort`), because both the result for inconsistent comparators and the
//! sequence of callback invocations are observable.

use std::collections::HashSet;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::argparse::zval_value_name;
use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// zend_sort (Zend/zend_sort.c): hybrid insertion sort / quicksort
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

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
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
        ],
    );
}

#[cfg(test)]
mod tests;
