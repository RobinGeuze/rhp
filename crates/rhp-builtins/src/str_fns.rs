//! Native string builtins (pure, non-callback, non-by-reference).
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see `str_fns/tests.rs` — expected values are hardcoded from
//! `php -r '...'` output).

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::zend::zpp::zval_get_string;
use crate::{Host, Thrown};

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
// soundex
// ---------------------------------------------------------------------------

/// `soundex($string)`: the four-character American Soundex code (a faithful port
/// of php's `php_soundex`). Non-letters are skipped; the empty / letterless input
/// yields `"0000"`.
fn fn_soundex(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // soundex_table[c-'A'] for A..Z; '0' means "no code" (vowel / h / w / y).
    const TABLE: &[u8; 26] = b"01230120022455012623010202";
    let s = conv::to_string_bytes(&args[0]);
    let mut out = Vec::with_capacity(4);
    let mut last = 0u8;
    for &b in &s {
        if out.len() >= 4 {
            break;
        }
        let up = b.to_ascii_uppercase();
        if !up.is_ascii_uppercase() {
            continue;
        }
        let code = TABLE[(up - b'A') as usize];
        if out.is_empty() {
            out.push(up);
            last = code;
        } else if code != last {
            if code != b'0' {
                out.push(code);
            }
            last = code;
        }
    }
    while out.len() < 4 {
        out.push(b'0');
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

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register all string builtins into the shared table.
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
            ("soundex", fn_soundex),
            ("addcslashes", fn_addcslashes),
            ("stripcslashes", fn_stripcslashes),
            ("count_chars", fn_count_chars),
        ],
    );
}

#[cfg(test)]
mod tests;
