// Expected values captured from php 8.5.8.
#![allow(clippy::approx_constant)] // deliberate literal test values, not PI

use super::*;
use crate::ext::standard::math::fn_number_format;
use crate::ext::standard::soundex::fn_soundex;
use crate::test_support::TestHost;
use crate::zend::builtin_functions::fn_strcasecmp;
use crate::zend::builtin_functions::fn_strcmp;
use crate::zend::builtin_functions::fn_strncasecmp;
use crate::zend::builtin_functions::fn_strncmp;
use rhp_values::PhpArray;

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
        .map(|(_, v)| String::from_utf8(rhp_values::conv::to_string_bytes(v)).unwrap())
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

fn i(n: i64) -> Value {
    Value::Int(n)
}

fn dumped(v: &Value) -> String {
    let mut out = Vec::new();
    crate::ext::standard::var::var_dump(&mut TestHost::new(), &mut out, v).expect("no throw");
    String::from_utf8_lossy(&out).into_owned()
}

fn list(items: Vec<Value>) -> Value {
    let mut a = PhpArray::new();
    for v in items {
        a.push(v);
    }
    Value::Array(a)
}

fn map(pairs: Vec<(&str, Value)>) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in pairs {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    }
    Value::Array(a)
}

fn imap(pairs: Vec<(i64, Value)>) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in pairs {
        a.insert(ArrayKey::Int(k), v);
    }
    Value::Array(a)
}

#[test]
fn str_split_cases() {
    assert_eq!(
        dumped(&call(fn_str_split, &[s("Hello")])),
        "array(5) {\n  [0]=>\n  string(1) \"H\"\n  [1]=>\n  string(1) \"e\"\n  [2]=>\n  string(1) \"l\"\n  [3]=>\n  string(1) \"l\"\n  [4]=>\n  string(1) \"o\"\n}\n"
    );
    assert_eq!(
        dumped(&call(fn_str_split, &[s("Hello World"), i(3)])),
        "array(4) {\n  [0]=>\n  string(3) \"Hel\"\n  [1]=>\n  string(3) \"lo \"\n  [2]=>\n  string(3) \"Wor\"\n  [3]=>\n  string(2) \"ld\"\n}\n"
    );
    assert_eq!(dumped(&call(fn_str_split, &[s("")])), "array(0) {\n}\n");
    assert!(fn_str_split(&mut TestHost::new(), &[s("abc"), i(0)]).is_err());
}

#[test]
fn strtr_pairs() {
    assert_eq!(
        calls(
            fn_strtr,
            &[
                s("Hello World"),
                map(vec![("Hello", s("Hi")), ("World", s("Earth"))])
            ]
        ),
        "Hi Earth"
    );
    // longest key wins (ab before a)
    assert_eq!(
        calls(
            fn_strtr,
            &[s("abcabc"), map(vec![("a", s("1")), ("ab", s("X"))])]
        ),
        "XcXc"
    );
    // output is not re-translated
    assert_eq!(
        calls(fn_strtr, &[s("aaa"), map(vec![("a", s("aa"))])]),
        "aaaaaa"
    );
    assert_eq!(calls(fn_strtr, &[s("hi"), list(vec![])]), "hi");
    // integer keys coerce to their string form; longest wins
    assert_eq!(
        calls(fn_strtr, &[s("12"), imap(vec![(1, s("a")), (12, s("bb"))])]),
        "bb"
    );
    assert_eq!(
        calls(
            fn_strtr,
            &[s("test"), map(vec![("t", s("T")), ("te", s("X"))])]
        ),
        "XsT"
    );
}

#[test]
fn strtr_from_to() {
    assert_eq!(calls(fn_strtr, &[s("Hello"), s("el"), s("ip")]), "Hippo");
    assert_eq!(
        calls(fn_strtr, &[s("The fox"), s("abcdefghij"), s("0123456789")]),
        "T74 5ox"
    );
    // to shorter than from: only the common prefix maps
    assert_eq!(calls(fn_strtr, &[s("abcd"), s("abcde"), s("xy")]), "xycd");
}

