//! `vprintf`, `fprintf` and `vfprintf`, over the sprintf core in
//! [`crate::str_fns::do_sprintf`] (`sprintf`, `printf` and `vsprintf` live
//! in `str_fns`). `fprintf`/`vfprintf` write to the stream argument.

use std::collections::HashMap;

use rhp_values::{conv, Value};

use crate::str_fns::do_sprintf;
use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

/// Deref a (possibly `Ref`) value to an owned value.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// `(string)` bytes of a value (after deref).
fn bytes_of(v: &Value) -> Vec<u8> {
    conv::to_string_bytes(&deref(v))
}

/// Pull the values out of an `array` argument (the `$values` of a `v*printf`),
/// or raise the php `TypeError` naming the function and argument position.
fn array_values(v: &Value, fname: &str, argno: usize) -> Result<Vec<Value>, Thrown> {
    match deref(v) {
        Value::Array(arr) => Ok(arr.iter().map(|(_, v)| v.clone()).collect()),
        _ => Err(Thrown::type_error(format!(
            "{fname}(): Argument #{argno} ($values) must be of type array"
        ))),
    }
}

// ---------------------------------------------------------------------------
// vprintf
// ---------------------------------------------------------------------------

/// `vprintf(string $format, array $values): int` — array-arg form of `printf`.
fn fn_vprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = bytes_of(&args[0]);
    let values = array_values(&args[1], "vprintf", 2)?;
    // `given` mimics the equivalent printf() call: format + each value.
    let out = do_sprintf(&fmt, &values, values.len() + 1)?;
    let len = out.len() as i64;
    host.write_out(&out);
    Ok(Value::Int(len))
}

// ---------------------------------------------------------------------------
// fprintf / vfprintf
// ---------------------------------------------------------------------------

/// `fprintf(resource $stream, string $format, mixed ...$values): int` — the
/// formatted string is written to `$stream`; the formatted length is returned
/// whether or not the write succeeds (php_stream_write's result is ignored).
fn fn_fprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = crate::fs_fns::stream_param(host, args, 0, "fprintf", "stream")?;
    let fmt = bytes_of(&args[1]);
    // Argument numbering follows php: the format is arg 2, values start at arg 3,
    // so the error-reporting `given` excludes the stream argument.
    let out = do_sprintf(&fmt, &args[2..], args.len() - 1)?;
    let len = out.len() as i64;
    host.stream_write(&h, &out);
    Ok(Value::Int(len))
}

/// `vfprintf(resource $stream, string $format, array $values): int` — array-arg
/// form of `fprintf`.
fn fn_vfprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = crate::fs_fns::stream_param(host, args, 0, "vfprintf", "stream")?;
    let fmt = bytes_of(&args[1]);
    let values = array_values(&args[2], "vfprintf", 3)?;
    let out = do_sprintf(&fmt, &values, values.len() + 1)?;
    let len = out.len() as i64;
    host.stream_write(&h, &out);
    Ok(Value::Int(len))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register the printf output-family builtins into the shared table.
///
/// `sprintf`/`printf`/`vsprintf` are registered by `str_fns` and must NOT be
/// re-registered here; this module adds `vprintf`, `fprintf` and `vfprintf`.
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
    put(m, "vprintf", fn_vprintf, 2, Some(2));
    put(m, "fprintf", fn_fprintf, 2, None);
    put(m, "vfprintf", fn_vfprintf, 3, Some(3));
}

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rhp_values::PhpArray;

    /// A host that captures everything written to output, for byte checks.
    #[derive(Default)]
    struct CapHost {
        out: Vec<u8>,
    }
    impl Host for CapHost {
        fn write_out(&mut self, bytes: &[u8]) {
            self.out.extend_from_slice(bytes);
        }
    }

    fn s(b: &str) -> Value {
        Value::str(b)
    }

    fn arr(vals: Vec<Value>) -> Value {
        let mut a = PhpArray::new();
        for v in vals {
            a.push(v);
        }
        Value::Array(a)
    }

    /// Call an output-family fn: return (captured output string, int return).
    fn run(f: crate::NativeFn, args: &[Value]) -> (String, i64) {
        let mut host = CapHost::default();
        let ret = f(&mut host, args).expect("no throw");
        let n = match ret {
            Value::Int(n) => n,
            other => panic!("expected int, got {}", other.type_name()),
        };
        (String::from_utf8(host.out).unwrap(), n)
    }

    #[test]
    fn vprintf_writes_and_returns_len() {
        // php: vprintf("%s/%s/%d", ["a","b",7]) -> "a/b/7", returns 5
        let (out, n) = run(
            fn_vprintf,
            &[s("%s/%s/%d"), arr(vec![s("a"), s("b"), Value::Int(7)])],
        );
        assert_eq!(out, "a/b/7");
        assert_eq!(n, 5);

        // php: vprintf("%2\$s-%1\$s", ["one","two"]) -> "two-one", returns 7
        let (out, n) = run(fn_vprintf, &[s("%2$s-%1$s"), arr(vec![s("one"), s("two")])]);
        assert_eq!(out, "two-one");
        assert_eq!(n, 7);
    }

    /// fprintf/vfprintf write to their stream (covered by the golden tests);
    /// a non-stream first argument is php's TypeError.
    #[test]
    fn fprintf_family_requires_a_stream() {
        let mut host = CapHost::default();
        let err = |r: Result<Value, Thrown>| match r {
            Err(Thrown::Error { message, .. }) => message,
            Err(other) => panic!("unexpected {other:?}"),
            Ok(_) => panic!("expected a TypeError"),
        };
        let m = err(fn_fprintf(
            &mut host,
            &[Value::Int(2), s("[%+d]"), Value::Int(9)],
        ));
        assert_eq!(
            m,
            "fprintf(): Argument #1 ($stream) must be of type resource, int given"
        );
        let m = err(fn_vfprintf(
            &mut host,
            &[Value::Null, s("%s"), arr(vec![s("x")])],
        ));
        assert_eq!(
            m,
            "vfprintf(): Argument #1 ($stream) must be of type resource, null given"
        );
        assert!(host.out.is_empty());
    }

    #[test]
    fn varray_arg_must_be_array() {
        let mut host = CapHost::default();
        assert!(fn_vprintf(&mut host, &[s("%s"), s("notarray")]).is_err());
        assert!(fn_vfprintf(&mut host, &[Value::Int(2), s("%s"), s("notarray")]).is_err());
    }

    #[test]
    fn format_errors_propagate() {
        let mut host = CapHost::default();
        // unknown specifier -> ValueError from the shared core
        assert!(fn_vprintf(&mut host, &[s("%i"), arr(vec![Value::Int(1)])]).is_err());
        // too few arguments -> ArgumentCountError from the shared core
        assert!(fn_vprintf(&mut host, &[s("%s %s"), arr(vec![s("a")])]).is_err());
    }
}
