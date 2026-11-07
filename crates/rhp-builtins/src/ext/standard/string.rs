//! ext/standard/string.c: the string functions.
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see `string/tests.rs` — expected values are hardcoded from
//! `php -r '...'` output).

use std::cell::RefCell;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, PhpStr, Value};

use crate::ext::random::mt_range;
use crate::ext::standard::file::path_arg;
use crate::zend::builtin_functions::{binary_strncasecmp, binary_strncmp};
use crate::zend::zpp::{bool_param, str_or_null_param, str_param, zstr_param, zval_get_string};
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

/// php's `php_charmask`: the bytes of `chars` with `a..z` ranges; a bad
/// range warns and its dots are taken one by one.
pub(crate) fn charmask(host: &mut dyn Host, chars: &[u8]) -> Result<[bool; 256], Thrown> {
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
            continue;
        }
        if i + 1 < len && c == b'.' && chars[i + 1] == b'.' {
            let msg: &[u8] = if i == 0 {
                b"Invalid '..'-range, no character to the left of '..'"
            } else if i + 2 >= len {
                b"Invalid '..'-range, no character to the right of '..'"
            } else if chars[i - 1] > chars[i + 2] {
                b"Invalid '..'-range, '..'-range needs to be incrementing"
            } else {
                b"Invalid '..'-range"
            };
            host.docref_diagnostic(crate::E_WARNING, msg)?;
        } else {
            mask[c as usize] = true;
        }
        i += 1;
    }
    Ok(mask)
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
    Ok(Value::str(&s[start..start + take]))
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
    let mut reps = match replace {
        Value::Array(reps) => Some(reps.values()),
        _ => None,
    };
    for sv in searches.values() {
        let sbytes = zval_get_string(host, sv)?;
        let rbytes = match (replace, &mut reps) {
            (_, Some(reps)) => match reps.next() {
                Some(v) => zval_get_string(host, v)?,
                None => Vec::new(),
            },
            (other, None) => conv::to_string_bytes(other),
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
        let res = replace_in_subject(host, s, &search, &replace, replace_fn, &mut n)?;
        match subject {
            // Nothing replaced: php returns the subject string itself.
            Value::Str(orig) if n == 0 => Value::Str(orig),
            _ => Value::str(res),
        }
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
    // The arguments' own bytes unless they need converting or folding.
    fn bytes(v: &Value, ci: bool) -> std::borrow::Cow<'_, [u8]> {
        match v {
            Value::Str(s) if !ci => std::borrow::Cow::Borrowed(s.as_bytes()),
            v => std::borrow::Cow::Owned(conv::to_string_bytes(v)),
        }
    }
    let mut haystack = bytes(&args[0], ci);
    let mut needle = bytes(&args[1], ci);
    let off = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let start = resolve_fwd_offset(off, haystack.len(), fname)?;
    if ci {
        haystack.to_mut().make_ascii_lowercase();
        needle.to_mut().make_ascii_lowercase();
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

/// `php_trim_int`: one byte is matched directly, more go through
/// [`charmask`].
fn trim_impl(
    host: &mut dyn Host,
    args: &[Value],
    left: bool,
    right: bool,
) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let chars = match args.get(1) {
        Some(v) => conv::to_string_bytes(v),
        None => TRIM_DEFAULT.to_vec(),
    };
    let mask = if chars.len() == 1 {
        let mut m = [false; 256];
        m[chars[0] as usize] = true;
        m
    } else {
        charmask(host, &chars)?
    };
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
    Ok(Value::str(&s[start..end]))
}

fn fn_trim(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    trim_impl(host, args, true, true)
}
fn fn_ltrim(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    trim_impl(host, args, true, false)
}
fn fn_rtrim(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    trim_impl(host, args, false, true)
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

fn fn_ucwords(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = conv::to_string_bytes(&args[0]);
    let delims = match args.get(1) {
        Some(v) => conv::to_string_bytes(v),
        None => b" \t\r\n\x0c\x0b".to_vec(),
    };
    let mask = charmask(host, &delims)?;
    let mut cap_next = true;
    for b in s.iter_mut() {
        if cap_next {
            b.make_ascii_uppercase();
        }
        cap_next = mask[*b as usize];
    }
    Ok(Value::str(s))
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
    let mut off = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if off < 0 {
        off += haystack.len() as i64;
    }
    if off < 0 || off > haystack.len() as i64 {
        return Err(Thrown::error(
            "ValueError",
            "substr_count(): Argument #3 ($offset) must be contained in argument #1 ($haystack)",
        ));
    }
    let start = off as usize;
    let rest = (haystack.len() - start) as i64;
    let end = match args.get(3).map(deref) {
        None | Some(Value::Null) => haystack.len(),
        Some(v) => {
            let mut l = conv::to_int(&v);
            if l < 0 {
                l += rest;
            }
            if l < 0 || l > rest {
                return Err(Thrown::error(
                    "ValueError",
                    "substr_count(): Argument #4 ($length) must be contained in argument #1 ($haystack)",
                ));
            }
            start + l as usize
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
            "wordwrap(): Argument #4 ($cut_long_words) cannot be true when argument #2 ($width) is 0",
        ));
    }

    let bytes = &text[..];
    let n = bytes.len() as i64;
    let w = width;
    // A multi-byte break or cut: php allocates the worst case up front
    // (zend_string_safe_falloc), refused past memory_limit.
    if brk.len() != 1 || cut {
        let ok = if w > 0 {
            let chk = bytes.len() / w as usize + 1;
            rhp_values::mem::reserve_safe(chk, brk.len(), rhp_values::mem::zstr_size(bytes.len()))
        } else {
            rhp_values::mem::reserve_safe(bytes.len(), brk.len() + 1, rhp_values::mem::zstr_size(0))
        };
        if !ok {
            return Ok(Value::Null);
        }
    }
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
fn fn_addcslashes(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let mask = charmask(host, &conv::to_string_bytes(&args[1]))?;
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
    if !len_default && len_val <= 0 {
        if len_val == 0 {
            return Ok(Value::Int(0));
        }
        return Err(Thrown::error(
            "ValueError",
            "substr_compare(): Argument #4 ($length) must be greater than or equal to 0",
        ));
    }
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
    let cmp_len = if len_val != 0 {
        len_val as usize
    } else {
        needle.len().max(main.len() - off)
    };
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
// strnatcmp / strnatcasecmp: a Rust translation, modified for rhp, of php-src's
// ext/standard/strnatcmp.c, Copyright (C) 2000 by Martin Pool (zlib licence;
// see THIRD-PARTY-NOTICES, section 23).
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
        Some(i) => Ok(Value::str(&s[i..])),
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
        Some(p) if before => Value::str(&h[..p]),
        Some(p) => Value::str(&h[p..]),
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
    reset_locale();
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
    let started = STRTOK.with(|st| {
        let mut st = st.borrow_mut();
        if tok.is_some() {
            st.string = Some(s.clone());
            st.last = 0;
        }
        st.string.is_some()
    });
    if !started {
        host.docref_diagnostic(
            crate::E_WARNING,
            b"Both arguments must be provided when starting tokenization",
        )?;
        return Ok(Value::Bool(false));
    }
    let tok = tok.unwrap_or(s);
    STRTOK.with(|st| {
        let mut st = st.borrow_mut();
        let Some(string) = st.string.as_ref() else {
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
        out.push(Value::str(chunk));
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// strtr
// ---------------------------------------------------------------------------

fn fn_strtr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    if args.len() == 2 {
        let Value::Array(pairs) = deref(&args[1]) else {
            return Err(Thrown::type_error(format!(
                "strtr(): Argument #2 ($from) must be of type array, {} given",
                crate::zend::zpp::zval_value_name(host, &args[1])
            )));
        };
        if s.is_empty() {
            return Ok(Value::str(s));
        }
        strtr_array(host, s, &pairs)
    } else {
        if s.is_empty() {
            return Ok(Value::str(s));
        }
        // php_strtr: for a byte listed twice in $from the last mapping wins.
        let from = conv::to_string_bytes(&args[1]);
        let to = conv::to_string_bytes(&args[2]);
        let n = from.len().min(to.len());
        let mut map: [u8; 256] = std::array::from_fn(|i| i as u8);
        for i in 0..n {
            map[from[i] as usize] = to[i];
        }
        let out: Vec<u8> = s.iter().map(|&b| map[b as usize]).collect();
        Ok(Value::str(out))
    }
}

/// `php_strtr_array`: the longest key matching at each position is
/// replaced; replacements are converted when used.
fn strtr_array(host: &mut dyn Host, s: Vec<u8>, pairs: &PhpArray) -> Result<Value, Thrown> {
    const EMPTY_KEY: &[u8] = b"Ignoring replacement of empty string";
    if pairs.is_empty() {
        return Ok(Value::str(s));
    }
    if pairs.len() == 1 {
        let (k, v) = pairs.iter().next().expect("one pair");
        let key = key_bytes(&k);
        let rep = zval_get_string(host, v)?;
        if key.is_empty() {
            host.docref_diagnostic(crate::E_WARNING, EMPTY_KEY)?;
            return Ok(Value::str(s));
        }
        let mut n = 0;
        return Ok(Value::str(replace_all(&s, &key, &rep, &mut n)));
    }
    let slen = s.len();
    let mut table: std::collections::HashMap<Vec<u8>, Value> =
        std::collections::HashMap::with_capacity(pairs.len());
    let mut first = [false; 256];
    let (mut minlen, mut maxlen) = (usize::MAX, 0usize);
    for (k, v) in pairs.iter() {
        let key = key_bytes(&k);
        if key.is_empty() {
            if matches!(k, ArrayKey::Str(_)) {
                host.docref_diagnostic(crate::E_WARNING, EMPTY_KEY)?;
            }
            continue;
        }
        if key.len() > slen {
            continue;
        }
        minlen = minlen.min(key.len());
        maxlen = maxlen.max(key.len());
        first[key[0] as usize] = true;
        table.entry(key).or_insert_with(|| v.clone());
    }
    if minlen > maxlen {
        return Ok(Value::str(s));
    }
    let mut out = Vec::with_capacity(slen);
    let (mut pos, mut old_pos) = (0usize, 0usize);
    while pos + minlen <= slen {
        if first[s[pos] as usize] {
            let mut len = maxlen.min(slen - pos);
            while len >= minlen {
                if let Some(v) = table.get(&s[pos..pos + len]) {
                    let v = v.clone();
                    let rep = zval_get_string(host, &v)?;
                    out.extend_from_slice(&s[old_pos..pos]);
                    out.extend_from_slice(&rep);
                    old_pos = pos + len;
                    pos = old_pos - 1;
                    break;
                }
                len -= 1;
            }
        }
        pos += 1;
    }
    out.extend_from_slice(&s[old_pos..]);
    Ok(Value::str(out))
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

fn fn_substr_replace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let subject = deref(&args[0]);
    let replace = deref(&args[1]);
    let start_arg = deref(&args[2]);
    let length_arg = args.get(3).map(deref);

    if let Value::Array(subj) = subject {
        let mut out = PhpArray::new();
        let values = |v: &Value| match v {
            Value::Array(a) => Some(a.clone()),
            _ => None,
        };
        let (ra, sa) = (values(&replace), values(&start_arg));
        let la = length_arg.as_ref().and_then(values);
        let mut ri = ra.as_ref().map(|a| a.values());
        let mut si = sa.as_ref().map(|a| a.values());
        let mut li = la.as_ref().map(|a| a.values());
        for (k, v) in subj.iter() {
            let sbytes = zval_get_string(host, v)?;
            // replace: array -> next element (else ""); scalar -> whole, for all.
            let rep = match &mut ri {
                Some(it) => match it.next() {
                    Some(rv) => zval_get_string(host, rv)?,
                    None => Vec::new(),
                },
                None => conv::to_string_bytes(&replace),
            };
            // start: array -> next element (else 0); scalar -> per-subject.
            let st = match &mut si {
                Some(it) => match it.next() {
                    Some(sv) => crate::zend::zpp::zval_get_long(host, sv)?,
                    None => 0,
                },
                None => conv::to_int(&start_arg),
            };
            // length: omitted -> to end; array -> next element (else full
            // string); scalar -> per-subject.
            let ln = match (&length_arg, &mut li) {
                (None, _) => None,
                (_, Some(it)) => Some(match it.next() {
                    Some(lv) => crate::zend::zpp::zval_get_long(host, lv)?,
                    None => sbytes.len() as i64,
                }),
                (Some(other), None) => Some(conv::to_int(other)),
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
            Value::Array(ra) => match ra.iter().next() {
                Some((_, rv)) => zval_get_string(host, rv)?,
                None => Vec::new(),
            },
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

/// `str_word_count`: words of letters, `'` and `-` (plus `$characters`);
/// a leading `'` or `-` and a trailing `-` are not part of a word.
fn fn_str_word_count(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let format = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if !matches!(format, 0..=2) {
        return Err(Thrown::error(
            "ValueError",
            "str_word_count(): Argument #2 ($format) must be a valid format value",
        ));
    }
    let mut words = PhpArray::new();
    if s.is_empty() {
        return Ok(if format == 0 {
            Value::Int(0)
        } else {
            Value::Array(words)
        });
    }
    let ch = match args.get(2).map(deref) {
        None | Some(Value::Null) => [false; 256],
        Some(v) => charmask(host, &conv::to_string_bytes(&v))?,
    };
    let mut p = 0;
    let mut e = s.len();
    if (s[0] == b'\'' && !ch[b'\'' as usize]) || (s[0] == b'-' && !ch[b'-' as usize]) {
        p += 1;
    }
    if s[e - 1] == b'-' && !ch[b'-' as usize] {
        e -= 1;
    }
    let mut count = 0i64;
    while p < e {
        let start = p;
        while p < e
            && (s[p].is_ascii_alphabetic() || ch[s[p] as usize] || s[p] == b'\'' || s[p] == b'-')
        {
            p += 1;
        }
        if p > start {
            match format {
                1 => words.push(Value::str(&s[start..p])),
                2 => words.insert(ArrayKey::Int(start as i64), Value::str(&s[start..p])),
                _ => count += 1,
            }
        }
        p += 1;
    }
    Ok(if format == 0 {
        Value::Int(count)
    } else {
        Value::Array(words)
    })
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
            for v in a.values() {
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

// ---------------------------------------------------------------------------
// str_getcsv
// ---------------------------------------------------------------------------

/// `str_getcsv($string, $separator = ",", $enclosure = '"', $escape = "\\")`.
/// Parses a single CSV record into an array of fields. An empty input yields
/// `[null]`, matching php.
fn fn_str_getcsv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&deref(&args[0]));
    let (sep, enc, esc) = crate::ext::standard::file::csv_args(host, args, "str_getcsv", 1)?;
    Ok(crate::ext::standard::file::csv_record(
        crate::ext::standard::file::php_fgetcsv(s, sep, enc, esc, None),
    ))
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

fn fn_implode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    implode(host, args, "implode")
}

/// `join`: implode under its own name.
fn fn_join(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    implode(host, args, "join")
}

fn implode(host: &mut dyn Host, args: &[Value], func: &str) -> Result<Value, Thrown> {
    // implode($array) or implode($separator, $array)
    let (glue, arr) = match (deref(&args[0]), args.get(1).map(deref)) {
        (Value::Array(a), None | Some(Value::Null)) => (Vec::new(), a),
        (Value::Array(_), Some(_)) => {
            return Err(Thrown::type_error(format!(
                "{func}(): Argument #1 ($separator) must be of type string, array given"
            )))
        }
        (g, Some(Value::Array(a))) => (conv::to_string_bytes(&g), a),
        _ => {
            return Err(Thrown::type_error(format!(
                "{func}(): If argument #1 ($separator) is of type string, argument #2 ($array) must be of type array, null given"
            )))
        }
    };
    // php_implode: arrays warn, objects use their string cast (or throw).
    let mut parts: Vec<std::borrow::Cow<[u8]>> = Vec::with_capacity(arr.len());
    for v in arr.values() {
        parts.push(match v {
            Value::Str(s) => std::borrow::Cow::Borrowed(s.as_bytes()),
            Value::Int(_) | Value::Bool(_) | Value::Null => {
                std::borrow::Cow::Owned(conv::to_string_bytes(v))
            }
            v => std::borrow::Cow::Owned(zval_get_string(host, v)?),
        });
    }
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
    let mut out = PhpStr::builder(len + glue.len() * parts.len().saturating_sub(1));
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push(&glue);
        }
        out.push(p);
    }
    Ok(Value::Str(out.finish()))
}

/// `explode`: php_explode / php_explode_negative_limit. The pieces are
/// built from the borrowed arguments (no copy of either string), and a
/// string without the separator is returned as is.
fn fn_explode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let delim = zstr_param(host, args, 0, "explode", "separator")?;
    let s = zstr_param(host, args, 1, "explode", "string")?;
    let limit = match args.get(2) {
        Some(v) => conv::to_int(&deref(v)),
        None => i64::MAX,
    };
    let delim = delim.as_bytes();
    if delim.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "explode(): Argument #1 ($separator) must not be empty",
        ));
    }
    let mut arr = PhpArray::new();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        if limit >= 0 {
            arr.push(Value::str(Vec::new()));
        }
        return Ok(Value::Array(arr));
    }
    let finder = memchr::memmem::Finder::new(delim);
    if limit > 1 {
        let mut p1 = 0;
        let Some(mut p2) = finder.find(bytes) else {
            arr.push(Value::Str(s.clone()));
            return Ok(Value::Array(arr));
        };
        let mut limit = limit;
        loop {
            // Each piece costs a zval and a string: stop once over the limit.
            if rhp_values::mem::has_oom() {
                return Ok(Value::Array(arr));
            }
            arr.push(Value::str(&bytes[p1..p2]));
            p1 = p2 + delim.len();
            match finder.find(&bytes[p1..]) {
                Some(o) => p2 = p1 + o,
                None => break,
            }
            limit -= 1;
            if limit <= 1 {
                break;
            }
        }
        arr.push(Value::str(&bytes[p1..]));
    } else if limit < 0 {
        let mut positions = vec![0];
        let mut p1 = 0;
        while let Some(o) = finder.find(&bytes[p1..]) {
            if rhp_values::mem::has_oom() {
                return Ok(Value::Array(arr));
            }
            p1 += o + delim.len();
            positions.push(p1);
        }
        if positions.len() == 1 {
            return Ok(Value::Array(arr));
        }
        let to_return = limit + positions.len() as i64;
        for i in 0..to_return.max(0) as usize {
            if rhp_values::mem::has_oom() {
                break;
            }
            let (a, b) = (positions[i], positions[i + 1] - delim.len());
            arr.push(Value::str(&bytes[a..b]));
        }
    } else {
        arr.push(Value::Str(s.clone()));
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
    let mut s = match deref(&args[0]) {
        Value::Str(s) => s,
        v => PhpStr::from_bytes(conv::to_string_bytes(&v)),
    };
    s.make_mut().make_ascii_uppercase();
    Ok(Value::Str(s))
}

fn fn_strtolower(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = match deref(&args[0]) {
        Value::Str(s) => s,
        v => PhpStr::from_bytes(conv::to_string_bytes(&v)),
    };
    s.make_mut().make_ascii_lowercase();
    Ok(Value::Str(s))
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
    crate::ext::random::shuffle(&mut s, &mut |lo, hi| Ok(mt_range(lo, hi)))?;
    Ok(Value::str(s))
}

/// `utf8_encode` / `utf8_decode`.
pub const PRELUDE_PHP: &str = include_str!("string/prelude.php");

/// string.c's constants: `STR_PAD_*`, `PATHINFO_*`, `LC_*`.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("STR_PAD_RIGHT", Value::Int(1)),
        ("STR_PAD_LEFT", Value::Int(0)),
        ("STR_PAD_BOTH", Value::Int(2)),
        ("PATHINFO_DIRNAME", Value::Int(1)),
        ("PATHINFO_BASENAME", Value::Int(2)),
        ("PATHINFO_EXTENSION", Value::Int(4)),
        ("PATHINFO_FILENAME", Value::Int(8)),
        ("PATHINFO_ALL", Value::Int(15)),
        ("LC_CTYPE", Value::Int(libc::LC_CTYPE as i64)),
        ("LC_NUMERIC", Value::Int(libc::LC_NUMERIC as i64)),
        ("LC_TIME", Value::Int(libc::LC_TIME as i64)),
        ("LC_COLLATE", Value::Int(libc::LC_COLLATE as i64)),
        ("LC_MONETARY", Value::Int(libc::LC_MONETARY as i64)),
        ("LC_MESSAGES", Value::Int(libc::LC_MESSAGES as i64)),
        ("LC_ALL", Value::Int(libc::LC_ALL as i64)),
    ]
    .into_iter()
    .chain(langinfo_constants())
    .collect()
}

/// The nl_langinfo() items (basic_functions.stub.php): the C library's
/// values, which differ per platform.
fn langinfo_constants() -> Vec<(&'static str, Value)> {
    use libc as l;
    #[cfg(target_os = "linux")]
    let (yesstr, nostr) = (0x50002, 0x50003);
    #[cfg(not(target_os = "linux"))]
    let (yesstr, nostr) = (l::YESSTR, l::NOSTR);
    let items: [(&'static str, l::nl_item); 56] = [
        ("ABDAY_1", l::ABDAY_1),
        ("ABDAY_2", l::ABDAY_2),
        ("ABDAY_3", l::ABDAY_3),
        ("ABDAY_4", l::ABDAY_4),
        ("ABDAY_5", l::ABDAY_5),
        ("ABDAY_6", l::ABDAY_6),
        ("ABDAY_7", l::ABDAY_7),
        ("DAY_1", l::DAY_1),
        ("DAY_2", l::DAY_2),
        ("DAY_3", l::DAY_3),
        ("DAY_4", l::DAY_4),
        ("DAY_5", l::DAY_5),
        ("DAY_6", l::DAY_6),
        ("DAY_7", l::DAY_7),
        ("ABMON_1", l::ABMON_1),
        ("ABMON_2", l::ABMON_2),
        ("ABMON_3", l::ABMON_3),
        ("ABMON_4", l::ABMON_4),
        ("ABMON_5", l::ABMON_5),
        ("ABMON_6", l::ABMON_6),
        ("ABMON_7", l::ABMON_7),
        ("ABMON_8", l::ABMON_8),
        ("ABMON_9", l::ABMON_9),
        ("ABMON_10", l::ABMON_10),
        ("ABMON_11", l::ABMON_11),
        ("ABMON_12", l::ABMON_12),
        ("MON_1", l::MON_1),
        ("MON_2", l::MON_2),
        ("MON_3", l::MON_3),
        ("MON_4", l::MON_4),
        ("MON_5", l::MON_5),
        ("MON_6", l::MON_6),
        ("MON_7", l::MON_7),
        ("MON_8", l::MON_8),
        ("MON_9", l::MON_9),
        ("MON_10", l::MON_10),
        ("MON_11", l::MON_11),
        ("MON_12", l::MON_12),
        ("AM_STR", l::AM_STR),
        ("PM_STR", l::PM_STR),
        ("D_T_FMT", l::D_T_FMT),
        ("D_FMT", l::D_FMT),
        ("T_FMT", l::T_FMT),
        ("T_FMT_AMPM", l::T_FMT_AMPM),
        ("ERA", l::ERA),
        ("ERA_D_T_FMT", l::ERA_D_T_FMT),
        ("ERA_D_FMT", l::ERA_D_FMT),
        ("ERA_T_FMT", l::ERA_T_FMT),
        ("ALT_DIGITS", l::ALT_DIGITS),
        ("CRNCYSTR", l::CRNCYSTR),
        ("RADIXCHAR", l::RADIXCHAR),
        ("THOUSEP", l::THOUSEP),
        ("YESEXPR", l::YESEXPR),
        ("NOEXPR", l::NOEXPR),
        ("YESSTR", yesstr),
        ("NOSTR", nostr),
    ];
    let mut v: Vec<(&'static str, Value)> = items
        .into_iter()
        .map(|(n, i)| (n, Value::Int(i as i64)))
        .collect();
    v.push(("CODESET", Value::Int(l::CODESET as i64)));
    v.push(("CHAR_MAX", Value::Int(libc::c_char::MAX as i64)));
    v
}

/// `__stdext_deprecated($message)`: E_DEPRECATED from a prelude function.
fn fn_stdext_deprecated(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = conv::to_string_bytes(&deref(&args[0]));
    host.emit_diagnostic(crate::E_DEPRECATED, &msg)?;
    Ok(Value::Null)
}

/// `__stdext_warning($message)`: E_WARNING from a prelude function.
fn fn_stdext_warning(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = conv::to_string_bytes(&deref(&args[0]));
    host.emit_diagnostic(crate::E_WARNING, &msg)?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// locales: setlocale, localeconv, nl_langinfo, strcoll
// ---------------------------------------------------------------------------
//
// The C library's locale is process-wide, as in php (ZTS builds too).

/// BG(locale_changed): a setlocale() call changed the locale this request.
static LOCALE_CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LOCALE_STARTUP: std::sync::Once = std::sync::Once::new();

/// zend_reset_lc_ctype_locale: LC_CTYPE is C.UTF-8 where it exists.
fn reset_lc_ctype() {
    // SAFETY: setlocale with valid C strings.
    unsafe {
        if libc::setlocale(libc::LC_CTYPE, c"C.UTF-8".as_ptr()).is_null() {
            libc::setlocale(libc::LC_CTYPE, c"C".as_ptr());
        }
    }
}

/// The locale php starts a request in: "C", with LC_CTYPE C.UTF-8, set at
/// startup and again after a request that changed it.
pub(crate) fn reset_locale() {
    LOCALE_STARTUP.call_once(reset_lc_ctype);
    if LOCALE_CHANGED.swap(false, std::sync::atomic::Ordering::Relaxed) {
        // SAFETY: as above.
        unsafe { libc::setlocale(libc::LC_ALL, c"C".as_ptr()) };
        reset_lc_ctype();
    }
}

/// LC_NUMERIC's decimal point (`*localeconv()->decimal_point`), for
/// printf's `%f` / `%g`.
pub(crate) fn locale_decimal_point() -> u8 {
    if !LOCALE_CHANGED.load(std::sync::atomic::Ordering::Relaxed) {
        return b'.';
    }
    // SAFETY: localeconv returns a valid struct with C strings.
    unsafe {
        let l = libc::localeconv();
        if l.is_null() || (*l).decimal_point.is_null() {
            return b'.';
        }
        *(*l).decimal_point as u8
    }
}

/// strcoll() on the strings as C strings.
pub(crate) fn strcoll(a: &[u8], b: &[u8]) -> i32 {
    let c = |s: &[u8]| {
        std::ffi::CString::new(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())])
            .unwrap_or_default()
    };
    let (a, b) = (c(a), c(b));
    // SAFETY: two valid C strings.
    unsafe { libc::strcoll(a.as_ptr(), b.as_ptr()) }
}

/// try_setlocale_str: the new (or, for "0", current) locale name.
fn try_setlocale(host: &mut dyn Host, cat: i64, loc: &[u8]) -> Result<Option<Vec<u8>>, Thrown> {
    let query = loc == b"0";
    if !query && loc.len() >= 255 {
        crate::warn(host, "setlocale", "Specified locale name is too long")?;
        return Ok(None);
    }
    let cloc =
        std::ffi::CString::new(&loc[..loc.iter().position(|&c| c == 0).unwrap_or(loc.len())])
            .unwrap_or_default();
    // SAFETY: setlocale with a valid category check by libc and a C string
    // (or NULL to query); the result is copied before the next call.
    let ret = unsafe {
        let r = libc::setlocale(
            cat as libc::c_int,
            if query {
                std::ptr::null()
            } else {
                cloc.as_ptr()
            },
        );
        if r.is_null() {
            return Ok(None);
        }
        std::ffi::CStr::from_ptr(r).to_bytes().to_vec()
    };
    if !query {
        LOCALE_CHANGED.store(true, std::sync::atomic::Ordering::Relaxed);
        if (cat == libc::LC_CTYPE as i64 || cat == libc::LC_ALL as i64) && ret == b"C" {
            return Ok(Some(b"C".to_vec()));
        }
        if ret == loc {
            return Ok(Some(loc.to_vec()));
        }
    }
    Ok(Some(ret))
}

fn fn_setlocale(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cat = conv::to_int(&deref(&args[0]));
    let strict = host.arg_uses_strict_types();
    // zend_parse_arg_str(.., check_null) for each non-array argument first.
    let mut strs: Vec<Option<Vec<u8>>> = Vec::with_capacity(args.len() - 1);
    for (i, a) in args[1..].iter().enumerate() {
        let v = deref(a);
        if matches!(v, Value::Array(_)) {
            strs.push(None);
            continue;
        }
        let s = match &v {
            Value::Null => Some(Vec::new()),
            Value::Str(s) => Some(s.as_bytes().to_vec()),
            // Without __toString: the TypeError below.
            Value::Object(_) => crate::zend::object_handlers::to_string(host, &v)?,
            Value::Int(_) | Value::Float(_) | Value::Bool(_) if !strict => {
                Some(conv::to_string_bytes(&v))
            }
            _ => None,
        };
        let Some(s) = s else {
            let label = if i == 0 { " ($locales)" } else { "" };
            return Err(Thrown::type_error(format!(
                "setlocale(): Argument #{}{label} must be of type array|string|null, {} given",
                i + 2,
                crate::zend::zpp::zval_type_name(host, &v)
            )));
        };
        strs.push(Some(s));
    }
    for (a, s) in args[1..].iter().zip(strs) {
        if let Some(s) = s {
            if let Some(r) = try_setlocale(host, cat, &s)? {
                return Ok(Value::str(r));
            }
            continue;
        }
        let Value::Array(arr) = deref(a) else {
            continue;
        };
        for e in arr.values() {
            let s = crate::zend::zpp::zval_get_string(host, &deref(e))?;
            if let Some(r) = try_setlocale(host, cat, &s)? {
                return Ok(Value::str(r));
            }
        }
    }
    Ok(Value::Bool(false))
}

fn fn_localeconv(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    // SAFETY: localeconv returns a valid struct of C strings and chars,
    // copied out before anything else runs.
    unsafe {
        let l = &*libc::localeconv();
        let s = |p: *const libc::c_char| -> Value {
            if p.is_null() {
                Value::str("")
            } else {
                Value::str(std::ffi::CStr::from_ptr(p).to_bytes())
            }
        };
        let grouping = |p: *const libc::c_char| -> Value {
            let mut a = PhpArray::new();
            if !p.is_null() {
                for &b in std::ffi::CStr::from_ptr(p).to_bytes() {
                    a.push(Value::Int(b as libc::c_char as i64));
                }
            }
            Value::Array(a)
        };
        let n = |c: libc::c_char| Value::Int(c as i64);
        let mut a = PhpArray::new();
        let mut set = |k: &str, v: Value| a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
        set("decimal_point", s(l.decimal_point));
        set("thousands_sep", s(l.thousands_sep));
        set("int_curr_symbol", s(l.int_curr_symbol));
        set("currency_symbol", s(l.currency_symbol));
        set("mon_decimal_point", s(l.mon_decimal_point));
        set("mon_thousands_sep", s(l.mon_thousands_sep));
        set("positive_sign", s(l.positive_sign));
        set("negative_sign", s(l.negative_sign));
        set("int_frac_digits", n(l.int_frac_digits));
        set("frac_digits", n(l.frac_digits));
        set("p_cs_precedes", n(l.p_cs_precedes));
        set("p_sep_by_space", n(l.p_sep_by_space));
        set("n_cs_precedes", n(l.n_cs_precedes));
        set("n_sep_by_space", n(l.n_sep_by_space));
        set("p_sign_posn", n(l.p_sign_posn));
        set("n_sign_posn", n(l.n_sign_posn));
        set("grouping", grouping(l.grouping));
        set("mon_grouping", grouping(l.mon_grouping));
        Ok(Value::Array(a))
    }
}

fn fn_nl_langinfo(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let item = conv::to_int(&deref(&args[0]));
    let valid = langinfo_constants()
        .iter()
        .any(|(n, v)| *n != "CHAR_MAX" && matches!(v, Value::Int(i) if *i == item));
    if !valid {
        crate::warn(host, "nl_langinfo", &format!("Item '{item}' is not valid"))?;
        return Ok(Value::Bool(false));
    }
    // SAFETY: nl_langinfo returns a C string (copied at once) or NULL.
    unsafe {
        let p = libc::nl_langinfo(item as libc::nl_item);
        if p.is_null() {
            return Ok(Value::Bool(false));
        }
        Ok(Value::str(std::ffi::CStr::from_ptr(p).to_bytes()))
    }
}

fn fn_strcoll(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = conv::to_string_bytes(&deref(&args[0]));
    let b = conv::to_string_bytes(&deref(&args[1]));
    Ok(Value::Int(strcoll(&a, &b) as i64))
}

fn fn_parse_str(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "parse_str", "string")?;
    let out = parse_query(host, s, "parse_str")?;
    if let Some(Value::Ref(r)) = args.get(1) {
        let old = std::mem::replace(&mut *r.borrow_mut(), Value::Array(out));
        rhp_values::gc::possible_root(&old);
    }
    Ok(Value::Null)
}

/// php_default_treat_data(PARSE_STRING) for `func` (parse_str,
/// mb_parse_str): the variables of query string `s`.
pub(crate) fn parse_query(host: &mut dyn Host, s: Vec<u8>, func: &str) -> Result<PhpArray, Thrown> {
    // C string: processing stops at the first NUL byte.
    let s = match s.iter().position(|&b| b == 0) {
        Some(p) => s[..p].to_vec(),
        None => s,
    };
    // php_default_treat_data(PARSE_STRING): arg_separator.input and
    // max_input_vars apply.
    let ini = |n: &[u8]| crate::ini_value(n).flatten().unwrap_or_default();
    let max_vars = crate::main::ini::c_atol(&ini(b"max_input_vars"));
    let (out, exceeded) = crate::main::php_variables::treat_data(
        crate::main::php_variables::TreatData::Form,
        &s,
        &ini(b"arg_separator.input"),
        max_vars,
    );
    if exceeded {
        crate::warn(
            host,
            func,
            &format!("Input variables exceeded {max_vars}. To increase the limit change max_input_vars in php.ini."),
        )?;
    }
    Ok(out)
}

/// `basename(path, suffix="")` — the trailing name component, byte-wise, matching
/// php (trailing slashes ignored; an exact suffix stripped unless it's the whole).
fn fn_basename(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_arg(args, 0);
    let suffix = args
        .get(1)
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_default();
    let mut base = php_basename(&path);
    if !suffix.is_empty() && base.len() > suffix.len() && base.ends_with(&suffix[..]) {
        base = &base[..base.len() - suffix.len()];
    }
    Ok(Value::str(base))
}

/// `php_basename` without a suffix: the last component, trailing slashes
/// ignored.
pub(crate) fn php_basename(path: &[u8]) -> &[u8] {
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
    &trimmed[start..]
}

/// `pathinfo(path, flags=PATHINFO_ALL)` — splits a path into dirname/basename/
/// extension/filename. Returns an assoc array for PATHINFO_ALL, else the
/// first part the flags ask for.
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

    let dirname = php_dirname(&path);

    use rhp_values::{ArrayKey as K, PhpArray};
    let mut arr = PhpArray::new();
    if flags & DIRNAME != 0 && !dirname.is_empty() {
        arr.insert(K::from_bytes(b"dirname"), Value::str(dirname));
    }
    if flags & BASENAME != 0 {
        arr.insert(K::from_bytes(b"basename"), Value::str(basename));
    }
    if flags & EXTENSION != 0 {
        if let Some(ext) = extension {
            arr.insert(K::from_bytes(b"extension"), Value::str(ext));
        }
    }
    if flags & FILENAME != 0 {
        arr.insert(K::from_bytes(b"filename"), Value::str(filename));
    }
    // The array for PATHINFO_ALL; for any other flags its first element.
    if flags == ALL {
        return Ok(Value::Array(arr));
    }
    Ok(arr
        .iter()
        .next()
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| Value::str(Vec::new())))
}

