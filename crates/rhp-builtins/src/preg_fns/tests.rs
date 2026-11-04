// tests — expected values captured from php 8.5.8

use super::*;
use std::cell::RefCell;
use std::rc::Rc;

struct NullHost;
impl Host for NullHost {
    fn write_out(&mut self, _: &[u8]) {}
}

/// A host that dispatches `call_callable` to a Rust closure, for the
/// preg_replace_callback test (mirrors how other modules stub a callback).
struct CbHost<'a> {
    #[allow(clippy::type_complexity)]
    f: Box<dyn FnMut(&[Value]) -> Value + 'a>,
}
impl Host for CbHost<'_> {
    fn write_out(&mut self, _: &[u8]) {}
    fn call_callable(&mut self, _c: &Value, args: &[Value]) -> Result<Value, Thrown> {
        Ok((self.f)(args))
    }
}

fn s(b: &str) -> Value {
    Value::str(b)
}
fn re(args: &[Value]) -> Value {
    fn_preg_match(&mut NullHost, args).unwrap()
}

/// A fresh by-ref slot to pass as `&$matches`.
fn slot() -> Value {
    Value::Ref(Rc::new(RefCell::new(Value::Null)))
}
fn slot_val(v: &Value) -> Value {
    match v {
        Value::Ref(rc) => rc.borrow().clone(),
        o => o.clone(),
    }
}

// ---- small array readers for assertions -------------------------------

fn arr_str(v: &Value, key: ArrayKey) -> String {
    let Value::Array(a) = v else {
        panic!("not array")
    };
    match a.get(&key) {
        Some(Value::Str(p)) => String::from_utf8_lossy(p.as_bytes()).into_owned(),
        other => panic!(
            "expected string at {key:?}, got {:?}",
            other.map(|v| v.type_name())
        ),
    }
}
fn as_str(v: &Value) -> String {
    match v {
        Value::Str(p) => String::from_utf8_lossy(p.as_bytes()).into_owned(),
        o => panic!("expected string, got {}", o.type_name()),
    }
}
fn as_bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::Str(p) => p.as_bytes().to_vec(),
        o => panic!("expected string, got {}", o.type_name()),
    }
}

#[test]
fn match_simple() {
    let m = slot();
    let r = re(&[s("/\\d+/"), s("abc123"), m.clone()]);
    assert!(matches!(r, Value::Int(1)));
    assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(0)), "123");
}

#[test]
fn match_no_match() {
    let m = slot();
    let r = re(&[s("/xyz/"), s("abc"), m.clone()]);
    assert!(matches!(r, Value::Int(0)));
    // $matches set to an empty array
    let Value::Array(a) = slot_val(&m) else {
        panic!()
    };
    assert_eq!(a.len(), 0);
}

#[test]
fn match_groups() {
    // php: array(0=>'12-34',1=>'12',2=>'34')
    let m = slot();
    re(&[s("/(\\d+)-(\\d+)/"), s("12-34"), m.clone()]);
    let v = slot_val(&m);
    assert_eq!(arr_str(&v, ArrayKey::Int(0)), "12-34");
    assert_eq!(arr_str(&v, ArrayKey::Int(1)), "12");
    assert_eq!(arr_str(&v, ArrayKey::Int(2)), "34");
}

#[test]
fn match_named_groups() {
    // php: 0=>'2024-01','year'=>'2024',1=>'2024',2=>'01' (name before index)
    let m = slot();
    re(&[s("/(?<year>\\d{4})-(\\d{2})/"), s("2024-01"), m.clone()]);
    let v = slot_val(&m);
    let Value::Array(a) = &v else { panic!() };
    let keys: Vec<ArrayKey> = a.iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(
        keys,
        vec![
            ArrayKey::Int(0),
            ArrayKey::Str(PhpStr::from("year")),
            ArrayKey::Int(1),
            ArrayKey::Int(2),
        ]
    );
    assert_eq!(arr_str(&v, ArrayKey::Str(PhpStr::from("year"))), "2024");
    assert_eq!(arr_str(&v, ArrayKey::Int(2)), "01");
}

