//! More pure string + array builtins (no callbacks, no by-reference):
//! `str_split`, `strtr`, `substr_replace`, `levenshtein`, `similar_text`,
//! `str_word_count`, `chunk_split`, `array_diff_key`, `array_intersect_key`,
//! `array_replace`, `array_merge_recursive`.
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see the `#[cfg(test)]` block — expected values hardcoded from
//! `php -r '...'` output).
//!
//! Wire this module into `lib.rs` with:
//! ```ignore
//! mod strarr2_fns;                 // near the other `mod` lines
//! // ...and inside registry(), after building `m`:
//! strarr2_fns::register(&mut m);
//! ```

use std::collections::HashMap;

use phrust_values::{conv, ArrayKey, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Deref a (possibly `Ref`) value to an owned value.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// `(string)` bytes of a value (after deref).
fn bytes_of(v: &Value) -> Vec<u8> {
    conv::to_string_bytes(&deref(v))
}

/// The byte form of an array key (int keys render as their decimal string).
fn key_bytes(k: &ArrayKey) -> Vec<u8> {
    match k {
        ArrayKey::Int(n) => n.to_string().into_bytes(),
        ArrayKey::Str(s) => s.as_bytes().to_vec(),
    }
}

/// Build php's `php_charmask` set (supports the `a..z` range syntax). Used by
/// `str_word_count`'s additional-characters argument.
fn charmask(chars: &[u8]) -> [bool; 256] {
    let mut mask = [false; 256];
    let len = chars.len();
    let mut i = 0;
    while i < len {
        let c = chars[i];
        if i + 3 < len && chars[i + 1] == b'.' && chars[i + 2] == b'.' && chars[i + 3] >= c {
            for b in c..=chars[i + 3] {
                mask[b as usize] = true;
            }
            i += 4;
        } else {
            mask[c as usize] = true;
            i += 1;
        }
    }
    mask
}

fn want_array(v: &Value, func: &str, argn: usize) -> Result<PhpArray, Thrown> {
    match deref(v) {
        Value::Array(a) => Ok(a),
        other => Err(Thrown::type_error(format!(
            "{func}(): Argument #{argn} must be of type array, {} given",
            type_name_lower(&other)
        ))),
    }
}

fn type_name_lower(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) | Value::Closure(_) => "object",
        Value::Ref(r) => type_name_lower(&r.borrow()),
    }
}

// ---------------------------------------------------------------------------
// str_split
// ---------------------------------------------------------------------------

