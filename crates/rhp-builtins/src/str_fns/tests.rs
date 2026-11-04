// Expected values captured from php 8.5.8.
#![allow(clippy::approx_constant)] // deliberate literal test values, not PI

use super::*;
use crate::ext::standard::math::fn_number_format;
use crate::test_support::TestHost;

fn s(b: &str) -> Value {
    Value::str(b)
}
fn call(f: crate::NativeFn, args: &[Value]) -> Value {
    f(&mut TestHost::new(), args).expect("no throw")
}
fn calls(f: crate::NativeFn, args: &[Value]) -> String {
    String::from_utf8(rhp_values::conv::to_string_bytes(&call(f, args))).unwrap()
}
/// Result as `int|false` (strpos family): `None` == php `false`.
fn call_pos(f: crate::NativeFn, args: &[Value]) -> Option<i64> {
    match call(f, args) {
        Value::Int(n) => Some(n),
        Value::Bool(false) => None,
        other => panic!("expected int|false, got {}", other.type_name()),
    }
}
fn call_int(f: crate::NativeFn, args: &[Value]) -> i64 {
    match call(f, args) {
        Value::Int(n) => n,
        other => panic!("expected int, got {}", other.type_name()),
    }
}
fn call_bool(f: crate::NativeFn, args: &[Value]) -> bool {
    match call(f, args) {
        Value::Bool(b) => b,
        other => panic!("expected bool, got {}", other.type_name()),
    }
}

#[test]
fn substr_cases() {
    assert_eq!(
        calls(fn_substr, &[s("Hello World"), Value::Int(6)]),
        "World"
    );
    assert_eq!(
        calls(fn_substr, &[s("Hello World"), Value::Int(-5)]),
        "World"
    );
    assert_eq!(
        calls(
            fn_substr,
            &[s("Hello World"), Value::Int(0), Value::Int(-6)]
        ),
        "Hello"
    );
    assert_eq!(
        calls(
            fn_substr,
            &[s("Hello World"), Value::Int(-5), Value::Int(-1)]
        ),
        "Worl"
    );
    assert_eq!(calls(fn_substr, &[s("Hello"), Value::Int(10)]), "");
    assert_eq!(calls(fn_substr, &[s("Hello"), Value::Int(-10)]), "Hello");
    assert_eq!(
        calls(fn_substr, &[s("Hello"), Value::Int(2), Value::Int(-10)]),
        ""
    );
    assert_eq!(
        calls(fn_substr, &[s("abc"), Value::Int(-1), Value::Int(-1)]),
        ""
    );
}

#[test]
fn str_replace_cases() {
    assert_eq!(
        calls(fn_str_replace, &[s("a"), s("b"), s("banana")]),
        "bbnbnb"
    );
    let search = {
        let mut a = PhpArray::new();
        a.push(s("a"));
        a.push(s("b"));
        Value::Array(a)
    };
    let repl = {
        let mut a = PhpArray::new();
        a.push(s("x"));
        a.push(s("y"));
        Value::Array(a)
    };
    assert_eq!(
        calls(fn_str_replace, &[search.clone(), repl, s("abcabc")]),
        "xycxyc"
    );
    assert_eq!(
        calls(fn_str_replace, &[search, s("z"), s("abcabc")]),
        "zzczzc"
    );
    // chained
    let se = {
        let mut a = PhpArray::new();
        a.push(s("a"));
        a.push(s("b"));
        Value::Array(a)
    };
    let re = {
        let mut a = PhpArray::new();
        a.push(s("b"));
        a.push(s("c"));
        Value::Array(a)
    };
    assert_eq!(calls(fn_str_replace, &[se, re, s("a")]), "c");
}