#[test]
fn match_named_groups_all_spellings() {
    // php accepts (?P<n>), (?<n>) and (?'n'); fancy-regex rejected the last.
    for pat in ["/(?P<y>\\d+)/", "/(?<y>\\d+)/", "/(?'y'\\d+)/"] {
        let m = slot();
        let r = re(&[s(pat), s("42"), m.clone()]);
        assert!(matches!(r, Value::Int(1)), "pattern {pat} failed to match");
        assert_eq!(
            arr_str(&slot_val(&m), ArrayKey::Str(PhpStr::from("y"))),
            "42"
        );
    }
}

#[test]
fn match_offset_capture() {
    // php: 0 => array('123', 3)
    let m = slot();
    re(&[
        s("/\\d+/"),
        s("abc123def"),
        m.clone(),
        Value::Int(PREG_OFFSET_CAPTURE),
    ]);
    let v = slot_val(&m);
    let Value::Array(a) = &v else { panic!() };
    let Some(Value::Array(inner)) = a.get(&ArrayKey::Int(0)) else {
        panic!()
    };
    assert_eq!(as_str(inner.get(&ArrayKey::Int(0)).unwrap()), "123");
    assert!(matches!(inner.get(&ArrayKey::Int(1)), Some(Value::Int(3))));
}

#[test]
fn match_trailing_unmatched_trimmed() {
    // php: (a)(b)?(c) on "ac" => 0=>'ac',1=>'a',2=>'',3=>'c'
    let m = slot();
    re(&[s("/(a)(b)?(c)/"), s("ac"), m.clone()]);
    let v = slot_val(&m);
    assert_eq!(arr_str(&v, ArrayKey::Int(2)), "");
    assert_eq!(arr_str(&v, ArrayKey::Int(3)), "c");
    // (a)(b)? on "a" trims trailing unmatched group 2 entirely
    let m2 = slot();
    re(&[s("/(a)(b)?/"), s("a"), m2.clone()]);
    let Value::Array(a2) = slot_val(&m2) else {
        panic!()
    };
    assert_eq!(a2.len(), 2); // only 0 and 1
}

#[test]
fn match_unmatched_as_null() {
    // php: (a)(b)?(c) on "ac" with UNMATCHED_AS_NULL => group 2 is null
    let m = slot();
    re(&[
        s("/(a)(b)?(c)/"),
        s("ac"),
        m.clone(),
        Value::Int(PREG_UNMATCHED_AS_NULL),
    ]);
    let Value::Array(a) = slot_val(&m) else {
        panic!()
    };
    assert!(matches!(a.get(&ArrayKey::Int(2)), Some(Value::Null)));
    assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(3)), "c");
}

#[test]
fn match_offset_arg() {
    // php: preg_match('/\d+/','ab12cd34',$m,0,4) => '34'
    let m = slot();
    re(&[
        s("/\\d+/"),
        s("ab12cd34"),
        m.clone(),
        Value::Int(0),
        Value::Int(4),
    ]);
    assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(0)), "34");
}

// ---- NEW: PCRE features fancy-regex could not handle -------------------

#[test]
fn feature_possessive_quantifier() {
    // php: /^a++a$/ on "aaa" => 0 (a++ eats all 'a', nothing left for the
    // trailing 'a'); /a*+b/ on "aaab" => 1. fancy-regex failed to compile.
    let m = slot();
    assert!(matches!(re(&[s("/^a++a$/"), s("aaa"), m]), Value::Int(0)));
    let m2 = slot();
    assert!(matches!(re(&[s("/a*+b/"), s("aaab"), m2]), Value::Int(1)));
}

#[test]
fn feature_lookbehind() {
    // php: /(?<=foo)bar/ on "foobar" => 1, match "bar"
    let m = slot();
    assert!(matches!(
        re(&[s("/(?<=foo)bar/"), s("foobar"), m.clone()]),
        Value::Int(1)
    ));
    assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(0)), "bar");
}

