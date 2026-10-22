//! The PCRE `preg_*` builtin family, backed by the `fancy-regex` crate.
//!
//! `fancy-regex` is chosen over the plain `regex` crate because PHP patterns in
//! the wild rely on lookaround and backreferences, which `regex` cannot express;
//! `fancy-regex` supports both while keeping a linear fast path. Every function
//! here is verified byte-for-byte against the local php 8.5.8 binary (see the
//! `#[cfg(test)]` block — expected values are hardcoded from `php -r '...'`).
//!
//! Wire this module into `lib.rs` with:
//! ```ignore
//! mod preg_fns;               // near the other `mod` lines
//! // ...and inside registry(), before returning `m`:
//! preg_fns::register(&mut m);
//! ```
//! and add the `PREG_*` constants (see [`preg_constants`]) to `constants()`.
//!
//! ## Byte-vs-UTF-8 semantics
//! PHP strings are *byte* strings, but `fancy-regex` matches over `&str`. We
//! match against `String::from_utf8_lossy(subject)`. For valid UTF-8 subjects
//! (the overwhelmingly common case) the byte offsets `fancy-regex` reports are
//! identical to the byte offsets php reports for `PREG_OFFSET_CAPTURE`, so the
//! results are byte-for-byte identical. For subjects containing invalid UTF-8,
//! `from_utf8_lossy` substitutes U+FFFD (3 bytes) for each bad byte, which can
//! shift offsets and alter the bytes that match; this is the one documented
//! lossy edge.
//!
//! ## PCRE features that do NOT translate to fancy-regex
//! - **Possessive quantifiers** (`a++`, `a*+`) and **atomic groups** (`(?>...)`):
//!   fancy-regex supports atomic groups but not possessive quantifiers; a
//!   pattern using `a++` fails to compile and we return `false`/`null` as php
//!   would on a bad pattern.
//! - **Recursion / subroutine calls** (`(?R)`, `(?1)`, `\g<1>`): unsupported;
//!   such a pattern fails to compile → `false`/`null`.
//! - **`\G`** (anchor to previous match end): unsupported by fancy-regex.
//! - **Named-group syntax**: php accepts `(?P<name>…)`, `(?<name>…)` and
//!   `(?'name'…)`. fancy-regex accepts the first two; the quoted form
//!   `(?'name'…)` is NOT supported and such a pattern fails to compile.
//! - **Inline modifier `U`/`X`/`A`/`D`** trailing flags: only `imsxu` are
//!   translated; any other trailing modifier letter is rejected (→ error) to
//!   avoid silently mis-compiling.
//! - **ASCII vs Unicode classes**: php's `\d \w \s` are ASCII-only unless the
//!   `u` flag is set, whereas fancy-regex treats them as Unicode-aware always.
//!   For ASCII subjects this is indistinguishable.

use std::collections::HashMap;

use fancy_regex::{Captures, Regex};
use phrust_values::{ArrayKey, PhpArray, PhpStr, Value};
use phrust_semantics::deref;

use crate::{Builtin, Host, Thrown};

// PCRE flag constants (mirrored into lib.rs `constants()`; see `preg_constants`).
const PREG_PATTERN_ORDER: i64 = 1;
const PREG_SET_ORDER: i64 = 2;
const PREG_OFFSET_CAPTURE: i64 = 256;
const PREG_SPLIT_NO_EMPTY: i64 = 1;
const PREG_SPLIT_DELIM_CAPTURE: i64 = 2;
const PREG_SPLIT_OFFSET_CAPTURE: i64 = 4;
const PREG_UNMATCHED_AS_NULL: i64 = 512;
const PREG_GREP_INVERT: i64 = 1;

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

/// `(string)` bytes of a value (after deref).
fn bytes_of(v: &Value) -> Vec<u8> {
    phrust_values::conv::to_string_bytes(&deref(v))
}

/// `(int)` of a value (after deref). Default when the arg is absent.
fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    args.get(i).map(|v| phrust_values::conv::to_int(&deref(v))).unwrap_or(default)
}