#[test]
fn strpos_family() {
    assert_eq!(call_pos(fn_strpos, &[s("Hello World"), s("o")]), Some(4));
    assert_eq!(
        call_pos(fn_strpos, &[s("Hello World"), s("o"), Value::Int(5)]),
        Some(7)
    );
    assert_eq!(call_pos(fn_strpos, &[s("Hello World"), s("x")]), None);
    assert_eq!(call_pos(fn_stripos, &[s("Hello World"), s("O")]), Some(4));
    assert_eq!(call_pos(fn_strrpos, &[s("Hello World"), s("o")]), Some(7));
    assert_eq!(
        call_pos(fn_strrpos, &[s("Hello World"), s("o"), Value::Int(-3)]),
        Some(7)
    );
    assert_eq!(
        call_pos(fn_strrpos, &[s("Hello World"), s("o"), Value::Int(-5)]),
        Some(4)
    );
    assert_eq!(
        call_pos(fn_strrpos, &[s("abcabcabc"), s("abc"), Value::Int(-4)]),
        Some(3)
    );
    assert_eq!(
        call_pos(fn_strrpos, &[s("abcabcabc"), s("abc"), Value::Int(-2)]),
        Some(6)
    );
    // offset out of range -> ValueError
    assert!(fn_strpos(&mut TestHost::new(), &[s("Hello"), s("l"), Value::Int(100)]).is_err());
}

#[test]
fn str_pad_cases() {
    assert_eq!(
        calls(fn_str_pad, &[s("5"), Value::Int(3), s("0"), Value::Int(0)]),
        "005"
    );
    assert_eq!(calls(fn_str_pad, &[s("5"), Value::Int(3)]), "5  ");
    assert_eq!(
        calls(
            fn_str_pad,
            &[s("abc"), Value::Int(10), s("*-"), Value::Int(2)]
        ),
        "*-*abc*-*-"
    );
    assert_eq!(
        calls(
            fn_str_pad,
            &[s("abc"), Value::Int(10), s("123"), Value::Int(0)]
        ),
        "1231231abc"
    );
    assert_eq!(
        calls(
            fn_str_pad,
            &[s("abc"), Value::Int(7), s("xy"), Value::Int(2)]
        ),
        "xyabcxy"
    );
}

#[test]
fn trim_cases() {
    assert_eq!(calls(fn_trim, &[s("  hello  ")]), "hello");
    assert_eq!(calls(fn_trim, &[s("\t\n hello \r\0\x0B")]), "hello");
    assert_eq!(calls(fn_ltrim, &[s("xxhelloxx"), s("x")]), "helloxx");
    assert_eq!(calls(fn_rtrim, &[s("xxhelloxx"), s("x")]), "xxhello");
    assert_eq!(calls(fn_trim, &[s("[hello]"), s("[]")]), "hello");
    assert_eq!(calls(fn_trim, &[s("1234hello5678"), s("0..9")]), "hello");
    assert_eq!(calls(fn_trim, &[s("..hello.."), s(".")]), "hello");
}

#[test]
fn case_and_rev() {
    assert_eq!(calls(fn_ucfirst, &[s("hello world")]), "Hello world");
    assert_eq!(calls(fn_lcfirst, &[s("Hello World")]), "hello World");
    assert_eq!(
        calls(fn_ucwords, &[s("hello world foo")]),
        "Hello World Foo"
    );
    assert_eq!(
        calls(fn_ucwords, &[s("hello-world foo"), s("-")]),
        "Hello-World foo"
    );
    assert_eq!(
        calls(fn_ucwords, &[s("hello\tworld\nfoo")]),
        "Hello\tWorld\nFoo"
    );
    assert_eq!(calls(fn_strrev, &[s("hello")]), "olleh");
}

#[test]
fn substr_count_cases() {
    assert_eq!(
        call_int(fn_substr_count, &[s("hello world hello"), s("hello")]),
        2
    );
    assert_eq!(call_int(fn_substr_count, &[s("abababab"), s("ab")]), 4);
    assert_eq!(call_int(fn_substr_count, &[s("aaa"), s("aa")]), 1);
    assert_eq!(
        call_int(
            fn_substr_count,
            &[s("hello world hello"), s("o"), Value::Int(5)]
        ),
        2
    );
    assert_eq!(
        call_int(
            fn_substr_count,
            &[s("hello world hello"), s("o"), Value::Int(0), Value::Int(5)]
        ),
        1
    );
}

