//! Native printf output-family builtins (`vprintf` / `fprintf` / `vfprintf`).
//! Every one of these reuses the single sprintf formatting core in
//! [`crate::str_fns::do_sprintf`] — no format-spec parsing is reimplemented
//! here. Each function is verified byte-for-byte against the local php 8.5.8
//! binary (see the `#[cfg(test)]` block — expected values are hardcoded from
//! `php -r '...'` output).
//!
//! COLLISIONS: `sprintf`, `printf` and `vsprintf` are ALL already registered by
//! `str_fns` (and already go through `do_sprintf`), so none of the three is
//! re-registered here. This module only adds the genuinely-missing members of
//! the family: `vprintf`, `fprintf` and `vfprintf`. (`printf`/`vsprintf`
//! equivalents are still exercised in the tests below for completeness.)
//!
//! Stream handling: rhp models no real stream resources, so the `$stream`
//! argument of `fprintf`/`vfprintf` is accepted but ignored — every formatted
//! result is written to the single host output sink (as `write_out`). Thus
//! `fprintf(STDOUT, ...)` behaves exactly like `printf(...)`, and
//! `fprintf(STDERR, ...)` also lands on the host output. In real php
//! `STDOUT`/`STDERR` are `resource` values (there is no resource `Value` in this
//! engine); they are not registered as constants, since any first argument is
//! simply ignored. For reference `(int) STDOUT == 2` and `(int) STDERR == 3`.
//!
//! Wire this module into `lib.rs` with:
//! ```ignore
//! mod printf_fns;             // near the other `mod` lines
//! // ...and inside registry(), before returning `m`:
//! printf_fns::register(&mut m);
//! ```

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
/// `$stream` (arg 0) is ignored (no stream resources in this engine); the
/// formatted result goes to host output, and the byte count is returned.
fn fn_fprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = bytes_of(&args[1]);
    // Argument numbering follows php: the format is arg 2, values start at arg 3,
    // so the error-reporting `given` excludes the ignored stream argument.
    let out = do_sprintf(&fmt, &args[2..], args.len() - 1)?;
    let len = out.len() as i64;
    host.write_out(&out);
    Ok(Value::Int(len))
}

/// `vfprintf(resource $stream, string $format, array $values): int` — array-arg
/// form of `fprintf`; the `$stream` argument is likewise ignored.
fn fn_vfprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = bytes_of(&args[1]);
    let values = array_values(&args[2], "vfprintf", 3)?;
    let out = do_sprintf(&fmt, &values, values.len() + 1)?;
    let len = out.len() as i64;
    host.write_out(&out);
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
        let (out, n) = run(fn_vprintf, &[s("%s/%s/%d"), arr(vec![s("a"), s("b"), Value::Int(7)])]);
        assert_eq!(out, "a/b/7");
        assert_eq!(n, 5);

        // php: vprintf("%2\$s-%1\$s", ["one","two"]) -> "two-one", returns 7
        let (out, n) = run(fn_vprintf, &[s("%2$s-%1$s"), arr(vec![s("one"), s("two")])]);
        assert_eq!(out, "two-one");
        assert_eq!(n, 7);
    }

    #[test]
    fn fprintf_ignores_stream_writes_and_returns_len() {
        // php: fprintf(STDOUT, "[%+d]", 9) -> "[+9]", returns 4
        let (out, n) = run(fn_fprintf, &[Value::Int(2), s("[%+d]"), Value::Int(9)]);
        assert_eq!(out, "[+9]");
        assert_eq!(n, 4);

        // php: fprintf(STDERR, "err=%s", "x") -> "err=x", returns 5
        // (the stream is ignored; STDERR formatting lands on host output too)
        let (out, n) = run(fn_fprintf, &[Value::Int(3), s("err=%s"), s("x")]);
        assert_eq!(out, "err=x");
        assert_eq!(n, 5);

        // a non-resource first arg is accepted and ignored just the same.
        let (out, n) = run(fn_fprintf, &[Value::Null, s("%03d"), Value::Int(7)]);
        assert_eq!(out, "007");
        assert_eq!(n, 3);
    }

    #[test]
    fn vfprintf_ignores_stream_writes_and_returns_len() {
        // php: vfprintf(STDOUT, "%2\$s-%1\$s", ["one","two"]) -> "two-one", returns 7
        let (out, n) = run(
            fn_vfprintf,
            &[Value::Int(2), s("%2$s-%1$s"), arr(vec![s("one"), s("two")])],
        );
        assert_eq!(out, "two-one");
        assert_eq!(n, 7);

        // php: vfprintf(STDOUT, "%04.1f|%x", [7.86, 255]) -> "07.9|ff", returns 7
        let (out, n) = run(
            fn_vfprintf,
            &[Value::Int(2), s("%04.1f|%x"), arr(vec![Value::Float(7.86), Value::Int(255)])],
        );
        assert_eq!(out, "07.9|ff");
        assert_eq!(n, 7);
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