/// `dirname(path, levels=1)` (`_zend_dirname`): a level less than 1 is a
/// ValueError; further levels stop once the path no longer shrinks.
fn fn_dirname(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut path = path_arg(args, 0);
    let mut levels = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(1);
    if levels < 1 {
        let msg = "must be greater than or equal to 1";
        return Err(crate::zend::zpp::value_error("dirname", 2, "levels", msg));
    }
    loop {
        let up = php_dirname(&path);
        let shrank = up.len() < path.len();
        path = up;
        levels -= 1;
        if !shrank || levels == 0 {
            break;
        }
    }
    Ok(Value::str(path))
}

/// `php_dirname` (zend_dirname): `path` without its last component and the
/// slashes around it; "/" for a path of slashes, "." when no slash is left,
/// and "" for "".
pub(crate) fn php_dirname(path: &[u8]) -> Vec<u8> {
    if path.is_empty() {
        return Vec::new();
    }
    let mut end = path.len();
    // Trailing slashes, then the file name, then the slashes before it.
    while end > 0 && path[end - 1] == b'/' {
        end -= 1;
    }
    if end == 0 {
        return b"/".to_vec();
    }
    while end > 0 && path[end - 1] != b'/' {
        end -= 1;
    }
    if end == 0 {
        return b".".to_vec();
    }
    while end > 0 && path[end - 1] == b'/' {
        end -= 1;
    }
    if end == 0 {
        return b"/".to_vec();
    }
    path[..end].to_vec()
}

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