#[test]
fn starts_ends_with() {
    assert!(call_bool(fn_str_starts_with, &[s("hello"), s("he")]));
    assert!(call_bool(fn_str_ends_with, &[s("hello"), s("lo")]));
    assert!(call_bool(fn_str_starts_with, &[s("hello"), s("")]));
    assert!(!call_bool(fn_str_ends_with, &[s("hello"), s("x")]));
}

#[test]
fn wordwrap_cases() {
    assert_eq!(
        calls(fn_wordwrap, &[s("The quick brown fox"), Value::Int(10)]),
        "The quick\nbrown fox"
    );
    assert_eq!(
        calls(
            fn_wordwrap,
            &[
                s("A very long woooooooooooooord"),
                Value::Int(10),
                s("\n"),
                Value::Bool(true)
            ]
        ),
        "A very\nlong\nwooooooooo\nooooord"
    );
    assert_eq!(
        calls(
            fn_wordwrap,
            &[
                s("A very long woooooooooooooord"),
                Value::Int(10),
                s("\n"),
                Value::Bool(false)
            ]
        ),
        "A very\nlong\nwoooooooooooooord"
    );
    assert_eq!(
        calls(
            fn_wordwrap,
            &[
                s("12345678901234567890"),
                Value::Int(5),
                s("\n"),
                Value::Bool(true)
            ]
        ),
        "12345\n67890\n12345\n67890"
    );
    assert_eq!(
        calls(
            fn_wordwrap,
            &[s("Hello World"), Value::Int(5), s("-"), Value::Bool(false)]
        ),
        "Hello-World"
    );
}

#[test]
fn wordwrap_differential() {
    // sweep captured from php 8.5.8
    let f = fn_wordwrap;
    assert_eq!(
        calls(f, &[s("The quick brown fox jumped"), Value::Int(10)]),
        "The quick\nbrown fox\njumped"
    );
    assert_eq!(
        calls(
            f,
            &[s("The quick brown fox jumped"), Value::Int(10), s("<br>")]
        ),
        "The quick<br>brown fox<br>jumped"
    );
    assert_eq!(
        calls(f, &[s("aaa bbb ccc"), Value::Int(3)]),
        "aaa\nbbb\nccc"
    );
    assert_eq!(
        calls(
            f,
            &[s("aaa bbb ccc"), Value::Int(3), s("\n"), Value::Bool(true)]
        ),
        "aaa\nbbb\nccc"
    );
    assert_eq!(
        calls(
            f,
            &[
                s("a aa aaa aaaa aaaaa"),
                Value::Int(4),
                s("|"),
                Value::Bool(true)
            ]
        ),
        "a aa|aaa|aaaa|aaaa|a"
    );
    assert_eq!(
        calls(f, &[s("already\nwrapped text here"), Value::Int(10)]),
        "already\nwrapped\ntext here"
    );
    assert_eq!(
        calls(f, &[s("test  double  spaces"), Value::Int(6)]),
        "test \ndouble\n spaces"
    );
    assert_eq!(
        calls(f, &[s("trailing space "), Value::Int(5)]),
        "trailing\nspace\n"
    );
    assert_eq!(
        calls(
            f,
            &[
                s("supercalifragilistic"),
                Value::Int(5),
                s("\n"),
                Value::Bool(true)
            ]
        ),
        "super\ncalif\nragil\nistic"
    );
    assert_eq!(calls(f, &[s("one two"), Value::Int(100)]), "one two");
}

