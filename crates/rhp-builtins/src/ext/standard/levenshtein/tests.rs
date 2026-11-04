use super::*;
use crate::test_support::TestHost;

fn s(b: &str) -> Value {
    Value::str(b)
}

fn i(n: i64) -> Value {
    Value::Int(n)
}

fn call(f: crate::NativeFn, args: &[Value]) -> Value {
    f(&mut TestHost::new(), args).expect("no throw")
}

fn call_int(f: crate::NativeFn, args: &[Value]) -> i64 {
    match call(f, args) {
        Value::Int(n) => n,
        other => panic!("expected int, got {}", other.type_name()),
    }
}

#[test]
fn levenshtein_cases() {
    assert_eq!(call_int(fn_levenshtein, &[s("kitten"), s("sitting")]), 3);
    assert_eq!(call_int(fn_levenshtein, &[s(""), s("abc")]), 3);
    assert_eq!(call_int(fn_levenshtein, &[s("abc"), s("")]), 3);
    assert_eq!(call_int(fn_levenshtein, &[s("abc"), s("abc")]), 0);
    assert_eq!(call_int(fn_levenshtein, &[s("flaw"), s("lawn")]), 2);
    assert_eq!(call_int(fn_levenshtein, &[s("gumbo"), s("gambol")]), 2);
    assert_eq!(call_int(fn_levenshtein, &[s("Hello"), s("hello")]), 1);
    // weighted costs: ins, rep, del
    assert_eq!(
        call_int(
            fn_levenshtein,
            &[s("kitten"), s("sitting"), i(1), i(2), i(1)]
        ),
        5
    );
    assert_eq!(
        call_int(fn_levenshtein, &[s("abc"), s("xyz"), i(10), i(1), i(10)]),
        3
    );
    assert_eq!(
        call_int(fn_levenshtein, &[s("ab"), s("abcd"), i(2), i(5), i(3)]),
        4
    );
}
