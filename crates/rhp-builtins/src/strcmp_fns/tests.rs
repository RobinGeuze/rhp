// Expected values hardcoded from local php 8.5.8.

use super::*;
use crate::test_support::TestHost;
use rhp_values::PhpArray;

fn s(b: &str) -> Value {
    Value::str(b)
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
fn calls(f: crate::NativeFn, args: &[Value]) -> String {
    String::from_utf8(bytes_of(&call(f, args))).unwrap()
}
fn throws(f: crate::NativeFn, args: &[Value]) -> bool {
    f(&mut TestHost::new(), args).is_err()
}

#[test]
fn strcmp_cases() {
    assert_eq!(call_int(fn_strcmp, &[s("a"), s("b")]), -1);
    assert_eq!(call_int(fn_strcmp, &[s("b"), s("a")]), 1);
    assert_eq!(call_int(fn_strcmp, &[s("a"), s("a")]), 0);
    assert_eq!(call_int(fn_strcmp, &[s("abc"), s("abd")]), -1);
    assert_eq!(call_int(fn_strcmp, &[s("Hello"), s("hello")]), -32);
    assert_eq!(call_int(fn_strcmp, &[s("a"), s("ab")]), -1);
    assert_eq!(call_int(fn_strcmp, &[s("abc"), s("abcde")]), -1);
    assert_eq!(call_int(fn_strcmp, &[s("abcde"), s("abc")]), 1);
    assert_eq!(call_int(fn_strcmp, &[s("10"), s("9")]), -8);
    // raw byte diff on single high byte: 0xff - 0x01 = 254
    assert_eq!(
        call_int(
            fn_strcmp,
            &[Value::str(vec![0xffu8]), Value::str(vec![0x01u8])]
        ),
        254
    );
}

#[test]
fn strcasecmp_cases() {
    assert_eq!(call_int(fn_strcasecmp, &[s("Hello"), s("hello")]), 0);
    assert_eq!(call_int(fn_strcasecmp, &[s("a"), s("B")]), -1);
    assert_eq!(call_int(fn_strcasecmp, &[s("B"), s("a")]), 1);
    assert_eq!(call_int(fn_strcasecmp, &[s("A"), s("z")]), -25);
    assert_eq!(call_int(fn_strcasecmp, &[s("Z"), s("a")]), 25);
    assert_eq!(call_int(fn_strcasecmp, &[s("ABC"), s("abcd")]), -1);
}

#[test]
fn strncmp_cases() {
    assert_eq!(
        call_int(fn_strncmp, &[s("Hello"), s("Help"), Value::Int(3)]),
        0
    );
    assert_eq!(
        call_int(fn_strncmp, &[s("Hello"), s("Help"), Value::Int(4)]),
        -4
    );
    assert_eq!(
        call_int(fn_strncmp, &[s("abc"), s("abd"), Value::Int(0)]),
        0
    );
    assert_eq!(
        call_int(fn_strncmp, &[s("abc"), s("abcde"), Value::Int(10)]),
        -1
    );
    assert_eq!(
        call_int(fn_strncmp, &[s("abcde"), s("abc"), Value::Int(10)]),
        1
    );
    assert_eq!(
        call_int(fn_strncmp, &[s("abc"), s("abcde"), Value::Int(3)]),
        0
    );
    assert!(throws(fn_strncmp, &[s("a"), s("b"), Value::Int(-1)]));
}

#[test]
fn strncasecmp_cases() {
    assert_eq!(
        call_int(fn_strncasecmp, &[s("Hello"), s("hello"), Value::Int(5)]),
        0
    );
    assert_eq!(
        call_int(fn_strncasecmp, &[s("Hello"), s("help"), Value::Int(3)]),
        0
    );
    assert_eq!(
        call_int(fn_strncasecmp, &[s("AX"), s("aZ"), Value::Int(2)]),
        -2
    );
    assert!(throws(fn_strncasecmp, &[s("a"), s("b"), Value::Int(-1)]));
}

#[test]
fn substr_compare_cases() {
    assert_eq!(
        call_int(
            fn_substr_compare,
            &[s("Hello World"), s("World"), Value::Int(6)]
        ),
        0
    );
    assert_eq!(
        call_int(
            fn_substr_compare,
            &[
                s("Hello World"),
                s("world"),
                Value::Int(6),
                Value::Int(5),
                Value::Bool(true)
            ]
        ),
        0
    );
    assert_eq!(
        call_int(fn_substr_compare, &[s("abcde"), s("bc"), Value::Int(1)]),
        1
    );
    assert_eq!(
        call_int(
            fn_substr_compare,
            &[s("abcde"), s("bc"), Value::Int(1), Value::Int(2)]
        ),
        0
    );
    assert_eq!(
        call_int(fn_substr_compare, &[s("abcde"), s("fg"), Value::Int(-2)]),
        -2
    );
    assert_eq!(
        call_int(
            fn_substr_compare,
            &[s("abcde"), s("abcXY"), Value::Int(0), Value::Int(5)]
        ),
        12
    );
    assert_eq!(
        call_int(fn_substr_compare, &[s("abc"), s("abcde"), Value::Int(0)]),
        -1
    );
    assert_eq!(
        call_int(
            fn_substr_compare,
            &[s("abcde"), s("abc"), Value::Int(0), Value::Int(10)]
        ),
        1
    );
    assert_eq!(
        call_int(
            fn_substr_compare,
            &[s("abcde"), s("XYZ"), Value::Int(1), Value::Int(0)]
        ),
        0
    );
    assert_eq!(
        call_int(fn_substr_compare, &[s("abc"), s("b"), Value::Int(3)]),
        -1
    );
    assert_eq!(
        call_int(fn_substr_compare, &[s("abc"), s("a"), Value::Int(-10)]),
        1
    );
    assert_eq!(
        call_int(
            fn_substr_compare,
            &[
                s("Hello"),
                s("hel"),
                Value::Int(0),
                Value::Int(3),
                Value::Bool(true)
            ]
        ),
        0
    );
    assert_eq!(
        call_int(
            fn_substr_compare,
            &[
                s("Hello"),
                s("hel"),
                Value::Int(0),
                Value::Int(5),
                Value::Bool(true)
            ]
        ),
        1
    );
    // errors
    assert!(throws(
        fn_substr_compare,
        &[s("abc"), s("b"), Value::Int(5)]
    ));
    assert!(throws(
        fn_substr_compare,
        &[s("abcde"), s("XYZ"), Value::Int(1), Value::Int(-1)]
    ));
}

#[test]
fn strnatcmp_cases() {
    assert_eq!(call_int(fn_strnatcmp, &[s("img10"), s("img2")]), 1);
    assert_eq!(call_int(fn_strnatcmp, &[s("img2"), s("img10")]), -1);
    assert_eq!(call_int(fn_strnatcmp, &[s("img12"), s("img12")]), 0);
    assert_eq!(call_int(fn_strnatcmp, &[s("1.10"), s("1.2")]), 1);
    assert_eq!(call_int(fn_strnatcmp, &[s("10"), s("9")]), 1);
    assert_eq!(call_int(fn_strnatcmp, &[s("a0"), s("a00")]), -1);
    assert_eq!(call_int(fn_strnatcmp, &[s("a10"), s("a10b")]), -1);
    assert_eq!(call_int(fn_strnatcmp, &[s("  a"), s("a")]), 0);
    assert_eq!(call_int(fn_strnatcmp, &[s("0001"), s("1")]), 0);
    assert_eq!(call_int(fn_strnatcmp, &[s("file1"), s("file10")]), -1);
    assert_eq!(call_int(fn_strnatcmp, &[s("ver1.2.3"), s("ver1.2.10")]), -1);
    assert_eq!(call_int(fn_strnatcmp, &[s("x"), s("X")]), 1);
    assert_eq!(call_int(fn_strnatcmp, &[s("100"), s("99")]), 1);
    assert_eq!(call_int(fn_strnatcmp, &[s("T-5"), s("T-50")]), -1);
    assert_eq!(call_int(fn_strnatcmp, &[s("a1b2"), s("a1b10")]), -1);
    assert_eq!(call_int(fn_strnatcmp, &[s("z"), s("z1")]), -1);
    assert_eq!(call_int(fn_strnatcmp, &[s(""), s("a")]), -1);
    assert_eq!(call_int(fn_strnatcmp, &[s("a"), s("")]), 1);
    assert_eq!(call_int(fn_strnatcmp, &[s(""), s("")]), 0);
}

#[test]
fn strnatcasecmp_cases() {
    assert_eq!(call_int(fn_strnatcasecmp, &[s("IMG10"), s("img2")]), 1);
    assert_eq!(call_int(fn_strnatcasecmp, &[s("Img10"), s("img10")]), 0);
    assert_eq!(call_int(fn_strnatcasecmp, &[s("x"), s("X")]), 0);
}

#[test]
fn str_ireplace_cases() {
    assert_eq!(
        calls(fn_str_ireplace, &[s("WORLD"), s("PHP"), s("Hello world")]),
        "Hello PHP"
    );
    assert_eq!(
        calls(fn_str_ireplace, &[s("o"), s("0"), s("FOO Boo")]),
        "F00 B00"
    );
    assert_eq!(calls(fn_str_ireplace, &[s("aa"), s("b"), s("aaaa")]), "bb");
    assert_eq!(calls(fn_str_ireplace, &[s(""), s("X"), s("abc")]), "abc");
    // array search + array replace
    let se = {
        let mut a = PhpArray::new();
        a.push(s("a"));
        a.push(s("B"));
        Value::Array(a)
    };
    let re = {
        let mut a = PhpArray::new();
        a.push(s("X"));
        a.push(s("Y"));
        Value::Array(a)
    };
    assert_eq!(calls(fn_str_ireplace, &[se, re, s("aAbB")]), "XXYY");
    // array search + scalar replace
    let se2 = {
        let mut a = PhpArray::new();
        a.push(s("a"));
        a.push(s("b"));
        Value::Array(a)
    };
    assert_eq!(calls(fn_str_ireplace, &[se2, s("Z"), s("abAB")]), "ZZZZ");
    // scalar search + array replace -> TypeError
    let re2 = {
        let mut a = PhpArray::new();
        a.push(s("b"));
        Value::Array(a)
    };
    assert!(throws(fn_str_ireplace, &[s("a"), re2, s("abc")]));
    // array subject
    let subj = {
        let mut a = PhpArray::new();
        a.push(s("aA"));
        a.push(s("bca"));
        Value::Array(a)
    };
    let out = call(fn_str_ireplace, &[s("a"), s("X"), subj]);
    let Value::Array(arr) = out else {
        panic!("want array")
    };
    let got: Vec<String> = arr
        .iter()
        .map(|(_, v)| String::from_utf8(bytes_of(v)).unwrap())
        .collect();
    assert_eq!(got, vec!["XX".to_string(), "bcX".to_string()]);
}

#[test]
fn strpbrk_cases() {
    assert_eq!(calls(fn_strpbrk, &[s("Hello World"), s("oW")]), "o World");
    assert_eq!(
        calls(fn_strpbrk, &[s("This is a test"), s("st")]),
        "s is a test"
    );
    assert_eq!(calls(fn_strpbrk, &[s("abcdef"), s("a")]), "abcdef");
    assert!(matches!(
        call(fn_strpbrk, &[s("Hello"), s("xyz")]),
        Value::Bool(false)
    ));
    assert!(throws(fn_strpbrk, &[s("abc"), s("")]));
}

#[test]
fn strspn_cases() {
    assert_eq!(
        call_int(fn_strspn, &[s("42 is the answer"), s("1234567890")]),
        2
    );
    assert_eq!(call_int(fn_strspn, &[s("abcdef"), s("abc")]), 3);
    assert_eq!(
        call_int(fn_strspn, &[s("foo123bar"), s("0123456789"), Value::Int(3)]),
        3
    );
    assert_eq!(
        call_int(
            fn_strspn,
            &[
                s("foo123bar"),
                s("0123456789"),
                Value::Int(3),
                Value::Int(2)
            ]
        ),
        2
    );
    assert_eq!(
        call_int(
            fn_strspn,
            &[s("12345"), s("123"), Value::Int(0), Value::Int(10)]
        ),
        3
    );
    assert_eq!(
        call_int(fn_strspn, &[s("abcdef"), s("abc"), Value::Int(-3)]),
        0
    );
    assert_eq!(
        call_int(fn_strspn, &[s("abc"), s("abc"), Value::Int(10)]),
        0
    );
    assert_eq!(
        call_int(
            fn_strspn,
            &[s("aaa"), s("a"), Value::Int(1), Value::Int(10)]
        ),
        2
    );
}

#[test]
fn strcspn_cases() {
    assert_eq!(call_int(fn_strcspn, &[s("hello, world"), s("o")]), 4);
    assert_eq!(call_int(fn_strcspn, &[s("hello"), s("xyz")]), 5);
    assert_eq!(
        call_int(
            fn_strcspn,
            &[s("hello world"), s("lo"), Value::Int(2), Value::Int(5)]
        ),
        0
    );
    assert_eq!(
        call_int(fn_strcspn, &[s("hello"), s("xyz"), Value::Int(-2)]),
        2
    );
    assert_eq!(call_int(fn_strcspn, &[s("abcABC"), s("abc")]), 0);
}

#[test]
fn str_rot13_cases() {
    assert_eq!(calls(fn_str_rot13, &[s("Hello World")]), "Uryyb Jbeyq");
    assert_eq!(calls(fn_str_rot13, &[s("PHP 8.5")]), "CUC 8.5");
    assert_eq!(calls(fn_str_rot13, &[s("Az09!~")]), "Nm09!~");
    assert_eq!(calls(fn_str_rot13, &[s("mMnN zZ")]), "zZaA mM");
}