#[test]
fn substr_replace_string() {
    assert_eq!(
        calls(fn_substr_replace, &[s("Hello World"), s("PHP"), i(6)]),
        "Hello PHP"
    );
    assert_eq!(
        calls(fn_substr_replace, &[s("Hello World"), s("PHP"), i(6), i(5)]),
        "Hello PHP"
    );
    assert_eq!(
        calls(fn_substr_replace, &[s("Hello World"), s("PHP"), i(-5)]),
        "Hello PHP"
    );
    assert_eq!(
        calls(
            fn_substr_replace,
            &[s("Hello World"), s("PHP"), i(-5), i(-1)]
        ),
        "Hello PHPd"
    );
    assert_eq!(
        calls(fn_substr_replace, &[s("Hello"), s("XXX"), i(0), i(0)]),
        "XXXHello"
    );
    assert_eq!(
        calls(fn_substr_replace, &[s("Hello"), s("XXX"), i(2), i(-1)]),
        "HeXXXo"
    );
    assert_eq!(
        calls(fn_substr_replace, &[s("abcdef"), s("-"), i(2), i(0)]),
        "ab-cdef"
    );
    assert_eq!(calls(fn_substr_replace, &[s("abc"), s("X"), i(10)]), "abcX");
    assert_eq!(
        calls(fn_substr_replace, &[s("abc"), s("X"), i(10), i(5)]),
        "abcX"
    );
    assert_eq!(
        calls(fn_substr_replace, &[s("abcdef"), s("XYZ"), i(-2), i(-5)]),
        "abcdXYZef"
    );
    assert_eq!(
        calls(fn_substr_replace, &[s("abc"), s("X"), i(-99), i(1)]),
        "Xbc"
    );
    assert_eq!(
        calls(fn_substr_replace, &[s("Hello World"), s("PHP"), i(6), i(0)]),
        "Hello PHPWorld"
    );
    // scalar subject with array replace uses the first element
    assert_eq!(
        calls(
            fn_substr_replace,
            &[s("aaa"), list(vec![s("X"), s("Y")]), i(1), i(1)]
        ),
        "aXa"
    );
    assert_eq!(
        calls(fn_substr_replace, &[s("abc"), list(vec![]), i(1), i(1)]),
        "ac"
    );
}

#[test]
fn substr_replace_array() {
    assert_eq!(
        dumped(&call(
            fn_substr_replace,
            &[list(vec![s("Hello"), s("World")]), s("X"), i(1), i(2)]
        )),
        "array(2) {\n  [0]=>\n  string(4) \"HXlo\"\n  [1]=>\n  string(4) \"WXld\"\n}\n"
    );
    assert_eq!(
        dumped(&call(
            fn_substr_replace,
            &[
                list(vec![s("aaa"), s("bbbb")]),
                list(vec![s("1"), s("2")]),
                i(1),
                i(1)
            ]
        )),
        "array(2) {\n  [0]=>\n  string(3) \"a1a\"\n  [1]=>\n  string(4) \"b2bb\"\n}\n"
    );
    assert_eq!(
        dumped(&call(
            fn_substr_replace,
            &[
                list(vec![s("aaa"), s("bbbb")]),
                list(vec![s("1"), s("2")]),
                list(vec![i(0), i(1)]),
                list(vec![i(1), i(2)])
            ]
        )),
        "array(2) {\n  [0]=>\n  string(3) \"1aa\"\n  [1]=>\n  string(3) \"b2b\"\n}\n"
    );
    // replace array shorter than subjects -> "" for the rest
    assert_eq!(
        dumped(&call(fn_substr_replace, &[list(vec![s("aaa"), s("bbb"), s("ccc")]), list(vec![s("X"), s("Y")]), i(1), i(1)])),
        "array(3) {\n  [0]=>\n  string(3) \"aXa\"\n  [1]=>\n  string(3) \"bYb\"\n  [2]=>\n  string(2) \"cc\"\n}\n"
    );
    // start/length arrays exhausted -> start 0, length full (whole replaced)
    assert_eq!(
        dumped(&call(
            fn_substr_replace,
            &[
                list(vec![s("aaa"), s("bbb")]),
                s("X"),
                list(vec![i(1)]),
                list(vec![i(1)])
            ]
        )),
        "array(2) {\n  [0]=>\n  string(3) \"aXa\"\n  [1]=>\n  string(1) \"X\"\n}\n"
    );
    // omitted length + start array
    assert_eq!(
        dumped(&call(fn_substr_replace, &[list(vec![s("aaa"), s("bbb"), s("ccc")]), s("X"), list(vec![i(0), i(1)])])),
        "array(3) {\n  [0]=>\n  string(1) \"X\"\n  [1]=>\n  string(2) \"bX\"\n  [2]=>\n  string(1) \"X\"\n}\n"
    );
    // keys preserved for associative subject
    assert_eq!(
        dumped(&call(
            fn_substr_replace,
            &[
                map(vec![("x", s("aaa")), ("y", s("bbb"))]),
                list(vec![s("1"), s("2")]),
                i(1),
                i(1)
            ]
        )),
        "array(2) {\n  [\"x\"]=>\n  string(3) \"a1a\"\n  [\"y\"]=>\n  string(3) \"b2b\"\n}\n"
    );
}

