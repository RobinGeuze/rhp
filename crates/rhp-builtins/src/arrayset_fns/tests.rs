// Expected values captured from php 8.5.8.

use super::*;
use crate::test_support::TestHost;

fn s(b: &str) -> Value {
    Value::str(b)
}
fn i(n: i64) -> Value {
    Value::Int(n)
}
fn dumped(v: &Value) -> String {
    let mut out = Vec::new();
    crate::ext::standard::var::var_dump(&TestHost::new(), &mut out, v, 0);
    String::from_utf8_lossy(&out).into_owned()
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
