// Expected strings captured from php 8.5.8.

use super::*;
use crate::ext::standard::array::fn_array_diff_ukey;
use crate::ext::standard::array::fn_array_udiff;
use crate::ext::standard::array::fn_array_uintersect;
use crate::ext::standard::r#type::fn_array_is_list;
use crate::test_support::TestHost;
use rhp_values::ArrayKey;
use rhp_values::PhpArray;

fn dumped(v: &Value) -> String {
    let mut out = Vec::new();
    crate::ext::standard::var::var_dump(&mut TestHost::new(), &mut out, v).expect("no throw");
    String::from_utf8_lossy(&out).into_owned()
}

fn call(f: crate::NativeFn, args: &[Value]) -> Value {
    f(&mut TestHost::new(), args).expect("no throw")
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

#[test]
fn zend_sort_matches_std_order() {
    for n in [0usize, 1, 2, 3, 5, 6, 7, 16, 17, 40, 1100] {
        let mut v: Vec<i64> = (0..n as i64).map(|i| (i * 7919) % 101).collect();
        let mut want = v.clone();
        want.sort();
        zend_sort(&mut v, &mut |a, b| (*a - *b).signum() as i32);
        assert_eq!(v, want, "n={n}");
    }
}

fn map(pairs: Vec<(&str, Value)>) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in pairs {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    }
    Value::Array(a)
}

/// An array like `['a'=>'green', 'b'=>'brown', 'c'=>'blue', 0=>'red']`.
fn mixed(str_pairs: Vec<(&str, Value)>, tail: Vec<Value>) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in str_pairs {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    }
    for v in tail {
        a.push(v);
    }
    Value::Array(a)
}

// --- array_diff_assoc -----------------------------------------------
// php -r '$a=["a"=>"green","b"=>"brown","c"=>"blue","red"];
//         $b=["a"=>"green","yellow","red"]; var_dump(array_diff_assoc($a,$b));'
#[test]
fn array_diff_assoc_cases() {
    let a = mixed(
        vec![("a", s("green")), ("b", s("brown")), ("c", s("blue"))],
        vec![s("red")],
    );
    let b = mixed(vec![("a", s("green"))], vec![s("yellow"), s("red")]);
    assert_eq!(
        dumped(&fn_array_diff_assoc(&mut TestHost::new(), &[a, b]).unwrap()),
        "array(3) {\n  [\"b\"]=>\n  string(5) \"brown\"\n  [\"c\"]=>\n  string(4) \"blue\"\n  [0]=>\n  string(3) \"red\"\n}\n"
    );
    // numeric/string values compared by their string form: 1 == "1".
    let x = map(vec![("x", i(1)), ("y", i(2))]);
    let y = map(vec![("x", s("1")), ("y", s("3"))]);
    assert_eq!(
        dumped(&fn_array_diff_assoc(&mut TestHost::new(), &[x, y]).unwrap()),
        "array(1) {\n  [\"y\"]=>\n  int(2)\n}\n"
    );
}

// --- array_intersect_assoc ------------------------------------------
#[test]
fn array_intersect_assoc_cases() {
    let a = mixed(
        vec![("a", s("green")), ("b", s("brown")), ("c", s("blue"))],
        vec![s("red")],
    );
    let b = mixed(
        vec![("a", s("green")), ("b", s("yellow"))],
        vec![s("blue"), s("red")],
    );
    assert_eq!(
        dumped(&fn_array_intersect_assoc(&mut TestHost::new(), &[a, b]).unwrap()),
        "array(1) {\n  [\"a\"]=>\n  string(5) \"green\"\n}\n"
    );
    let x = map(vec![("x", i(1)), ("y", i(2))]);
    let y = map(vec![("x", s("1")), ("y", s("3"))]);
    assert_eq!(
        dumped(&fn_array_intersect_assoc(&mut TestHost::new(), &[x, y]).unwrap()),
        "array(1) {\n  [\"x\"]=>\n  int(1)\n}\n"
    );
}

