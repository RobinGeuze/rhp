//! ext/standard/string.c: the string functions.
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see `string/tests.rs` — expected values are hardcoded from
//! `php -r '...'` output).

use std::cell::RefCell;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::ext::random::mt_range;
use crate::file_fns::path_arg;
use crate::query_fns::{register_variable, url_decode};
use crate::zend::builtin_functions::{binary_strncasecmp, binary_strncmp};
use crate::zend::zpp::{bool_param, str_or_null_param, str_param, zval_get_string};
use crate::{deprecated, Host, Thrown};

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

/// Leftmost occurrence of `needle` in `haystack` (byte search).
fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Replace every (non-overlapping, left-to-right) occurrence of `search`.
fn replace_all(subject: &[u8], search: &[u8], replace: &[u8], n: &mut i64) -> Vec<u8> {
    if search.is_empty() {
        return subject.to_vec();
    }
    let mut out = Vec::with_capacity(subject.len());
    let mut i = 0;
    while i < subject.len() {
        if i + search.len() <= subject.len() && &subject[i..i + search.len()] == search {
            out.extend_from_slice(replace);
            *n += 1;
            i += search.len();
        } else {
            out.push(subject[i]);
            i += 1;
        }
    }
    out
}

/// Build php's `php_charmask` set (supports the `a..z` range syntax).
fn charmask(chars: &[u8]) -> [bool; 256] {
    let mut mask = [false; 256];
    let len = chars.len();
    let mut i = 0;
    while i < len {
        let c = chars[i];
        // range: c '.' '.' d  (needs a 4th char present, and d >= c)
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

/// Default trim charlist: " \t\n\r\0\x0B".
const TRIM_DEFAULT: &[u8] = b" \t\n\r\0\x0B";

// ---------------------------------------------------------------------------
// substr
// ---------------------------------------------------------------------------

/// Clamp a php start/offset against `len`, returning a byte index in `0..=len`.
fn clamp_start(start: i64, len: usize) -> usize {
    let len = len as i64;
    if start < 0 {
        (len + start).max(0) as usize
    } else {
        start.min(len) as usize
    }
}

fn fn_substr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let len = s.len();
    let start = clamp_start(conv::to_int(&deref(&args[1])), len);
    // remaining length after start
    let rest = len - start;
    let take = match args.get(2).map(deref) {
        None | Some(Value::Null) => rest,
        Some(v) => {
            let l = conv::to_int(&v);
            if l < 0 {
                // cut |l| bytes off the end of the remainder
                let drop = (-l) as usize;
                rest.saturating_sub(drop)
            } else {
                (l as usize).min(rest)
            }
        }
    };
    Ok(Value::str(s[start..start + take].to_vec()))
}

// ---------------------------------------------------------------------------
// str_replace
// ---------------------------------------------------------------------------

/// php's `php_str_replace_in_subject`: search and replace elements go
/// through `zval_get_string`, an empty subject skips them, and the loop stops
/// once the subject becomes empty.
fn replace_in_subject(
    host: &mut dyn Host,
    subject: Vec<u8>,
    search: &Value,
    replace: &Value,
    replace_fn: ReplaceFn,
    n: &mut i64,
) -> Result<Vec<u8>, Thrown> {
    if subject.is_empty() {
        return Ok(subject);
    }
    let Value::Array(searches) = search else {
        return Ok(replace_fn(
            &subject,
            &conv::to_string_bytes(search),
            &conv::to_string_bytes(replace),
            n,
        ));
    };
    let mut cur = subject;
    for (i, (_, sv)) in searches.iter().enumerate() {
        let sbytes = zval_get_string(host, sv)?;
        let rbytes = match replace {
            Value::Array(reps) => match reps.get_index(i) {
                Some((_, v)) => zval_get_string(host, v)?,
                None => Vec::new(),
            },
            other => conv::to_string_bytes(other),
        };
        if sbytes.is_empty() {
            continue;
        }
        let before = *n;
        cur = replace_fn(&cur, &sbytes, &rbytes, n);
        if *n != before && cur.is_empty() {
            break;
        }
    }
    Ok(cur)
}

pub(crate) type ReplaceFn = fn(&[u8], &[u8], &[u8], &mut i64) -> Vec<u8>;

/// php's `_php_str_replace_common`, shared by str_replace and str_ireplace.
/// Every subject element is converted with `zval_get_string` and counted.
pub(crate) fn str_replace_common(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    replace_fn: ReplaceFn,
) -> Result<Value, Thrown> {
    let search = deref(&args[0]);
    let replace = deref(&args[1]);
    let subject = deref(&args[2]);
    if !matches!(search, Value::Array(_)) && matches!(replace, Value::Array(_)) {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #2 ($replace) must be of type string when argument #1 ($search) is a string"
        )));
    }
    let mut n = 0;
    let out = if let Value::Array(items) = subject {
        let mut out = PhpArray::new();
        for (k, v) in items.iter() {
            let s = zval_get_string(host, v)?;
            let res = replace_in_subject(host, s, &search, &replace, replace_fn, &mut n)?;
            out.insert(k.clone(), Value::str(res));
        }
        Value::Array(out)
    } else {
        let s = conv::to_string_bytes(&subject);
        Value::str(replace_in_subject(
            host, s, &search, &replace, replace_fn, &mut n,
        )?)
    };
    // `&$count`, written when passed.
    if let Some(Value::Ref(r)) = args.get(3) {
        *r.borrow_mut() = Value::Int(n);
    }
    Ok(out)
}

fn fn_str_replace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    str_replace_common(host, args, "str_replace", replace_all)
}

// ---------------------------------------------------------------------------
// strpos / stripos / strrpos / strripos
// ---------------------------------------------------------------------------

/// php offset bounds check for strpos-family: `-len <= offset <= len`.
fn resolve_fwd_offset(offset: i64, len: usize, fname: &str) -> Result<usize, Thrown> {
    let len_i = len as i64;
    if offset < -len_i || offset > len_i {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"
            ),
        ));
    }
    Ok(if offset < 0 {
        (len_i + offset) as usize
    } else {
        offset as usize
    })
}

fn strpos_impl(args: &[Value], ci: bool, fname: &str) -> Result<Value, Thrown> {
    let mut haystack = conv::to_string_bytes(&args[0]);
    let mut needle = conv::to_string_bytes(&args[1]);
    let off = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let start = resolve_fwd_offset(off, haystack.len(), fname)?;
    if ci {
        haystack.make_ascii_lowercase();
        needle.make_ascii_lowercase();
    }
    match find_sub(&haystack[start..], &needle) {
        Some(p) => Ok(Value::Int((start + p) as i64)),
        None => Ok(Value::Bool(false)),
    }
}

fn strrpos_impl(args: &[Value], ci: bool, fname: &str) -> Result<Value, Thrown> {
    let mut haystack = conv::to_string_bytes(&args[0]);
    let mut needle = conv::to_string_bytes(&args[1]);
    let off = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if ci {
        haystack.make_ascii_lowercase();
        needle.make_ascii_lowercase();
    }
    let len = haystack.len() as i64;
    let nlen = needle.len() as i64;
    // Bounds: -len <= off <= len.
    if off < -len || off > len {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"
            ),
        ));
    }
    // php_memnrstr semantics: compute [p_min, e_max] (inclusive start-index window).
    let (p_min, e_max): (i64, i64) = if off >= 0 {
        (off, len - nlen)
    } else if nlen > -off {
        (0, len - nlen)
    } else {
        (0, len + off)
    };
    if e_max < p_min || e_max < 0 {
        return Ok(Value::Bool(false));
    }
    let mut idx = e_max;
    while idx >= p_min {
        let i = idx as usize;
        if haystack[i..i + needle.len()] == needle[..] {
            return Ok(Value::Int(idx));
        }
        idx -= 1;
    }
    Ok(Value::Bool(false))
}

fn fn_strpos(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strpos_impl(args, false, "strpos")
}
fn fn_stripos(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strpos_impl(args, true, "stripos")
}
fn fn_strrpos(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strrpos_impl(args, false, "strrpos")
}
fn fn_strripos(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strrpos_impl(args, true, "strripos")
}

// ---------------------------------------------------------------------------
// str_pad
// ---------------------------------------------------------------------------

