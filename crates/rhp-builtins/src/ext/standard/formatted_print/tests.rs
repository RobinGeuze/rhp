// Expected values captured from php 8.5.8.

use super::*;
use crate::test_support::TestHost;
use rhp_values::PhpArray;

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
    let mut host = TestHost::new();
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
    let mut host = TestHost::new();
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
    let mut host = TestHost::new();
    assert!(fn_vprintf(&mut host, &[s("%s"), s("notarray")]).is_err());
    assert!(fn_vfprintf(&mut host, &[Value::Int(2), s("%s"), s("notarray")]).is_err());
}

#[test]
fn format_errors_propagate() {
    let mut host = TestHost::new();
    // unknown specifier -> ValueError from the shared core
    assert!(fn_vprintf(&mut host, &[s("%i"), arr(vec![Value::Int(1)])]).is_err());
    // too few arguments -> ArgumentCountError from the shared core
    assert!(fn_vprintf(&mut host, &[s("%s %s"), arr(vec![s("a")])]).is_err());
}

fn call(f: crate::NativeFn, args: &[Value]) -> Value {
    f(&mut TestHost::new(), args).expect("no throw")
}

fn calls(f: crate::NativeFn, args: &[Value]) -> String {
    String::from_utf8(rhp_values::conv::to_string_bytes(&call(f, args))).unwrap()
}

fn call_int(f: crate::NativeFn, args: &[Value]) -> i64 {
    match call(f, args) {
        Value::Int(n) => n,
        other => panic!("expected int, got {}", other.type_name()),
    }
}

#[test]
fn sprintf_basic() {
    assert_eq!(calls(fn_sprintf, &[s("%s"), s("hello")]), "hello");
    assert_eq!(calls(fn_sprintf, &[s("%d"), Value::Int(42)]), "42");
    assert_eq!(calls(fn_sprintf, &[s("%05d"), Value::Int(42)]), "00042");
    assert_eq!(calls(fn_sprintf, &[s("%-5d|"), Value::Int(42)]), "42   |");
    assert_eq!(
        calls(fn_sprintf, &[s("%+d %+d"), Value::Int(42), Value::Int(-42)]),
        "+42 -42"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%5.2f"), Value::Float(3.14159)]),
        " 3.14"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%.2f"), Value::Float(3.14159)]),
        "3.14"
    );
    assert_eq!(calls(fn_sprintf, &[s("%-10s|"), s("hi")]), "hi        |");
    assert_eq!(calls(fn_sprintf, &[s("%10s|"), s("hi")]), "        hi|");
    assert_eq!(
        calls(fn_sprintf, &[s("%x %X"), Value::Int(255), Value::Int(255)]),
        "ff FF"
    );
    assert_eq!(calls(fn_sprintf, &[s("%o"), Value::Int(8)]), "10");
    assert_eq!(calls(fn_sprintf, &[s("%b"), Value::Int(5)]), "101");
    assert_eq!(calls(fn_sprintf, &[s("%c"), Value::Int(65)]), "A");
    assert_eq!(calls(fn_sprintf, &[s("%%")]), "%");
}

#[test]
fn sprintf_advanced() {
    assert_eq!(
        calls(fn_sprintf, &[s("%08.2f"), Value::Float(-3.14159)]),
        "-0003.14"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%+08.2f"), Value::Float(3.14159)]),
        "+0003.14"
    );
    assert_eq!(calls(fn_sprintf, &[s("%5.3s|"), s("hello")]), "  hel|");
    assert_eq!(
        calls(fn_sprintf, &[s("%'*10d|"), Value::Int(42)]),
        "********42|"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%'*10d|"), Value::Int(-42)]),
        "*******-42|"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%'*-10d|"), Value::Int(42)]),
        "42********|"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%010.3f"), Value::Float(-1.5)]),
        "-00001.500"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%x"), Value::Int(-1)]),
        "ffffffffffffffff"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%u"), Value::Int(-1)]),
        "18446744073709551615"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%b"), Value::Int(-1)]),
        "1".repeat(64)
    );
    assert_eq!(calls(fn_sprintf, &[s("%c"), Value::Int(321)]), "A");
    assert_eq!(calls(fn_sprintf, &[s("%5c|"), Value::Int(65)]), "A|");
    assert_eq!(calls(fn_sprintf, &[s("%.0f"), Value::Float(2.5)]), "2");
    assert_eq!(calls(fn_sprintf, &[s("%.0f"), Value::Float(3.5)]), "4");
    assert_eq!(calls(fn_sprintf, &[s("%-05d|"), Value::Int(-42)]), "-42  |");
    assert_eq!(calls(fn_sprintf, &[s("%010s|"), s("hi")]), "00000000hi|");
    // positional
    assert_eq!(
        calls(fn_sprintf, &[s("%1$s %2$s %1$s"), s("a"), s("b")]),
        "a b a"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%2$03d-%1$s"), s("x"), Value::Int(5)]),
        "005-x"
    );
}