#[test]
fn feature_g_anchor() {
    // php: preg_match_all('/\G\d/','123 45') => ['1','2','3'] (stops at space)
    let (r, v) = match_all(&[s("/\\G\\d/"), s("123 45")]);
    assert!(matches!(r, Value::Int(3)));
    let Value::Array(a) = &v else { panic!() };
    let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else {
        panic!()
    };
    let got: Vec<String> = g0.iter().map(|(_, x)| as_str(x)).collect();
    assert_eq!(got, vec!["1", "2", "3"]);
}

#[test]
fn feature_recursion_and_subroutines() {
    // php: balanced parens via (?1); fancy-regex could not compile these.
    let m = slot();
    assert!(matches!(
        re(&[s("/^(\\((?:[^()]++|(?1))*\\))$/"), s("(a(b)c)"), m]),
        Value::Int(1)
    ));
    // (?R) whole-pattern recursion
    let m2 = slot();
    assert!(matches!(
        re(&[s("/\\((?:[^()]++|(?R))*\\)/"), s("(a(b)c)"), m2]),
        Value::Int(1)
    ));
    // \g<1> subroutine call
    let m3 = slot();
    assert!(matches!(
        re(&[s("/(\\d+)-\\g<1>/"), s("12-34"), m3.clone()]),
        Value::Int(1)
    ));
    assert_eq!(arr_str(&slot_val(&m3), ArrayKey::Int(0)), "12-34");
}

#[test]
fn feature_ascii_vs_unicode_classes() {
    // php: \d is ASCII-only without /u, Unicode-aware with /u.
    // '٥' is U+0665 ARABIC-INDIC DIGIT FIVE.
    let m = slot();
    assert!(
        matches!(re(&[s("/^\\d+$/"), s("٥"), m]), Value::Int(0)),
        "without /u, \\d must not match an Arabic-Indic digit"
    );
    let m2 = slot();
    assert!(
        matches!(re(&[s("/^\\d+$/u"), s("٥"), m2]), Value::Int(1)),
        "with /u, \\d must match an Arabic-Indic digit"
    );
    // \w likewise: 'ñ' (U+00F1) only matches under /u.
    let m3 = slot();
    assert!(matches!(re(&[s("/^\\w+$/"), s("ñ"), m3]), Value::Int(0)));
    let m4 = slot();
    assert!(matches!(re(&[s("/^\\w+$/u"), s("ñ"), m4]), Value::Int(1)));
}

#[test]
fn feature_invalid_utf8_subject_under_u() {
    // php: an invalid UTF-8 subject with /u returns false (PREG_BAD_UTF8).
    let m = slot();
    let bad = Value::str(vec![0xff, 0xfe]);
    assert!(matches!(re(&[s("/./u"), bad, m]), Value::Bool(false)));
    // without /u, raw bytes match fine.
    let m2 = slot();
    let bad2 = Value::str(vec![0xff, 0xfe]);
    assert!(matches!(re(&[s("/./"), bad2, m2]), Value::Int(1)));
}

#[test]
fn feature_extra_modifiers() {
    // U (ungreedy): a.+ matches as little as possible.
    let m = slot();
    re(&[s("/a.+/U"), s("axxbxx"), m.clone()]);
    assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(0)), "ax");
    // A (anchored): only matches at the start.
    let m2 = slot();
    assert!(matches!(re(&[s("/bar/A"), s("foobar"), m2]), Value::Int(0)));
    let m3 = slot();
    assert!(matches!(re(&[s("/foo/A"), s("foobar"), m3]), Value::Int(1)));
    // D (dollar end only): $ does not match before a trailing newline.
    let m4 = slot();
    assert!(matches!(re(&[s("/foo$/D"), s("foo\n"), m4]), Value::Int(0)));
    let m5 = slot();
    assert!(matches!(re(&[s("/foo$/"), s("foo\n"), m5]), Value::Int(1)));
    // S / X are accepted as no-ops.
    let m6 = slot();
    assert!(matches!(re(&[s("/foo/SX"), s("foo"), m6]), Value::Int(1)));
    // an unknown modifier is rejected => false.
    let m7 = slot();
    assert!(matches!(
        re(&[s("/foo/z"), s("foo"), m7]),
        Value::Bool(false)
    ));
}