fn fn_str_pad(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let target = conv::to_int(&deref(&args[1]));
    let pad = match args.get(2) {
        Some(v) => conv::to_string_bytes(v),
        None => b" ".to_vec(),
    };
    let ptype = args.get(3).map(|v| conv::to_int(&deref(v))).unwrap_or(1); // STR_PAD_RIGHT
    if target <= s.len() as i64 {
        return Ok(Value::str(s));
    }
    if pad.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "str_pad(): Argument #3 ($pad_string) must not be empty",
        ));
    }
    let total_pad = target as usize - s.len();
    // zend_string_safe_falloc(1, len, pad_chars), after the argument checks.
    if (0..=2).contains(&ptype)
        && !rhp_values::mem::reserve_safe(1, s.len(), rhp_values::mem::zstr_size(total_pad))
    {
        return Ok(Value::Null);
    }
    let make = |n: usize| -> Vec<u8> {
        let mut v = Vec::with_capacity(n);
        while v.len() < n {
            let take = (n - v.len()).min(pad.len());
            v.extend_from_slice(&pad[..take]);
        }
        v
    };
    let out = match ptype {
        0 => {
            // STR_PAD_LEFT
            let mut v = make(total_pad);
            v.extend_from_slice(&s);
            v
        }
        1 => {
            // STR_PAD_RIGHT
            let mut v = s.clone();
            v.extend_from_slice(&make(total_pad));
            v
        }
        2 => {
            // STR_PAD_BOTH: left = floor(total/2), right = remainder
            let left = total_pad / 2;
            let right = total_pad - left;
            let mut v = make(left);
            v.extend_from_slice(&s);
            v.extend_from_slice(&make(right));
            v
        }
        _ => {
            return Err(Thrown::error(
                "ValueError",
                "str_pad(): Argument #4 ($pad_type) must be STR_PAD_LEFT, STR_PAD_RIGHT, or STR_PAD_BOTH",
            ))
        }
    };
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// trim / ltrim / rtrim
// ---------------------------------------------------------------------------

fn trim_impl(args: &[Value], left: bool, right: bool) -> Value {
    let s = conv::to_string_bytes(&args[0]);
    let chars = match args.get(1) {
        Some(v) => conv::to_string_bytes(v),
        None => TRIM_DEFAULT.to_vec(),
    };
    let mask = charmask(&chars);
    let mut start = 0;
    let mut end = s.len();
    if left {
        while start < end && mask[s[start] as usize] {
            start += 1;
        }
    }
    if right {
        while end > start && mask[s[end - 1] as usize] {
            end -= 1;
        }
    }
    Value::str(s[start..end].to_vec())
}

fn fn_trim(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(trim_impl(args, true, true))
}
fn fn_ltrim(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(trim_impl(args, true, false))
}
fn fn_rtrim(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(trim_impl(args, false, true))
}

// ---------------------------------------------------------------------------
// ucfirst / lcfirst / ucwords / strrev
// ---------------------------------------------------------------------------

fn fn_ucfirst(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = conv::to_string_bytes(&args[0]);
    if let Some(b) = s.first_mut() {
        b.make_ascii_uppercase();
    }
    Ok(Value::str(s))
}

fn fn_lcfirst(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = conv::to_string_bytes(&args[0]);
    if let Some(b) = s.first_mut() {
        b.make_ascii_lowercase();
    }
    Ok(Value::str(s))
}

fn fn_ucwords(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = conv::to_string_bytes(&args[0]);
    let delims = match args.get(1) {
        Some(v) => conv::to_string_bytes(v),
        None => b" \t\r\n\x0c\x0b".to_vec(),
    };
    let mask = charmask_plain(&delims);
    let mut cap_next = true;
    for b in s.iter_mut() {
        if cap_next {
            b.make_ascii_uppercase();
        }
        cap_next = mask[*b as usize];
    }
    Ok(Value::str(s))
}

/// ucwords uses a plain char set (no `a..z` range interpretation).
fn charmask_plain(chars: &[u8]) -> [bool; 256] {
    let mut mask = [false; 256];
    for &c in chars {
        mask[c as usize] = true;
    }
    mask
}

fn fn_strrev(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = conv::to_string_bytes(&args[0]);
    s.reverse();
    Ok(Value::str(s))
}

// ---------------------------------------------------------------------------
// substr_count
// ---------------------------------------------------------------------------