#[test]
fn substr_replace_errors() {
    assert!(fn_substr_replace(
        &mut TestHost::new(),
        &[s("abc"), s("X"), list(vec![i(1)]), list(vec![i(1)])]
    )
    .is_err());
    assert!(fn_substr_replace(
        &mut TestHost::new(),
        &[s("abc"), s("X"), i(1), list(vec![i(1)])]
    )
    .is_err());
}

#[test]
fn similar_text_cases() {
    assert_eq!(call_int(fn_similar_text, &[s("World"), s("word")]), 3);
    assert_eq!(call_int(fn_similar_text, &[s("Hello"), s("World")]), 1);
    assert_eq!(call_int(fn_similar_text, &[s("abcde"), s("ace")]), 3);
    assert_eq!(call_int(fn_similar_text, &[s(""), s("abc")]), 0);
    assert_eq!(call_int(fn_similar_text, &[s("abc"), s("")]), 0);
    assert_eq!(call_int(fn_similar_text, &[s(""), s("")]), 0);
    assert_eq!(
        call_int(fn_similar_text, &[s("PHP IS GREAT"), s("WITH MYSQL")]),
        3
    );
    assert_eq!(
        call_int(fn_similar_text, &[s("Hello World"), s("Hello PHP World")]),
        11
    );
    assert_eq!(call_int(fn_similar_text, &[s("test"), s("test")]), 4);
    assert_eq!(call_int(fn_similar_text, &[s("aaa"), s("aa")]), 2);
    assert_eq!(
        call_int(
            fn_similar_text,
            &[s("The quick brown fox"), s("The quick fox")]
        ),
        13
    );
}

#[test]
fn str_word_count_cases() {
    assert_eq!(call_int(fn_str_word_count, &[s("Hello World Foo")]), 3);
    assert_eq!(call_int(fn_str_word_count, &[s("The quick brown fox")]), 4);
    assert_eq!(call_int(fn_str_word_count, &[s("  spaces   between  ")]), 2);
    assert_eq!(call_int(fn_str_word_count, &[s("don't can't")]), 2);
    assert_eq!(call_int(fn_str_word_count, &[s("hello123world")]), 2);
    assert_eq!(call_int(fn_str_word_count, &[s("a-b c")]), 2);
    assert_eq!(
        dumped(&call(fn_str_word_count, &[s("Hello World Foo"), i(1)])),
        "array(3) {\n  [0]=>\n  string(5) \"Hello\"\n  [1]=>\n  string(5) \"World\"\n  [2]=>\n  string(3) \"Foo\"\n}\n"
    );
    assert_eq!(
        dumped(&call(fn_str_word_count, &[s(""), i(1)])),
        "array(0) {\n}\n"
    );
    assert_eq!(
        dumped(&call(fn_str_word_count, &[s("Hello World"), i(2)])),
        "array(2) {\n  [0]=>\n  string(5) \"Hello\"\n  [6]=>\n  string(5) \"World\"\n}\n"
    );
    // additional characters (with range syntax)
    assert_eq!(
        dumped(&call(
            fn_str_word_count,
            &[s("hi123 there"), i(1), s("123")]
        )),
        "array(2) {\n  [0]=>\n  string(5) \"hi123\"\n  [1]=>\n  string(5) \"there\"\n}\n"
    );
    assert_eq!(
        call_int(fn_str_word_count, &[s("a1b c"), i(0), s("0..9")]),
        2
    );
    assert!(fn_str_word_count(&mut TestHost::new(), &[s("hi"), i(5)]).is_err());
}

#[test]
fn chunk_split_cases() {
    assert_eq!(
        calls(fn_chunk_split, &[s("abcdefgh"), i(3), s("-")]),
        "abc-def-gh-"
    );
    assert_eq!(calls(fn_chunk_split, &[s("abcdefgh")]), "abcdefgh\r\n");
    assert_eq!(
        calls(fn_chunk_split, &[s("abcdefgh"), i(3)]),
        "abc\r\ndef\r\ngh\r\n"
    );
    assert_eq!(calls(fn_chunk_split, &[s("abc"), i(5), s("|")]), "abc|");
    assert_eq!(calls(fn_chunk_split, &[s(""), i(3), s("-")]), "-");
    assert_eq!(
        calls(fn_chunk_split, &[s("abcdef"), i(2), s("\r\n")]),
        "ab\r\ncd\r\nef\r\n"
    );
    assert_eq!(
        calls(fn_chunk_split, &[s("abcdefgh"), i(4), s("")]),
        "abcdefgh"
    );
    assert_eq!(
        calls(fn_chunk_split, &[s("abcdefgh"), i(4), s("-")]),
        "abcd-efgh-"
    );
    assert_eq!(calls(fn_chunk_split, &[s("abcd"), i(4), s("-")]), "abcd-");
    assert!(fn_chunk_split(&mut TestHost::new(), &[s("abc"), i(0)]).is_err());
}