/// Lowercase hex encoding of a byte slice.
pub(crate) fn to_hex(bytes: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize]);
        out.push(HEX[(b & 0x0f) as usize]);
    }
    out
}

fn fn_bin2hex(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    Ok(Value::str(to_hex(&s)))
}

fn fn_hex2bin(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    // Odd length or non-hex input: php emits a warning and returns false.
    if !s.len().is_multiple_of(2) {
        crate::warn(
            host,
            "hex2bin",
            "Hexadecimal input string must have an even length",
        )?;
        return Ok(Value::Bool(false));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in s.as_chunks::<2>().0 {
        let hi = hex_nibble(pair[0]);
        let lo = hex_nibble(pair[1]);
        match (hi, lo) {
            (Some(h), Some(l)) => out.push((h << 4) | l),
            _ => {
                crate::warn(host, "hex2bin", "Input string must be hexadecimal string")?;
                return Ok(Value::Bool(false));
            }
        }
    }
    Ok(Value::str(out))
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
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
            ("join", fn_join),
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
            ("localeconv", fn_localeconv),
            ("nl_langinfo", fn_nl_langinfo),
            ("strcoll", fn_strcoll),
            ("parse_str", fn_parse_str),
            ("basename", fn_basename),
            ("dirname", fn_dirname),
            ("pathinfo", fn_pathinfo),
            ("bin2hex", fn_bin2hex),
            ("hex2bin", fn_hex2bin),
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