fn fn_substr_count(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let haystack = conv::to_string_bytes(&args[0]);
    let needle = conv::to_string_bytes(&args[1]);
    if needle.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "substr_count(): Argument #2 ($needle) must not be empty",
        ));
    }
    let len = haystack.len() as i64;
    let off = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if off < -len || off > len {
        return Err(Thrown::error(
            "ValueError",
            "substr_count(): Argument #3 ($offset) must be contained in argument #1 ($haystack)",
        ));
    }
    let start = if off < 0 {
        (len + off) as usize
    } else {
        off as usize
    };
    let end = match args.get(3).map(deref) {
        None | Some(Value::Null) => haystack.len(),
        Some(v) => {
            let l = conv::to_int(&v);
            let avail = (haystack.len() - start) as i64;
            let window = if l < 0 { avail + l } else { l.min(avail) };
            if window < 0 {
                return Err(Thrown::error(
                    "ValueError",
                    "substr_count(): Argument #4 ($length) must be contained in argument #1 ($haystack)",
                ));
            }
            start + window as usize
        }
    };
    let slice = &haystack[start..end];
    let mut count = 0i64;
    let mut i = 0;
    while i + needle.len() <= slice.len() {
        if &slice[i..i + needle.len()] == needle.as_slice() {
            count += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    Ok(Value::Int(count))
}

// ---------------------------------------------------------------------------
// str_starts_with / str_ends_with
// ---------------------------------------------------------------------------

fn fn_str_starts_with(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = conv::to_string_bytes(&args[0]);
    let n = conv::to_string_bytes(&args[1]);
    Ok(Value::Bool(h.len() >= n.len() && h[..n.len()] == n[..]))
}

fn fn_str_ends_with(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = conv::to_string_bytes(&args[0]);
    let n = conv::to_string_bytes(&args[1]);
    Ok(Value::Bool(
        h.len() >= n.len() && h[h.len() - n.len()..] == n[..],
    ))
}

// ---------------------------------------------------------------------------
// wordwrap
// ---------------------------------------------------------------------------

fn fn_wordwrap(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let text = conv::to_string_bytes(&args[0]);
    let width = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(75);
    let brk = match args.get(2) {
        Some(v) => conv::to_string_bytes(v),
        None => b"\n".to_vec(),
    };
    let cut = args
        .get(3)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);

    if text.is_empty() {
        return Ok(Value::str(Vec::new()));
    }
    if brk.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "wordwrap(): Argument #3 ($break) must not be empty",
        ));
    }
    if width == 0 && cut {
        return Err(Thrown::error(
            "ValueError",
            "wordwrap(): Argument #2 ($width) must be greater than 0 when argument #4 ($cut) is set to true",
        ));
    }

    let bytes = &text[..];
    let n = bytes.len() as i64;
    let w = width;
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() + bytes.len() / 8 + 8);
    let bl = brk.len();
    let mut laststart: i64 = 0;
    let mut lastspace: i64 = 0;
    let mut current: i64 = 0;

    while current < n {
        let ci = current as usize;
        if bl <= (n - current) as usize && bytes[ci..ci + bl] == brk[..] {
            // existing break: copy through it.
            out.extend_from_slice(&bytes[laststart as usize..ci + bl]);
            current += bl as i64;
            laststart = current;
            lastspace = current;
        } else if bytes[ci] == b' ' {
            if current - laststart >= w {
                out.extend_from_slice(&bytes[laststart as usize..ci]);
                out.extend_from_slice(&brk);
                laststart = current + 1;
            }
            lastspace = current;
            current += 1;
        } else if current - laststart >= w && laststart >= lastspace {
            if cut {
                out.extend_from_slice(&bytes[laststart as usize..ci]);
                out.extend_from_slice(&brk);
                laststart = current;
                lastspace = current;
            }
            current += 1;
        } else if current - laststart >= w && laststart < lastspace {
            out.extend_from_slice(&bytes[laststart as usize..lastspace as usize]);
            out.extend_from_slice(&brk);
            laststart = lastspace + 1;
            lastspace = laststart;
            current += 1;
        } else {
            current += 1;
        }
    }
    if current > laststart {
        out.extend_from_slice(&bytes[laststart as usize..current as usize]);
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// addcslashes / stripcslashes
// ---------------------------------------------------------------------------

/// `addcslashes($string, $characters)`: backslash-escape every byte present in
/// `$characters` (control bytes become `\n`, `\t`, … or a 3-digit octal escape).
fn fn_addcslashes(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let mask = charmask(&conv::to_string_bytes(&args[1]));
    let mut out = Vec::with_capacity(s.len());
    for &c in &s {
        if mask[c as usize] {
            if !(32..=126).contains(&c) {
                out.push(b'\\');
                match c {
                    b'\n' => out.push(b'n'),
                    b'\t' => out.push(b't'),
                    b'\r' => out.push(b'r'),
                    0x07 => out.push(b'a'),
                    0x0b => out.push(b'v'),
                    0x08 => out.push(b'b'),
                    0x0c => out.push(b'f'),
                    _ => out.extend_from_slice(format!("{c:03o}").as_bytes()),
                }
            } else {
                out.push(b'\\');
                out.push(c);
            }
        } else {
            out.push(c);
        }
    }
    Ok(Value::str(out))
}

/// `stripcslashes($string)`: the inverse of [`fn_addcslashes`] (a faithful port
/// of php's `php_stripcslashes`), decoding `\n`, `\xHH`, octal `\NNN`, etc.
fn fn_stripcslashes(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let n = s.len();
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        if s[i] == b'\\' && i + 1 < n {
            i += 1;
            let c = s[i];
            match c {
                b'n' => {
                    out.push(b'\n');
                    i += 1;
                }
                b't' => {
                    out.push(b'\t');
                    i += 1;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 1;
                }
                b'a' => {
                    out.push(0x07);
                    i += 1;
                }
                b'v' => {
                    out.push(0x0b);
                    i += 1;
                }
                b'b' => {
                    out.push(0x08);
                    i += 1;
                }
                b'f' => {
                    out.push(0x0c);
                    i += 1;
                }
                b'\\' => {
                    out.push(b'\\');
                    i += 1;
                }
                b'x' => {
                    if i + 1 < n && s[i + 1].is_ascii_hexdigit() {
                        let mut val = hex_val(s[i + 1]);
                        if i + 2 < n && s[i + 2].is_ascii_hexdigit() {
                            val = val * 16 + hex_val(s[i + 2]);
                            i += 3;
                        } else {
                            i += 2;
                        }
                        out.push(val);
                    } else {
                        out.push(b'x');
                        i += 1;
                    }
                }
                _ => {
                    let mut k = 0;
                    let mut val: u32 = 0;
                    while i < n && k < 3 && (b'0'..=b'7').contains(&s[i]) {
                        val = val * 8 + (s[i] - b'0') as u32;
                        i += 1;
                        k += 1;
                    }
                    if k > 0 {
                        out.push(val as u8);
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
            }
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    Ok(Value::str(out))
}

fn hex_val(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// count_chars
// ---------------------------------------------------------------------------

/// `count_chars($string, $mode = 0)`: per-byte frequency statistics. Modes 0–2
/// return an int-keyed array; modes 3–4 return a byte string.
fn fn_count_chars(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let mode = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if !(0..=4).contains(&mode) {
        return Err(Thrown::error(
            "ValueError",
            "count_chars(): Argument #2 ($mode) must be between 0 and 4 (inclusive)",
        ));
    }
    let mut counts = [0i64; 256];
    for &b in &s {
        counts[b as usize] += 1;
    }
    match mode {
        3 | 4 => {
            let want_present = mode == 3;
            let mut out = Vec::new();
            for (b, &c) in counts.iter().enumerate() {
                if (c > 0) == want_present {
                    out.push(b as u8);
                }
            }
            Ok(Value::str(out))
        }
        _ => {
            let mut arr = PhpArray::new();
            for (b, &c) in counts.iter().enumerate() {
                let keep = match mode {
                    0 => true,
                    1 => c > 0,
                    _ => c == 0, // mode 2
                };
                if keep {
                    arr.insert(ArrayKey::Int(b as i64), Value::Int(c));
                }
            }
            Ok(Value::Array(arr))
        }
    }
}

// String comparison builtins (pure, non-callback). The strcmp family
// itself is Zend's (zend/builtin_functions.rs).

// ---------------------------------------------------------------------------
// substr_compare
// ---------------------------------------------------------------------------

fn fn_substr_compare(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let main = conv::to_string_bytes(&args[0]);
    let needle = conv::to_string_bytes(&args[1]);
    let offset = conv::to_int(&deref(&args[2]));

    // length: arg #4, optional, may be null (= "default").
    let len_default = matches!(args.get(3).map(deref), None | Some(Value::Null));
    let len_val = if len_default {
        0
    } else {
        conv::to_int(&deref(&args[3]))
    };
    if !len_default && len_val < 0 {
        return Err(Thrown::error(
            "ValueError",
            "substr_compare(): Argument #4 ($length) must be greater than or equal to 0",
        ));
    }

    // resolve offset
    let mlen = main.len() as i64;
    let off = if offset < 0 {
        (mlen + offset).max(0)
    } else {
        offset
    };
    if off > mlen {
        return Err(Thrown::error(
            "ValueError",
            "substr_compare(): Argument #3 ($offset) must be contained in argument #1 ($haystack)",
        ));
    }
    let off = off as usize;
    let remain = main.len() - off;

    let cmp_len = if len_default {
        needle.len().max(remain)
    } else if (len_val as usize) > remain {
        remain
    } else {
        len_val as usize
    };
    if cmp_len == 0 && !len_default {
        return Ok(Value::Int(0));
    }

    let ci = args.get(4).map(conv::to_bool).unwrap_or(false);
    let main_sub = &main[off..];
    let r = if ci {
        binary_strncasecmp(main_sub, &needle, cmp_len)
    } else {
        binary_strncmp(main_sub, &needle, cmp_len)
    };
    Ok(Value::Int(r))
}

// ---------------------------------------------------------------------------
// strnatcmp / strnatcasecmp (Martin Pool natural-order algorithm, as php)
// ---------------------------------------------------------------------------

#[inline]
fn at(s: &[u8], i: usize) -> u8 {
    if i < s.len() {
        s[i]
    } else {
        0 // virtual NUL sentinel (mirrors php's NUL-terminated zend_string)
    }
}

#[inline]
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `compare_right`: longest run of digits wins; else greatest value, decided by
/// the first differing magnitude once both runs are known equal-length.
fn compare_right(
    a: &[u8],
    ap: &mut usize,
    aend: usize,
    b: &[u8],
    bp: &mut usize,
    bend: usize,
) -> i64 {
    let mut bias = 0i64;
    loop {
        let a_digit = *ap < aend && at(a, *ap).is_ascii_digit();
        let b_digit = *bp < bend && at(b, *bp).is_ascii_digit();
        if !a_digit && !b_digit {
            return bias;
        } else if !a_digit {
            return -1;
        } else if !b_digit {
            return 1;
        } else {
            let ca = at(a, *ap);
            let cb = at(b, *bp);
            if ca < cb {
                if bias == 0 {
                    bias = -1;
                }
            } else if ca > cb && bias == 0 {
                bias = 1;
            }
        }
        *ap += 1;
        *bp += 1;
    }
}

/// `compare_left`: left-aligned numbers; first differing value wins.
fn compare_left(
    a: &[u8],
    ap: &mut usize,
    aend: usize,
    b: &[u8],
    bp: &mut usize,
    bend: usize,
) -> i64 {
    loop {
        let a_digit = *ap < aend && at(a, *ap).is_ascii_digit();
        let b_digit = *bp < bend && at(b, *bp).is_ascii_digit();
        if !a_digit && !b_digit {
            return 0;
        } else if !a_digit {
            return -1;
        } else if !b_digit {
            return 1;
        } else {
            let ca = at(a, *ap);
            let cb = at(b, *bp);
            if ca < cb {
                return -1;
            } else if ca > cb {
                return 1;
            }
        }
        *ap += 1;
        *bp += 1;
    }
}

pub(crate) fn strnatcmp_ex(a: &[u8], b: &[u8], fold_case: bool) -> i64 {
    let aend = a.len();
    let bend = b.len();
    if a.is_empty() || b.is_empty() {
        return if a.len() == b.len() {
            0
        } else if a.len() > b.len() {
            1
        } else {
            -1
        };
    }

    let mut ap = 0usize;
    let mut bp = 0usize;
    let mut leading = true;
    loop {
        let mut ca = at(a, ap);
        let mut cb = at(b, bp);

        // skip over leading zeros (only at the very start of the strings)
        while leading && ca == b'0' && ap + 1 < aend && at(a, ap + 1).is_ascii_digit() {
            ap += 1;
            ca = at(a, ap);
        }
        while leading && cb == b'0' && bp + 1 < bend && at(b, bp + 1).is_ascii_digit() {
            bp += 1;
            cb = at(b, bp);
        }
        leading = false;

        // skip consecutive whitespace
        while is_space(ca) {
            ap += 1;
            ca = at(a, ap);
        }
        while is_space(cb) {
            bp += 1;
            cb = at(b, bp);
        }

        // process a run of digits
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let fractional = ca == b'0' || cb == b'0';
            let result = if fractional {
                compare_left(a, &mut ap, aend, b, &mut bp, bend)
            } else {
                compare_right(a, &mut ap, aend, b, &mut bp, bend)
            };
            if result != 0 {
                return result;
            } else if ap == aend && bp == bend {
                return 0;
            } else if ap == aend {
                return -1;
            } else if bp == bend {
                return 1;
            } else {
                continue;
            }
        }

        if fold_case {
            ca = ca.to_ascii_uppercase();
            cb = cb.to_ascii_uppercase();
        }
        if ca < cb {
            return -1;
        } else if ca > cb {
            return 1;
        }

        ap += 1;
        bp += 1;
        if ap >= aend && bp >= bend {
            return 0;
        } else if ap >= aend {
            return -1;
        } else if bp >= bend {
            return 1;
        }
    }
}

fn fn_strnatcmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(strnatcmp_ex(
        &conv::to_string_bytes(&args[0]),
        &conv::to_string_bytes(&args[1]),
        false,
    )))
}

fn fn_strnatcasecmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(strnatcmp_ex(
        &conv::to_string_bytes(&args[0]),
        &conv::to_string_bytes(&args[1]),
        true,
    )))
}

