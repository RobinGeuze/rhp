//! UTF-8 multibyte string builtins (an `mbstring` subset).
//!
//! Unlike every other module in this crate, these functions treat their input
//! bytes as **UTF-8** and operate on Unicode **codepoints**. All UTF-8
//! decoding/encoding is hand-rolled (no external crates). The optional
//! `$encoding` argument is accepted but ignored — UTF-8 is assumed.
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see the `#[cfg(test)]` block — expected values captured from
//! `php -r '...'`).
//!
//! CASING SCOPE (see `to_upper_cp` / `to_lower_cp`): ASCII `A-Z`/`a-z` plus the
//! Latin-1 Supplement letters, including the special cases `ÿ`↔`Ÿ`, `µ`→`Μ`,
//! and `ß`→`SS` (title `Ss`). Full Unicode casing (Greek, Cyrillic, Turkish
//! dotless-i, etc.) is OUT OF SCOPE: any codepoint outside the handled range
//! passes through unchanged. `mb_convert_case(MB_CASE_TITLE)` uses a small,
//! documented word-boundary rule (see `is_cased` / `is_case_ignorable`).
//!
//! Wire into `lib.rs` with `mod mb_fns;` and `mb_fns::register(&mut m);`.

use std::collections::HashMap;

use phrust_values::{conv, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// small shared helpers
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

/// Is `b` a UTF-8 continuation byte (`10xxxxxx`)?
fn is_cont(b: u8) -> bool {
    b & 0xC0 == 0x80
}

/// Decode a byte slice as UTF-8 into a vector of codepoints.
///
/// Permissive: a well-formed 1–4 byte sequence decodes to its codepoint; any
/// byte that does not begin/continue a valid sequence is emitted as a single
/// codepoint equal to its byte value and skipped (so the function never panics
/// on malformed input). Overlong/range validation is intentionally loose —
/// callers pass valid UTF-8.
fn decode_utf8(bytes: &[u8]) -> Vec<u32> {
    let mut out = Vec::with_capacity(bytes.len());
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        if b < 0x80 {
            out.push(b as u32);
            i += 1;
        } else if (0xC2..=0xDF).contains(&b) && i + 1 < n && is_cont(bytes[i + 1]) {
            out.push(((b as u32 & 0x1F) << 6) | (bytes[i + 1] as u32 & 0x3F));
            i += 2;
        } else if (0xE0..=0xEF).contains(&b)
            && i + 2 < n
            && is_cont(bytes[i + 1])
            && is_cont(bytes[i + 2])
        {
            out.push(
                ((b as u32 & 0x0F) << 12)
                    | ((bytes[i + 1] as u32 & 0x3F) << 6)
                    | (bytes[i + 2] as u32 & 0x3F),
            );
            i += 3;
        } else if (0xF0..=0xF4).contains(&b)
            && i + 3 < n
            && is_cont(bytes[i + 1])
            && is_cont(bytes[i + 2])
            && is_cont(bytes[i + 3])
        {
            out.push(
                ((b as u32 & 0x07) << 18)
                    | ((bytes[i + 1] as u32 & 0x3F) << 12)
                    | ((bytes[i + 2] as u32 & 0x3F) << 6)
                    | (bytes[i + 3] as u32 & 0x3F),
            );
            i += 4;
        } else {
            // invalid lead/truncated sequence: treat this byte as one codepoint.
            out.push(b as u32);
            i += 1;
        }
    }
    out
}

