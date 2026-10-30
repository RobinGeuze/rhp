//! Array set-operation and aggregate extras (no by-reference args):
//! `array_replace_recursive`, `array_diff_assoc`, `array_intersect_assoc`,
//! `array_find`, `array_find_key`, `array_any`, `array_all`, and the
//! `key_exists` alias of `array_key_exists`.
//!
//! The `array_find` family (PHP 8.4+) is callback-based: the callback is
//! invoked with `(value, key)` in that order — verified against php 8.5.8.
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see the `#[cfg(test)]` block — expected values hardcoded from
//! `php -r '...'` output). Collisions with existing modules are skipped:
//! `array_sum`, `array_product`, `in_array`, `array_key_exists` live in
//! `lib.rs`; `array_replace` and `array_merge_recursive` live in
//! `strarr2_fns.rs`.
//!
//! Wire this module into `lib.rs` with:
//! ```ignore
//! mod arrayset_fns;                 // near the other `mod` lines
//! // ...and inside registry(), after building `m`:
//! arrayset_fns::register(&mut m);
//! ```

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
    put(m, "array_diff_assoc", fn_array_diff_assoc, 2, None);
    put(
        m,
        "array_intersect_assoc",
        fn_array_intersect_assoc,
        2,
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

// ---------------------------------------------------------------------------
// tests (expected values captured from php 8.5.8)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Host that dispatches `call_callable` to a boxed Rust closure, so the
    /// callback-based functions can be exercised as unit tests.
    struct ClosureHost<'a> {
        #[allow(clippy::type_complexity)]
        cb: Box<dyn FnMut(&[Value]) -> Value + 'a>,
    }
    impl Host for ClosureHost<'_> {
        fn write_out(&mut self, _: &[u8]) {}
        fn call_callable(&mut self, _callable: &Value, args: &[Value]) -> Result<Value, Thrown> {
            Ok((self.cb)(args))
        }
    }

    struct NullHost;
    impl Host for NullHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    fn s(b: &str) -> Value {
        Value::str(b)
    }
    fn i(n: i64) -> Value {
        Value::Int(n)
    }
    fn dumped(v: &Value) -> String {
        let mut out = Vec::new();
        crate::dump::var_dump(&NullHost, &mut out, v, 0);
        String::from_utf8_lossy(&out).into_owned()
    }
    fn map(pairs: Vec<(&str, Value)>) -> Value {
        let mut a = PhpArray::new();
        for (k, v) in pairs {
            a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
        }
        Value::Array(a)
    }
    /// An array like `['a'=>'green', 'b'=>'brown', 'c'=>'blue', 0=>'red']`.
    fn mixed(str_pairs: Vec<(&str, Value)>, tail: Vec<Value>) -> Value {
        let mut a = PhpArray::new();
        for (k, v) in str_pairs {
            a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
        }
        for v in tail {
            a.push(v);
        }
        Value::Array(a)
    }

    // --- array_diff_assoc -----------------------------------------------
    // php -r '$a=["a"=>"green","b"=>"brown","c"=>"blue","red"];
    //         $b=["a"=>"green","yellow","red"]; var_dump(array_diff_assoc($a,$b));'
    #[test]
    fn array_diff_assoc_cases() {
        let a = mixed(
            vec![("a", s("green")), ("b", s("brown")), ("c", s("blue"))],
            vec![s("red")],
        );
        let b = mixed(vec![("a", s("green"))], vec![s("yellow"), s("red")]);
        assert_eq!(
            dumped(&fn_array_diff_assoc(&mut NullHost, &[a, b]).unwrap()),
            "array(3) {\n  [\"b\"]=>\n  string(5) \"brown\"\n  [\"c\"]=>\n  string(4) \"blue\"\n  [0]=>\n  string(3) \"red\"\n}\n"
        );
        // numeric/string values compared by their string form: 1 == "1".
        let x = map(vec![("x", i(1)), ("y", i(2))]);
        let y = map(vec![("x", s("1")), ("y", s("3"))]);
        assert_eq!(
            dumped(&fn_array_diff_assoc(&mut NullHost, &[x, y]).unwrap()),
            "array(1) {\n  [\"y\"]=>\n  int(2)\n}\n"
        );
    }

    // --- array_intersect_assoc ------------------------------------------
    #[test]
    fn array_intersect_assoc_cases() {
        let a = mixed(
            vec![("a", s("green")), ("b", s("brown")), ("c", s("blue"))],
            vec![s("red")],
        );
        let b = mixed(
            vec![("a", s("green")), ("b", s("yellow"))],
            vec![s("blue"), s("red")],
        );
        assert_eq!(
            dumped(&fn_array_intersect_assoc(&mut NullHost, &[a, b]).unwrap()),
            "array(1) {\n  [\"a\"]=>\n  string(5) \"green\"\n}\n"
        );
        let x = map(vec![("x", i(1)), ("y", i(2))]);
        let y = map(vec![("x", s("1")), ("y", s("3"))]);
        assert_eq!(
            dumped(&fn_array_intersect_assoc(&mut NullHost, &[x, y]).unwrap()),
            "array(1) {\n  [\"x\"]=>\n  int(1)\n}\n"
        );
    }

    // --- array_replace_recursive ----------------------------------------
    #[test]
    fn array_replace_recursive_cases() {
        // nested arrays merge key-by-key; non-overlapping keys survive.
        let mut a1 = PhpArray::new();
        a1.insert(ArrayKey::Int(0), map(vec![("a", i(1)), ("b", i(2))]));
        a1.insert(ArrayKey::Int(1), s("x"));
        let mut a2 = PhpArray::new();
        a2.insert(ArrayKey::Int(0), map(vec![("b", i(9)), ("c", i(3))]));
        a2.insert(ArrayKey::Int(2), s("y"));
        assert_eq!(
            dumped(&fn_array_replace_recursive(&mut NullHost, &[Value::Array(a1), Value::Array(a2)]).unwrap()),
            "array(3) {\n  [0]=>\n  array(3) {\n    [\"a\"]=>\n    int(1)\n    [\"b\"]=>\n    int(9)\n    [\"c\"]=>\n    int(3)\n  }\n  [1]=>\n  string(1) \"x\"\n  [2]=>\n  string(1) \"y\"\n}\n"
        );
        // a scalar replacement overwrites an existing array outright.
        let base = map(vec![("k", map(vec![("a", i(1))]))]);
        let rep = map(vec![("k", s("str"))]);
        assert_eq!(
            dumped(&fn_array_replace_recursive(&mut NullHost, &[base, rep]).unwrap()),
            "array(1) {\n  [\"k\"]=>\n  string(3) \"str\"\n}\n"
        );
    }

    // --- array_find family ----------------------------------------------
    // Callback is invoked with (value, key); verified against php 8.5.8.
    fn abc() -> Value {
        map(vec![("x", i(10)), ("y", i(20)), ("z", i(30))])
    }

    #[test]
    fn array_find_returns_first_value() {
        let mut host = ClosureHost {
            cb: Box::new(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 15)),
        };
        let r = fn_array_find(&mut host, &[abc(), Value::Null]).unwrap();
        assert_eq!(dumped(&r), "int(20)\n");
        // no match -> null
        let mut host2 = ClosureHost {
            cb: Box::new(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 100)),
        };
        assert_eq!(
            dumped(&fn_array_find(&mut host2, &[abc(), Value::Null]).unwrap()),
            "NULL\n"
        );
    }

    #[test]
    fn array_find_passes_value_then_key() {
        // First invocation must receive [value=10, key="x"].
        let mut first: Option<(Value, Value)> = None;
        {
            let first_ref = &mut first;
            let mut host = ClosureHost {
                cb: Box::new(move |a: &[Value]| {
                    if first_ref.is_none() {
                        *first_ref = Some((a[0].clone(), a[1].clone()));
                    }
                    Value::Bool(false)
                }),
            };
            let _ = fn_array_find(&mut host, &[abc(), Value::Null]).unwrap();
        }
        let (v, k) = first.unwrap();
        assert_eq!(dumped(&v), "int(10)\n");
        assert_eq!(dumped(&k), "string(1) \"x\"\n");
    }

    #[test]
    fn array_find_key_returns_key() {
        let mut host = ClosureHost {
            cb: Box::new(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 15)),
        };
        assert_eq!(
            dumped(&fn_array_find_key(&mut host, &[abc(), Value::Null]).unwrap()),
            "string(1) \"y\"\n"
        );
        let mut host2 = ClosureHost {
            cb: Box::new(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 100)),
        };
        assert_eq!(
            dumped(&fn_array_find_key(&mut host2, &[abc(), Value::Null]).unwrap()),
            "NULL\n"
        );
    }

    #[test]
    fn array_any_and_all() {
        let mut h = ClosureHost {
            cb: Box::new(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 25)),
        };
        assert_eq!(
            dumped(&fn_array_any(&mut h, &[abc(), Value::Null]).unwrap()),
            "bool(true)\n"
        );
        let mut h2 = ClosureHost {
            cb: Box::new(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 5)),
        };
        assert_eq!(
            dumped(&fn_array_all(&mut h2, &[abc(), Value::Null]).unwrap()),
            "bool(true)\n"
        );
        let mut h3 = ClosureHost {
            cb: Box::new(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 15)),
        };
        assert_eq!(
            dumped(&fn_array_all(&mut h3, &[abc(), Value::Null]).unwrap()),
            "bool(false)\n"
        );
    }

    // --- key_exists ------------------------------------------------------
    #[test]
    fn key_exists_cases() {
        let a = map(vec![("a", i(1)), ("b", i(2))]);
        assert_eq!(
            dumped(&fn_key_exists(&mut NullHost, &[s("a"), a.clone()]).unwrap()),
            "bool(true)\n"
        );
        assert_eq!(
            dumped(&fn_key_exists(&mut NullHost, &[s("zz"), a]).unwrap()),
            "bool(false)\n"
        );
    }
}
