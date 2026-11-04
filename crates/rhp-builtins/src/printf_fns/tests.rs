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
