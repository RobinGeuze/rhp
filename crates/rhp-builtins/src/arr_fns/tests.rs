// Expected strings captured from php 8.5.8.

use super::*;
use crate::ext::standard::r#type::fn_array_is_list;
use crate::test_support::TestHost;

fn dumped(v: &Value) -> String {
    let mut out = Vec::new();
    crate::dump::var_dump(&TestHost::new(), &mut out, v, 0);
    String::from_utf8_lossy(&out).into_owned()
}

fn call(f: crate::NativeFn, args: &[Value]) -> Value {
    f(&mut TestHost::new(), args).expect("builtin returned Err")
}

fn arr(items: Vec<(ArrayKey, Value)>) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in items {
        a.insert(k, v);
    }
    Value::Array(a)
}

fn list(items: Vec<Value>) -> Value {
    let mut a = PhpArray::new();
    for v in items {
        a.push(v);
    }
    Value::Array(a)
}

fn s(b: &str) -> Value {
    Value::str(b)
}
fn i(n: i64) -> Value {
    Value::Int(n)
}
fn ik(n: i64) -> ArrayKey {
    ArrayKey::Int(n)
}
fn sk(b: &str) -> ArrayKey {
    ArrayKey::from_bytes(b.as_bytes())
}

#[test]
fn t_array_slice() {
    let base = list(vec![s("a"), s("b"), s("c"), s("d"), s("e")]);
    let r = call(fn_array_slice, &[base.clone(), i(2)]);
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [0]=>\n  string(1) \"c\"\n  [1]=>\n  string(1) \"d\"\n  [2]=>\n  string(1) \"e\"\n}\n"
    );
    let r = call(fn_array_slice, &[base.clone(), i(-2), i(1)]);
    assert_eq!(dumped(&r), "array(1) {\n  [0]=>\n  string(1) \"d\"\n}\n");
    let r = call(fn_array_slice, &[base.clone(), i(1), i(-1)]);
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [0]=>\n  string(1) \"b\"\n  [1]=>\n  string(1) \"c\"\n  [2]=>\n  string(1) \"d\"\n}\n"
    );
    let r = call(fn_array_slice, &[base, i(1), i(2), Value::Bool(true)]);
    assert_eq!(
        dumped(&r),
        "array(2) {\n  [1]=>\n  string(1) \"b\"\n  [2]=>\n  string(1) \"c\"\n}\n"
    );
}

#[test]
fn t_array_flip() {
    let base = arr(vec![(sk("a"), i(1)), (sk("b"), i(2)), (sk("c"), i(1))]);
    let r = call(fn_array_flip, &[base]);
    assert_eq!(
        dumped(&r),
        "array(2) {\n  [1]=>\n  string(1) \"c\"\n  [2]=>\n  string(1) \"b\"\n}\n"
    );
}

#[test]
fn t_array_fill() {
    let r = call(fn_array_fill, &[i(5), i(3), s("v")]);
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [5]=>\n  string(1) \"v\"\n  [6]=>\n  string(1) \"v\"\n  [7]=>\n  string(1) \"v\"\n}\n"
    );
    let r = call(fn_array_fill, &[i(-3), i(3), s("v")]);
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [-3]=>\n  string(1) \"v\"\n  [-2]=>\n  string(1) \"v\"\n  [-1]=>\n  string(1) \"v\"\n}\n"
    );
    assert!(fn_array_fill(&mut TestHost::new(), &[i(0), i(-1), s("v")]).is_err());
}

#[test]
fn t_array_fill_keys() {
    let keys = list(vec![s("a"), s("b"), i(7)]);
    let r = call(fn_array_fill_keys, &[keys, i(0)]);
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [\"a\"]=>\n  int(0)\n  [\"b\"]=>\n  int(0)\n  [7]=>\n  int(0)\n}\n"
    );
    let keys = list(vec![
        i(1),
        s("1"),
        s("01"),
        Value::Float(2.9),
        Value::Bool(true),
    ]);
    let r = call(fn_array_fill_keys, &[keys, s("x")]);
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [1]=>\n  string(1) \"x\"\n  [\"01\"]=>\n  string(1) \"x\"\n  [\"2.9\"]=>\n  string(1) \"x\"\n}\n"
    );
}

#[test]
fn t_array_combine() {
    let keys = list(vec![s("a"), Value::Float(2.9), Value::Bool(true), s("123")]);
    let vals = list(vec![i(1), i(2), i(3), i(4)]);
    let r = call(fn_array_combine, &[keys, vals]);
    assert_eq!(
        dumped(&r),
        "array(4) {\n  [\"a\"]=>\n  int(1)\n  [\"2.9\"]=>\n  int(2)\n  [1]=>\n  int(3)\n  [123]=>\n  int(4)\n}\n"
    );
    assert!(fn_array_combine(
        &mut TestHost::new(),
        &[list(vec![i(1), i(2)]), list(vec![i(1)])]
    )
    .is_err());
}

