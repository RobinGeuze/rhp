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