// --- array_replace_recursive ----------------------------------------
#[test]
fn array_replace_recursive_cases() {
    // nested arrays merge key-by-key; non-overlapping keys survive.
    let mut a1 = PhpArray::new();
    a1.insert(ArrayKey::Int(0), map(vec![("a", i(1)), ("b", i(2))]));
    a1.insert(ArrayKey::Int(1), s("x"));
    let mut a2 = PhpArray::new();
    a2.insert(ArrayKey::Int(0), map(vec![("b", i(9)), ("c", i(3))]));
    a2.insert(ArrayKey::Int(2), s("y"));
    assert_eq!(
        dumped(&fn_array_replace_recursive(&mut TestHost::new(), &[Value::Array(a1), Value::Array(a2)]).unwrap()),
        "array(3) {\n  [0]=>\n  array(3) {\n    [\"a\"]=>\n    int(1)\n    [\"b\"]=>\n    int(9)\n    [\"c\"]=>\n    int(3)\n  }\n  [1]=>\n  string(1) \"x\"\n  [2]=>\n  string(1) \"y\"\n}\n"
    );
    // a scalar replacement overwrites an existing array outright.
    let base = map(vec![("k", map(vec![("a", i(1))]))]);
    let rep = map(vec![("k", s("str"))]);
    assert_eq!(
        dumped(&fn_array_replace_recursive(&mut TestHost::new(), &[base, rep]).unwrap()),
        "array(1) {\n  [\"k\"]=>\n  string(3) \"str\"\n}\n"
    );
}

// --- array_find family ----------------------------------------------
// Callback is invoked with (value, key); verified against php 8.5.8.
fn abc() -> Value {
    map(vec![("x", i(10)), ("y", i(20)), ("z", i(30))])
}

#[test]
fn array_find_returns_first_value() {
    let mut host = TestHost::with_callback(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 15));
    let r = fn_array_find(&mut host, &[abc(), Value::Null]).unwrap();
    assert_eq!(dumped(&r), "int(20)\n");
    // no match -> null
    let mut host2 = TestHost::with_callback(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 100));
    assert_eq!(
        dumped(&fn_array_find(&mut host2, &[abc(), Value::Null]).unwrap()),
        "NULL\n"
    );
}

#[test]
fn array_find_passes_value_then_key() {
    // First invocation must receive [value=10, key="x"].
    let mut first: Option<(Value, Value)> = None;
    {
        let first_ref = &mut first;
        let mut host = TestHost::with_callback(move |a: &[Value]| {
            if first_ref.is_none() {
                *first_ref = Some((a[0].clone(), a[1].clone()));
            }
            Value::Bool(false)
        });
        let _ = fn_array_find(&mut host, &[abc(), Value::Null]).unwrap();
    }
    let (v, k) = first.unwrap();
    assert_eq!(dumped(&v), "int(10)\n");
    assert_eq!(dumped(&k), "string(1) \"x\"\n");
}

#[test]
fn array_find_key_returns_key() {
    let mut host = TestHost::with_callback(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 15));
    assert_eq!(
        dumped(&fn_array_find_key(&mut host, &[abc(), Value::Null]).unwrap()),
        "string(1) \"y\"\n"
    );
    let mut host2 = TestHost::with_callback(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 100));
    assert_eq!(
        dumped(&fn_array_find_key(&mut host2, &[abc(), Value::Null]).unwrap()),
        "NULL\n"
    );
}

#[test]
fn array_any_and_all() {
    let mut h = TestHost::with_callback(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 25));
    assert_eq!(
        dumped(&fn_array_any(&mut h, &[abc(), Value::Null]).unwrap()),
        "bool(true)\n"
    );
    let mut h2 = TestHost::with_callback(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 5));
    assert_eq!(
        dumped(&fn_array_all(&mut h2, &[abc(), Value::Null]).unwrap()),
        "bool(true)\n"
    );
    let mut h3 = TestHost::with_callback(|a: &[Value]| Value::Bool(conv::to_int(&a[0]) > 15));
    assert_eq!(
        dumped(&fn_array_all(&mut h3, &[abc(), Value::Null]).unwrap()),
        "bool(false)\n"
    );
}

// --- key_exists ------------------------------------------------------
#[test]
fn key_exists_cases() {
    let a = map(vec![("a", i(1)), ("b", i(2))]);
    assert_eq!(
        dumped(&fn_key_exists(&mut TestHost::new(), &[s("a"), a.clone()]).unwrap()),
        "bool(true)\n"
    );
    assert_eq!(
        dumped(&fn_key_exists(&mut TestHost::new(), &[s("zz"), a]).unwrap()),
        "bool(false)\n"
    );
}

