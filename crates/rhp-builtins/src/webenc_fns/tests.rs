// Expected values captured from php 8.5.8.

use super::*;
use crate::test_support::TestHost;

fn s(b: &str) -> Value {
    Value::str(b)
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
fn urlencode_cases() {
    assert_eq!(
        call_str(fn_urlencode, &[s("a b+c&d=e/f?g~h.i_j-k")]),
        "a+b%2Bc%26d%3De%2Ff%3Fg%7Eh.i_j-k"
    );
    assert_eq!(
        call_bytes(fn_urlencode, &[sb(b"\xff\x00A~")]),
        b"%FF%00A%7E"
    );
}

#[test]
fn rawurlencode_cases() {
    assert_eq!(
        call_str(fn_rawurlencode, &[s("a b+c&d=e/f?g~h.i_j-k")]),
        "a%20b%2Bc%26d%3De%2Ff%3Fg~h.i_j-k"
    );
    assert_eq!(
        call_bytes(fn_rawurlencode, &[sb(b"\xff\x00A~")]),
        b"%FF%00A~"
    );
}

#[test]
fn urldecode_cases() {
    assert_eq!(
        call_str(fn_urldecode, &[s("a+b%20c%26d%ZZ%4")]),
        "a b c&d%ZZ%4"
    );
    assert_eq!(call_str(fn_urldecode, &[s("abc%")]), "abc%");
    assert_eq!(call_str(fn_rawurldecode, &[s("a+b%20c%26d")]), "a+b c&d");
    assert_eq!(call_str(fn_rawurldecode, &[s("ab%2")]), "ab%2");
}