#[test]
fn number_format_cases() {
    assert_eq!(
        calls(fn_number_format, &[Value::Float(1234567.891)]),
        "1,234,568"
    );
    assert_eq!(
        calls(
            fn_number_format,
            &[Value::Float(1234567.891), Value::Int(2)]
        ),
        "1,234,567.89"
    );
    assert_eq!(
        calls(
            fn_number_format,
            &[Value::Float(1234567.891), Value::Int(2), s(","), s(".")]
        ),
        "1.234.567,89"
    );
    assert_eq!(
        calls(fn_number_format, &[Value::Float(-1234.5678), Value::Int(2)]),
        "-1,234.57"
    );
    assert_eq!(calls(fn_number_format, &[Value::Float(0.5)]), "1");
    assert_eq!(
        calls(fn_number_format, &[Value::Float(1.995), Value::Int(2)]),
        "2.00"
    );
    assert_eq!(
        calls(fn_number_format, &[Value::Float(-0.0), Value::Int(2)]),
        "0.00"
    );
    assert_eq!(
        calls(fn_number_format, &[Value::Float(2.675), Value::Int(2)]),
        "2.68"
    );
    assert_eq!(
        calls(fn_number_format, &[Value::Float(1.005), Value::Int(2)]),
        "1.01"
    );
    assert_eq!(calls(fn_number_format, &[Value::Float(-0.5)]), "-1");
    assert_eq!(calls(fn_number_format, &[Value::Float(2.5)]), "3");
    assert_eq!(
        calls(fn_number_format, &[Value::Int(123), Value::Int(4)]),
        "123.0000"
    );
    assert_eq!(
        calls(fn_number_format, &[Value::Float(-0.001), Value::Int(2)]),
        "0.00"
    );
    assert_eq!(
        calls(fn_number_format, &[Value::Float(1.5), Value::Int(-1)]),
        "0"
    );
    assert_eq!(
        calls(fn_number_format, &[Value::Float(99.9999), Value::Int(2)]),
        "100.00"
    );
}

#[test]
fn soundex_cases() {
    // expected values captured from php 8.5.8
    assert_eq!(calls(fn_soundex, &[s("Robert")]), "R163");
    assert_eq!(calls(fn_soundex, &[s("Tymczak")]), "T522");
    assert_eq!(calls(fn_soundex, &[s("Ashcraft")]), "A226");
    assert_eq!(calls(fn_soundex, &[s("Lukasiewicz")]), "L222");
    assert_eq!(calls(fn_soundex, &[s("a")]), "A000");
    assert_eq!(calls(fn_soundex, &[s("")]), "0000");
    assert_eq!(calls(fn_soundex, &[s("123")]), "0000");
}

#[test]
fn addcslashes_cases() {
    assert_eq!(
        calls(fn_addcslashes, &[s("Hello World"), s("W")]),
        "Hello \\World"
    );
    assert_eq!(calls(fn_addcslashes, &[s("foo[ ]"), s("A..Z")]), "foo[ ]");
    assert_eq!(
        calls(fn_addcslashes, &[s("abc\n\t\r"), s("\0..\u{1f}")]),
        "abc\\n\\t\\r"
    );
    assert_eq!(
        calls(
            fn_addcslashes,
            &[
                Value::str(b"foo\x7f\x80bar".to_vec()),
                Value::str(b"\x7f..\xff".to_vec()),
            ]
        ),
        "foo\\177\\200bar"
    );
}

#[test]
fn stripcslashes_cases() {
    assert_eq!(calls(fn_stripcslashes, &[s("\\101\\x41")]), "AA");
    assert_eq!(calls(fn_stripcslashes, &[s("He\\llo")]), "Hello");
    assert_eq!(calls(fn_stripcslashes, &[s("\\xg")]), "xg");
    assert_eq!(calls(fn_stripcslashes, &[s("end\\")]), "end\\");
    assert_eq!(calls(fn_stripcslashes, &[s("\\n\\t\\\\")]), "\n\t\\");
}

#[test]
fn count_chars_cases() {
    assert_eq!(calls(fn_count_chars, &[s("hello"), Value::Int(3)]), "ehlo");
    assert_eq!(calls(fn_count_chars, &[s("abcabc"), Value::Int(3)]), "abc");
    // mode 1: present bytes with counts, keys ascending by byte value
    let r = call(fn_count_chars, &[s("hello"), Value::Int(1)]);
    let Value::Array(a) = r else {
        panic!("expected array")
    };
    let pairs: Vec<(i64, i64)> = a
        .iter()
        .map(|(k, v)| {
            let ArrayKey::Int(n) = k else { panic!() };
            (*n, conv::to_int(v))
        })
        .collect();
    assert_eq!(pairs, vec![(101, 1), (104, 1), (108, 2), (111, 1)]);
    assert!(fn_count_chars(&mut TestHost::new(), &[s("x"), Value::Int(5)]).is_err());
}