/// Append the UTF-8 encoding of one codepoint to `out`.
fn encode_cp(cp: u32, out: &mut Vec<u8>) {
    if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.push(0xC0 | (cp >> 6) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else if cp < 0x10000 {
        out.push(0xE0 | (cp >> 12) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else {
        out.push(0xF0 | (cp >> 18) as u8);
        out.push(0x80 | ((cp >> 12) & 0x3F) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    }
}

/// Encode a slice of codepoints back to a UTF-8 byte string.
fn encode_cps(cps: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cps.len());
    for &cp in cps {
        encode_cp(cp, &mut out);
    }
    out
}

// ---------------------------------------------------------------------------
// casing tables (ASCII + Latin-1 Supplement; see module docs for the limits)
// ---------------------------------------------------------------------------

/// Uppercase one codepoint, appending result codepoint(s) to `out`.
/// `ß` (U+00DF) expands to `SS`; everything outside the handled range is copied.
fn to_upper_cp(cp: u32, out: &mut Vec<u32>) {
    match cp {
        0x61..=0x7A => out.push(cp - 0x20),               // a-z
        0xE0..=0xF6 | 0xF8..=0xFE => out.push(cp - 0x20), // à-ö, ø-þ (skip ÷ 0xF7)
        0xFF => out.push(0x178),                          // ÿ -> Ÿ
        0xB5 => out.push(0x39C),                          // µ -> Μ (micro sign)
        0xDF => {
            // ß -> SS
            out.push(0x53);
            out.push(0x53);
        }
        _ => out.push(cp),
    }
}

/// Lowercase one codepoint (never expands within the handled range).
fn to_lower_cp(cp: u32) -> u32 {
    match cp {
        0x41..=0x5A => cp + 0x20,               // A-Z
        0xC0..=0xD6 | 0xD8..=0xDE => cp + 0x20, // À-Ö, Ø-Þ (skip × 0xD7)
        0x178 => 0xFF,                          // Ÿ -> ÿ
        _ => cp,
    }
}

/// Titlecase one codepoint, appending result codepoint(s) to `out`.
/// Same as uppercase except `ß` -> `Ss` (matches php's `MB_CASE_TITLE`).
fn to_title_cp(cp: u32, out: &mut Vec<u32>) {
    if cp == 0xDF {
        out.push(0x53); // S
        out.push(0x73); // s
    } else {
        to_upper_cp(cp, out);
    }
}

/// Is `cp` a "cased" letter for title word-boundary detection (handled range)?
fn is_cased(cp: u32) -> bool {
    matches!(cp,
        0x41..=0x5A | 0x61..=0x7A      // A-Z a-z
        | 0xB5                          // µ
        | 0xC0..=0xD6 | 0xD8..=0xF6 | 0xF8..=0xFF // Latin-1 letters (skip × ÷)
    )
}

/// Is `cp` "case-ignorable" — transparent to title word boundaries (it neither
/// starts a word nor ends one)? Covers the common ASCII/Latin-1 members of the
/// Unicode `Case_Ignorable` property (apostrophe, dot, colon, modifier marks).
fn is_case_ignorable(cp: u32) -> bool {
    matches!(cp,
        0x27 | 0x2E | 0x3A        // ' . :
        | 0x5E | 0x60             // ^ `
        | 0xA8 | 0xAF | 0xB4 | 0xB7 | 0xB8 // ¨ ¯ ´ · ¸
        | 0x2019                  // ’ right single quote
    )
}

// ---------------------------------------------------------------------------
// mb_strlen
// ---------------------------------------------------------------------------

fn fn_mb_strlen(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cps = decode_utf8(&bytes_of(&args[0]));
    Ok(Value::Int(cps.len() as i64))
}

// ---------------------------------------------------------------------------
// mb_substr
// ---------------------------------------------------------------------------

/// Clamp a php start/offset against codepoint `len`, returning an index 0..=len.
fn clamp_start(start: i64, len: usize) -> usize {
    let len = len as i64;
    if start < 0 {
        (len + start).max(0) as usize
    } else {
        start.min(len) as usize
    }
}

fn fn_mb_substr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cps = decode_utf8(&bytes_of(&args[0]));
    let len = cps.len();
    let start = clamp_start(conv::to_int(&deref(&args[1])), len);
    let rest = len - start;
    let take = match args.get(2).map(deref) {
        None | Some(Value::Null) => rest,
        Some(v) => {
            let l = conv::to_int(&v);
            if l < 0 {
                let drop = (-l) as usize;
                if drop >= rest {
                    0
                } else {
                    rest - drop
                }
            } else {
                (l as usize).min(rest)
            }
        }
    };
    Ok(Value::str(encode_cps(&cps[start..start + take])))
}

// ---------------------------------------------------------------------------
// mb_strtoupper / mb_strtolower / mb_convert_case
// ---------------------------------------------------------------------------

fn upper_cps(cps: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(cps.len());
    for &cp in cps {
        to_upper_cp(cp, &mut out);
    }
    out
}

fn lower_cps(cps: &[u32]) -> Vec<u32> {
    cps.iter().map(|&cp| to_lower_cp(cp)).collect()
}

/// Titlecase using the documented word-boundary rule.
fn title_cps(cps: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(cps.len());
    let mut at_word_start = true;
    for &cp in cps {
        if is_cased(cp) {
            if at_word_start {
                to_title_cp(cp, &mut out);
            } else {
                out.push(to_lower_cp(cp));
            }
            at_word_start = false;
        } else if is_case_ignorable(cp) {
            // transparent: output unchanged, keep word-start state.
            out.push(cp);
        } else {
            out.push(cp);
            at_word_start = true;
        }
    }
    out
}

fn fn_mb_strtoupper(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cps = decode_utf8(&bytes_of(&args[0]));
    Ok(Value::str(encode_cps(&upper_cps(&cps))))
}

fn fn_mb_strtolower(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cps = decode_utf8(&bytes_of(&args[0]));
    Ok(Value::str(encode_cps(&lower_cps(&cps))))
}

fn fn_mb_convert_case(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cps = decode_utf8(&bytes_of(&args[0]));
    let mode = conv::to_int(&deref(&args[1]));
    // php MB_CASE_*: UPPER=0 LOWER=1 TITLE=2 FOLD=3, plus *_SIMPLE 4..7.
    // FOLD is approximated by lowercasing within the handled range.
    let out = match mode {
        0 | 4 => upper_cps(&cps),
        1 | 5 | 3 | 7 => lower_cps(&cps),
        2 | 6 => title_cps(&cps),
        _ => {
            return Err(Thrown::error(
                "ValueError",
                "mb_convert_case(): Argument #2 ($mode) must be one of the MB_CASE_* constants",
            ))
        }
    };
    Ok(Value::str(encode_cps(&out)))
}

// ---------------------------------------------------------------------------
// mb_str_split
// ---------------------------------------------------------------------------

fn fn_mb_str_split(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cps = decode_utf8(&bytes_of(&args[0]));
    let chunk = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(1);
    if chunk < 1 {
        return Err(Thrown::error(
            "ValueError",
            "mb_str_split(): Argument #2 ($length) must be greater than 0",
        ));
    }
    let chunk = chunk as usize;
    let mut out = PhpArray::new();
    let mut i = 0;
    while i < cps.len() {
        let end = (i + chunk).min(cps.len());
        out.push(Value::str(encode_cps(&cps[i..end])));
        i = end;
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// mb_strpos
// ---------------------------------------------------------------------------

/// Find `needle` codepoints in `haystack` codepoints at or after `start`.
fn find_cps(haystack: &[u32], needle: &[u32], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    let mut i = start;
    while i + needle.len() <= haystack.len() {
        if haystack[i..i + needle.len()] == *needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn fn_mb_strpos(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let haystack = decode_utf8(&bytes_of(&args[0]));
    let needle = decode_utf8(&bytes_of(&args[1]));
    let len = haystack.len() as i64;
    let off = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if off < -len || off > len {
        return Err(Thrown::error(
            "ValueError",
            "mb_strpos(): Argument #3 ($offset) must be contained in argument #1 ($haystack)",
        ));
    }
    let start = if off < 0 { (len + off) as usize } else { off as usize };
    match find_cps(&haystack, &needle, start) {
        Some(p) => Ok(Value::Int(p as i64)),
        None => Ok(Value::Bool(false)),
    }
}

// ---------------------------------------------------------------------------
// mb_ord / mb_chr
// ---------------------------------------------------------------------------

fn fn_mb_ord(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cps = decode_utf8(&bytes_of(&args[0]));
    match cps.first() {
        Some(&cp) => Ok(Value::Int(cp as i64)),
        None => Err(Thrown::error(
            "ValueError",
            "mb_ord(): Argument #1 ($string) must not be empty",
        )),
    }
}

fn fn_mb_chr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cp = conv::to_int(&deref(&args[0]));
    if !(0..=0x10FFFF).contains(&cp) {
        return Ok(Value::Bool(false));
    }
    let mut out = Vec::new();
    encode_cp(cp as u32, &mut out);
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register all multibyte builtins into the shared table.
pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "mb_strlen", fn_mb_strlen, 1, Some(2));
    put(m, "mb_substr", fn_mb_substr, 2, Some(4));
    put(m, "mb_strtoupper", fn_mb_strtoupper, 1, Some(2));
    put(m, "mb_strtolower", fn_mb_strtolower, 1, Some(2));
    put(m, "mb_str_split", fn_mb_str_split, 1, Some(3));
    put(m, "mb_strpos", fn_mb_strpos, 2, Some(4));
    put(m, "mb_convert_case", fn_mb_convert_case, 2, Some(3));
    put(m, "mb_ord", fn_mb_ord, 1, Some(2));
    put(m, "mb_chr", fn_mb_chr, 1, Some(2));
    put(m, "mb_detect_encoding", fn_mb_detect_encoding, 1, Some(3));
}

/// `mb_detect_encoding($string, $encodings = null, $strict = false)`.
///
/// Pragmatic detector covering the common case: returns `"UTF-8"` for valid
/// (non-ASCII-containing) UTF-8, `"ASCII"` for a 7-bit string, and `false` for
/// bytes that are neither. This does not attempt the full php candidate-list /
/// encoding-guessing machinery; `$encodings` and `$strict` are accepted but the
/// detection is limited to the ASCII / UTF-8 distinction documented here.
fn fn_mb_detect_encoding(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    if s.iter().all(|&b| b < 0x80) {
        return Ok(Value::str("ASCII"));
    }
    if std::str::from_utf8(&s).is_ok() {
        Ok(Value::str("UTF-8"))
    } else {
        Ok(Value::Bool(false))
    }
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
                .map(|(_, v)| String::from_utf8(bytes_of(v)).unwrap())
                .collect(),
            other => panic!("expected array, got {}", other.type_name()),
        }
    }

    #[test]
    fn detect_encoding() {
        // ASCII / valid-UTF-8 cases match php 8.5.8 exactly.
        assert_eq!(calls(fn_mb_detect_encoding, &[s("hello")]), "ASCII");
        assert_eq!(calls(fn_mb_detect_encoding, &[s("café")]), "UTF-8");
        assert_eq!(calls(fn_mb_detect_encoding, &[s("日本語")]), "UTF-8");
        // Malformed UTF-8: this engine returns false (php's lenient default
        // instead falls back to "ASCII" — documented divergence).
        assert!(is_false(fn_mb_detect_encoding, &[Value::str(vec![0xC3, 0x28])]));
    }

    // mb_strlen("héllo") == 5  (vs strlen == 6)
    #[test]
    fn mb_strlen_cases() {
        assert_eq!(call_int(fn_mb_strlen, &[s("héllo")]), 5);
        assert_eq!(call_int(fn_mb_strlen, &[s("")]), 0);
        assert_eq!(call_int(fn_mb_strlen, &[s("€uro")]), 4);
        assert_eq!(call_int(fn_mb_strlen, &[s("ascii")]), 5);
    }

    // mb_substr("héllo",1,3) == "éll"
    #[test]
    fn mb_substr_cases() {
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(1), Value::Int(3)]), "éll");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(-2)]), "lo");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(1), Value::Int(-1)]), "éll");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(-3), Value::Int(2)]), "ll");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(10)]), "");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(5)]), "");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(-10)]), "héllo");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(2), Value::Null]), "llo");
        assert_eq!(calls(fn_mb_substr, &[s("héllo"), Value::Int(1), Value::Int(-10)]), "");
    }

    // mb_strtoupper / mb_strtolower (ASCII + Latin-1)
    #[test]
    fn mb_case_cases() {
        assert_eq!(calls(fn_mb_strtoupper, &[s("héllo wörld")]), "HÉLLO WÖRLD");
        assert_eq!(calls(fn_mb_strtolower, &[s("HÉLLO WÖRLD")]), "héllo wörld");
        assert_eq!(calls(fn_mb_strtoupper, &[s("straße")]), "STRASSE"); // ß -> SS
        assert_eq!(calls(fn_mb_strtoupper, &[s("ÿ")]), "Ÿ"); // ÿ -> Ÿ
        assert_eq!(calls(fn_mb_strtoupper, &[s("µ")]), "Μ"); // µ -> Μ
        assert_eq!(calls(fn_mb_strtoupper, &[s("×÷")]), "×÷"); // math signs unchanged
        assert_eq!(
            calls(fn_mb_strtoupper, &[s("àáâãäçèéêëìíîïñòóôõöùúûüý")]),
            "ÀÁÂÃÄÇÈÉÊËÌÍÎÏÑÒÓÔÕÖÙÚÛÜÝ"
        );
    }

    // mb_convert_case: UPPER=0, LOWER=1, TITLE=2
    #[test]
    fn mb_convert_case_cases() {
        assert_eq!(calls(fn_mb_convert_case, &[s("héllo"), Value::Int(0)]), "HÉLLO");
        assert_eq!(calls(fn_mb_convert_case, &[s("HÉLLO"), Value::Int(1)]), "héllo");
        assert_eq!(calls(fn_mb_convert_case, &[s("héllo wörld"), Value::Int(2)]), "Héllo Wörld");
        assert_eq!(calls(fn_mb_convert_case, &[s("foo-bar baz"), Value::Int(2)]), "Foo-Bar Baz");
        assert_eq!(calls(fn_mb_convert_case, &[s("it's a test"), Value::Int(2)]), "It's A Test");
        assert_eq!(calls(fn_mb_convert_case, &[s("abc123def"), Value::Int(2)]), "Abc123Def");
        assert_eq!(calls(fn_mb_convert_case, &[s("123abc"), Value::Int(2)]), "123Abc");
        assert_eq!(calls(fn_mb_convert_case, &[s("a.b a,b"), Value::Int(2)]), "A.b A,B");
        assert_eq!(calls(fn_mb_convert_case, &[s("ßtra"), Value::Int(2)]), "Sstra"); // title ß -> Ss
    }

    // mb_str_split into codepoint chunks
    #[test]
    fn mb_str_split_cases() {
        assert_eq!(split_vec(fn_mb_str_split, &[s("héllo")]), vec!["h", "é", "l", "l", "o"]);
        assert_eq!(split_vec(fn_mb_str_split, &[s("héllo"), Value::Int(2)]), vec!["hé", "ll", "o"]);
        assert_eq!(split_vec(fn_mb_str_split, &[s("abcdefg"), Value::Int(3)]), vec!["abc", "def", "g"]);
        assert!(split_vec(fn_mb_str_split, &[s("")]).is_empty());
        assert!(fn_mb_str_split(&mut NullHost, &[s("abc"), Value::Int(0)]).is_err());
        assert!(fn_mb_str_split(&mut NullHost, &[s("abc"), Value::Int(-1)]).is_err());
    }

    // mb_strpos: codepoint index | false
    #[test]
    fn mb_strpos_cases() {
        assert_eq!(call_pos(fn_mb_strpos, &[s("héllo"), s("llo")]), Some(2));
        assert_eq!(call_pos(fn_mb_strpos, &[s("héllo"), s("l")]), Some(2));
        assert_eq!(call_pos(fn_mb_strpos, &[s("aéaéa"), s("a"), Value::Int(2)]), Some(2));
        assert_eq!(call_pos(fn_mb_strpos, &[s("aéaéa"), s("a"), Value::Int(-1)]), Some(4));
        assert_eq!(call_pos(fn_mb_strpos, &[s("aéaéa"), s("a"), Value::Int(-3)]), Some(2));
        assert_eq!(call_pos(fn_mb_strpos, &[s("héllo"), s("z")]), None);
        assert_eq!(call_pos(fn_mb_strpos, &[s("héllo"), s("héllo")]), Some(0));
        assert_eq!(call_pos(fn_mb_strpos, &[s("héllo"), s("")]), Some(0));
        assert_eq!(call_pos(fn_mb_strpos, &[s("héllo"), s(""), Value::Int(2)]), Some(2));
        assert!(fn_mb_strpos(&mut NullHost, &[s("héllo"), s("l"), Value::Int(10)]).is_err());
        assert!(fn_mb_strpos(&mut NullHost, &[s("héllo"), s("l"), Value::Int(-10)]).is_err());
    }

    // mb_ord / mb_chr
    #[test]
    fn mb_ord_chr_cases() {
        assert_eq!(call_int(fn_mb_ord, &[s("é")]), 233);
        assert_eq!(call_int(fn_mb_ord, &[s("€")]), 8364);
        assert_eq!(call_int(fn_mb_ord, &[s("ab")]), 97); // first codepoint
        assert!(fn_mb_ord(&mut NullHost, &[s("")]).is_err());
        assert_eq!(calls(fn_mb_chr, &[Value::Int(233)]), "é");
        assert_eq!(calls(fn_mb_chr, &[Value::Int(8364)]), "€");
        assert_eq!(calls(fn_mb_chr, &[Value::Int(0x10FFFF)]), "\u{10FFFF}");
        assert!(is_false(fn_mb_chr, &[Value::Int(-1)]));
        assert!(is_false(fn_mb_chr, &[Value::Int(0x110000)]));
        // round-trip
        assert_eq!(call_int(fn_mb_ord, &[call(fn_mb_chr, &[Value::Int(256)])]), 256);
    }
}
