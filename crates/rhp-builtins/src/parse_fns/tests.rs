// tests — expected values captured from php 8.5.8

use super::*;

struct NoopHost;
impl Host for NoopHost {
    fn write_out(&mut self, _: &[u8]) {}
}

fn s(b: &str) -> Value {
    Value::str(b)
}

fn csv(args: &[Value]) -> Vec<Option<String>> {
    let r = fn_str_getcsv(&mut NoopHost, args).unwrap();
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

/// Canonical string form of a value, for structural comparison (Value has no
/// `PartialEq`). Floats use a short rounded repr adequate for these tests.
fn vrepr(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Int(n) => format!("i:{n}"),
        Value::Float(f) => format!("f:{:.6}", f),
        Value::Str(s) => format!("s:{}", String::from_utf8_lossy(s.as_bytes())),
        Value::Bool(b) => format!("b:{b}"),
        _ => "?".to_string(),
    }
}

fn scan_arr(input: &str, fmt: &str) -> Option<Vec<String>> {
    match fn_sscanf(&mut NoopHost, &[s(input), s(fmt)]).unwrap() {
        Value::Array(a) => Some(a.iter().map(|(_, v)| vrepr(v)).collect()),
        Value::Null => None,
        _ => unreachable!(),
    }
}

fn v(items: &[&str]) -> Option<Vec<String>> {
    Some(items.iter().map(|x| x.to_string()).collect())
}

#[test]
fn sscanf_basic() {
    assert_eq!(scan_arr("age:25", "age:%d"), v(&["i:25"]));
    assert_eq!(scan_arr("3.14 x", "%f %s"), v(&["f:3.140000", "s:x"]));
    assert_eq!(scan_arr("hello world", "%s %s"), v(&["s:hello", "s:world"]));
    assert_eq!(scan_arr("12ab", "%d%s"), v(&["i:12", "s:ab"]));
    assert_eq!(scan_arr("  42", "%d"), v(&["i:42"]));
    assert_eq!(scan_arr("255", "%3d"), v(&["i:255"]));
    assert_eq!(scan_arr("abcdef", "%3s"), v(&["s:abc"]));
    assert_eq!(scan_arr("hello", "%2c"), v(&["s:he"]));
    assert_eq!(scan_arr("ff", "%x"), v(&["i:255"]));
    assert_eq!(scan_arr("-17", "%d"), v(&["i:-17"]));
}

#[test]
fn sscanf_failures() {
    // whole-NULL when EOF before any match.
    assert_eq!(scan_arr("", "%d %d"), None);
    // partial -> trailing NULLs.
    assert_eq!(scan_arr("5", "%d %d %d"), v(&["i:5", "null", "null"]));
    // mismatch on non-empty input -> array with NULL.
    assert_eq!(scan_arr("hello", "%d"), v(&["null"]));
    // no conversions -> empty array.
    assert_eq!(scan_arr("%", "%%"), v(&[]));
}

#[test]
fn sscanf_assign_form() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let a = Rc::new(RefCell::new(Value::Null));
    let b = Rc::new(RefCell::new(Value::Null));
    let n = fn_sscanf(
        &mut NoopHost,
        &[
            s("42 foo"),
            s("%d %s"),
            Value::Ref(a.clone()),
            Value::Ref(b.clone()),
        ],
    )
    .unwrap();
    assert!(matches!(n, Value::Int(2)));
    assert!(matches!(&*a.borrow(), Value::Int(42)));
    assert!(matches!(&*b.borrow(), Value::Str(s) if s.as_bytes() == b"foo"));

    // count 1 when only the first matches.
    let a2 = Rc::new(RefCell::new(Value::Null));
    let b2 = Rc::new(RefCell::new(Value::Null));
    let n2 = fn_sscanf(
        &mut NoopHost,
        &[s("5"), s("%d %d"), Value::Ref(a2.clone()), Value::Ref(b2)],
    )
    .unwrap();
    assert!(matches!(n2, Value::Int(1)));

    // 0 matched -> NULL.
    let a3 = Rc::new(RefCell::new(Value::Null));
    let n3 = fn_sscanf(&mut NoopHost, &[s("nomatch"), s("%d"), Value::Ref(a3)]).unwrap();
    assert!(matches!(n3, Value::Null));
}