#[test]
fn sprintf_floats() {
    assert_eq!(
        calls(fn_sprintf, &[s("%e"), Value::Float(12345.678)]),
        "1.234568e+4"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%E"), Value::Float(12345.678)]),
        "1.234568E+4"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%e"), Value::Float(0.0)]),
        "0.000000e+0"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%.2e"), Value::Float(12345.678)]),
        "1.23e+4"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%g"), Value::Float(12345.678)]),
        "12345.7"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%g"), Value::Float(0.00001)]),
        "1.0e-5"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%g"), Value::Float(1000000.0)]),
        "1.0e+6"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%g"), Value::Float(1234567.0)]),
        "1.23457e+6"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%.3g"), Value::Float(12345.0)]),
        "1.23e+4"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%f"), Value::Float(f64::INFINITY)]),
        "INF"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%f"), Value::Float(f64::NEG_INFINITY)]),
        "-INF"
    );
    assert_eq!(calls(fn_sprintf, &[s("%f"), Value::Float(f64::NAN)]), "NaN");
    assert_eq!(
        calls(fn_sprintf, &[s("%f"), Value::Float(-0.0)]),
        "0.000000"
    );
    assert_eq!(
        calls(fn_sprintf, &[s("%.2f"), Value::Float(-0.001)]),
        "-0.00"
    );
}

#[test]
fn sprintf_differential() {
    // sweep captured from php 8.5.8
    let f = fn_sprintf;
    assert_eq!(
        calls(
            f,
            &[
                s("%5.2f|%-5.2f|%+.1f"),
                Value::Float(1.5),
                Value::Float(2.5),
                Value::Float(3.0)
            ]
        ),
        " 1.50|2.50 |+3.0"
    );
    assert_eq!(calls(f, &[s("%08b"), Value::Int(5)]), "00000101");
    assert_eq!(calls(f, &[s("%'.10d"), Value::Int(42)]), "........42");
    assert_eq!(calls(f, &[s("%.3d"), Value::Int(5)]), "5"); // precision ignored for int
    assert_eq!(calls(f, &[s("%5.2e"), Value::Float(1234.5)]), "1.23e+3");
    assert_eq!(
        calls(f, &[s("%-10.3e|"), Value::Float(0.000123)]),
        "1.230e-4  |"
    );
    assert_eq!(
        calls(
            f,
            &[
                s("%d|%d|%d"),
                Value::Bool(true),
                Value::Bool(false),
                Value::Null
            ]
        ),
        "1|0|0"
    );
    assert_eq!(
        calls(
            f,
            &[
                s("%s|%s"),
                Value::Float(3.14159265358979),
                Value::Float(1000000.0)
            ]
        ),
        "3.1415926535898|1000000"
    );
    assert_eq!(
        calls(f, &[s("%X %x"), Value::Int(-255), Value::Int(255)]),
        "FFFFFFFFFFFFFF01 ff"
    );
    assert_eq!(
        calls(
            f,
            &[s("%c%c%c"), Value::Int(72), Value::Int(73), Value::Int(33)]
        ),
        "HI!"
    );
    assert_eq!(calls(f, &[s("%06.2f%%"), Value::Float(3.14159)]), "003.14%");
    assert_eq!(calls(f, &[s("%'02d"), Value::Int(7)]), "07");
    assert_eq!(
        calls(f, &[s("%2$s=%1$d"), Value::Int(42), s("answer")]),
        "answer=42"
    );
    assert_eq!(calls(f, &[s("%+08.3f"), Value::Float(-1.5)]), "-001.500");
    assert_eq!(
        calls(
            f,
            &[
                s("%g %g %g"),
                Value::Float(0.00001),
                Value::Float(123456.0),
                Value::Float(0.1)
            ]
        ),
        "1.0e-5 123456 0.1"
    );
    assert_eq!(
        calls(f, &[s("%.10g"), Value::Float(3.14159265358979)]),
        "3.141592654"
    );
    assert_eq!(
        calls(f, &[s("%o %o"), Value::Int(64), Value::Int(-1)]),
        "100 1777777777777777777777"
    );
}

#[test]
fn sprintf_errors() {
    assert!(fn_sprintf(&mut TestHost::new(), &[s("%i"), Value::Int(1)]).is_err());
    assert!(fn_sprintf(&mut TestHost::new(), &[s("abc%")]).is_err());
    assert!(fn_sprintf(&mut TestHost::new(), &[s("%s %s"), s("a")]).is_err());
}

#[test]
fn vsprintf_and_printf() {
    let vals = {
        let mut a = PhpArray::new();
        a.push(s("a"));
        a.push(s("b"));
        Value::Array(a)
    };
    assert_eq!(calls(fn_vsprintf, &[s("%s-%s"), vals]), "a-b");
    // printf returns the written length
    assert_eq!(call_int(fn_printf, &[s("%s!"), s("hi")]), 3);
}
