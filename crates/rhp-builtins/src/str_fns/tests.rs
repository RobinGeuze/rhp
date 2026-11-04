// Expected values captured from php 8.5.8.
#![allow(clippy::approx_constant)] // deliberate literal test values, not PI

use super::*;
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