// ---------------------------------------------------------------------------
// str_ireplace (case-insensitive str_replace)
// ---------------------------------------------------------------------------

/// Case-insensitive ascii equality of two equal-length byte slices.
fn eq_ci(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// Replace every non-overlapping (left-to-right) case-insensitive occurrence.
fn ireplace_all(subject: &[u8], search: &[u8], replace: &[u8], n: &mut i64) -> Vec<u8> {
    if search.is_empty() {
        return subject.to_vec();
    }
    let mut out = Vec::with_capacity(subject.len());
    let mut i = 0;
    while i < subject.len() {
        if i + search.len() <= subject.len() && eq_ci(&subject[i..i + search.len()], search) {
            out.extend_from_slice(replace);
            *n += 1;
            i += search.len();
        } else {
            out.push(subject[i]);
            i += 1;
        }
    }
    out
}

fn fn_str_ireplace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    str_replace_common(host, args, "str_ireplace", ireplace_all)
}

// ---------------------------------------------------------------------------
// strpbrk
// ---------------------------------------------------------------------------

fn fn_strpbrk(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let chars = conv::to_string_bytes(&args[1]);
    if chars.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "strpbrk(): Argument #2 ($characters) must be a non-empty string",
        ));
    }
    let mut set = [false; 256];
    for &c in &chars {
        set[c as usize] = true;
    }
    match s.iter().position(|&c| set[c as usize]) {
        Some(i) => Ok(Value::str(s[i..].to_vec())),
        None => Ok(Value::Bool(false)),
    }
}

// ---------------------------------------------------------------------------
// strspn / strcspn
// ---------------------------------------------------------------------------

/// Shared offset/length clamping for strspn/strcspn. Returns the (start, len)
/// window into `subject`.
fn spn_window(subject_len: usize, args: &[Value]) -> (usize, usize) {
    let slen = subject_len as i64;
    let mut start = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if start < 0 {
        start += slen;
        if start < 0 {
            start = 0;
        }
    } else if start > slen {
        start = slen;
    }
    let remain = slen - start;

    let len_default = matches!(args.get(3).map(deref), None | Some(Value::Null));
    let len = if len_default {
        remain
    } else {
        let mut l = conv::to_int(&deref(&args[3]));
        if l < 0 {
            l += remain;
            if l < 0 {
                l = 0;
            }
        } else if l > remain {
            l = remain;
        }
        l
    };
    (start as usize, len as usize)
}

fn fn_strspn(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let subject = conv::to_string_bytes(&args[0]);
    let mask = conv::to_string_bytes(&args[1]);
    let (start, len) = spn_window(subject.len(), args);
    let mut set = [false; 256];
    for &c in &mask {
        set[c as usize] = true;
    }
    let window = &subject[start..start + len];
    let n = window.iter().take_while(|&&c| set[c as usize]).count();
    Ok(Value::Int(n as i64))
}

fn fn_strcspn(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let subject = conv::to_string_bytes(&args[0]);
    let mask = conv::to_string_bytes(&args[1]);
    let (start, len) = spn_window(subject.len(), args);
    let mut set = [false; 256];
    for &c in &mask {
        set[c as usize] = true;
    }
    let window = &subject[start..start + len];
    let n = window.iter().take_while(|&&c| !set[c as usize]).count();
    Ok(Value::Int(n as i64))
}

// ---------------------------------------------------------------------------
// str_rot13
// ---------------------------------------------------------------------------

fn fn_str_rot13(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut b = conv::to_string_bytes(&args[0]);
    for c in b.iter_mut() {
        *c = match *c {
            b'A'..=b'Z' => b'A' + (*c - b'A' + 13) % 26,
            b'a'..=b'z' => b'a' + (*c - b'a' + 13) % 26,
            other => other,
        };
    }
    Ok(Value::str(b))
}

// Substring-returning search builtins from `ext/standard/string.c`:
// `strstr` / `strchr`, `stristr`, `strrchr`, and the stateful `strtok`.

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(0);
    }
    if n.len() > h.len() {
        return None;
    }
    h.windows(n.len()).position(|w| w == n)
}

fn find_ci(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(0);
    }
    if n.len() > h.len() {
        return None;
    }
    h.windows(n.len()).position(|w| w.eq_ignore_ascii_case(n))
}

fn split_result(h: &[u8], at: Option<usize>, before: bool) -> Value {
    match at {
        None => Value::Bool(false),
        Some(p) if before => Value::str(h[..p].to_vec()),
        Some(p) => Value::str(h[p..].to_vec()),
    }
}

fn fn_strstr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = str_param(host, args, 0, "strstr", "haystack")?;
    let n = str_param(host, args, 1, "strstr", "needle")?;
    let before = bool_param(host, args, 2, "strstr", "before_needle", false)?;
    Ok(split_result(&h, find(&h, &n), before))
}

fn fn_strchr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = str_param(host, args, 0, "strchr", "haystack")?;
    let n = str_param(host, args, 1, "strchr", "needle")?;
    let before = bool_param(host, args, 2, "strchr", "before_needle", false)?;
    Ok(split_result(&h, find(&h, &n), before))
}

fn fn_stristr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = str_param(host, args, 0, "stristr", "haystack")?;
    let n = str_param(host, args, 1, "stristr", "needle")?;
    let before = bool_param(host, args, 2, "stristr", "before_needle", false)?;
    Ok(split_result(&h, find_ci(&h, &n), before))
}

/// `strrchr`: only the needle's first byte is searched for (an empty needle
/// searches for its NUL terminator, as php's `*ZSTR_VAL(needle)` does).
fn fn_strrchr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = str_param(host, args, 0, "strrchr", "haystack")?;
    let n = str_param(host, args, 1, "strrchr", "needle")?;
    let before = bool_param(host, args, 2, "strrchr", "before_needle", false)?;
    let c = n.first().copied().unwrap_or(0);
    Ok(split_result(&h, h.iter().rposition(|&b| b == c), before))
}

/// strtok's per-request state (`BG(strtok_string)` / `BG(strtok_last)`).
struct TokState {
    string: Option<Vec<u8>>,
    last: usize,
}

thread_local! {
    static STRTOK: RefCell<TokState> = const { RefCell::new(TokState { string: None, last: 0 }) };
}