/// Byte length of the UTF-8 char starting at `pos` in `s` (for advancing past a
/// zero-width match). Falls back to 1 at the end of the string.
fn char_len_at(s: &str, pos: usize) -> usize {
    s[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(1)
}

// ---------------------------------------------------------------------------
// pattern parsing: `/body/flags`, `#body#flags`, `(body)flags`, …
// ---------------------------------------------------------------------------

/// Parse a delimited PHP pattern and compile it. Returns `None` on any invalid
/// pattern (bad delimiter, unterminated, unknown modifier, uncompilable body),
/// mirroring php's "warn and return false/null" behaviour (we skip the warning
/// and surface the failure to the caller).
fn compile(pattern: &[u8]) -> Option<Regex> {
    // Leading whitespace is not permitted by php; the first byte is the delimiter.
    if pattern.is_empty() {
        return None;
    }
    let delim = pattern[0];
    let close = match delim {
        b'(' => b')',
        b'{' => b'}',
        b'[' => b']',
        b'<' => b'>',
        // Delimiter may not be a backslash, alphanumeric, or whitespace.
        d if d.is_ascii_alphanumeric() || d == b'\\' || d.is_ascii_whitespace() => return None,
        d => d,
    };
    let bracketed = delim != close;

    // Locate the closing delimiter. For bracket delimiters we balance nesting
    // (respecting backslash escapes); for symmetric delimiters the closing one
    // is the last unescaped occurrence.
    let body_start = 1;
    let close_pos = if bracketed {
        let mut depth = 1usize;
        let mut i = body_start;
        let mut found = None;
        while i < pattern.len() {
            match pattern[i] {
                b'\\' => i += 1, // skip the escaped byte
                b if b == delim => depth += 1,
                b if b == close => {
                    depth -= 1;
                    if depth == 0 {
                        found = Some(i);
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        found?
    } else {
        // Last unescaped occurrence of the delimiter after position 0.
        let mut i = body_start;
        let mut last = None;
        while i < pattern.len() {
            if pattern[i] == b'\\' {
                i += 2;
                continue;
            }
            if pattern[i] == close {
                last = Some(i);
            }
            i += 1;
        }
        last?
    };

    let body = &pattern[body_start..close_pos];
    let flag_bytes = &pattern[close_pos + 1..];

    // Translate trailing modifier letters into an inline flag group.
    let mut inline = String::new();
    for &f in flag_bytes {
        match f {
            b'i' | b'm' | b's' | b'x' => inline.push(f as char),
            b'u' => {} // Unicode: fancy-regex matches over `&str` already.
            _ => return None, // unknown / unsupported modifier (U, A, D, X, …)
        }
    }

    // The body is bytes; for the regex engine we need a `str`. A pattern with
    // invalid UTF-8 bytes is treated leniently via lossy decoding.
    let body_str = String::from_utf8_lossy(body);
    let full = if inline.is_empty() {
        body_str.into_owned()
    } else {
        format!("(?{}){}", inline, body_str)
    };
    Regex::new(&full).ok()
}

// ---------------------------------------------------------------------------
// building the `$matches` array for one match
// ---------------------------------------------------------------------------

/// The index→name map for a compiled regex (index 0 and unnamed groups = None).
fn capture_names(re: &Regex) -> Vec<Option<String>> {
    re.capture_names().map(|o| o.map(|s| s.to_string())).collect()
}

/// The highest group index that participated in this match (group 0 always
/// does). php trims *trailing* unmatched groups from the result.
fn last_participating(caps: &Captures) -> usize {
    let n = caps.len();
    let mut last = 0;
    for i in 1..n {
        if caps.get(i).is_some() {
            last = i;
        }
    }
    last
}

/// The value for one capture group in the result array. Honours
/// `PREG_OFFSET_CAPTURE` (wrap as `[text, offset]`) and `PREG_UNMATCHED_AS_NULL`.
fn group_value(caps: &Captures, i: usize, offset_capture: bool, unmatched_null: bool) -> Value {
    match caps.get(i) {
        Some(m) => {
            let s = Value::str(m.as_str().as_bytes().to_vec());
            if offset_capture {
                pair(s, m.start() as i64)
            } else {
                s
            }
        }
        None => {
            let empty = if unmatched_null { Value::Null } else { Value::str("") };
            if offset_capture {
                pair(empty, -1)
            } else {
                empty
            }
        }
    }
}

/// A two-element `[value, offset]` array, as `PREG_OFFSET_CAPTURE` produces.
fn pair(value: Value, offset: i64) -> Value {
    let mut a = PhpArray::new();
    a.push(value);
    a.push(Value::Int(offset));
    Value::Array(a)
}

/// Build a single match's `$matches` array: group 0, then each group (named
/// groups appear *by name first, then by index*, exactly as php orders them).
fn build_match(
    caps: &Captures,
    names: &[Option<String>],
    offset_capture: bool,
    unmatched_null: bool,
) -> PhpArray {
    let last = last_participating(caps);
    let mut arr = PhpArray::new();
    for i in 0..=last {
        let val = group_value(caps, i, offset_capture, unmatched_null);
        if let Some(name) = names.get(i).and_then(|o| o.as_ref()) {
            arr.insert(ArrayKey::Str(PhpStr::from(name.as_str())), val.clone());
        }
        arr.insert(ArrayKey::Int(i as i64), val);
    }
    arr
}

/// Write an out-value through a by-reference `$matches`/`$count` argument.
/// Mirrors `sort_fns`: the compiler passes arg 2 as a `Value::Ref`; if it is
/// not a ref (the optional out-param was omitted), this is a no-op.
fn write_ref(arg: Option<&Value>, value: Value) {
    if let Some(Value::Ref(rc)) = arg {
        *rc.borrow_mut() = value;
    }
}

// ---------------------------------------------------------------------------
// preg_match
// ---------------------------------------------------------------------------

fn fn_preg_match(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(re) = compile(&bytes_of(&args[0])) else {
        return Ok(Value::Bool(false));
    };
    let subject = bytes_of(&args[1]);
    let text = String::from_utf8_lossy(&subject);
    let flags = int_arg(args, 3, 0);
    let offset = int_arg(args, 4, 0).max(0) as usize;
    let offset_capture = flags & PREG_OFFSET_CAPTURE != 0;
    let unmatched_null = flags & PREG_UNMATCHED_AS_NULL != 0;

    let start = offset.min(text.len());
    match re.captures_from_pos(&text, start) {
        Ok(Some(caps)) => {
            let names = capture_names(&re);
            let arr = build_match(&caps, &names, offset_capture, unmatched_null);
            write_ref(args.get(2), Value::Array(arr));
            Ok(Value::Int(1))
        }
        Ok(None) => {
            write_ref(args.get(2), Value::Array(PhpArray::new()));
            Ok(Value::Int(0))
        }
        Err(_) => Ok(Value::Bool(false)),
    }
}

// ---------------------------------------------------------------------------
// preg_match_all
// ---------------------------------------------------------------------------

fn fn_preg_match_all(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(re) = compile(&bytes_of(&args[0])) else {
        return Ok(Value::Bool(false));
    };
    let subject = bytes_of(&args[1]);
    let text = String::from_utf8_lossy(&subject);
    let flags_raw = int_arg(args, 3, 0);
    // The ordering flag lives in the low bits; default is PREG_PATTERN_ORDER.
    let set_order = flags_raw & PREG_SET_ORDER != 0;
    let offset_capture = flags_raw & PREG_OFFSET_CAPTURE != 0;
    let unmatched_null = flags_raw & PREG_UNMATCHED_AS_NULL != 0;
    let offset = int_arg(args, 4, 0).max(0) as usize;
    let names = capture_names(&re);
    let ngroups = re.captures_len();

    // Collect every match (handling zero-width matches by advancing one char).
    let mut all: Vec<Captures> = Vec::new();
    let mut pos = offset.min(text.len());
    loop {
        match re.captures_from_pos(&text, pos) {
            Ok(Some(caps)) => {
                let m = caps.get(0).unwrap();
                let (s, e) = (m.start(), m.end());
                all.push(caps);
                pos = if e == s { e + char_len_at(&text, e) } else { e };
                if pos > text.len() {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => return Ok(Value::Bool(false)),
        }
    }

    let count = all.len();
    let result = if set_order {
        // SET_ORDER: an array of per-match arrays (each like preg_match's).
        let mut outer = PhpArray::new();
        for caps in &all {
            outer.push(Value::Array(build_match(caps, &names, offset_capture, unmatched_null)));
        }
        outer
    } else {
        // PATTERN_ORDER: one array per group (named group by name then index).
        // All groups are included (no trailing trim). With zero matches, php
        // emits only `[0 => []]`.
        let mut outer = PhpArray::new();
        let groups = if count == 0 { 1 } else { ngroups };
        for i in 0..groups {
            let mut col = PhpArray::new();
            for caps in &all {
                col.push(group_value(caps, i, offset_capture, unmatched_null));
            }
            if let Some(name) = names.get(i).and_then(|o| o.as_ref()) {
                outer.insert(ArrayKey::Str(PhpStr::from(name.as_str())), Value::Array(col.clone()));
            }
            outer.insert(ArrayKey::Int(i as i64), Value::Array(col));
        }
        outer
    };

    write_ref(args.get(2), Value::Array(result));
    Ok(Value::Int(count as i64))
}

// ---------------------------------------------------------------------------
// replacement template expansion (`$1`, `\1`, `${1}`)
// ---------------------------------------------------------------------------

/// Expand one replacement template against a match, appending bytes to `out`.
///
/// php backreference rules (verified against php 8.5.8):
/// - `$1`, `\1` → group 1; `$12`/`\12` read *up to two* digits (so `$12` is
///   group 12, not group 1 then `'2'`); an out-of-range group yields "".
/// - `${12}` reads all digits inside the braces.
/// - `\$` → literal `$`; `\\` → literal `\`; `\` + other → kept verbatim
///   (`\n` stays `\n`). `$` not followed by a digit or `{` is a literal `$`.
fn expand(template: &[u8], caps: &Captures, out: &mut Vec<u8>) {
    let group = |n: usize| -> &str { caps.get(n).map(|m| m.as_str()).unwrap_or("") };
    let b = template;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'$' => {
                if i + 1 < b.len() && b[i + 1] == b'{' {
                    // ${ digits }
                    let mut j = i + 2;
                    let ds = j;
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j > ds && j < b.len() && b[j] == b'}' {
                        let n: usize = std::str::from_utf8(&b[ds..j]).unwrap().parse().unwrap_or(usize::MAX);
                        out.extend_from_slice(group(n).as_bytes());
                        i = j + 1;
                    } else {
                        out.push(b'$');
                        i += 1;
                    }
                } else if let Some(n) = read_num(b, i + 1) {
                    out.extend_from_slice(group(n.0).as_bytes());
                    i = n.1;
                } else {
                    out.push(b'$');
                    i += 1;
                }
            }
            b'\\' => {
                if i + 1 >= b.len() {
                    out.push(b'\\');
                    i += 1;
                } else if let Some(n) = read_num(b, i + 1) {
                    out.extend_from_slice(group(n.0).as_bytes());
                    i = n.1;
                } else if b[i + 1] == b'\\' {
                    out.push(b'\\');
                    i += 2;
                } else if b[i + 1] == b'$' {
                    out.push(b'$');
                    i += 2;
                } else {
                    out.push(b'\\');
                    out.push(b[i + 1]);
                    i += 2;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
}

/// Read up to two decimal digits starting at `start`; returns (group, next_idx).
fn read_num(b: &[u8], start: usize) -> Option<(usize, usize)> {
    if start >= b.len() || !b[start].is_ascii_digit() {
        return None;
    }
    let mut j = start + 1;
    if j < b.len() && b[j].is_ascii_digit() {
        j += 1; // at most two digits
    }
    let n: usize = std::str::from_utf8(&b[start..j]).unwrap().parse().ok()?;
    Some((n, j))
}

// ---------------------------------------------------------------------------
// the core replace driver (shared by preg_replace / preg_replace_callback)
// ---------------------------------------------------------------------------

/// Replace matches in `text`, producing bytes. `make_rep` produces the
/// replacement bytes for each match (template expansion or a callback). Returns
/// `(output, replacement_count)`. `limit < 0` means unlimited.
fn replace_driver<F>(
    re: &Regex,
    text: &str,
    limit: i64,
    mut make_rep: F,
) -> Result<(Vec<u8>, usize), ()>
where
    F: FnMut(&Captures, &mut Vec<u8>),
{
    let mut out: Vec<u8> = Vec::with_capacity(text.len());
    let mut last = 0usize; // end of the last copied/replaced region
    let mut pos = 0usize;
    let mut n = 0usize;
    loop {
        if limit >= 0 && n as i64 >= limit {
            break;
        }
        match re.captures_from_pos(text, pos) {
            Ok(Some(caps)) => {
                let m = caps.get(0).unwrap();
                let (s, e) = (m.start(), m.end());
                out.extend_from_slice(&text.as_bytes()[last..s]);
                make_rep(&caps, &mut out);
                n += 1;
                if e == s {
                    // zero-width match: emit the next char verbatim and advance.
                    if e < text.len() {
                        let cl = char_len_at(text, e);
                        out.extend_from_slice(&text.as_bytes()[e..e + cl]);
                        last = e + cl;
                        pos = e + cl;
                    } else {
                        last = e;
                        pos = e + 1; // force termination
                    }
                } else {
                    last = e;
                    pos = e;
                }
                if pos > text.len() {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => return Err(()),
        }
    }
    out.extend_from_slice(&text.as_bytes()[last.min(text.len())..]);
    Ok((out, n))
}

// ---------------------------------------------------------------------------
// preg_replace
// ---------------------------------------------------------------------------

/// Apply one (pattern, replacement-template) pair to a byte subject.
fn replace_one(pattern: &[u8], replacement: &[u8], subject: &[u8], limit: i64) -> Option<(Vec<u8>, usize)> {
    let re = compile(pattern)?;
    let text = String::from_utf8_lossy(subject);
    replace_driver(&re, &text, limit, |caps, out| expand(replacement, caps, out)).ok()
}

/// Run every (pattern, replacement) pair over a single subject in sequence.
/// `pats`/`reps` are parallel lists; a missing replacement means "".
fn replace_subject(pats: &[Vec<u8>], reps: &[Vec<u8>], subject: &[u8], limit: i64, count: &mut usize) -> Option<Vec<u8>> {
    let mut cur = subject.to_vec();
    for (i, pat) in pats.iter().enumerate() {
        let rep = reps.get(i).map(|v| v.as_slice()).unwrap_or(b"");
        let (next, n) = replace_one(pat, rep, &cur, limit)?;
        *count += n;
        cur = next;
    }
    Some(cur)
}

/// Collect a pattern/replacement argument into a list of byte vectors. A scalar
/// becomes a one-element list; an array keeps its values in order.
fn as_list(v: &Value) -> (Vec<Vec<u8>>, bool) {
    match deref(v) {
        Value::Array(a) => (a.iter().map(|(_, x)| bytes_of(x)).collect(), true),
        other => (vec![bytes_of(&other)], false),
    }
}

fn fn_preg_replace(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (pats, _pat_is_arr) = as_list(&args[0]);
    let (reps, rep_is_arr) = as_list(&args[1]);
    let limit = int_arg(args, 3, -1);
    let mut count = 0usize;

    // If replacement is a scalar but pattern is an array, reuse it for each.
    let reps: Vec<Vec<u8>> = if rep_is_arr {
        reps
    } else {
        vec![reps.into_iter().next().unwrap_or_default()]
    };
    let reps_for_each: Vec<Vec<u8>> = if rep_is_arr {
        reps.clone()
    } else {
        // scalar replacement: apply the same template to every pattern
        vec![reps[0].clone(); pats.len()]
    };

    let result = match deref(&args[2]) {
        Value::Array(subjects) => {
            // subject is an array: return an array with the same keys.
            let mut out = PhpArray::new();
            for (k, sv) in subjects.iter() {
                let sb = bytes_of(sv);
                match replace_subject(&pats, &reps_for_each, &sb, limit, &mut count) {
                    Some(r) => out.insert(k.clone(), Value::str(r)),
                    None => return Ok(Value::Null),
                }
            }
            Value::Array(out)
        }
        other => {
            let sb = bytes_of(&other);
            match replace_subject(&pats, &reps_for_each, &sb, limit, &mut count) {
                Some(r) => Value::str(r),
                None => return Ok(Value::Null),
            }
        }
    };

    write_ref(args.get(4), Value::Int(count as i64));
    Ok(result)
}

// ---------------------------------------------------------------------------
// preg_replace_callback
// ---------------------------------------------------------------------------

fn fn_preg_replace_callback(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (pats, _) = as_list(&args[0]);
    let cb = deref(&args[1]);
    let limit = int_arg(args, 3, -1);
    let mut count = 0usize;

    // Mirror preg_replace's subject handling (scalar or array of subjects).
    let run = |host: &mut dyn Host, subject: &[u8], count: &mut usize| -> Result<Option<Vec<u8>>, Thrown> {
        let mut cur = subject.to_vec();
        for pat in &pats {
            let Some(re) = compile(pat) else { return Ok(None) };
            let names = capture_names(&re);
            let text = String::from_utf8_lossy(&cur);
            // Build replacements by invoking the callback; errors from the
            // callback propagate out as a `Thrown`.
            let mut pending: Result<(), Thrown> = Ok(());
            let (bytes, n) = match replace_driver(&re, &text, limit, |caps, out| {
                if pending.is_err() {
                    return;
                }
                let m = build_match(caps, &names, false, false);
                match host.call_callable(&cb, &[Value::Array(m)]) {
                    Ok(v) => out.extend_from_slice(&bytes_of(&v)),
                    Err(e) => pending = Err(e),
                }
            }) {
                Ok(r) => r,
                Err(()) => return Ok(None),
            };
            pending?;
            *count += n;
            cur = bytes;
        }
        Ok(Some(cur))
    };

    let result = match deref(&args[2]) {
        Value::Array(subjects) => {
            let mut out = PhpArray::new();
            for (k, sv) in subjects.iter() {
                let sb = bytes_of(sv);
                match run(host, &sb, &mut count)? {
                    Some(r) => out.insert(k.clone(), Value::str(r)),
                    None => return Ok(Value::Null),
                }
            }
            Value::Array(out)
        }
        other => {
            let sb = bytes_of(&other);
            match run(host, &sb, &mut count)? {
                Some(r) => Value::str(r),
                None => return Ok(Value::Null),
            }
        }
    };

    write_ref(args.get(4), Value::Int(count as i64));
    Ok(result)
}

// ---------------------------------------------------------------------------
// preg_split
// ---------------------------------------------------------------------------

fn fn_preg_split(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(re) = compile(&bytes_of(&args[0])) else {
        return Ok(Value::Bool(false));
    };
    let subject = bytes_of(&args[1]);
    let text = String::from_utf8_lossy(&subject);
    let limit = int_arg(args, 2, -1);
    let flags = int_arg(args, 3, 0);
    let no_empty = flags & PREG_SPLIT_NO_EMPTY != 0;
    let delim_capture = flags & PREG_SPLIT_DELIM_CAPTURE != 0;
    let offset_capture = flags & PREG_SPLIT_OFFSET_CAPTURE != 0;

    let mut out = PhpArray::new();
    // Emit a piece honouring NO_EMPTY and OFFSET_CAPTURE.
    let emit = |out: &mut PhpArray, piece: &str, off: usize| {
        if no_empty && piece.is_empty() {
            return;
        }
        let v = Value::str(piece.as_bytes().to_vec());
        out.push(if offset_capture { pair(v, off as i64) } else { v });
    };

    let mut next = 0usize; // start of the current (unemitted) text piece
    let mut pos = 0usize;
    let mut pieces = 0usize; // emitted text pieces, for the limit
    loop {
        // limit>0: once we have limit-1 text pieces, the remainder is the last.
        if limit > 0 && pieces as i64 >= limit - 1 {
            break;
        }
        match re.captures_from_pos(&text, pos) {
            Ok(Some(caps)) => {
                let m = caps.get(0).unwrap();
                let (s, e) = (m.start(), m.end());
                emit(&mut out, &text[next..s], next);
                pieces += 1;
                if delim_capture {
                    let last = last_participating(&caps);
                    for i in 1..=last {
                        if let Some(g) = caps.get(i) {
                            emit(&mut out, g.as_str(), g.start());
                        } else {
                            emit(&mut out, "", s);
                        }
                    }
                }
                next = e;
                if e == s {
                    pos = e + char_len_at(&text, e);
                } else {
                    pos = e;
                }
                if pos > text.len() {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => return Ok(Value::Bool(false)),
        }
    }
    emit(&mut out, &text[next.min(text.len())..], next.min(text.len()));

    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// preg_quote
// ---------------------------------------------------------------------------

/// Escape the PCRE metacharacters (and an optional delimiter). NUL becomes the
/// literal escape `\000`; every metacharacter is prefixed with a backslash.
fn fn_preg_quote(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    // delimiter: first byte of the second argument, if any.
    let delim = args.get(1).map(bytes_of).filter(|d| !d.is_empty()).map(|d| d[0]);
    let mut out = Vec::with_capacity(s.len() + s.len() / 2);
    for &b in &s {
        match b {
            0 => out.extend_from_slice(b"\\000"),
            b'.' | b'\\' | b'+' | b'*' | b'?' | b'[' | b'^' | b']' | b'$' | b'(' | b')'
            | b'{' | b'}' | b'=' | b'!' | b'<' | b'>' | b'|' | b':' | b'-' | b'#' => {
                out.push(b'\\');
                out.push(b);
            }
            other => {
                if Some(other) == delim {
                    out.push(b'\\');
                }
                out.push(other);
            }
        }
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// preg_grep
// ---------------------------------------------------------------------------

fn fn_preg_grep(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(re) = compile(&bytes_of(&args[0])) else {
        return Ok(Value::Bool(false));
    };
    let Value::Array(input) = deref(&args[1]) else {
        return Err(Thrown::type_error("preg_grep(): Argument #2 ($array) must be of type array"));
    };
    let invert = int_arg(args, 2, 0) & PREG_GREP_INVERT != 0;
    let mut out = PhpArray::new();
    for (k, v) in input.iter() {
        let b = bytes_of(v);
        let text = String::from_utf8_lossy(&b);
        let matched = match re.find(&text) {
            Ok(opt) => opt.is_some(),
            Err(_) => return Ok(Value::Bool(false)),
        };
        if matched != invert {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// registration + constants
// ---------------------------------------------------------------------------

/// The `PREG_*` constants this module relies on, for inclusion in lib.rs
/// `constants()`. Values verified against `php -r 'echo PREG_*;'` (php 8.5.8).
pub fn preg_constants() -> Vec<(&'static str, Value)> {
    vec![
        ("PREG_PATTERN_ORDER", Value::Int(PREG_PATTERN_ORDER)),
        ("PREG_SET_ORDER", Value::Int(PREG_SET_ORDER)),
        ("PREG_OFFSET_CAPTURE", Value::Int(PREG_OFFSET_CAPTURE)),
        ("PREG_SPLIT_NO_EMPTY", Value::Int(PREG_SPLIT_NO_EMPTY)),
        ("PREG_SPLIT_DELIM_CAPTURE", Value::Int(PREG_SPLIT_DELIM_CAPTURE)),
        ("PREG_SPLIT_OFFSET_CAPTURE", Value::Int(PREG_SPLIT_OFFSET_CAPTURE)),
        ("PREG_UNMATCHED_AS_NULL", Value::Int(PREG_UNMATCHED_AS_NULL)),
        ("PREG_GREP_INVERT", Value::Int(PREG_GREP_INVERT)),
    ]
}

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
    // `&$matches` / `&$count` are by-reference OUT params (arg 2 for the match
    // functions, the final arg for the replace functions).
    put(m, "preg_match", fn_preg_match, 2, Some(5));
    put(m, "preg_match_all", fn_preg_match_all, 2, Some(5));
    put(m, "preg_replace", fn_preg_replace, 3, Some(5));
    put(m, "preg_replace_callback", fn_preg_replace_callback, 3, Some(5));
    put(m, "preg_split", fn_preg_split, 2, Some(4));
    put(m, "preg_quote", fn_preg_quote, 1, Some(2));
    put(m, "preg_grep", fn_preg_grep, 2, Some(3));
}

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
        let Value::Array(a) = v else { panic!("not array") };
        match a.get(&key) {
            Some(Value::Str(p)) => String::from_utf8_lossy(p.as_bytes()).into_owned(),
            other => panic!("expected string at {key:?}, got {:?}", other.map(|v| v.type_name())),
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
        assert_eq!(matches!(r, Value::Int(1)), true);
        assert_eq!(arr_str(&slot_val(&m), ArrayKey::Int(0)), "123");
    }

    #[test]
    fn match_no_match() {
        let m = slot();
        let r = re(&[s("/xyz/"), s("abc"), m.clone()]);
        assert!(matches!(r, Value::Int(0)));
        // $matches set to an empty array
        let Value::Array(a) = slot_val(&m) else { panic!() };
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
    fn match_offset_capture() {
        // php: 0 => array('123', 3)
        let m = slot();
        re(&[s("/\\d+/"), s("abc123def"), m.clone(), Value::Int(PREG_OFFSET_CAPTURE)]);
        let v = slot_val(&m);
        let Value::Array(a) = &v else { panic!() };
        let Some(Value::Array(inner)) = a.get(&ArrayKey::Int(0)) else { panic!() };
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
        let Value::Array(a2) = slot_val(&m2) else { panic!() };
        assert_eq!(a2.len(), 2); // only 0 and 1
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
        let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else { panic!() };
        assert_eq!(as_str(g0.get(&ArrayKey::Int(1)).unwrap()), "2b");
        let Some(Value::Array(g2)) = a.get(&ArrayKey::Int(2)) else { panic!() };
        assert_eq!(as_str(g2.get(&ArrayKey::Int(2)).unwrap()), "c");
    }

    #[test]
    fn match_all_set_order() {
        // php SET_ORDER: [[ '1a','1','a'], ['2b','2','b'], ['3c','3','c']]
        let (r, v) = match_all(&[s("/(\\d)(\\w)/"), s("1a2b3c"), Value::Int(PREG_SET_ORDER)]);
        assert!(matches!(r, Value::Int(3)));
        let Value::Array(a) = &v else { panic!() };
        let Some(Value::Array(set1)) = a.get(&ArrayKey::Int(1)) else { panic!() };
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
        let Some(Value::Array(g0)) = a.get(&ArrayKey::Int(0)) else { panic!() };
        assert_eq!(g0.len(), 0);
    }

    fn replace(args: &[Value]) -> Value {
        fn_preg_replace(&mut NullHost, args).unwrap()
    }

    #[test]
    fn replace_backrefs() {
        // swap words: $2 $1
        assert_eq!(as_str(&replace(&[s("/(\\w+)\\s(\\w+)/"), s("$2 $1"), s("hello world")])), "world hello");
        // \1 backslash form
        assert_eq!(as_str(&replace(&[s("/(\\d+)/"), s("[\\1]"), s("a1b22c")])), "a[1]b[22]c");
        // ${1} brace form, and disambiguation ${1}2 => group1 + '2'
        assert_eq!(as_str(&replace(&[s("/(\\d)(\\d)/"), s("${1}2"), s("ab45cd")])), "ab42cd");
        // $12 reads two digits => group 12 (nonexistent) => ''
        assert_eq!(as_str(&replace(&[s("/(\\d)(\\d)/"), s("$12"), s("ab45cd")])), "abcd");
        // \$ is a literal dollar, \n stays verbatim
        assert_eq!(as_str(&replace(&[s("/a/"), s("x\\$1y"), s("a")])), "x$1y");
        assert_eq!(as_str(&replace(&[s("/a/"), s("x\\nz"), s("a")])), "x\\nz");
        // no match returns subject unchanged
        assert_eq!(as_str(&replace(&[s("/z/"), s("#"), s("abc")])), "abc");
    }

    #[test]
    fn replace_limit_and_count() {
        // limit 2
        assert_eq!(as_str(&replace(&[s("/\\d/"), s("#"), s("1234"), Value::Int(2)])), "##34");
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
                let n = phrust_values::conv::to_int(whole);
                Value::str((n * 2).to_string())
            }),
        };
        let r = fn_preg_replace_callback(&mut host, &[s("/\\d+/"), Value::Null, s("a3b4")]).unwrap();
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
            fn_preg_replace_callback(&mut host2, &[s("/(\\d)(\\d)/"), Value::Null, s("x12y34")]).unwrap();
        assert_eq!(as_str(&r2), "x21y43");
    }

    fn split(args: &[Value]) -> Vec<String> {
        let Value::Array(a) = fn_preg_split(&mut NullHost, args).unwrap() else { panic!() };
        a.iter().map(|(_, v)| as_str(v)).collect()
    }

    #[test]
    fn split_basic_and_limit() {
        assert_eq!(split(&[s("/,/"), s("a,b,c")]), vec!["a", "b", "c"]);
        assert_eq!(split(&[s("/,/"), s("a,b,c,d"), Value::Int(2)]), vec!["a", "b,c,d"]);
        assert_eq!(split(&[s("/\\s+/"), s("hello   world  foo")]), vec!["hello", "world", "foo"]);
    }

    #[test]
    fn split_flags() {
        // NO_EMPTY
        assert_eq!(
            split(&[s("/,/"), s("a,,b,"), Value::Int(-1), Value::Int(PREG_SPLIT_NO_EMPTY)]),
            vec!["a", "b"]
        );
        // DELIM_CAPTURE
        assert_eq!(
            split(&[s("/([,;])/"), s("a,b;c"), Value::Int(-1), Value::Int(PREG_SPLIT_DELIM_CAPTURE)]),
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
            split(&[s("/(,)|(;)/"), s("a,b;c"), Value::Int(-1), Value::Int(PREG_SPLIT_DELIM_CAPTURE)]),
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
        let Value::Array(a) = fn_preg_grep(&mut NullHost, args).unwrap() else { panic!() };
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
        assert_eq!(r, vec![(ArrayKey::Int(0), "a1".into()), (ArrayKey::Int(2), "c3".into())]);
        // INVERT => [1=>'bb',3=>'dd']
        let r2 = grep(&[s("/\\d/"), Value::Array(input), Value::Int(PREG_GREP_INVERT)]);
        assert_eq!(r2, vec![(ArrayKey::Int(1), "bb".into()), (ArrayKey::Int(3), "dd".into())]);
    }

    #[test]
    fn invalid_pattern_returns_false() {
        let m = slot();
        assert!(matches!(re(&[s("/(/"), s("abc"), m]), Value::Bool(false)));
        assert!(matches!(fn_preg_split(&mut NullHost, &[s("bad"), s("x")]).unwrap(), Value::Bool(false)));
        // preg_replace returns null on a bad pattern
        assert!(matches!(replace(&[s("/(/"), s("x"), s("abc")]), Value::Null));
    }

    #[test]
    fn flag_modifiers() {
        // case-insensitive
        let m = slot();
        assert!(matches!(re(&[s("/ABC/i"), s("xabcy"), m]), Value::Int(1)));
        // dotall: a.b matches across newline
        let m2 = slot();
        assert!(matches!(re(&[s("/a.b/s"), s("a\nb"), m2]), Value::Int(1)));
        // alternate delimiters
        let m3 = slot();
        assert!(matches!(re(&[s("#/path/#"), s("/path/"), m3]), Value::Int(1)));
        let m4 = slot();
        assert!(matches!(re(&[s("{\\d+}"), s("a12b"), m4]), Value::Int(1)));
    }
}