fn imap(pairs: Vec<(i64, Value)>) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in pairs {
        a.insert(ArrayKey::Int(k), v);
    }
    Value::Array(a)
}

#[test]
fn array_diff_key_cases() {
    assert_eq!(
        dumped(&call(
            fn_array_diff_key,
            &[
                map(vec![("a", i(1)), ("b", i(2)), ("c", i(3))]),
                map(vec![("b", i(9)), ("d", i(9))])
            ]
        )),
        "array(2) {\n  [\"a\"]=>\n  int(1)\n  [\"c\"]=>\n  int(3)\n}\n"
    );
    assert_eq!(
        dumped(&call(
            fn_array_diff_key,
            &[
                imap(vec![(10, s("x")), (20, s("y")), (30, s("z"))]),
                imap(vec![(20, s("q"))])
            ]
        )),
        "array(2) {\n  [10]=>\n  string(1) \"x\"\n  [30]=>\n  string(1) \"z\"\n}\n"
    );
    assert_eq!(
        dumped(&call(
            fn_array_diff_key,
            &[
                map(vec![("a", i(1)), ("b", i(2)), ("c", i(3))]),
                map(vec![("a", i(9))]),
                map(vec![("c", i(9))])
            ]
        )),
        "array(1) {\n  [\"b\"]=>\n  int(2)\n}\n"
    );
}

#[test]
fn array_intersect_key_cases() {
    assert_eq!(
        dumped(&call(
            fn_array_intersect_key,
            &[
                map(vec![("a", i(1)), ("b", i(2)), ("c", i(3))]),
                map(vec![("b", i(9)), ("c", i(8)), ("d", i(7))])
            ]
        )),
        "array(2) {\n  [\"b\"]=>\n  int(2)\n  [\"c\"]=>\n  int(3)\n}\n"
    );
    assert_eq!(
        dumped(&call(
            fn_array_intersect_key,
            &[
                imap(vec![(0, s("a")), (1, s("b")), (2, s("c"))]),
                imap(vec![(1, s("x")), (2, s("y"))])
            ]
        )),
        "array(2) {\n  [1]=>\n  string(1) \"b\"\n  [2]=>\n  string(1) \"c\"\n}\n"
    );
    assert_eq!(
        dumped(&call(
            fn_array_intersect_key,
            &[
                map(vec![("a", i(1)), ("b", i(2)), ("c", i(3))]),
                map(vec![("a", i(0)), ("b", i(0))]),
                map(vec![("b", i(0)), ("c", i(0))])
            ]
        )),
        "array(1) {\n  [\"b\"]=>\n  int(2)\n}\n"
    );
}

#[test]
fn array_replace_cases() {
    assert_eq!(
        dumped(&call(
            fn_array_replace,
            &[
                map(vec![("a", i(1)), ("b", i(2))]),
                map(vec![("b", i(9)), ("c", i(3))])
            ]
        )),
        "array(3) {\n  [\"a\"]=>\n  int(1)\n  [\"b\"]=>\n  int(9)\n  [\"c\"]=>\n  int(3)\n}\n"
    );
    assert_eq!(
        dumped(&call(fn_array_replace, &[list(vec![i(1), i(2), i(3)]), imap(vec![(0, s("x")), (2, s("z"))])])),
        "array(3) {\n  [0]=>\n  string(1) \"x\"\n  [1]=>\n  int(2)\n  [2]=>\n  string(1) \"z\"\n}\n"
    );
    assert_eq!(
        dumped(&call(
            fn_array_replace,
            &[
                map(vec![("a", i(1))]),
                map(vec![("a", i(2))]),
                map(vec![("a", i(3)), ("b", i(4))])
            ]
        )),
        "array(2) {\n  [\"a\"]=>\n  int(3)\n  [\"b\"]=>\n  int(4)\n}\n"
    );
}