pub(crate) fn reset_state() {
    STRTOK.with(|t| {
        *t.borrow_mut() = TokState {
            string: None,
            last: 0,
        }
    });
}

fn fn_strtok(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "strtok", "string")?;
    let tok = str_or_null_param(host, args, 1, "strtok", "token")?;
    STRTOK.with(|st| {
        let mut st = st.borrow_mut();
        let tok = match tok {
            None => s,
            Some(t) => {
                st.string = Some(s);
                st.last = 0;
                t
            }
        };
        let Some(string) = st.string.as_ref() else {
            // php: Warning "Both arguments must be provided when starting tokenization"
            return Ok(Value::Bool(false));
        };
        let pe = string.len();
        let mut p = st.last;
        if p >= pe {
            return Ok(Value::Bool(false));
        }
        let mut table = [false; 256];
        for &b in &tok {
            table[b as usize] = true;
        }
        let mut skipped = 0;
        while table[string[p] as usize] {
            p += 1;
            if p >= pe {
                st.string = None;
                return Ok(Value::Bool(false));
            }
            skipped += 1;
        }
        loop {
            p += 1;
            if p >= pe || table[string[p] as usize] {
                break;
            }
        }
        if p - st.last > 0 {
            let out = string[st.last + skipped..p].to_vec();
            st.last = p + 1;
            Ok(Value::str(out))
        } else {
            st.string = None;
            Ok(Value::Bool(false))
        }
    })
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// The byte form of an array key (int keys render as their decimal string).
fn key_bytes(k: &ArrayKey) -> Vec<u8> {
    match k {
        ArrayKey::Int(n) => n.to_string().into_bytes(),
        ArrayKey::Str(s) => s.as_bytes().to_vec(),
    }
}