#[test]
fn match_trailing_unmatched_trimmed_middle_kept() {
    // middle unmatched group kept, trailing trimmed: nothing new, sanity.
    let m = slot();
    re(&[s("/(a)(b)?(c)/"), s("ac"), m.clone()]);
    let Value::Array(a) = slot_val(&m) else {
        panic!()
    };
    assert_eq!(a.len(), 4);
}

fn match_all(args: &[Value]) -> (Value, Value) {
    let m = slot();
    let mut full = args.to_vec();
    full.insert(2, m.clone());
    let r = fn_preg_match_all(&mut NullHost, &full).unwrap();
    (r, slot_val(&m))
}

#[test]
fn match_all_pattern_order() {
    // php PATTERN_ORDER: 0=>['1a','2b','3c'],1=>['1','2','3'],2=>['a','b','c']
    let (r, v) = match_all(&[s("/(\\d)(\\w)/"), s("1a2b3c")]);
    assert!(matches!(r, Value::Int(3)));
    let Value::Array(a) = &v else { panic!() };
    let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else {
        panic!()
    };
    assert_eq!(as_str(g0.get(&ArrayKey::Int(1)).unwrap()), "2b");
    let Some(Value::Array(g2)) = a.get(&ArrayKey::Int(2)) else {
        panic!()
    };
    assert_eq!(as_str(g2.get(&ArrayKey::Int(2)).unwrap()), "c");
}

#[test]
fn match_all_set_order() {
    // php SET_ORDER: [[ '1a','1','a'], ['2b','2','b'], ['3c','3','c']]
    let (r, v) = match_all(&[s("/(\\d)(\\w)/"), s("1a2b3c"), Value::Int(PREG_SET_ORDER)]);
    assert!(matches!(r, Value::Int(3)));
    let Value::Array(a) = &v else { panic!() };
    let Some(Value::Array(set1)) = a.get(&ArrayKey::Int(1)) else {
        panic!()
    };
    assert_eq!(as_str(set1.get(&ArrayKey::Int(0)).unwrap()), "2b");
    assert_eq!(as_str(set1.get(&ArrayKey::Int(2)).unwrap()), "b");
}

#[test]
fn match_all_zero() {
    // php: returns 0, $matches == [0 => []]
    let (r, v) = match_all(&[s("/z/"), s("abc")]);
    assert!(matches!(r, Value::Int(0)));
    let Value::Array(a) = &v else { panic!() };
    assert_eq!(a.len(), 1);
    let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else {
        panic!()
    };
    assert_eq!(g0.len(), 0);
}

#[test]
fn match_all_empty_lazy() {
    // php: preg_match_all('/a*?/','aaa') => ['','a','','a','','a',''] (7).
    // This is the key case the old naive loop got wrong (would give 4).
    let (r, v) = match_all(&[s("/a*?/"), s("aaa")]);
    assert!(matches!(r, Value::Int(7)), "expected 7 matches");
    let Value::Array(a) = &v else { panic!() };
    let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else {
        panic!()
    };
    let got: Vec<String> = g0.iter().map(|(_, x)| as_str(x)).collect();
    assert_eq!(got, vec!["", "a", "", "a", "", "a", ""]);
}

#[test]
fn match_all_empty_star() {
    // php: preg_match_all('/x*/','axb') => ['','x','',''] (4)
    let (r, v) = match_all(&[s("/x*/"), s("axb")]);
    assert!(matches!(r, Value::Int(4)));
    let Value::Array(a) = &v else { panic!() };
    let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else {
        panic!()
    };
    let got: Vec<String> = g0.iter().map(|(_, x)| as_str(x)).collect();
    assert_eq!(got, vec!["", "x", "", ""]);
}

fn replace(args: &[Value]) -> Value {
    fn_preg_replace(&mut NullHost, args).unwrap()
}

