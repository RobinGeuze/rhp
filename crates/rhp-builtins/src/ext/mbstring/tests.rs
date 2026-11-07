use super::*;
use crate::test_support::TestHost;

#[test]
fn convert_variables_recursion_warns() {
    // $r = ["\xe9"]; $r[] = &$r;
    let cell: rhp_values::PhpRef = Rc::new(RefCell::new(Value::Null));
    let mut a = PhpArray::new();
    a.push(Value::str(b"\xe9"));
    a.push(Value::Ref(cell.clone()));
    *cell.borrow_mut() = Value::Array(a);
    for from in ["ISO-8859-1", "ISO-8859-1,UTF-8"] {
        let (r, d) = crate::test_support::run(
            fn_mb_convert_variables,
            &[
                Value::str("UTF-8"),
                Value::str(from),
                Value::Ref(cell.clone()),
            ],
        );
        assert!(matches!(r, Ok(Value::Bool(false))));
        assert_eq!(
            d,
            vec!["Warning: mb_convert_variables(): Cannot handle recursive references"]
        );
    }
    // the single-encoding pass converted the first element before failing
    let Value::Array(a) = &*cell.borrow() else {
        panic!()
    };
    assert_eq!(
        rhp_values::conv::to_string_bytes(a.get_index(0).unwrap().1),
        b"\xc3\xa9"
    );
    let (r, d) = crate::test_support::run(
        fn_mb_convert_variables,
        &[
            Value::str("UTF-8"),
            Value::str("auto"),
            Value::str(b"h\xe9llo"),
        ],
    );
    assert!(d.is_empty());
    assert_eq!(
        rhp_values::conv::to_string_bytes(&r.ok().unwrap()),
        b"ASCII"
    );
}

/// Nesting deeper than any fixed limit converts, as in php (whose limit is
/// its stack size). The walk keeps its stack on the heap; the big thread
/// stack is for building and dropping the nested arrays.
#[test]
fn convert_variables_deep_nesting() {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(|| {
            const DEPTH: usize = 50_000;
            let mut v = Value::str(b"\xe9");
            for _ in 0..DEPTH {
                let mut a = PhpArray::new();
                a.push(v);
                v = Value::Array(a);
            }
            let cell: rhp_values::PhpRef = Rc::new(RefCell::new(v));
            for (from, want) in [("ISO-8859-1", "ISO-8859-1"), ("ASCII,UTF-8", "UTF-8")] {
                let (r, d) = crate::test_support::run(
                    fn_mb_convert_variables,
                    &[
                        Value::str("UTF-8"),
                        Value::str(from),
                        Value::Ref(cell.clone()),
                    ],
                );
                assert!(d.is_empty(), "{d:?}");
                assert_eq!(
                    rhp_values::conv::to_string_bytes(&r.ok().unwrap()),
                    want.as_bytes()
                );
            }
            let mut v = cell.borrow().clone();
            for _ in 0..DEPTH {
                let Value::Array(a) = v else { panic!() };
                v = a.get_index(0).unwrap().1.clone();
            }
            assert_eq!(rhp_values::conv::to_string_bytes(&v), b"\xc3\xa9");
        })
        .unwrap()
        .join()
        .unwrap();
}

fn s(b: &str) -> Value {
    Value::str(b)
}
fn call(f: crate::NativeFn, args: &[Value]) -> Value {
    f(&mut TestHost::new(), args).expect("no throw")
}
fn calls(f: crate::NativeFn, args: &[Value]) -> String {
    String::from_utf8(rhp_values::conv::to_string_bytes(&call(f, args))).unwrap()
}
fn call_hex(f: crate::NativeFn, args: &[Value]) -> String {
    rhp_values::conv::to_string_bytes(&call(f, args))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn call_int(f: crate::NativeFn, args: &[Value]) -> i64 {
    match call(f, args) {
        Value::Int(n) => n,
        other => panic!("expected int, got {}", other.type_name()),
    }
}
fn call_pos(f: crate::NativeFn, args: &[Value]) -> Option<i64> {
    match call(f, args) {
        Value::Int(n) => Some(n),
        Value::Bool(false) => None,
        other => panic!("expected int|false, got {}", other.type_name()),
    }
}
fn is_false(f: crate::NativeFn, args: &[Value]) -> bool {
    matches!(call(f, args), Value::Bool(false))
}
fn split_vec(f: crate::NativeFn, args: &[Value]) -> Vec<String> {
    match call(f, args) {
        Value::Array(a) => a
            .iter()
            .map(|(_, v)| String::from_utf8(rhp_values::conv::to_string_bytes(v)).unwrap())
            .collect(),
        other => panic!("expected array, got {}", other.type_name()),
    }
}

#[test]
fn strlen_cases() {
    assert_eq!(call_int(fn_mb_strlen, &[s("héllo")]), 5);
    assert_eq!(call_int(fn_mb_strlen, &[s("")]), 0);
    assert_eq!(call_int(fn_mb_strlen, &[s("€uro")]), 4);
    assert_eq!(call_int(fn_mb_strlen, &[s("日本語")]), 3);
    // invalid UTF-8 chunking (ED A0 80 → 3 errors)
    assert_eq!(
        call_int(fn_mb_strlen, &[Value::str(vec![0xED, 0xA0, 0x80])]),
        3
    );
    assert_eq!(call_int(fn_mb_strlen, &[Value::str(vec![0xC3, 0x28])]), 2);
}

#[test]
fn substr_cases() {
    assert_eq!(
        calls(fn_mb_substr, &[s("héllo"), Value::Int(1), Value::Int(3)]),
        "éll"
    );
    assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(-2)]), "lo");
    assert_eq!(
        calls(fn_mb_substr, &[s("héllo"), Value::Int(1), Value::Int(-1)]),
        "éll"
    );
    assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(-10)]), "héllo");
    // invalid lead byte → substitute
    assert_eq!(
        call_hex(
            fn_mb_substr,
            &[
                Value::str(vec![0xC3, 0x28, 0x61, 0x62]),
                Value::Int(0),
                Value::Int(2)
            ]
        ),
        "3f28"
    );
}