/// Build php's `php_charmask` set (supports the `a..z` range syntax). Used by
/// `str_word_count`'s additional-characters argument.
fn word_charmask(chars: &[u8]) -> [bool; 256] {
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

// ---------------------------------------------------------------------------
// str_split
// ---------------------------------------------------------------------------

fn fn_str_split(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
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

fn fn_strtr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    if args.len() == 2 {
        // Pairs form: longest matching key at each position wins.
        let Value::Array(pairs) = deref(&args[1]) else {
            return Err(Thrown::type_error(format!(
                "strtr(): Argument #2 ($from) must be of type array, {} given",
                crate::zend::zpp::zval_value_name(host, &args[1])
            )));
        };
        let table: Vec<(Vec<u8>, Vec<u8>)> = pairs
            .iter()
            .filter_map(|(k, v)| {
                let kb = key_bytes(k);
                // php ignores empty-string keys (and emits a warning to stderr).
                if kb.is_empty() {
                    None
                } else {
                    Some((kb, conv::to_string_bytes(v)))
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
        let from = conv::to_string_bytes(&args[1]);
        let to = conv::to_string_bytes(&args[2]);
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
            let sbytes = conv::to_string_bytes(v);
            // replace: array -> nth element (else ""); scalar -> whole, for all.
            let rep = match &replace {
                Value::Array(ra) => ra
                    .get_index(i)
                    .map(|(_, rv)| conv::to_string_bytes(rv))
                    .unwrap_or_default(),
                other => conv::to_string_bytes(other),
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
            out.insert(
                k.clone(),
                Value::str(substr_replace_one(&sbytes, st, ln, &rep)),
            );
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
            Value::Array(ra) => ra
                .iter()
                .next()
                .map(|(_, rv)| conv::to_string_bytes(rv))
                .unwrap_or_default(),
            other => conv::to_string_bytes(other),
        };
        let st = conv::to_int(&start_arg);
        let ln = length_arg.as_ref().map(conv::to_int);
        Ok(Value::str(substr_replace_one(&sbytes, st, ln, &rep)))
    }
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
    let a = conv::to_string_bytes(&args[0]);
    let b = conv::to_string_bytes(&args[1]);
    let total = a.len() + b.len();
    let sim = if total == 0 { 0 } else { similar_char(&a, &b) };
    // `&$percent`, written when passed.
    if let Some(Value::Ref(r)) = args.get(2) {
        let pct = if total == 0 {
            0.0
        } else {
            sim as f64 * 200.0 / total as f64
        };
        *r.borrow_mut() = Value::Float(pct);
    }
    Ok(Value::Int(sim as i64))
}

// ---------------------------------------------------------------------------
// str_word_count
// ---------------------------------------------------------------------------

fn fn_str_word_count(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let format = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if !matches!(format, 0..=2) {
        return Err(Thrown::error(
            "ValueError",
            "str_word_count(): Argument #2 ($format) must be a valid format value",
        ));
    }
    let extra = args.get(2).map(conv::to_string_bytes);
    let extra_mask = extra.map(|e| word_charmask(&e)).unwrap_or([false; 256]);
    let is_word =
        |b: u8| b.is_ascii_alphabetic() || b == b'\'' || b == b'-' || extra_mask[b as usize];

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
                2 => words.insert(
                    ArrayKey::Int(start as i64),
                    Value::str(s[start..i].to_vec()),
                ),
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
    let s = conv::to_string_bytes(&args[0]);
    let chunklen = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(76);
    if chunklen < 1 {
        return Err(Thrown::error(
            "ValueError",
            "chunk_split(): Argument #2 ($length) must be greater than 0",
        ));
    }
    let end = match args.get(2) {
        Some(v) => conv::to_string_bytes(v),
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

/// C `isspace`: space, \t, \n, \v, \f, \r.
fn c_isspace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

// ---------------------------------------------------------------------------
// nl2br
// ---------------------------------------------------------------------------

fn fn_nl2br(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let is_xhtml = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(true);
    let br: &[u8] = if is_xhtml { b"<br />" } else { b"<br>" };
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'\r' | b'\n' => {
                out.extend_from_slice(br);
                // consume a two-char newline pair (\r\n or \n\r) as one unit.
                if i + 1 < s.len() && (s[i + 1] == b'\r' || s[i + 1] == b'\n') && s[i + 1] != s[i] {
                    out.push(s[i]);
                    out.push(s[i + 1]);
                    i += 2;
                } else {
                    out.push(s[i]);
                    i += 1;
                }
            }
            o => {
                out.push(o);
                i += 1;
            }
        }
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// addslashes / stripslashes / quotemeta
// ---------------------------------------------------------------------------

fn fn_addslashes(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    for &b in &s {
        match b {
            b'\'' | b'"' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            0 => {
                out.push(b'\\');
                out.push(b'0');
            }
            o => out.push(o),
        }
    }
    Ok(Value::str(out))
}

fn fn_stripslashes(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' {
            if i + 1 >= s.len() {
                // trailing backslash is dropped
                break;
            }
            let n = s[i + 1];
            if n == b'0' {
                out.push(0);
            } else {
                out.push(n);
            }
            i += 2;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    Ok(Value::str(out))
}

fn fn_quotemeta(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    for &b in &s {
        if matches!(
            b,
            b'.' | b'\\' | b'+' | b'*' | b'?' | b'[' | b'^' | b']' | b'$' | b'(' | b')'
        ) {
            out.push(b'\\');
        }
        out.push(b);
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// strip_tags
// ---------------------------------------------------------------------------

fn fn_strip_tags(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let allow = match args.get(1).map(deref) {
        None | Some(Value::Null) => None,
        // Each element becomes `<tag>`; an empty array allows nothing.
        Some(Value::Array(a)) => {
            let mut set = Vec::new();
            for (_, v) in a.iter() {
                set.push(b'<');
                set.extend_from_slice(&crate::zend::zpp::zval_get_string(host, v)?);
                set.push(b'>');
            }
            (!set.is_empty()).then_some(set)
        }
        Some(v) => Some(conv::to_string_bytes(&v)),
    };
    Ok(Value::str(strip_tags(
        &conv::to_string_bytes(&args[0]),
        allow.as_deref(),
    )))
}

/// php_tag_find: is the tag in `tag` (`<a href=..>`, `</a>`), normalized to
/// `<a>`, in the allowed `set`?
fn tag_find(tag: &[u8], set: &[u8]) -> bool {
    if tag.is_empty() {
        return false;
    }
    let at = |i: usize| tag.get(i).copied().unwrap_or(0);
    let mut norm = Vec::with_capacity(tag.len() + 1);
    let mut seen = false;
    let mut t = 0;
    while t < tag.len() {
        let c = at(t).to_ascii_lowercase();
        match c {
            b'<' => norm.push(c),
            b'>' => break,
            _ if !c_isspace(c) => {
                seen = true;
                if c != b'/' || (t > 0 && at(t - 1) != b'<' && at(t + 1) != b'>') {
                    norm.push(c);
                }
            }
            _ if seen => break,
            _ => {}
        }
        t += 1;
    }
    norm.push(b'>');
    // strstr: the set ends at its first NUL.
    let set = &set[..set.iter().position(|&b| b == 0).unwrap_or(set.len())];
    set.windows(norm.len()).any(|w| w == norm)
}

/// php_strip_tags_ex: the state machine over text (0), an HTML tag (1),
/// PHP code (2), `<!...>` (3) and an HTML comment (4). With `allow`, a tag
/// in that set is kept.
fn strip_tags(s: &[u8], allow: Option<&[u8]>) -> Vec<u8> {
    let allow = allow.map(|a| a.to_ascii_lowercase());
    // The C reads one byte past the end, a NUL.
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut out = Vec::with_capacity(s.len());
    let mut tbuf: Vec<u8> = Vec::new();
    let (mut lc, mut br, mut depth, mut in_q) = (0u8, 0i32, 0i32, 0u8);
    let mut is_xml = false;
    let mut state = 0u8;
    let toggle = |in_q: u8, c: u8| if in_q != 0 { 0 } else { c };
    for p in 0..s.len() {
        let c = s[p];
        let prev = |n: usize| if p >= n { s[p - n] } else { 0 };
        // A byte of the tag being read in state 1.
        let mut reg_char = false;
        match state {
            0 => match c {
                0 => {}
                b'<' if in_q != 0 => {}
                b'<' if c_isspace(at(p + 1)) => out.push(c),
                b'<' => {
                    lc = b'<';
                    state = 1;
                    if allow.is_some() {
                        tbuf.push(b'<');
                    }
                }
                b'>' if depth > 0 => depth -= 1,
                b'>' if in_q != 0 => {}
                _ => out.push(c),
            },
            1 => match c {
                0 => {}
                b'<' if in_q != 0 => {}
                b'<' if c_isspace(at(p + 1)) => reg_char = true,
                b'<' => depth += 1,
                b'>' if depth > 0 => depth -= 1,
                b'>' if in_q != 0 => {}
                b'>' => {
                    lc = b'>';
                    if !(is_xml && prev(1) == b'-') {
                        in_q = 0;
                        state = 0;
                        is_xml = false;
                        if let Some(set) = &allow {
                            tbuf.push(b'>');
                            if tag_find(&tbuf, set) {
                                out.extend_from_slice(&tbuf);
                            }
                            tbuf.clear();
                        }
                    }
                }
                b'"' | b'\'' => {
                    if p != 0 && (in_q == 0 || c == in_q) {
                        in_q = toggle(in_q, c);
                    }
                    reg_char = true;
                }
                b'!' if prev(1) == b'<' => {
                    state = 3;
                    lc = c;
                }
                b'?' if prev(1) == b'<' => {
                    br = 0;
                    state = 2;
                }
                _ => reg_char = true,
            },
            2 => match c {
                b'(' if lc != b'"' && lc != b'\'' => {
                    lc = b'(';
                    br += 1;
                }
                b')' if lc != b'"' && lc != b'\'' => {
                    lc = b')';
                    br -= 1;
                }
                b'>' if depth > 0 => depth -= 1,
                b'>' if in_q != 0 => {}
                b'>' => {
                    if br == 0 && p >= 1 && lc != b'"' && prev(1) == b'?' {
                        in_q = 0;
                        state = 0;
                        tbuf.clear();
                    }
                }
                b'"' | b'\'' if p >= 1 && prev(1) != b'\\' => {
                    if lc == c {
                        lc = 0;
                    } else if lc != b'\\' {
                        lc = c;
                    }
                    if in_q == 0 || c == in_q {
                        in_q = toggle(in_q, c);
                    }
                }
                // `<?xml` is not PHP: back to a tag.
                b'l' | b'L'
                    if p > 4
                        && prev(1).eq_ignore_ascii_case(&b'm')
                        && prev(2).eq_ignore_ascii_case(&b'x')
                        && prev(3) == b'?'
                        && prev(4) == b'<' =>
                {
                    state = 1;
                    is_xml = true;
                }
                _ => {}
            },
            3 => match c {
                b'>' if depth > 0 => depth -= 1,
                b'>' if in_q != 0 => {}
                b'>' => {
                    in_q = 0;
                    state = 0;
                    tbuf.clear();
                }
                b'"' | b'\'' if p != 0 && prev(1) != b'\\' && (in_q == 0 || c == in_q) => {
                    in_q = toggle(in_q, c);
                }
                b'-' if p >= 2 && prev(1) == b'-' && prev(2) == b'!' => state = 4,
                // `<!DOCTYPE` is a tag.
                b'E' | b'e'
                    if p > 6 && (1..=6).all(|n| prev(n).eq_ignore_ascii_case(&b"epytcod"[n])) =>
                {
                    state = 1
                }
                _ => {}
            },
            _ => {
                if c == b'>' && in_q == 0 && p >= 2 && prev(1) == b'-' && prev(2) == b'-' {
                    in_q = 0;
                    state = 0;
                    tbuf.clear();
                }
            }
        }
        if reg_char && allow.is_some() {
            tbuf.push(c);
        }
    }
    out
}

fn first_byte(v: Option<&Value>, default: u8) -> u8 {
    match v {
        None => default,
        Some(v) => conv::to_string_bytes(&deref(v))
            .first()
            .copied()
            .unwrap_or(default),
    }
}

// ---------------------------------------------------------------------------
// str_getcsv
// ---------------------------------------------------------------------------

/// `str_getcsv($string, $separator = ",", $enclosure = '"', $escape = "\\")`.
/// Parses a single CSV record into an array of fields. An empty input yields
/// `[null]`, matching php.
fn fn_str_getcsv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if args.len() < 4 {
        crate::deprecated(
            host,
            "str_getcsv",
            "the $escape parameter must be provided as its default value will change",
        )?;
    }
    let s = conv::to_string_bytes(&deref(&args[0]));
    let sep = first_byte(args.get(1), b',');
    let enc = first_byte(args.get(2), b'"');
    // The escape may be disabled by passing "" in php; default is backslash.
    let esc: Option<u8> = match args.get(3) {
        Some(v) => conv::to_string_bytes(&deref(v)).first().copied(),
        None => Some(b'\\'),
    };

    Ok(Value::Array(parse_csv(&s, sep, enc, esc)))
}

/// One CSV record into its fields (`str_getcsv` / `fgetcsv`); an empty
/// record is `[null]`.
pub(crate) fn parse_csv(s: &[u8], sep: u8, enc: u8, esc: Option<u8>) -> PhpArray {
    let mut out = PhpArray::new();
    if s.is_empty() {
        out.push(Value::Null);
        return out;
    }

    let bytes = s;
    let mut i = 0usize;
    loop {
        let mut field = Vec::new();
        if bytes.get(i) == Some(&enc) {
            // Quoted field.
            i += 1;
            loop {
                match bytes.get(i) {
                    None => break,
                    Some(&c) if Some(c) == esc && i + 1 < bytes.len() => {
                        // php keeps BOTH the escape char and the escaped char.
                        field.push(c);
                        field.push(bytes[i + 1]);
                        i += 2;
                    }
                    Some(&c) if c == enc => {
                        if bytes.get(i + 1) == Some(&enc) {
                            // Doubled enclosure -> one literal enclosure.
                            field.push(enc);
                            i += 2;
                        } else {
                            i += 1; // closing enclosure
                            break;
                        }
                    }
                    Some(&c) => {
                        field.push(c);
                        i += 1;
                    }
                }
            }
            // Trailing content after the closing quote, up to the separator, is
            // appended to the field (php behaviour).
            while let Some(&c) = bytes.get(i) {
                if c == sep {
                    break;
                }
                field.push(c);
                i += 1;
            }
        } else {
            // Unquoted field: read until the separator.
            while let Some(&c) = bytes.get(i) {
                if c == sep {
                    break;
                }
                field.push(c);
                i += 1;
            }
        }
        out.push(Value::str(field));
        match bytes.get(i) {
            None => break,
            Some(&c) if c == sep => {
                i += 1;
                if i == bytes.len() {
                    // Trailing separator -> one final empty field.
                    out.push(Value::str(Vec::new()));
                    break;
                }
            }
            _ => break,
        }
    }
    out
}

fn fn_str_repeat(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let n = conv::to_int(&args[1]);
    if n < 0 {
        return Err(Thrown::error(
            "ValueError",
            "str_repeat(): Argument #2 ($times) must be greater than or equal to 0",
        ));
    }
    let n = n as usize;
    // zend_string_safe_falloc(len, mult, 0): php refuses past memory_limit.
    if !s.is_empty()
        && n != 0
        && !rhp_values::mem::reserve_safe(s.len(), n, rhp_values::mem::zstr_size(0))
    {
        return Ok(Value::Null);
    }
    Ok(Value::str(s.repeat(n)))
}

fn fn_implode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // implode($array) or implode($separator, $array)
    let (glue, arr) = match (deref(&args[0]), args.get(1).map(deref)) {
        (Value::Array(a), None | Some(Value::Null)) => (Vec::new(), a),
        (Value::Array(_), Some(_)) => {
            return Err(Thrown::type_error(
                "implode(): Argument #1 ($separator) must be of type string, array given",
            ))
        }
        (g, Some(Value::Array(a))) => (conv::to_string_bytes(&g), a),
        _ => {
            return Err(Thrown::type_error(
                "implode(): If argument #1 ($separator) is of type string, argument #2 ($array) must be of type array, null given",
            ))
        }
    };
    let parts: Vec<std::borrow::Cow<[u8]>> = arr
        .iter()
        .map(|(_, v)| match v {
            Value::Str(s) => std::borrow::Cow::Borrowed(s.as_bytes()),
            v => std::borrow::Cow::Owned(conv::to_string_bytes(v)),
        })
        .collect();
    let len: usize = parts.iter().map(|p| p.len()).sum();
    // zend_string_safe_falloc(n - 1, glue_len, len): the glue repeated per
    // element can dwarf the input, so php checks the limit first.
    if parts.len() > 1
        && !rhp_values::mem::reserve_safe(
            parts.len() - 1,
            glue.len(),
            rhp_values::mem::zstr_size(len),
        )
    {
        return Ok(Value::Null);
    }
    let mut out = Vec::with_capacity(len + glue.len() * parts.len().saturating_sub(1));
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(&glue);
        }
        out.extend_from_slice(p);
    }
    Ok(Value::str(out))
}

fn fn_explode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let delim = conv::to_string_bytes(&deref(&args[0]));
    let s = conv::to_string_bytes(&deref(&args[1]));
    if delim.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "explode(): Argument #1 ($separator) must not be empty",
        ));
    }
    // All pieces, in order.
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut start = 0;
    while let Some(pos) = find_subslice(&s[start..], &delim) {
        parts.push(s[start..start + pos].to_vec());
        start += pos + delim.len();
    }
    parts.push(s[start..].to_vec());

    let limit = args.get(2).map(|v| conv::to_int(&deref(v)));
    let n = parts.len();
    let pieces: Vec<Vec<u8>> = match limit {
        None => parts,
        // Positive: at most `limit` pieces; the last keeps the remainder.
        Some(l) if l > 0 => {
            let l = l as usize;
            if l >= n {
                parts
            } else {
                let mut out: Vec<Vec<u8>> = parts[..l - 1].to_vec();
                out.push(parts[l - 1..].join(&delim[..]));
                out
            }
        }
        // Zero is treated as 1: the whole string as one piece.
        Some(0) => vec![s.clone()],
        // Negative: drop the last `|limit|` pieces.
        Some(l) => {
            let drop = (-l) as usize;
            if drop >= n {
                Vec::new()
            } else {
                parts[..n - drop].to_vec()
            }
        }
    };
    let mut arr = PhpArray::new();
    for p in pieces {
        arr.push(Value::str(p));
    }
    Ok(Value::Array(arr))
}

