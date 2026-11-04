// Expected values captured from php 8.5.8.

use super::*;
use crate::test_support::TestHost;
use rhp_values::{ArrayKey, PhpArray};

fn s(b: &str) -> Value {
    Value::str(b)
}
fn i(n: i64) -> Value {
    Value::Int(n)
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
fn dumped(v: &Value) -> String {
    let mut out = Vec::new();
    crate::ext::standard::var::var_dump(&TestHost::new(), &mut out, v, 0);
    String::from_utf8_lossy(&out).into_owned()
}
fn list(items: Vec<Value>) -> Value {
    let mut a = PhpArray::new();
    for v in items {
        a.push(v);
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

// --- str_split -------------------------------------------------------

// --- strtr -----------------------------------------------------------

// --- substr_replace --------------------------------------------------

// --- levenshtein -----------------------------------------------------

#[test]
fn levenshtein_cases() {
    assert_eq!(call_int(fn_levenshtein, &[s("kitten"), s("sitting")]), 3);
    assert_eq!(call_int(fn_levenshtein, &[s(""), s("abc")]), 3);
    assert_eq!(call_int(fn_levenshtein, &[s("abc"), s("")]), 3);
    assert_eq!(call_int(fn_levenshtein, &[s("abc"), s("abc")]), 0);
    assert_eq!(call_int(fn_levenshtein, &[s("flaw"), s("lawn")]), 2);
    assert_eq!(call_int(fn_levenshtein, &[s("gumbo"), s("gambol")]), 2);
    assert_eq!(call_int(fn_levenshtein, &[s("Hello"), s("hello")]), 1);
    // weighted costs: ins, rep, del
    assert_eq!(
        call_int(
            fn_levenshtein,
            &[s("kitten"), s("sitting"), i(1), i(2), i(1)]
        ),
        5
    );
    assert_eq!(
        call_int(fn_levenshtein, &[s("abc"), s("xyz"), i(10), i(1), i(10)]),
        3
    );
    assert_eq!(
        call_int(fn_levenshtein, &[s("ab"), s("abcd"), i(2), i(5), i(3)]),
        4
    );
}

// --- similar_text ----------------------------------------------------

// --- str_word_count --------------------------------------------------

// --- chunk_split -----------------------------------------------------

// --- array_diff_key / array_intersect_key ----------------------------

// --- array_replace ---------------------------------------------------

// --- array_merge_recursive -------------------------------------------

// --- array_udiff / array_uintersect / array_diff_ukey ----------------

use crate::ext::standard::array::{fn_array_diff_ukey, fn_array_udiff, fn_array_uintersect};

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