fn sb(b: &[u8]) -> Value {
    Value::str(b.to_vec())
}

fn call_bytes(f: crate::NativeFn, args: &[Value]) -> Vec<u8> {
    match f(&mut TestHost::new(), args).expect("no throw") {
        Value::Str(b) => b.as_bytes().to_vec(),
        other => panic!("expected string, got {}", other.type_name()),
    }
}

fn call_str(f: crate::NativeFn, args: &[Value]) -> String {
    String::from_utf8(call_bytes(f, args)).unwrap()
}

#[test]
fn nl2br_cases() {
    assert_eq!(
        call_bytes(fn_nl2br, &[sb(b"a\nb\r\nc\rd\n\re")]),
        b"a<br />\nb<br />\r\nc<br />\rd<br />\n\re"
    );
    assert_eq!(
        call_bytes(fn_nl2br, &[sb(b"a\nb"), Value::Bool(false)]),
        b"a<br>\nb"
    );
}

#[test]
fn addslashes_cases() {
    assert_eq!(
        call_bytes(fn_addslashes, &[sb(b"a'b\"c\\d\x00e")]),
        b"a\\'b\\\"c\\\\d\\0e"
    );
}

#[test]
fn stripslashes_cases() {
    assert_eq!(
        call_bytes(fn_stripslashes, &[sb(b"a\\'b\\\"c\\\\d\\0e\\n")]),
        b"a'b\"c\\d\x00en"
    );
    assert_eq!(call_bytes(fn_stripslashes, &[sb(b"\\0")]), b"\x00");
    assert_eq!(call_bytes(fn_stripslashes, &[sb(b"ab\\")]), b"ab");
}

#[test]
fn quotemeta_cases() {
    assert_eq!(
        call_str(fn_quotemeta, &[s("1+1=2. (a|b) [x] *? \\^$ end")]),
        "1\\+1=2\\. \\(a|b\\) \\[x\\] \\*\\? \\\\\\^\\$ end"
    );
}

#[test]
fn differential_sweep() {
    // extra inputs cross-checked against php 8.5.8 (hex of output)
    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{:02x}", x)).collect()
    }
    let cases = [
        "<b>a</b> & 'x' <!--c--> <?p ?> <d e=\">\">f",
        "A&amp;B&#38;C&#x26;D",
        "café &eacute; &#233;",
        "<!--a--b-->z",
        "<a b=1>c",
    ];
    let php_strip = [
        "6120262027782720202066",
        "4126616d703b42262333383b4326237832363b44",
        "636166c3a920266561637574653b2026233233333b",
        "7a",
        "63",
    ];
    for i in 0..cases.len() {
        assert_eq!(
            hex(&call_bytes(fn_strip_tags, &[s(cases[i])])),
            php_strip[i],
            "strip_tags case {i}"
        );
    }
}