pub(crate) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

fn fn_strtoupper(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut b = conv::to_string_bytes(&deref(&args[0]));
    b.make_ascii_uppercase();
    Ok(Value::str(b))
}

fn fn_strtolower(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut b = conv::to_string_bytes(&deref(&args[0]));
    b.make_ascii_lowercase();
    Ok(Value::str(b))
}

fn fn_str_contains(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let haystack = conv::to_string_bytes(&deref(&args[0]));
    let needle = conv::to_string_bytes(&deref(&args[1]));
    Ok(Value::Bool(
        needle.is_empty() || find_subslice(&haystack, &needle).is_some(),
    ))
}

fn fn_ord(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let b = conv::to_string_bytes(&deref(&args[0]));
    match b.len() {
        1 => {}
        0 => deprecated(host, "ord", "Providing an empty string is deprecated")?,
        _ => deprecated(
            host,
            "ord",
            "Providing a string that is not one byte long is deprecated. Use ord($str[0]) instead",
        )?,
    }
    Ok(Value::Int(b.first().copied().unwrap_or(0) as i64))
}

fn fn_chr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let c = conv::to_int(&deref(&args[0]));
    if !(0..=255).contains(&c) {
        deprecated(
            host,
            "chr",
            "Providing a value not in-between 0 and 255 is deprecated, this is because a byte \
             value must be in the [0, 255] interval. The value used will be constrained using % 256",
        )?;
    }
    Ok(Value::str(vec![(c & 0xff) as u8]))
}

fn check_alnum(s: &[u8], func: &str) -> Result<(), Thrown> {
    if s.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): Argument #1 ($string) must not be empty"),
        ));
    }
    if !s.iter().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): Argument #1 ($string) must be composed only of alphanumeric ASCII characters"),
        ));
    }
    Ok(())
}

fn fn_str_increment(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "str_increment", "string")?;
    check_alnum(&s, "str_increment")?;
    let mut v = s.clone();
    let mut pos = v.len();
    let mut carry = true;
    while carry && pos > 0 {
        pos -= 1;
        match v[pos] {
            b'z' | b'Z' => {
                v[pos] -= 25;
            }
            b'9' => v[pos] = b'0',
            _ => {
                v[pos] += 1;
                carry = false;
            }
        }
    }
    if carry {
        let first = if v[0] == b'0' { b'1' } else { v[0] };
        v.insert(0, first);
    }
    Ok(Value::str(v))
}

fn out_of_range(s: &[u8]) -> Thrown {
    Thrown::error(
        "ValueError",
        format!(
            "str_decrement(): Argument #1 ($string) \"{}\" is out of decrement range",
            String::from_utf8_lossy(s)
        ),
    )
}

fn fn_str_decrement(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "str_decrement", "string")?;
    check_alnum(&s, "str_decrement")?;
    if s[0] == b'0' {
        return Err(out_of_range(&s));
    }
    let mut v = s.clone();
    let mut pos = v.len();
    let mut carry = true;
    while carry && pos > 0 {
        pos -= 1;
        match v[pos] {
            b'a' | b'A' => v[pos] += 25,
            b'0' => v[pos] = b'9',
            _ => {
                v[pos] -= 1;
                carry = false;
            }
        }
    }
    if carry || (v[0] == b'0' && v.len() > 1) {
        if v.len() == 1 {
            return Err(out_of_range(&s));
        }
        v.remove(0);
    }
    Ok(Value::str(v))
}

fn fn_str_shuffle(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = str_param(host, args, 0, "str_shuffle", "string")?;
    let n = s.len();
    if n > 1 {
        let mut left = n - 1;
        while left > 0 {
            let j = mt_range(0, left as i64) as usize;
            s.swap(left, j);
            left -= 1;
        }
    }
    Ok(Value::str(s))
}

/// `utf8_encode` / `utf8_decode`.
pub const PRELUDE_PHP: &str = include_str!("string/prelude.php");

/// `__stdext_deprecated($message)`: E_DEPRECATED from a prelude function.
fn fn_stdext_deprecated(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = conv::to_string_bytes(&deref(&args[0]));
    host.emit_diagnostic(crate::E_DEPRECATED, &String::from_utf8_lossy(&msg))?;
    Ok(Value::Null)
}