#[test]
fn replace_backrefs() {
    // swap words: $2 $1
    assert_eq!(
        as_str(&replace(&[
            s("/(\\w+)\\s(\\w+)/"),
            s("$2 $1"),
            s("hello world")
        ])),
        "world hello"
    );
    // \1 backslash form
    assert_eq!(
        as_str(&replace(&[s("/(\\d+)/"), s("[\\1]"), s("a1b22c")])),
        "a[1]b[22]c"
    );
    // ${1} brace form, and disambiguation ${1}2 => group1 + '2'
    assert_eq!(
        as_str(&replace(&[s("/(\\d)(\\d)/"), s("${1}2"), s("ab45cd")])),
        "ab42cd"
    );
    // $12 reads two digits => group 12 (nonexistent) => ''
    assert_eq!(
        as_str(&replace(&[s("/(\\d)(\\d)/"), s("$12"), s("ab45cd")])),
        "abcd"
    );
    // \$ is a literal dollar, \n stays verbatim
    assert_eq!(as_str(&replace(&[s("/a/"), s("x\\$1y"), s("a")])), "x$1y");
    assert_eq!(as_str(&replace(&[s("/a/"), s("x\\nz"), s("a")])), "x\\nz");
    // no match returns subject unchanged
    assert_eq!(as_str(&replace(&[s("/z/"), s("#"), s("abc")])), "abc");
}

#[test]
fn replace_limit_and_count() {
    // limit 2
    assert_eq!(
        as_str(&replace(&[s("/\\d/"), s("#"), s("1234"), Value::Int(2)])),
        "##34"
    );
    // count out-param
    let c = slot();
    fn_preg_replace(
        &mut NullHost,
        &[s("/\\d/"), s("#"), s("1a2b3"), Value::Int(-1), c.clone()],
    )
    .unwrap();
    assert!(matches!(slot_val(&c), Value::Int(3)));
}

#[test]
fn replace_array_subject_and_pattern() {
    // subject array: keys preserved
    let mut subj = PhpArray::new();
    subj.push(s("a1"));
    subj.push(s("b2c3"));
    let r = replace(&[s("/\\d/"), s("#"), Value::Array(subj)]);
    let Value::Array(a) = &r else { panic!() };
    assert_eq!(as_str(a.get(&ArrayKey::Int(0)).unwrap()), "a#");
    assert_eq!(as_str(a.get(&ArrayKey::Int(1)).unwrap()), "b#c#");
    // pattern array + replacement array, applied in sequence: XYc
    let mut pats = PhpArray::new();
    pats.push(s("/a/"));
    pats.push(s("/b/"));
    let mut reps = PhpArray::new();
    reps.push(s("X"));
    reps.push(s("Y"));
    let r2 = replace(&[Value::Array(pats), Value::Array(reps), s("abc")]);
    assert_eq!(as_str(&r2), "XYc");
}

#[test]
fn replace_empty_matches() {
    // php: preg_replace('/x*/','-','axb') => '-a--b-'
    assert_eq!(as_str(&replace(&[s("/x*/"), s("-"), s("axb")])), "-a--b-");
    // php: preg_replace('//','-','ab') => '-a-b-'
    assert_eq!(as_str(&replace(&[s("//"), s("-"), s("ab")])), "-a-b-");
}

#[test]
fn replace_callback() {
    // double each number: 'a3b4' => 'a6b8'
    let mut host = CbHost {
        f: Box::new(|args: &[Value]| {
            let Value::Array(m) = &args[0] else { panic!() };
            let whole = m.get(&ArrayKey::Int(0)).unwrap();
            let n = rhp_values::conv::to_int(whole);
            Value::str((n * 2).to_string())
        }),
    };
    let r =
        fn_preg_replace_callback(&mut host, &[s("/\\d+/"), Value::Null, s("a3b4")]).unwrap();
    assert_eq!(as_str(&r), "a6b8");

    // use capture groups: swap the two digits of each pair
    let mut host2 = CbHost {
        f: Box::new(|args: &[Value]| {
            let Value::Array(m) = &args[0] else { panic!() };
            let g1 = as_str(m.get(&ArrayKey::Int(1)).unwrap());
            let g2 = as_str(m.get(&ArrayKey::Int(2)).unwrap());
            Value::str(format!("{g2}{g1}"))
        }),
    };
    let r2 =
        fn_preg_replace_callback(&mut host2, &[s("/(\\d)(\\d)/"), Value::Null, s("x12y34")])
            .unwrap();
    assert_eq!(as_str(&r2), "x21y43");
}