#[test]
fn strip_tags_cases() {
    let f = fn_strip_tags;
    assert_eq!(
        call_str(f, &[s("<p>Hello <b>World</b></p>")]),
        "Hello World"
    );
    assert_eq!(
        call_str(f, &[s("<a href=\"x\">link</a> text")]),
        "link text"
    );
    assert_eq!(call_str(f, &[s("a<!-- comment -->b")]), "ab");
    assert_eq!(call_str(f, &[s("a < b and c > d")]), "a < b and c > d");
    assert_eq!(call_str(f, &[s("text <b>bold")]), "text bold");
    assert_eq!(call_str(f, &[s("<a title=\"<b>\">x</a>")]), "x");
    assert_eq!(call_str(f, &[s("a<?php echo 1; ?>b")]), "ab");
    assert_eq!(
        call_str(f, &[s("<script>alert(1)</script>hi")]),
        "alert(1)hi"
    );
    assert_eq!(call_str(f, &[s("a &lt;b&gt; c")]), "a &lt;b&gt; c");
    assert_eq!(call_str(f, &[s("abc<")]), "abc");
    assert_eq!(call_str(f, &[s("<div")]), "");
    // nested '<' depth, comment/decl/php end conditions
    assert_eq!(call_str(f, &[s("<a<b>c>d")]), "d");
    assert_eq!(call_str(f, &[s("a<!--x-->b<!--y--->c")]), "abc");
    assert_eq!(call_str(f, &[s("a<!-- > -->b")]), "ab");
    assert_eq!(call_str(f, &[s("a<!DOCTYPE html>b")]), "ab");
    assert_eq!(call_str(f, &[s("a<? x > y ?>b")]), "ab");
    assert_eq!(call_str(f, &[s("a<![CDATA[x<y>]]>b")]), "a]]>b");
    assert_eq!(call_str(f, &[s("<input value=\">\">x")]), "x");
    assert_eq!(call_str(f, &[s("<a t='>'>z")]), "z");
    assert_eq!(call_str(f, &[s("a<?php x")]), "a");
    assert_eq!(call_str(f, &[s("a<!-- x")]), "a");
    assert_eq!(call_bytes(f, &[sb(b"a<\x0cb>c")]), b"a<\x0cb>c");
}

fn csv(args: &[Value]) -> Vec<Option<String>> {
    let r = fn_str_getcsv(&mut TestHost::new(), args).unwrap();
    match r {
        Value::Array(a) => a
            .iter()
            .map(|(_, v)| match v {
                Value::Null => None,
                Value::Str(s) => Some(String::from_utf8(s.as_bytes().to_vec()).unwrap()),
                _ => unreachable!(),
            })
            .collect(),
        _ => unreachable!(),
    }
}

fn some(items: &[&str]) -> Vec<Option<String>> {
    items.iter().map(|x| Some(x.to_string())).collect()
}

#[test]
fn csv_basic() {
    assert_eq!(csv(&[s("a,b,c")]), some(&["a", "b", "c"]));
    assert_eq!(csv(&[s("a,\"b,c\",d")]), some(&["a", "b,c", "d"]));
    assert_eq!(
        csv(&[s("\"he said \"\"hi\"\"\",x")]),
        some(&["he said \"hi\"", "x"])
    );
    assert_eq!(csv(&[s("a;b;c"), s(";")]), some(&["a", "b", "c"]));
    assert_eq!(csv(&[s("a,b,,")]), some(&["a", "b", "", ""]));
    assert_eq!(csv(&[s("")]), vec![None]);
    assert_eq!(csv(&[s("  a , b ")]), some(&["  a ", " b "]));
    assert_eq!(csv(&[s("a,\"b\nc\",d")]), some(&["a", "b\nc", "d"]));
}

#[test]
fn csv_escape() {
    // escape char keeps both chars; close-quote handling.
    assert_eq!(
        csv(&[s("x,\"y\\\"z\""), s(","), s("\""), s("\\")]),
        some(&["x", "y\\\"z"])
    );
    assert_eq!(
        csv(&[s("a\"b,c"), s(","), s("\""), s("\\")]),
        some(&["a\"b", "c"])
    );
    assert_eq!(
        csv(&[s("\"a\"b,c"), s(","), s("\""), s("\\")]),
        some(&["ab", "c"])
    );
    assert_eq!(
        csv(&[s("a\\,b,c"), s(","), s("\""), s("\\")]),
        some(&["a\\", "b", "c"])
    );
}

/// True when the result is php `false` (used for failure returns).
fn is_false(f: crate::NativeFn, args: &[Value]) -> bool {
    matches!(call(f, args), Value::Bool(false))
}

#[test]
fn bin2hex_hex2bin_vectors() {
    assert_eq!(calls(fn_bin2hex, &[s("abc")]), "616263");
    assert_eq!(calls(fn_bin2hex, &[s("")]), "");
    assert_eq!(
        calls(fn_bin2hex, &[Value::str(vec![0xff, 0x00, 0x10])]),
        "ff0010"
    );
    assert_eq!(calls(fn_hex2bin, &[s("616263")]), "abc");
    assert_eq!(calls(fn_hex2bin, &[s("")]), "");
    assert_eq!(
        rhp_values::conv::to_string_bytes(&call(fn_hex2bin, &[s("ff0010")])),
        vec![0xff, 0x00, 0x10]
    );
    // odd length -> false
    assert!(is_false(fn_hex2bin, &[s("abc")]));
    // non-hex -> false
    assert!(is_false(fn_hex2bin, &[s("xy")]));
}