/// `__stdext_warning($message)`: E_WARNING from a prelude function.
fn fn_stdext_warning(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = conv::to_string_bytes(&deref(&args[0]));
    host.emit_diagnostic(crate::E_WARNING, &String::from_utf8_lossy(&msg))?;
    Ok(Value::Null)
}

fn fn_setlocale(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // We don't model locales; report the C locale.
    Ok(Value::str("C"))
}

fn fn_parse_str(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "parse_str", "string")?;
    // C string: processing stops at the first NUL byte.
    let s = match s.iter().position(|&b| b == 0) {
        Some(p) => s[..p].to_vec(),
        None => s,
    };
    let mut out = PhpArray::new();
    let mut count = 0usize;
    for var in s.split(|&b| b == b'&').filter(|t| !t.is_empty()) {
        count += 1;
        if count > 1000 {
            // php: Warning "Input variables exceeded 1000 ..."
            break;
        }
        let (name, val) = match var.iter().position(|&b| b == b'=') {
            Some(p) => (&var[..p], url_decode(&var[p + 1..])),
            None => (var, Vec::new()),
        };
        let name = url_decode(name);
        register_variable(&mut out, &name, val);
    }
    if let Some(Value::Ref(r)) = args.get(1) {
        *r.borrow_mut() = Value::Array(out);
    }
    Ok(Value::Null)
}

/// `basename(path, suffix="")` — the trailing name component, byte-wise, matching
/// php (trailing slashes ignored; an exact suffix stripped unless it's the whole).
fn fn_basename(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_arg(args, 0);
    let suffix = args
        .get(1)
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_default();
    // drop trailing '/'
    let end = path
        .iter()
        .rposition(|&b| b != b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let trimmed = &path[..end];
    let start = trimmed
        .iter()
        .rposition(|&b| b == b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut base = &trimmed[start..];
    if !suffix.is_empty() && base.len() > suffix.len() && base.ends_with(&suffix[..]) {
        base = &base[..base.len() - suffix.len()];
    }
    Ok(Value::str(base.to_vec()))
}

/// `pathinfo(path, flags=PATHINFO_ALL)` — splits a path into dirname/basename/
/// extension/filename. Returns an assoc array, or a single string when exactly
/// one flag bit is requested.
fn fn_pathinfo(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const DIRNAME: i64 = 1;
    const BASENAME: i64 = 2;
    const EXTENSION: i64 = 4;
    const FILENAME: i64 = 8;
    const ALL: i64 = DIRNAME | BASENAME | EXTENSION | FILENAME;

    let path = path_arg(args, 0);
    let flags = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(ALL);

    // basename (php keeps trailing-slash handling consistent with basename()).
    let end = path
        .iter()
        .rposition(|&b| b != b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let trimmed = &path[..end];
    let base_start = trimmed
        .iter()
        .rposition(|&b| b == b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let basename = &trimmed[base_start..];
    // extension / filename: split basename on the LAST '.', but a leading dot
    // (dotfile with no other dot) has no extension.
    let (filename, extension): (&[u8], Option<&[u8]>) =
        match basename.iter().rposition(|&b| b == b'.') {
            None => (basename, None),
            Some(dot) => (&basename[..dot], Some(&basename[dot + 1..])),
        };

    let dir = dirname_once(&path);
    // php: dirname is "." for a bare name only when the input had no slash.
    let dirname: Vec<u8> = if path.contains(&b'/') {
        dir
    } else {
        b".".to_vec()
    };

    // Single-component request -> return a string.
    if flags != ALL && (flags & (flags - 1)) == 0 {
        let s: Vec<u8> = match flags {
            DIRNAME => dirname,
            BASENAME => basename.to_vec(),
            EXTENSION => extension.unwrap_or(b"").to_vec(),
            FILENAME => filename.to_vec(),
            _ => Vec::new(),
        };
        return Ok(Value::str(s));
    }

    use rhp_values::{ArrayKey as K, PhpArray};
    let mut arr = PhpArray::new();
    if flags & DIRNAME != 0 {
        arr.insert(K::from_bytes(b"dirname"), Value::str(dirname));
    }
    if flags & BASENAME != 0 {
        arr.insert(K::from_bytes(b"basename"), Value::str(basename.to_vec()));
    }
    if flags & EXTENSION != 0 {
        if let Some(ext) = extension {
            arr.insert(K::from_bytes(b"extension"), Value::str(ext.to_vec()));
        }
    }
    if flags & FILENAME != 0 {
        arr.insert(K::from_bytes(b"filename"), Value::str(filename.to_vec()));
    }
    Ok(Value::Array(arr))
}

/// `dirname(path, levels=1)` — matching php: "." when no slash, "/" at root.
fn fn_dirname(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut path = path_arg(args, 0);
    let levels = args
        .get(1)
        .map(|v| conv::to_int(&deref(v)))
        .unwrap_or(1)
        .max(1);
    for _ in 0..levels {
        path = dirname_once(&path);
    }
    Ok(Value::str(path))
}

fn dirname_once(path: &[u8]) -> Vec<u8> {
    // drop trailing slashes
    let end = path
        .iter()
        .rposition(|&b| b != b'/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let trimmed = &path[..end];
    match trimmed.iter().rposition(|&b| b == b'/') {
        None => b".".to_vec(),
        Some(0) => b"/".to_vec(),
        Some(i) => trimmed[..i].to_vec(),
    }
}

/// Register all string builtins into the shared table.
// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("substr", fn_substr),
            ("str_replace", fn_str_replace),
            ("strpos", fn_strpos),
            ("stripos", fn_stripos),
            ("strrpos", fn_strrpos),
            ("strripos", fn_strripos),
            ("str_pad", fn_str_pad),
            ("trim", fn_trim),
            ("ltrim", fn_ltrim),
            ("rtrim", fn_rtrim),
            ("chop", fn_rtrim),
            ("ucfirst", fn_ucfirst),
            ("lcfirst", fn_lcfirst),
            ("ucwords", fn_ucwords),
            ("strrev", fn_strrev),
            ("substr_count", fn_substr_count),
            ("str_starts_with", fn_str_starts_with),
            ("str_ends_with", fn_str_ends_with),
            ("wordwrap", fn_wordwrap),
            ("addcslashes", fn_addcslashes),
            ("stripcslashes", fn_stripcslashes),
            ("count_chars", fn_count_chars),
            ("substr_compare", fn_substr_compare),
            ("strnatcmp", fn_strnatcmp),
            ("strnatcasecmp", fn_strnatcasecmp),
            ("str_ireplace", fn_str_ireplace),
            ("strpbrk", fn_strpbrk),
            ("strspn", fn_strspn),
            ("strcspn", fn_strcspn),
            ("str_rot13", fn_str_rot13),
            ("strstr", fn_strstr),
            ("strchr", fn_strchr),
            ("stristr", fn_stristr),
            ("strrchr", fn_strrchr),
            ("strtok", fn_strtok),
            ("str_split", fn_str_split),
            ("strtr", fn_strtr),
            ("substr_replace", fn_substr_replace),
            ("similar_text", fn_similar_text),
            ("str_word_count", fn_str_word_count),
            ("chunk_split", fn_chunk_split),
            ("nl2br", fn_nl2br),
            ("addslashes", fn_addslashes),
            ("stripslashes", fn_stripslashes),
            ("quotemeta", fn_quotemeta),
            ("strip_tags", fn_strip_tags),
            ("str_getcsv", fn_str_getcsv),
            ("str_repeat", fn_str_repeat),
            ("implode", fn_implode),
            ("join", fn_implode),
            ("explode", fn_explode),
            ("strtoupper", fn_strtoupper),
            ("strtolower", fn_strtolower),
            ("str_contains", fn_str_contains),
            ("ord", fn_ord),
            ("chr", fn_chr),
            ("str_increment", fn_str_increment),
            ("str_decrement", fn_str_decrement),
            ("str_shuffle", fn_str_shuffle),
            ("setlocale", fn_setlocale),
            ("parse_str", fn_parse_str),
            ("basename", fn_basename),
            ("dirname", fn_dirname),
            ("pathinfo", fn_pathinfo),
        ],
    );
    crate::register_helpers(
        m,
        &[
            ("__stdext_deprecated", fn_stdext_deprecated, 1, Some(1)),
            ("__stdext_warning", fn_stdext_warning, 1, Some(1)),
        ],
    );
}

#[cfg(test)]
mod tests;