#[test]
fn case_cases() {
    assert_eq!(calls(fn_mb_strtoupper, &[s("héllo wörld")]), "HÉLLO WÖRLD");
    assert_eq!(calls(fn_mb_strtolower, &[s("HÉLLO WÖRLD")]), "héllo wörld");
    assert_eq!(calls(fn_mb_strtoupper, &[s("straße")]), "STRASSE");
    assert_eq!(calls(fn_mb_strtoupper, &[s("µ")]), "Μ");
    assert_eq!(calls(fn_mb_strtoupper, &[s("×÷")]), "×÷"); // math signs unchanged
}

#[test]
fn final_sigma_cases() {
    // "ΟΣΟΣ" → "οσος" with the LAST sigma as final sigma ς (cf82)
    assert_eq!(call_hex(fn_mb_strtolower, &[s("ΟΣΟΣ")]), "cebfcf83cebfcf82");
    // isolated Σ (no preceding cased) → non-final σ
    assert_eq!(call_hex(fn_mb_strtolower, &[s("Σ")]), "cf83");
}

#[test]
fn convert_case_cases() {
    assert_eq!(
        calls(fn_mb_convert_case, &[s("héllo wörld"), Value::Int(2)]),
        "Héllo Wörld"
    );
    assert_eq!(
        calls(fn_mb_convert_case, &[s("it's a test"), Value::Int(2)]),
        "It's A Test"
    );
    assert_eq!(
        calls(fn_mb_convert_case, &[s("a.b a,b"), Value::Int(2)]),
        "A.b A,B"
    );
    assert_eq!(
        calls(fn_mb_convert_case, &[s("abc123def"), Value::Int(2)]),
        "Abc123Def"
    );
    assert_eq!(
        calls(fn_mb_convert_case, &[s("ßtra"), Value::Int(2)]),
        "Sstra"
    );
    // FOLD: ß → ss, µ → μ
    assert_eq!(
        call_hex(fn_mb_convert_case, &[s("ß"), Value::Int(3)]),
        "7373"
    );
    assert_eq!(
        call_hex(fn_mb_convert_case, &[s("µ"), Value::Int(3)]),
        "cebc"
    );
    // SIMPLE upper keeps ß
    assert_eq!(
        call_hex(fn_mb_convert_case, &[s("ß"), Value::Int(4)]),
        "c39f"
    );
}

#[test]
fn str_split_cases() {
    assert_eq!(
        split_vec(fn_mb_str_split, &[s("héllo")]),
        vec!["h", "é", "l", "l", "o"]
    );
    assert_eq!(
        split_vec(fn_mb_str_split, &[s("héllo"), Value::Int(2)]),
        vec!["hé", "ll", "o"]
    );
    assert!(fn_mb_str_split(&mut TestHost::new(), &[s("abc"), Value::Int(0)]).is_err());
}

#[test]
fn pos_cases() {
    assert_eq!(call_pos(fn_mb_strpos, &[s("héllo"), s("llo")]), Some(2));
    assert_eq!(
        call_pos(fn_mb_strpos, &[s("aéaéa"), s("a"), Value::Int(-1)]),
        Some(4)
    );
    assert_eq!(
        call_pos(fn_mb_stripos, &[s("Héllo WÖRLD"), s("wörld")]),
        Some(6)
    );
    assert!(is_false(fn_mb_stripos, &[s("maße"), s("MASSE")])); // simple fold: ß ≠ ss
    assert_eq!(call_pos(fn_mb_strrpos, &[s("aéaéa"), s("a")]), Some(4));
    assert_eq!(
        call_pos(fn_mb_strrpos, &[s("aéaéa"), s("a"), Value::Int(-2)]),
        Some(2)
    );
    assert_eq!(
        call_pos(fn_mb_strrpos, &[s("abcabc"), s("bc"), Value::Int(-3)]),
        Some(1)
    );
    assert_eq!(call_pos(fn_mb_strripos, &[s("AéAéA"), s("a")]), Some(4));
}

