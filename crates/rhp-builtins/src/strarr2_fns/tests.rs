// tests — expected values captured from php 8.5.8

use super::*;
use crate::test_support::TestHost;

fn s(b: &str) -> Value {
    Value::str(b)
}
fn i(n: i64) -> Value {
    Value::Int(n)
}
fn call(f: crate::NativeFn, args: &[Value]) -> Value {
    f(&mut TestHost::new(), args).expect("no throw")
}
fn calls(f: crate::NativeFn, args: &[Value]) -> String {
    String::from_utf8(bytes_of(&call(f, args))).unwrap()
}
fn call_int(f: crate::NativeFn, args: &[Value]) -> i64 {
    match call(f, args) {
        Value::Int(n) => n,
        other => panic!("expected int, got {}", other.type_name()),
    }
}
fn dumped(v: &Value) -> String {
    let mut out = Vec::new();
    crate::dump::var_dump(&TestHost::new(), &mut out, v, 0);
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

// --- str_split -------------------------------------------------------

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

// --- strtr -----------------------------------------------------------

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

// --- substr_replace --------------------------------------------------

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

// --- str_word_count --------------------------------------------------

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

// --- chunk_split -----------------------------------------------------

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

// --- array_diff_key / array_intersect_key ----------------------------

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

// --- array_replace ---------------------------------------------------

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

// --- array_merge_recursive -------------------------------------------

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

// --- array_udiff / array_uintersect / array_diff_ukey ----------------

use crate::arraycmp_fns::{fn_array_diff_ukey, fn_array_udiff, fn_array_uintersect};

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