#[test]
fn t_array_search() {
    let base = arr(vec![(sk("x"), s("a")), (sk("y"), s("b"))]);
    assert_eq!(
        dumped(&call(fn_array_search, &[s("b"), base])),
        "string(1) \"y\"\n"
    );
    let base = list(vec![s("a"), s("b")]);
    assert_eq!(
        dumped(&call(fn_array_search, &[i(0), base])),
        "bool(false)\n"
    );
    let base = list(vec![i(1), i(2), i(3)]);
    assert_eq!(
        dumped(&call(fn_array_search, &[s("1"), base.clone()])),
        "int(0)\n"
    );
    assert_eq!(
        dumped(&call(fn_array_search, &[s("1"), base, Value::Bool(true)])),
        "bool(false)\n"
    );
}

#[test]
fn t_array_unique() {
    let base = list(vec![i(1), s("1"), Value::Float(1.0), s("a"), s("a"), i(2)]);
    let r = call(fn_array_unique, &[base]);
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [0]=>\n  int(1)\n  [3]=>\n  string(1) \"a\"\n  [5]=>\n  int(2)\n}\n"
    );
}

#[test]
fn t_array_pad() {
    let base = list(vec![i(1), i(2), i(3)]);
    let r = call(fn_array_pad, &[base.clone(), i(5), i(0)]);
    assert_eq!(
        dumped(&r),
        "array(5) {\n  [0]=>\n  int(1)\n  [1]=>\n  int(2)\n  [2]=>\n  int(3)\n  [3]=>\n  int(0)\n  [4]=>\n  int(0)\n}\n"
    );
    let r = call(fn_array_pad, &[base.clone(), i(-5), i(0)]);
    assert_eq!(
        dumped(&r),
        "array(5) {\n  [0]=>\n  int(0)\n  [1]=>\n  int(0)\n  [2]=>\n  int(1)\n  [3]=>\n  int(2)\n  [4]=>\n  int(3)\n}\n"
    );
    let assoc = arr(vec![(sk("a"), i(1)), (sk("b"), i(2))]);
    let r = call(fn_array_pad, &[assoc, i(4), i(9)]);
    assert_eq!(
        dumped(&r),
        "array(4) {\n  [\"a\"]=>\n  int(1)\n  [\"b\"]=>\n  int(2)\n  [0]=>\n  int(9)\n  [1]=>\n  int(9)\n}\n"
    );
    let sparse = arr(vec![(ik(5), s("a")), (ik(9), s("b"))]);
    let r = call(fn_array_pad, &[sparse, i(2), i(0)]);
    assert_eq!(
        dumped(&r),
        "array(2) {\n  [5]=>\n  string(1) \"a\"\n  [9]=>\n  string(1) \"b\"\n}\n"
    );
}

#[test]
fn t_key_first_last() {
    let base = arr(vec![(sk("a"), i(1)), (sk("b"), i(2))]);
    assert_eq!(
        dumped(&call(fn_array_key_first, &[base])),
        "string(1) \"a\"\n"
    );
    let base = arr(vec![(ik(3), s("x")), (ik(7), s("y"))]);
    assert_eq!(dumped(&call(fn_array_key_last, &[base])), "int(7)\n");
    assert_eq!(dumped(&call(fn_array_key_first, &[list(vec![])])), "NULL\n");
    assert_eq!(dumped(&call(fn_array_key_last, &[list(vec![])])), "NULL\n");
}

#[test]
fn t_array_is_list() {
    assert_eq!(
        dumped(&call(fn_array_is_list, &[list(vec![])])),
        "bool(true)\n"
    );
    assert_eq!(
        dumped(&call(fn_array_is_list, &[list(vec![i(1), i(2), i(3)])])),
        "bool(true)\n"
    );
    let swapped = arr(vec![(ik(1), s("a")), (ik(0), s("b"))]);
    assert_eq!(dumped(&call(fn_array_is_list, &[swapped])), "bool(false)\n");
    let gap = arr(vec![(ik(0), s("a")), (ik(2), s("b"))]);
    assert_eq!(dumped(&call(fn_array_is_list, &[gap])), "bool(false)\n");
}