#[test]
fn count_width_cases() {
    assert_eq!(call_int(fn_mb_substr_count, &[s("ababab"), s("ab")]), 3);
    assert_eq!(call_int(fn_mb_substr_count, &[s("aaa"), s("aa")]), 1); // non-overlapping
    assert!(fn_mb_substr_count(&mut TestHost::new(), &[s("abc"), s("")]).is_err());
    assert_eq!(call_int(fn_mb_strwidth, &[s("日本語abc")]), 9);
    assert_eq!(call_int(fn_mb_strwidth, &[s("😀")]), 2);
}

#[test]
fn strimwidth_cases() {
    assert_eq!(
        calls(
            fn_mb_strimwidth,
            &[s("Hello World"), Value::Int(0), Value::Int(10), s("...")]
        ),
        "Hello W..."
    );
    assert_eq!(
        calls(
            fn_mb_strimwidth,
            &[s("日本語テスト"), Value::Int(0), Value::Int(6), s("...")]
        ),
        "日..."
    );
    assert_eq!(
        calls(
            fn_mb_strimwidth,
            &[s("Hello"), Value::Int(0), Value::Int(3)]
        ),
        "Hel"
    );
    assert_eq!(
        calls(
            fn_mb_strimwidth,
            &[s("日本語テスト"), Value::Int(2), Value::Int(6), s("..")]
        ),
        "語テ.."
    );
}

#[test]
fn scrub_cases() {
    assert_eq!(calls(fn_mb_scrub, &[s("valid")]), "valid");
    assert_eq!(
        calls(fn_mb_scrub, &[Value::str(b"abc\xff\xfedef")]),
        "abc??def"
    );
    assert_eq!(calls(fn_mb_scrub, &[Value::str(b"\xe6\x97")]), "?");
}

#[test]
fn ord_chr_cases() {
    assert_eq!(call_int(fn_mb_ord, &[s("é")]), 233);
    assert_eq!(call_int(fn_mb_ord, &[s("€")]), 8364);
    assert!(fn_mb_ord(&mut TestHost::new(), &[s("")]).is_err()); // php throws on empty
    assert_eq!(calls(fn_mb_chr, &[Value::Int(233)]), "é");
    assert!(is_false(fn_mb_chr, &[Value::Int(0x110000)]));
}

#[test]
fn str_pad_cases() {
    assert_eq!(
        calls(
            fn_mb_str_pad,
            &[s("héllo"), Value::Int(10), s("-*"), Value::Int(1)]
        ),
        "héllo-*-*-"
    );
    assert_eq!(
        calls(
            fn_mb_str_pad,
            &[s("héllo"), Value::Int(10), s("xy"), Value::Int(2)]
        ),
        "xyhélloxyx"
    );
    assert_eq!(calls(fn_mb_str_pad, &[s("héllo"), Value::Int(3)]), "héllo");
}

#[test]
fn encoding_cases() {
    // mb_convert_encoding($str, $to, $from): café UTF-8 → ISO-8859-1 (é → 0xe9)
    assert_eq!(
        call_hex(
            fn_mb_convert_encoding,
            &[s("café"), s("ISO-8859-1"), s("UTF-8")]
        ),
        "636166e9"
    );
    // reinterpreting those UTF-8 bytes as Latin-1 → UTF-8 (Ã,©)
    assert_eq!(
        call_hex(
            fn_mb_convert_encoding,
            &[s("café"), s("UTF-8"), s("ISO-8859-1")]
        ),
        "636166c383c2a9"
    );
    // "latin1" is an accepted alias for "ISO-8859-1"
    assert_eq!(
        call_hex(
            fn_mb_convert_encoding,
            &[s("café"), s("UTF-8"), s("latin1")]
        ),
        "636166c383c2a9"
    );
    // unmappable → '?'
    assert_eq!(
        call_hex(fn_mb_convert_encoding, &[s("café"), s("ASCII"), s("UTF-8")]),
        "6361663f"
    );
    assert!(matches!(
        call(fn_mb_check_encoding, &[s("héllo"), s("UTF-8")]),
        Value::Bool(true)
    ));
    assert!(is_false(
        fn_mb_check_encoding,
        &[Value::str(vec![0xC3, 0x28]), s("UTF-8")]
    ));
    // Without arguments: whether this request converted without losses
    // (the "café" to ASCII above lost one character).
    assert!(is_false(fn_mb_check_encoding, &[]));
    // invalid encoding name → ValueError
    assert!(fn_mb_strlen(&mut TestHost::new(), &[s("x"), s("bogus")]).is_err());
}

#[test]
fn internal_encoding_roundtrip() {
    assert_eq!(calls(fn_mb_internal_encoding, &[]), "UTF-8");
    assert!(matches!(
        call(fn_mb_internal_encoding, &[s("ISO-8859-1")]),
        Value::Bool(true)
    ));
    assert_eq!(calls(fn_mb_internal_encoding, &[]), "ISO-8859-1");
    // restore so other tests on this thread see the default
    let _ = call(fn_mb_internal_encoding, &[s("UTF-8")]);
    assert!(fn_mb_internal_encoding(&mut TestHost::new(), &[s("bogus")]).is_err());
}