#[test]
fn array_merge_recursive_cases() {
    assert_eq!(
        dumped(&call(fn_array_merge_recursive, &[map(vec![("a", i(1))]), map(vec![("a", i(2))])])),
        "array(1) {\n  [\"a\"]=>\n  array(2) {\n    [0]=>\n    int(1)\n    [1]=>\n    int(2)\n  }\n}\n"
    );
    assert_eq!(
        dumped(&call(fn_array_merge_recursive, &[map(vec![("a", map(vec![("x", i(1))]))]), map(vec![("a", map(vec![("y", i(2))]))])])),
        "array(1) {\n  [\"a\"]=>\n  array(2) {\n    [\"x\"]=>\n    int(1)\n    [\"y\"]=>\n    int(2)\n  }\n}\n"
    );
    assert_eq!(
        dumped(&call(fn_array_merge_recursive, &[list(vec![i(1), i(2)]), list(vec![i(3), i(4)])])),
        "array(4) {\n  [0]=>\n  int(1)\n  [1]=>\n  int(2)\n  [2]=>\n  int(3)\n  [3]=>\n  int(4)\n}\n"
    );
    // scalar + array -> merged list
    assert_eq!(
        dumped(&call(fn_array_merge_recursive, &[map(vec![("a", i(1))]), map(vec![("a", list(vec![i(2), i(3)]))])])),
        "array(1) {\n  [\"a\"]=>\n  array(3) {\n    [0]=>\n    int(1)\n    [1]=>\n    int(2)\n    [2]=>\n    int(3)\n  }\n}\n"
    );
    // array + scalar -> scalar appended
    assert_eq!(
        dumped(&call(fn_array_merge_recursive, &[map(vec![("a", list(vec![i(1)]))]), map(vec![("a", i(2))])])),
        "array(1) {\n  [\"a\"]=>\n  array(2) {\n    [0]=>\n    int(1)\n    [1]=>\n    int(2)\n  }\n}\n"
    );
    // nested deep recursion
    assert_eq!(
        dumped(&call(fn_array_merge_recursive, &[map(vec![("a", map(vec![("b", i(1))]))]), map(vec![("a", map(vec![("b", i(2))]))])])),
        "array(1) {\n  [\"a\"]=>\n  array(1) {\n    [\"b\"]=>\n    array(2) {\n      [0]=>\n      int(1)\n      [1]=>\n      int(2)\n    }\n  }\n}\n"
    );
}

/// A host whose callbacks ignore the callable and compare their two
/// arguments as integers with `<=>`: enough to drive the comparator-based
/// set functions.
fn cmp_host() -> TestHost<'static> {
    TestHost::with_callback(|a| {
        Value::Int(match conv::to_int(&a[0]).cmp(&conv::to_int(&a[1])) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        })
    })
}

fn cb() -> Value {
    Value::str("cmp")
}

#[test]
fn array_udiff_cases() {
    // php: array_udiff([3,1,2,1], [2,4], <=>) => [0=>3, 1=>1, 3=>1]
    let r = fn_array_udiff(
        &mut cmp_host(),
        &[
            list(vec![i(3), i(1), i(2), i(1)]),
            list(vec![i(2), i(4)]),
            cb(),
        ],
    )
    .unwrap();
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [0]=>\n  int(3)\n  [1]=>\n  int(1)\n  [3]=>\n  int(1)\n}\n"
    );
}

#[test]
fn array_uintersect_cases() {
    // php: array_uintersect([1,2,3,2], [2,3,4], <=>) => [1=>2, 2=>3, 3=>2]
    let r = fn_array_uintersect(
        &mut cmp_host(),
        &[
            list(vec![i(1), i(2), i(3), i(2)]),
            list(vec![i(2), i(3), i(4)]),
            cb(),
        ],
    )
    .unwrap();
    assert_eq!(
        dumped(&r),
        "array(3) {\n  [1]=>\n  int(2)\n  [2]=>\n  int(3)\n  [3]=>\n  int(2)\n}\n"
    );
}

#[test]
fn array_diff_ukey_cases() {
    // php: array_diff_ukey([1,2,3], [0=>9, 2=>9], <=>) => [1=>2]
    let r = fn_array_diff_ukey(
        &mut cmp_host(),
        &[
            list(vec![i(1), i(2), i(3)]),
            imap(vec![(0, i(9)), (2, i(9))]),
            cb(),
        ],
    )
    .unwrap();
    assert_eq!(dumped(&r), "array(1) {\n  [1]=>\n  int(2)\n}\n");
}