fn fn_str_split(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let n = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(1);
    if n < 1 {
        return Err(Thrown::error(
            "ValueError",
            "str_split(): Argument #2 ($length) must be greater than 0",
        ));
    }
    let n = n as usize;
    let mut out = PhpArray::new();
    // php 8.2+: an empty subject yields an empty array.
    for chunk in s.chunks(n) {
        out.push(Value::str(chunk.to_vec()));
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// strtr
// ---------------------------------------------------------------------------

fn fn_strtr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    if args.len() == 2 {
        // Pairs form: longest matching key at each position wins.
        let Value::Array(pairs) = deref(&args[1]) else {
            return Err(Thrown::type_error(
                "strtr(): Argument #2 ($from) must be of type array, string given",
            ));
        };
        let table: Vec<(Vec<u8>, Vec<u8>)> = pairs
            .iter()
            .filter_map(|(k, v)| {
                let kb = key_bytes(k);
                // php ignores empty-string keys (and emits a warning to stderr).
                if kb.is_empty() {
                    None
                } else {
                    Some((kb, bytes_of(v)))
                }
            })
            .collect();
        let maxlen = table.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        if maxlen == 0 {
            return Ok(Value::str(s));
        }
        let mut out = Vec::with_capacity(s.len());
        let mut i = 0;
        while i < s.len() {
            let mut matched = false;
            let upper = maxlen.min(s.len() - i);
            for len in (1..=upper).rev() {
                let sub = &s[i..i + len];
                if let Some((_, rep)) = table.iter().find(|(k, _)| k.as_slice() == sub) {
                    out.extend_from_slice(rep);
                    i += len;
                    matched = true;
                    break;
                }
            }
            if !matched {
                out.push(s[i]);
                i += 1;
            }
        }
        Ok(Value::str(out))
    } else {
        // from/to form: byte-for-byte over the common prefix length.
        let from = bytes_of(&args[1]);
        let to = bytes_of(&args[2]);
        let n = from.len().min(to.len());
        let mut map: [Option<u8>; 256] = [None; 256];
        for i in 0..n {
            // php: first mapping for a given source byte wins.
            if map[from[i] as usize].is_none() {
                map[from[i] as usize] = Some(to[i]);
            }
        }
        let out: Vec<u8> = s.iter().map(|&b| map[b as usize].unwrap_or(b)).collect();
        Ok(Value::str(out))
    }
}

// ---------------------------------------------------------------------------
// substr_replace
// ---------------------------------------------------------------------------

/// Apply one substr_replace to a single subject. `length == None` means "to end".
fn substr_replace_one(subject: &[u8], start: i64, length: Option<i64>, replace: &[u8]) -> Vec<u8> {
    let len = subject.len() as i64;
    let f = if start < 0 {
        (len + start).max(0)
    } else {
        start.min(len)
    };
    let l = match length {
        None => len - f,
        Some(l) if l < 0 => (len - f + l).max(0),
        Some(l) => l.min(len - f),
    };
    let (f, l) = (f as usize, l as usize);
    let mut out = Vec::with_capacity(subject.len() - l + replace.len());
    out.extend_from_slice(&subject[..f]);
    out.extend_from_slice(replace);
    out.extend_from_slice(&subject[f + l..]);
    out
}

fn fn_substr_replace(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let subject = deref(&args[0]);
    let replace = deref(&args[1]);
    let start_arg = deref(&args[2]);
    let length_arg = args.get(3).map(deref);

    if let Value::Array(subj) = subject {
        let mut out = PhpArray::new();
        for (i, (k, v)) in subj.iter().enumerate() {
            let sbytes = bytes_of(v);
            // replace: array -> nth element (else ""); scalar -> whole, for all.
            let rep = match &replace {
                Value::Array(ra) => ra.get_index(i).map(|(_, rv)| bytes_of(rv)).unwrap_or_default(),
                other => bytes_of(other),
            };
            // start: array -> nth element (else 0); scalar -> per-subject.
            let st = match &start_arg {
                Value::Array(sa) => sa
                    .get_index(i)
                    .map(|(_, sv)| conv::to_int(&deref(sv)))
                    .unwrap_or(0),
                other => conv::to_int(other),
            };
            // length: omitted -> to end; array -> nth element (else full string);
            // scalar -> per-subject.
            let ln = match &length_arg {
                None => None,
                Some(Value::Array(la)) => Some(
                    la.get_index(i)
                        .map(|(_, lv)| conv::to_int(&deref(lv)))
                        .unwrap_or(sbytes.len() as i64),
                ),
                Some(other) => Some(conv::to_int(other)),
            };
            out.insert(k.clone(), Value::str(substr_replace_one(&sbytes, st, ln, &rep)));
        }
        Ok(Value::Array(out))
    } else {
        // Single-string subject: offset and length may not be arrays.
        if matches!(start_arg, Value::Array(_)) {
            return Err(Thrown::type_error(
                "substr_replace(): Argument #3 ($offset) cannot be an array when working on a single string",
            ));
        }
        if matches!(length_arg, Some(Value::Array(_))) {
            return Err(Thrown::type_error(
                "substr_replace(): Argument #4 ($length) cannot be an array when working on a single string",
            ));
        }
        let sbytes = conv::to_string_bytes(&subject);
        let rep = match &replace {
            Value::Array(ra) => ra.iter().next().map(|(_, rv)| bytes_of(rv)).unwrap_or_default(),
            other => bytes_of(other),
        };
        let st = conv::to_int(&start_arg);
        let ln = length_arg.as_ref().map(conv::to_int);
        Ok(Value::str(substr_replace_one(&sbytes, st, ln, &rep)))
    }
}

// ---------------------------------------------------------------------------
// levenshtein
// ---------------------------------------------------------------------------

fn fn_levenshtein(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = bytes_of(&args[0]);
    let b = bytes_of(&args[1]);
    let cost_ins = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(1);
    let cost_rep = args.get(3).map(|v| conv::to_int(&deref(v))).unwrap_or(1);
    let cost_del = args.get(4).map(|v| conv::to_int(&deref(v))).unwrap_or(1);

    let (la, lb) = (a.len(), b.len());
    if la == 0 {
        return Ok(Value::Int(lb as i64 * cost_ins));
    }
    if lb == 0 {
        return Ok(Value::Int(la as i64 * cost_del));
    }
    // Two-row DP (prev -> previous row, cur -> current row).
    let mut prev: Vec<i64> = (0..=lb as i64).map(|j| j * cost_ins).collect();
    let mut cur: Vec<i64> = vec![0; lb + 1];
    for i in 1..=la {
        cur[0] = i as i64 * cost_del;
        for j in 1..=lb {
            let del = prev[j] + cost_del;
            let ins = cur[j - 1] + cost_ins;
            let sub = prev[j - 1] + if a[i - 1] == b[j - 1] { 0 } else { cost_rep };
            cur[j] = del.min(ins).min(sub);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Ok(Value::Int(prev[lb]))
}

// ---------------------------------------------------------------------------
// similar_text (returns the common-char count; the by-ref percent out-param
// is intentionally not implemented)
// ---------------------------------------------------------------------------

/// php's `php_similar_str`: longest common substring and its positions.
fn similar_str(t1: &[u8], t2: &[u8]) -> (usize, usize, usize) {
    let (mut pos1, mut pos2, mut max) = (0usize, 0usize, 0usize);
    for p in 0..t1.len() {
        for q in 0..t2.len() {
            let mut l = 0;
            while p + l < t1.len() && q + l < t2.len() && t1[p + l] == t2[q + l] {
                l += 1;
            }
            if l > max {
                max = l;
                pos1 = p;
                pos2 = q;
            }
        }
    }
    (pos1, pos2, max)
}

/// php's `php_similar_char`: recursive sum of common characters.
fn similar_char(t1: &[u8], t2: &[u8]) -> usize {
    let (pos1, pos2, max) = similar_str(t1, t2);
    let mut sum = max;
    if sum > 0 {
        if pos1 > 0 && pos2 > 0 {
            sum += similar_char(&t1[..pos1], &t2[..pos2]);
        }
        if pos1 + max < t1.len() && pos2 + max < t2.len() {
            sum += similar_char(&t1[pos1 + max..], &t2[pos2 + max..]);
        }
    }
    sum
}

fn fn_similar_text(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = bytes_of(&args[0]);
    let b = bytes_of(&args[1]);
    Ok(Value::Int(similar_char(&a, &b) as i64))
}

// ---------------------------------------------------------------------------
// str_word_count
// ---------------------------------------------------------------------------

fn fn_str_word_count(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let format = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if !matches!(format, 0 | 1 | 2) {
        return Err(Thrown::error(
            "ValueError",
            "str_word_count(): Argument #2 ($format) must be a valid format value",
        ));
    }
    let extra = args.get(2).map(bytes_of);
    let extra_mask = extra.map(|e| charmask(&e)).unwrap_or([false; 256]);
    let is_word = |b: u8| b.is_ascii_alphabetic() || b == b'\'' || b == b'-' || extra_mask[b as usize];

    let mut count = 0i64;
    let mut words = PhpArray::new();
    let mut i = 0;
    while i < s.len() {
        if is_word(s[i]) {
            let start = i;
            while i < s.len() && is_word(s[i]) {
                i += 1;
            }
            count += 1;
            match format {
                1 => words.push(Value::str(s[start..i].to_vec())),
                2 => words.insert(ArrayKey::Int(start as i64), Value::str(s[start..i].to_vec())),
                _ => {}
            }
        } else {
            i += 1;
        }
    }
    if format == 0 {
        Ok(Value::Int(count))
    } else {
        Ok(Value::Array(words))
    }
}

// ---------------------------------------------------------------------------
// chunk_split
// ---------------------------------------------------------------------------

fn fn_chunk_split(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let chunklen = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(76);
    if chunklen < 1 {
        return Err(Thrown::error(
            "ValueError",
            "chunk_split(): Argument #2 ($length) must be greater than 0",
        ));
    }
    let end = match args.get(2) {
        Some(v) => bytes_of(v),
        None => b"\r\n".to_vec(),
    };
    // php appends `end` once for an empty subject.
    if s.is_empty() {
        return Ok(Value::str(end));
    }
    let chunklen = chunklen as usize;
    let mut out = Vec::with_capacity(s.len() + end.len());
    for chunk in s.chunks(chunklen) {
        out.extend_from_slice(chunk);
        out.extend_from_slice(&end);
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// array_diff_key / array_intersect_key
// ---------------------------------------------------------------------------

fn fn_array_diff_key(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = want_array(&args[0], "array_diff_key", 1)?;
    let mut others = Vec::new();
    for (i, arg) in args[1..].iter().enumerate() {
        others.push(want_array(arg, "array_diff_key", i + 2)?);
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        if !others.iter().any(|o| o.get(k).is_some()) {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_intersect_key(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = want_array(&args[0], "array_intersect_key", 1)?;
    let mut others = Vec::new();
    for (i, arg) in args[1..].iter().enumerate() {
        others.push(want_array(arg, "array_intersect_key", i + 2)?);
    }
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        if others.iter().all(|o| o.get(k).is_some()) {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_replace
// ---------------------------------------------------------------------------

fn fn_array_replace(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = want_array(&args[0], "array_replace", 1)?;
    for (i, arg) in args[1..].iter().enumerate() {
        let a = want_array(arg, "array_replace", i + 2)?;
        for (k, v) in a.iter() {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// array_merge_recursive
// ---------------------------------------------------------------------------

/// Merge `src` into `dest` per php's `php_array_merge_recursive`.
fn merge_recursive_into(dest: &mut PhpArray, src: &PhpArray) {
    for (k, v) in src.iter() {
        match k {
            // Integer keys are always appended (renumbered), never merged.
            ArrayKey::Int(_) => dest.push(v.clone()),
            ArrayKey::Str(_) => {
                let existing = dest.get(k).cloned();
                match existing {
                    None => {
                        dest.insert(k.clone(), v.clone());
                    }
                    Some(cur) => {
                        // Coerce the existing value to an array (scalars wrap as
                        // a single-element list), then fold in the new value.
                        let mut cur_arr = match cur {
                            Value::Array(a) => a,
                            other => {
                                let mut a = PhpArray::new();
                                a.push(other);
                                a
                            }
                        };
                        match deref(v) {
                            Value::Array(src_arr) => merge_recursive_into(&mut cur_arr, &src_arr),
                            other => cur_arr.push(other),
                        }
                        dest.insert(k.clone(), Value::Array(cur_arr));
                    }
                }
            }
        }
    }
}

fn fn_array_merge_recursive(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = PhpArray::new();
    for (i, arg) in args.iter().enumerate() {
        let a = want_array(arg, "array_merge_recursive", i + 1)?;
        merge_recursive_into(&mut out, &a);
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    }
    put(m, "str_split", fn_str_split, 1, Some(2));
    put(m, "strtr", fn_strtr, 2, Some(3));
    put(m, "substr_replace", fn_substr_replace, 3, Some(4));
    put(m, "levenshtein", fn_levenshtein, 2, Some(5));
    put(m, "similar_text", fn_similar_text, 2, Some(3));
    put(m, "str_word_count", fn_str_word_count, 1, Some(3));
    put(m, "chunk_split", fn_chunk_split, 1, Some(3));
    put(m, "array_diff_key", fn_array_diff_key, 2, None);
    put(m, "array_intersect_key", fn_array_intersect_key, 2, None);
    put(m, "array_replace", fn_array_replace, 1, None);
    put(m, "array_merge_recursive", fn_array_merge_recursive, 0, None);
}

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct NullHost;
    impl Host for NullHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    fn s(b: &str) -> Value {
        Value::str(b)
    }
    fn i(n: i64) -> Value {
        Value::Int(n)
    }
    fn call(f: crate::NativeFn, args: &[Value]) -> Value {
        f(&mut NullHost, args).expect("no throw")
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
        crate::dump::var_dump(&NullHost, &mut out, v, 0);
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
        assert!(fn_str_split(&mut NullHost, &[s("abc"), i(0)]).is_err());
    }

    // --- strtr -----------------------------------------------------------

    #[test]
    fn strtr_pairs() {
        assert_eq!(
            calls(fn_strtr, &[s("Hello World"), map(vec![("Hello", s("Hi")), ("World", s("Earth"))])]),
            "Hi Earth"
        );
        // longest key wins (ab before a)
        assert_eq!(
            calls(fn_strtr, &[s("abcabc"), map(vec![("a", s("1")), ("ab", s("X"))])]),
            "XcXc"
        );
        // output is not re-translated
        assert_eq!(calls(fn_strtr, &[s("aaa"), map(vec![("a", s("aa"))])]), "aaaaaa");
        assert_eq!(calls(fn_strtr, &[s("hi"), list(vec![])]), "hi");
        // integer keys coerce to their string form; longest wins
        assert_eq!(
            calls(fn_strtr, &[s("12"), imap(vec![(1, s("a")), (12, s("bb"))])]),
            "bb"
        );
        assert_eq!(
            calls(fn_strtr, &[s("test"), map(vec![("t", s("T")), ("te", s("X"))])]),
            "XsT"
        );
    }

    #[test]
    fn strtr_from_to() {
        assert_eq!(calls(fn_strtr, &[s("Hello"), s("el"), s("ip")]), "Hippo");
        assert_eq!(calls(fn_strtr, &[s("The fox"), s("abcdefghij"), s("0123456789")]), "T74 5ox");
        // to shorter than from: only the common prefix maps
        assert_eq!(calls(fn_strtr, &[s("abcd"), s("abcde"), s("xy")]), "xycd");
    }

    // --- substr_replace --------------------------------------------------

    #[test]
    fn substr_replace_string() {
        assert_eq!(calls(fn_substr_replace, &[s("Hello World"), s("PHP"), i(6)]), "Hello PHP");
        assert_eq!(calls(fn_substr_replace, &[s("Hello World"), s("PHP"), i(6), i(5)]), "Hello PHP");
        assert_eq!(calls(fn_substr_replace, &[s("Hello World"), s("PHP"), i(-5)]), "Hello PHP");
        assert_eq!(calls(fn_substr_replace, &[s("Hello World"), s("PHP"), i(-5), i(-1)]), "Hello PHPd");
        assert_eq!(calls(fn_substr_replace, &[s("Hello"), s("XXX"), i(0), i(0)]), "XXXHello");
        assert_eq!(calls(fn_substr_replace, &[s("Hello"), s("XXX"), i(2), i(-1)]), "HeXXXo");
        assert_eq!(calls(fn_substr_replace, &[s("abcdef"), s("-"), i(2), i(0)]), "ab-cdef");
        assert_eq!(calls(fn_substr_replace, &[s("abc"), s("X"), i(10)]), "abcX");
        assert_eq!(calls(fn_substr_replace, &[s("abc"), s("X"), i(10), i(5)]), "abcX");
        assert_eq!(calls(fn_substr_replace, &[s("abcdef"), s("XYZ"), i(-2), i(-5)]), "abcdXYZef");
        assert_eq!(calls(fn_substr_replace, &[s("abc"), s("X"), i(-99), i(1)]), "Xbc");
        assert_eq!(calls(fn_substr_replace, &[s("Hello World"), s("PHP"), i(6), i(0)]), "Hello PHPWorld");
        // scalar subject with array replace uses the first element
        assert_eq!(calls(fn_substr_replace, &[s("aaa"), list(vec![s("X"), s("Y")]), i(1), i(1)]), "aXa");
        assert_eq!(calls(fn_substr_replace, &[s("abc"), list(vec![]), i(1), i(1)]), "ac");
    }

    #[test]
    fn substr_replace_array() {
        assert_eq!(
            dumped(&call(fn_substr_replace, &[list(vec![s("Hello"), s("World")]), s("X"), i(1), i(2)])),
            "array(2) {\n  [0]=>\n  string(4) \"HXlo\"\n  [1]=>\n  string(4) \"WXld\"\n}\n"
        );
        assert_eq!(
            dumped(&call(fn_substr_replace, &[list(vec![s("aaa"), s("bbbb")]), list(vec![s("1"), s("2")]), i(1), i(1)])),
            "array(2) {\n  [0]=>\n  string(3) \"a1a\"\n  [1]=>\n  string(4) \"b2bb\"\n}\n"
        );
        assert_eq!(
            dumped(&call(fn_substr_replace, &[list(vec![s("aaa"), s("bbbb")]), list(vec![s("1"), s("2")]), list(vec![i(0), i(1)]), list(vec![i(1), i(2)])])),
            "array(2) {\n  [0]=>\n  string(3) \"1aa\"\n  [1]=>\n  string(3) \"b2b\"\n}\n"
        );
        // replace array shorter than subjects -> "" for the rest
        assert_eq!(
            dumped(&call(fn_substr_replace, &[list(vec![s("aaa"), s("bbb"), s("ccc")]), list(vec![s("X"), s("Y")]), i(1), i(1)])),
            "array(3) {\n  [0]=>\n  string(3) \"aXa\"\n  [1]=>\n  string(3) \"bYb\"\n  [2]=>\n  string(2) \"cc\"\n}\n"
        );
        // start/length arrays exhausted -> start 0, length full (whole replaced)
        assert_eq!(
            dumped(&call(fn_substr_replace, &[list(vec![s("aaa"), s("bbb")]), s("X"), list(vec![i(1)]), list(vec![i(1)])])),
            "array(2) {\n  [0]=>\n  string(3) \"aXa\"\n  [1]=>\n  string(1) \"X\"\n}\n"
        );
        // omitted length + start array
        assert_eq!(
            dumped(&call(fn_substr_replace, &[list(vec![s("aaa"), s("bbb"), s("ccc")]), s("X"), list(vec![i(0), i(1)])])),
            "array(3) {\n  [0]=>\n  string(1) \"X\"\n  [1]=>\n  string(2) \"bX\"\n  [2]=>\n  string(1) \"X\"\n}\n"
        );
        // keys preserved for associative subject
        assert_eq!(
            dumped(&call(fn_substr_replace, &[map(vec![("x", s("aaa")), ("y", s("bbb"))]), list(vec![s("1"), s("2")]), i(1), i(1)])),
            "array(2) {\n  [\"x\"]=>\n  string(3) \"a1a\"\n  [\"y\"]=>\n  string(3) \"b2b\"\n}\n"
        );
    }

    #[test]
    fn substr_replace_errors() {
        assert!(fn_substr_replace(&mut NullHost, &[s("abc"), s("X"), list(vec![i(1)]), list(vec![i(1)])]).is_err());
        assert!(fn_substr_replace(&mut NullHost, &[s("abc"), s("X"), i(1), list(vec![i(1)])]).is_err());
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
        assert_eq!(call_int(fn_levenshtein, &[s("kitten"), s("sitting"), i(1), i(2), i(1)]), 5);
        assert_eq!(call_int(fn_levenshtein, &[s("abc"), s("xyz"), i(10), i(1), i(10)]), 3);
        assert_eq!(call_int(fn_levenshtein, &[s("ab"), s("abcd"), i(2), i(5), i(3)]), 4);
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
        assert_eq!(call_int(fn_similar_text, &[s("PHP IS GREAT"), s("WITH MYSQL")]), 3);
        assert_eq!(call_int(fn_similar_text, &[s("Hello World"), s("Hello PHP World")]), 11);
        assert_eq!(call_int(fn_similar_text, &[s("test"), s("test")]), 4);
        assert_eq!(call_int(fn_similar_text, &[s("aaa"), s("aa")]), 2);
        assert_eq!(call_int(fn_similar_text, &[s("The quick brown fox"), s("The quick fox")]), 13);
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
        assert_eq!(dumped(&call(fn_str_word_count, &[s(""), i(1)])), "array(0) {\n}\n");
        assert_eq!(
            dumped(&call(fn_str_word_count, &[s("Hello World"), i(2)])),
            "array(2) {\n  [0]=>\n  string(5) \"Hello\"\n  [6]=>\n  string(5) \"World\"\n}\n"
        );
        // additional characters (with range syntax)
        assert_eq!(
            dumped(&call(fn_str_word_count, &[s("hi123 there"), i(1), s("123")])),
            "array(2) {\n  [0]=>\n  string(5) \"hi123\"\n  [1]=>\n  string(5) \"there\"\n}\n"
        );
        assert_eq!(call_int(fn_str_word_count, &[s("a1b c"), i(0), s("0..9")]), 2);
        assert!(fn_str_word_count(&mut NullHost, &[s("hi"), i(5)]).is_err());
    }

    // --- chunk_split -----------------------------------------------------

    #[test]
    fn chunk_split_cases() {
        assert_eq!(calls(fn_chunk_split, &[s("abcdefgh"), i(3), s("-")]), "abc-def-gh-");
        assert_eq!(calls(fn_chunk_split, &[s("abcdefgh")]), "abcdefgh\r\n");
        assert_eq!(calls(fn_chunk_split, &[s("abcdefgh"), i(3)]), "abc\r\ndef\r\ngh\r\n");
        assert_eq!(calls(fn_chunk_split, &[s("abc"), i(5), s("|")]), "abc|");
        assert_eq!(calls(fn_chunk_split, &[s(""), i(3), s("-")]), "-");
        assert_eq!(calls(fn_chunk_split, &[s("abcdef"), i(2), s("\r\n")]), "ab\r\ncd\r\nef\r\n");
        assert_eq!(calls(fn_chunk_split, &[s("abcdefgh"), i(4), s("")]), "abcdefgh");
        assert_eq!(calls(fn_chunk_split, &[s("abcdefgh"), i(4), s("-")]), "abcd-efgh-");
        assert_eq!(calls(fn_chunk_split, &[s("abcd"), i(4), s("-")]), "abcd-");
        assert!(fn_chunk_split(&mut NullHost, &[s("abc"), i(0)]).is_err());
    }

    // --- array_diff_key / array_intersect_key ----------------------------

    #[test]
    fn array_diff_key_cases() {
        assert_eq!(
            dumped(&call(fn_array_diff_key, &[map(vec![("a", i(1)), ("b", i(2)), ("c", i(3))]), map(vec![("b", i(9)), ("d", i(9))])])),
            "array(2) {\n  [\"a\"]=>\n  int(1)\n  [\"c\"]=>\n  int(3)\n}\n"
        );
        assert_eq!(
            dumped(&call(fn_array_diff_key, &[imap(vec![(10, s("x")), (20, s("y")), (30, s("z"))]), imap(vec![(20, s("q"))])])),
            "array(2) {\n  [10]=>\n  string(1) \"x\"\n  [30]=>\n  string(1) \"z\"\n}\n"
        );
        assert_eq!(
            dumped(&call(fn_array_diff_key, &[map(vec![("a", i(1)), ("b", i(2)), ("c", i(3))]), map(vec![("a", i(9))]), map(vec![("c", i(9))])])),
            "array(1) {\n  [\"b\"]=>\n  int(2)\n}\n"
        );
    }

    #[test]
    fn array_intersect_key_cases() {
        assert_eq!(
            dumped(&call(fn_array_intersect_key, &[map(vec![("a", i(1)), ("b", i(2)), ("c", i(3))]), map(vec![("b", i(9)), ("c", i(8)), ("d", i(7))])])),
            "array(2) {\n  [\"b\"]=>\n  int(2)\n  [\"c\"]=>\n  int(3)\n}\n"
        );
        assert_eq!(
            dumped(&call(fn_array_intersect_key, &[imap(vec![(0, s("a")), (1, s("b")), (2, s("c"))]), imap(vec![(1, s("x")), (2, s("y"))])])),
            "array(2) {\n  [1]=>\n  string(1) \"b\"\n  [2]=>\n  string(1) \"c\"\n}\n"
        );
        assert_eq!(
            dumped(&call(fn_array_intersect_key, &[map(vec![("a", i(1)), ("b", i(2)), ("c", i(3))]), map(vec![("a", i(0)), ("b", i(0))]), map(vec![("b", i(0)), ("c", i(0))])])),
            "array(1) {\n  [\"b\"]=>\n  int(2)\n}\n"
        );
    }

    // --- array_replace ---------------------------------------------------

    #[test]
    fn array_replace_cases() {
        assert_eq!(
            dumped(&call(fn_array_replace, &[map(vec![("a", i(1)), ("b", i(2))]), map(vec![("b", i(9)), ("c", i(3))])])),
            "array(3) {\n  [\"a\"]=>\n  int(1)\n  [\"b\"]=>\n  int(9)\n  [\"c\"]=>\n  int(3)\n}\n"
        );
        assert_eq!(
            dumped(&call(fn_array_replace, &[list(vec![i(1), i(2), i(3)]), imap(vec![(0, s("x")), (2, s("z"))])])),
            "array(3) {\n  [0]=>\n  string(1) \"x\"\n  [1]=>\n  int(2)\n  [2]=>\n  string(1) \"z\"\n}\n"
        );
        assert_eq!(
            dumped(&call(fn_array_replace, &[map(vec![("a", i(1))]), map(vec![("a", i(2))]), map(vec![("a", i(3)), ("b", i(4))])])),
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
}