fn split(args: &[Value]) -> Vec<String> {
    let Value::Array(a) = fn_preg_split(&mut NullHost, args).unwrap() else {
        panic!()
    };
    a.iter().map(|(_, v)| as_str(v)).collect()
}

#[test]
fn split_basic_and_limit() {
    assert_eq!(split(&[s("/,/"), s("a,b,c")]), vec!["a", "b", "c"]);
    assert_eq!(
        split(&[s("/,/"), s("a,b,c,d"), Value::Int(2)]),
        vec!["a", "b,c,d"]
    );
    assert_eq!(
        split(&[s("/\\s+/"), s("hello   world  foo")]),
        vec!["hello", "world", "foo"]
    );
}

#[test]
fn split_flags() {
    // NO_EMPTY
    assert_eq!(
        split(&[
            s("/,/"),
            s("a,,b,"),
            Value::Int(-1),
            Value::Int(PREG_SPLIT_NO_EMPTY)
        ]),
        vec!["a", "b"]
    );
    // DELIM_CAPTURE
    assert_eq!(
        split(&[
            s("/([,;])/"),
            s("a,b;c"),
            Value::Int(-1),
            Value::Int(PREG_SPLIT_DELIM_CAPTURE)
        ]),
        vec!["a", ",", "b", ";", "c"]
    );
    // DELIM_CAPTURE | NO_EMPTY
    assert_eq!(
        split(&[
            s("/(\\d)/"),
            s("a1b2"),
            Value::Int(-1),
            Value::Int(PREG_SPLIT_DELIM_CAPTURE | PREG_SPLIT_NO_EMPTY)
        ]),
        vec!["a", "1", "b", "2"]
    );
}

#[test]
fn split_offset_capture() {
    // php: preg_split('/,/','a,bb',-1,PREG_SPLIT_OFFSET_CAPTURE)
    //   => [['a',0],['bb',2]]
    let Value::Array(a) = fn_preg_split(
        &mut NullHost,
        &[
            s("/,/"),
            s("a,bb"),
            Value::Int(-1),
            Value::Int(PREG_SPLIT_OFFSET_CAPTURE),
        ],
    )
    .unwrap() else {
        panic!()
    };
    let Some(Value::Array(p0)) = a.get(&ArrayKey::Int(0)) else {
        panic!()
    };
    assert_eq!(as_str(p0.get(&ArrayKey::Int(0)).unwrap()), "a");
    assert!(matches!(p0.get(&ArrayKey::Int(1)), Some(Value::Int(0))));
    let Some(Value::Array(p1)) = a.get(&ArrayKey::Int(1)) else {
        panic!()
    };
    assert_eq!(as_str(p1.get(&ArrayKey::Int(0)).unwrap()), "bb");
    assert!(matches!(p1.get(&ArrayKey::Int(1)), Some(Value::Int(2))));
}

#[test]
fn split_empty_and_edges() {
    // empty pattern: '',a,b,c,''
    assert_eq!(split(&[s("//"), s("abc")]), vec!["", "a", "b", "c", ""]);
    // /a*/ on 'aba' => '','','b','',''
    assert_eq!(split(&[s("/a*/"), s("aba")]), vec!["", "", "b", "", ""]);
    // leading / trailing delimiters
    assert_eq!(split(&[s("/,/"), s(",a,b")]), vec!["", "a", "b"]);
    assert_eq!(split(&[s("/,/"), s("a,b,")]), vec!["a", "b", ""]);
}