#[test]
fn t_array_diff() {
    let base = list(vec![i(1), i(2), i(3), i(4)]);
    let r = call(
        fn_array_diff,
        &[base, list(vec![i(2), i(4)]), list(vec![i(3)])],
    );
    assert_eq!(dumped(&r), "array(1) {\n  [0]=>\n  int(1)\n}\n");
    let base = list(vec![i(1), s("1"), i(2)]);
    let r = call(fn_array_diff, &[base, list(vec![s("1")])]);
    assert_eq!(dumped(&r), "array(1) {\n  [2]=>\n  int(2)\n}\n");
}

#[test]
fn t_array_intersect() {
    let base = list(vec![i(1), i(2), i(3), i(4)]);
    let r = call(
        fn_array_intersect,
        &[base, list(vec![i(2), i(4), i(5)]), list(vec![i(4), i(2)])],
    );
    assert_eq!(
        dumped(&r),
        "array(2) {\n  [1]=>\n  int(2)\n  [3]=>\n  int(4)\n}\n"
    );
}

#[test]
fn t_array_column() {
    let rows = list(vec![
        arr(vec![(sk("id"), i(1)), (sk("name"), s("a"))]),
        arr(vec![(sk("id"), i(2)), (sk("name"), s("b"))]),
    ]);
    let r = call(fn_array_column, &[rows.clone(), s("name")]);
    assert_eq!(
        dumped(&r),
        "array(2) {\n  [0]=>\n  string(1) \"a\"\n  [1]=>\n  string(1) \"b\"\n}\n"
    );
    let r = call(fn_array_column, &[rows, s("name"), s("id")]);
    assert_eq!(
        dumped(&r),
        "array(2) {\n  [1]=>\n  string(1) \"a\"\n  [2]=>\n  string(1) \"b\"\n}\n"
    );
    let rows = list(vec![
        arr(vec![(sk("id"), i(5)), (sk("name"), s("a"))]),
        arr(vec![(sk("id"), i(6)), (sk("name"), s("b"))]),
    ]);
    let r = call(fn_array_column, &[rows, Value::Null, s("id")]);
    assert_eq!(
        dumped(&r),
        "array(2) {\n  [5]=>\n  array(2) {\n    [\"id\"]=>\n    int(5)\n    [\"name\"]=>\n    string(1) \"a\"\n  }\n  [6]=>\n  array(2) {\n    [\"id\"]=>\n    int(6)\n    [\"name\"]=>\n    string(1) \"b\"\n  }\n}\n"
    );
}

#[test]
fn t_array_column_missing_index() {
    let rows = list(vec![
        arr(vec![(sk("id"), i(1)), (sk("name"), s("a"))]),
        arr(vec![(sk("name"), s("b"))]),
        arr(vec![(sk("id"), i(3)), (sk("name"), s("c"))]),
    ]);
    let r = call(fn_array_column, &[rows, s("name"), s("id")]);
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [1]=>\n  string(1) \"a\"\n  [2]=>\n  string(1) \"b\"\n  [3]=>\n  string(1) \"c\"\n}\n"
    );
}

#[test]
fn t_array_chunk() {
    let base = list(vec![i(1), i(2), i(3), i(4), i(5)]);
    let r = call(fn_array_chunk, &[base, i(2)]);
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [0]=>\n  array(2) {\n    [0]=>\n    int(1)\n    [1]=>\n    int(2)\n  }\n  [1]=>\n  array(2) {\n    [0]=>\n    int(3)\n    [1]=>\n    int(4)\n  }\n  [2]=>\n  array(1) {\n    [0]=>\n    int(5)\n  }\n}\n"
    );
    let assoc = arr(vec![(sk("a"), i(1)), (sk("b"), i(2)), (sk("c"), i(3))]);
    let r = call(fn_array_chunk, &[assoc, i(2), Value::Bool(true)]);
    assert_eq!(
        dumped(&r),
        "array(2) {\n  [0]=>\n  array(2) {\n    [\"a\"]=>\n    int(1)\n    [\"b\"]=>\n    int(2)\n  }\n  [1]=>\n  array(1) {\n    [\"c\"]=>\n    int(3)\n  }\n}\n"
    );
    assert!(fn_array_chunk(&mut TestHost::new(), &[list(vec![i(1)]), i(0)]).is_err());
}

#[test]
fn t_array_count_values() {
    let base = list(vec![
        i(1),
        i(1),
        s("1"),
        s("hello"),
        s("hello"),
        Value::Float(2.5),
        s("123"),
    ]);
    let r = call(fn_array_count_values, &[base]);
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [1]=>\n  int(3)\n  [\"hello\"]=>\n  int(2)\n  [123]=>\n  int(1)\n}\n"
    );
}