#[test]
fn split_delim_capture_optional() {
    // (,)|(;) on 'a,b;c': a, ',', b, '', ';', c
    assert_eq!(
        split(&[
            s("/(,)|(;)/"),
            s("a,b;c"),
            Value::Int(-1),
            Value::Int(PREG_SPLIT_DELIM_CAPTURE)
        ]),
        vec!["a", ",", "b", "", ";", "c"]
    );
}

fn quote(args: &[Value]) -> String {
    as_str(&fn_preg_quote(&mut NullHost, args).unwrap())
}

#[test]
fn quote_cases() {
    assert_eq!(quote(&[s("a.b*c+d?e(f)g")]), "a\\.b\\*c\\+d\\?e\\(f\\)g");
    assert_eq!(quote(&[s("a/b/c"), s("/")]), "a\\/b\\/c");
    // full metachar sweep (delimiterless: '/' not escaped)
    assert_eq!(
        quote(&[s(". \\ + * ? [ ^ ] $ ( ) { } = ! < > | : - # /")]),
        "\\. \\\\ \\+ \\* \\? \\[ \\^ \\] \\$ \\( \\) \\{ \\} \\= \\! \\< \\> \\| \\: \\- \\# /"
    );
    // NUL byte -> \000
    let nul = fn_preg_quote(&mut NullHost, &[Value::str(vec![b'a', 0, b'b'])]).unwrap();
    assert_eq!(as_bytes(&nul), b"a\\000b");
}

fn grep(args: &[Value]) -> Vec<(ArrayKey, String)> {
    let Value::Array(a) = fn_preg_grep(&mut NullHost, args).unwrap() else {
        panic!()
    };
    a.iter().map(|(k, v)| (k.clone(), as_str(v))).collect()
}

#[test]
fn grep_cases() {
    let mut input = PhpArray::new();
    input.push(s("a1"));
    input.push(s("bb"));
    input.push(s("c3"));
    input.push(s("dd"));
    // php preg_grep('/\d/', ...) => [0=>'a1',2=>'c3'] (keys preserved)
    let r = grep(&[s("/\\d/"), Value::Array(input.clone())]);
    assert_eq!(
        r,
        vec![
            (ArrayKey::Int(0), "a1".into()),
            (ArrayKey::Int(2), "c3".into())
        ]
    );
    // INVERT => [1=>'bb',3=>'dd']
    let r2 = grep(&[
        s("/\\d/"),
        Value::Array(input),
        Value::Int(PREG_GREP_INVERT),
    ]);
    assert_eq!(
        r2,
        vec![
            (ArrayKey::Int(1), "bb".into()),
            (ArrayKey::Int(3), "dd".into())
        ]
    );
}

#[test]
fn invalid_pattern_returns_false() {
    let m = slot();
    assert!(matches!(re(&[s("/(/"), s("abc"), m]), Value::Bool(false)));
    assert!(matches!(
        fn_preg_split(&mut NullHost, &[s("bad"), s("x")]).unwrap(),
        Value::Bool(false)
    ));
    // preg_replace returns null on a bad pattern
    assert!(matches!(
        replace(&[s("/(/"), s("x"), s("abc")]),
        Value::Null
    ));
}

#[test]
fn flag_modifiers() {
    // case-insensitive
    let m = slot();
    assert!(matches!(re(&[s("/ABC/i"), s("xabcy"), m]), Value::Int(1)));
    // dotall: a.b matches across newline
    let m2 = slot();
    assert!(matches!(re(&[s("/a.b/s"), s("a\nb"), m2]), Value::Int(1)));
    // multiline: ^ matches after a newline
    let m2b = slot();
    assert!(matches!(re(&[s("/^b/m"), s("a\nb"), m2b]), Value::Int(1)));
    // extended: whitespace in the pattern is ignored
    let m2c = slot();
    assert!(matches!(re(&[s("/a b c/x"), s("abc"), m2c]), Value::Int(1)));
    // alternate delimiters
    let m3 = slot();
    assert!(matches!(
        re(&[s("#/path/#"), s("/path/"), m3]),
        Value::Int(1)
    ));
    let m4 = slot();
    assert!(matches!(re(&[s("{\\d+}"), s("a12b"), m4]), Value::Int(1)));
}
