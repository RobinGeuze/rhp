//! Native string builtins (pure, non-callback, non-by-reference).
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see `str_fns/tests.rs` — expected values are hardcoded from
//! `php -r '...'` output).

use rhp_semantics::deref;
use rhp_values::{conv, fmt, ArrayKey, PhpArray, Value};

use crate::argparse::zval_get_string;
use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

/// `(string)` bytes of a value (after deref).
fn bytes_of(v: &Value) -> Vec<u8> {
    conv::to_string_bytes(&deref(v))
}

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
    let s = bytes_of(&args[0]);
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
            &bytes_of(search),
            &bytes_of(replace),
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
            other => bytes_of(other),
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
        let s = bytes_of(&subject);
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
    let mut haystack = bytes_of(&args[0]);
    let mut needle = bytes_of(&args[1]);
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
    let mut haystack = bytes_of(&args[0]);
    let mut needle = bytes_of(&args[1]);
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
    let s = bytes_of(&args[0]);
    let target = conv::to_int(&deref(&args[1]));
    let pad = match args.get(2) {
        Some(v) => bytes_of(v),
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
    let s = bytes_of(&args[0]);
    let chars = match args.get(1) {
        Some(v) => bytes_of(v),
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
    let mut s = bytes_of(&args[0]);
    if let Some(b) = s.first_mut() {
        b.make_ascii_uppercase();
    }
    Ok(Value::str(s))
}

fn fn_lcfirst(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = bytes_of(&args[0]);
    if let Some(b) = s.first_mut() {
        b.make_ascii_lowercase();
    }
    Ok(Value::str(s))
}

fn fn_ucwords(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = bytes_of(&args[0]);
    let delims = match args.get(1) {
        Some(v) => bytes_of(v),
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
    let mut s = bytes_of(&args[0]);
    s.reverse();
    Ok(Value::str(s))
}

// ---------------------------------------------------------------------------
// substr_count
// ---------------------------------------------------------------------------

fn fn_substr_count(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let haystack = bytes_of(&args[0]);
    let needle = bytes_of(&args[1]);
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
    let h = bytes_of(&args[0]);
    let n = bytes_of(&args[1]);
    Ok(Value::Bool(h.len() >= n.len() && h[..n.len()] == n[..]))
}

fn fn_str_ends_with(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = bytes_of(&args[0]);
    let n = bytes_of(&args[1]);
    Ok(Value::Bool(
        h.len() >= n.len() && h[h.len() - n.len()..] == n[..],
    ))
}

// ---------------------------------------------------------------------------
// wordwrap
// ---------------------------------------------------------------------------

fn fn_wordwrap(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let text = bytes_of(&args[0]);
    let width = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(75);
    let brk = match args.get(2) {
        Some(v) => bytes_of(v),
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
// number_format
// ---------------------------------------------------------------------------

/// Increment a big-endian ascii-digit string (`"199"` -> `"200"`),
/// growing at the front on carry (`"99"` -> `"100"`).
fn increment_digits(d: &mut Vec<u8>) {
    let mut i = d.len();
    loop {
        if i == 0 {
            d.insert(0, b'1');
            return;
        }
        i -= 1;
        if d[i] == b'9' {
            d[i] = b'0';
        } else {
            d[i] += 1;
            return;
        }
    }
}

/// Round |value| (given as decimal digit strings) to `dec` decimal places using
/// half-away-from-zero, operating on the shortest decimal representation.
/// Returns (int_digits, frac_digits) where frac has length `max(dec,0)`.
fn round_decimal(int_digits: &[u8], frac_digits: &[u8], dec: i64) -> (Vec<u8>, Vec<u8>) {
    let int_len = int_digits.len() as i64;
    let mut all: Vec<u8> = Vec::with_capacity(int_digits.len() + frac_digits.len());
    all.extend_from_slice(int_digits);
    all.extend_from_slice(frac_digits);

    let keep = int_len + dec; // number of leading digits to retain
    let round_up = if keep < 0 || keep as usize >= all.len() {
        false
    } else {
        all[keep as usize] >= b'5'
    };

    let kept_len = keep.max(0) as usize;
    let mut kept: Vec<u8> = all.iter().take(kept_len).copied().collect();
    while kept.len() < kept_len {
        kept.push(b'0'); // value had fewer digits than requested
    }
    if round_up {
        increment_digits(&mut kept);
    }
    // normalize kept to a plain integer (strip leading zeros, keep >= "0")
    let first_nonzero = kept.iter().position(|&b| b != b'0').unwrap_or(kept.len());
    let kept_int: Vec<u8> = if first_nonzero == kept.len() {
        vec![b'0']
    } else {
        kept[first_nonzero..].to_vec()
    };
    let is_zero = kept_int == *b"0";

    if dec <= 0 {
        // No fractional output. For dec<0 the kept integer scales by 10^(-dec).
        let mut int_str = kept_int;
        if !is_zero && dec < 0 {
            int_str.extend(std::iter::repeat_n(b'0', (-dec) as usize));
        }
        (int_str, Vec::new())
    } else {
        let dec = dec as usize;
        let mut kint = kept_int;
        while kint.len() <= dec {
            kint.insert(0, b'0'); // ensure at least one integer digit
        }
        let split = kint.len() - dec;
        let int_str = kint[..split].to_vec();
        let frac_str = kint[split..].to_vec();
        // strip leading zeros of int part (keep one)
        let fnz = int_str
            .iter()
            .position(|&b| b != b'0')
            .unwrap_or(int_str.len());
        let int_str = if fnz == int_str.len() {
            vec![b'0']
        } else {
            int_str[fnz..].to_vec()
        };
        (int_str, frac_str)
    }
}

fn fn_number_format(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let num = conv::to_float(&deref(&args[0]));
    let dec = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let dec_point = match args.get(2) {
        Some(v) => bytes_of(v),
        None => b".".to_vec(),
    };
    let thousands = match args.get(3) {
        Some(v) => bytes_of(v),
        None => b",".to_vec(),
    };

    if !num.is_finite() {
        // php emits "nan"/"inf"; rare, keep it simple.
        return Ok(Value::str(fmt::float_echo(num)));
    }

    let negative = num < 0.0;
    let abs = num.abs();
    // Shortest decimal (Rust's Display is always non-exponential for f64).
    let s = format!("{abs}");
    let (int_part, frac_part) = match s.split_once('.') {
        Some((a, b)) => (a.as_bytes().to_vec(), b.as_bytes().to_vec()),
        None => (s.as_bytes().to_vec(), Vec::new()),
    };

    let (int_digits, frac_digits) = round_decimal(&int_part, &frac_part, dec);

    let value_is_zero = int_digits == *b"0" && frac_digits.iter().all(|&b| b == b'0');

    // Group the integer part with the thousands separator.
    let mut grouped: Vec<u8> = Vec::new();
    let n = int_digits.len();
    for (i, &d) in int_digits.iter().enumerate() {
        if i > 0 && (n - i) % 3 == 0 {
            grouped.extend_from_slice(&thousands);
        }
        grouped.push(d);
    }

    let mut out = Vec::new();
    if negative && !value_is_zero {
        out.push(b'-');
    }
    out.extend_from_slice(&grouped);
    if dec > 0 {
        out.extend_from_slice(&dec_point);
        out.extend_from_slice(&frac_digits);
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// sprintf / printf / vsprintf
// ---------------------------------------------------------------------------

struct Conv {
    arg: Option<usize>, // explicit `N$` (1-based)
    left: bool,
    plus: bool,
    pad: u8,
    width: usize,
    prec: Option<usize>,
    spec: u8,
}

enum Piece {
    Lit(Vec<u8>),
    Conv(Conv),
}

fn parse_format(fmt: &[u8]) -> Result<Vec<Piece>, Thrown> {
    let mut pieces = Vec::new();
    let mut lit = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' {
            lit.push(fmt[i]);
            i += 1;
            continue;
        }
        // '%' found
        i += 1;
        if i < fmt.len() && fmt[i] == b'%' {
            lit.push(b'%');
            i += 1;
            continue;
        }
        if !lit.is_empty() {
            pieces.push(Piece::Lit(std::mem::take(&mut lit)));
        }

        // optional argnum: digits followed by '$'
        let mut arg = None;
        {
            let mut j = i;
            while j < fmt.len() && fmt[j].is_ascii_digit() {
                j += 1;
            }
            if j > i && j < fmt.len() && fmt[j] == b'$' {
                let n: usize = std::str::from_utf8(&fmt[i..j])
                    .unwrap()
                    .parse()
                    .unwrap_or(0);
                arg = Some(n);
                i = j + 1;
            }
        }

        // flags
        let mut left = false;
        let mut plus = false;
        let mut pad = b' ';
        loop {
            if i >= fmt.len() {
                break;
            }
            match fmt[i] {
                b'-' => left = true,
                b'+' => plus = true,
                b' ' => pad = b' ',
                b'0' => pad = b'0',
                b'\'' => {
                    i += 1;
                    if i >= fmt.len() {
                        return Err(Thrown::error(
                            "ValueError",
                            "Missing format specifier at end of string",
                        ));
                    }
                    pad = fmt[i];
                }
                _ => break,
            }
            i += 1;
        }

        // width
        let ws = i;
        while i < fmt.len() && fmt[i].is_ascii_digit() {
            i += 1;
        }
        let width: usize = if i > ws {
            std::str::from_utf8(&fmt[ws..i])
                .unwrap()
                .parse()
                .unwrap_or(0)
        } else {
            0
        };

        // precision
        let mut prec = None;
        if i < fmt.len() && fmt[i] == b'.' {
            i += 1;
            let ps = i;
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                i += 1;
            }
            prec = Some(if i > ps {
                std::str::from_utf8(&fmt[ps..i])
                    .unwrap()
                    .parse()
                    .unwrap_or(0)
            } else {
                0
            });
        }

        // A C length modifier is skipped.
        if i < fmt.len() && fmt[i] == b'l' {
            i += 1;
        }
        if i >= fmt.len() {
            return Err(Thrown::error(
                "ValueError",
                "Missing format specifier at end of string",
            ));
        }
        let spec = fmt[i];
        i += 1;
        if !matches!(
            spec,
            b's' | b'd'
                | b'f'
                | b'F'
                | b'b'
                | b'o'
                | b'x'
                | b'X'
                | b'c'
                | b'u'
                | b'e'
                | b'E'
                | b'g'
                | b'G'
        ) {
            return Err(Thrown::error(
                "ValueError",
                format!("Unknown format specifier \"{}\"", spec as char),
            ));
        }
        pieces.push(Piece::Conv(Conv {
            arg,
            left,
            plus,
            pad,
            width,
            prec,
            spec,
        }));
    }
    if !lit.is_empty() {
        pieces.push(Piece::Lit(lit));
    }
    Ok(pieces)
}

/// Field-pad a string conversion (pad char used as-is on either side).
fn pad_field(body: Vec<u8>, width: usize, pad: u8, left: bool) -> Vec<u8> {
    if body.len() >= width {
        return body;
    }
    let fill = width - body.len();
    let mut out = Vec::with_capacity(width);
    if left {
        out.extend_from_slice(&body);
        out.extend(std::iter::repeat_n(pad, fill));
    } else {
        out.extend(std::iter::repeat_n(pad, fill));
        out.extend_from_slice(&body);
    }
    out
}

/// Field-pad a numeric conversion (sign kept with the magnitude; `'0'` pads
/// after the sign when right-aligned, and degrades to space when left-aligned).
fn pad_numeric(sign: &[u8], mag: &[u8], width: usize, pad: u8, left: bool) -> Vec<u8> {
    let total = sign.len() + mag.len();
    let mut out = Vec::with_capacity(width.max(total));
    if total >= width {
        out.extend_from_slice(sign);
        out.extend_from_slice(mag);
        return out;
    }
    let fill = width - total;
    if left {
        let p = if pad == b'0' { b' ' } else { pad };
        out.extend_from_slice(sign);
        out.extend_from_slice(mag);
        out.extend(std::iter::repeat_n(p, fill));
    } else if pad == b'0' {
        out.extend_from_slice(sign);
        out.extend(std::iter::repeat_n(b'0', fill));
        out.extend_from_slice(mag);
    } else {
        out.extend(std::iter::repeat_n(pad, fill));
        out.extend_from_slice(sign);
        out.extend_from_slice(mag);
    }
    out
}

/// php-style exponent reformat: Rust's `{:e}` gives `1.23e4`; php wants
/// `1.23e+4` (signed, minimal digits). Applies to the already-built mantissa.
fn reformat_exp(rust_e: &str, upper: bool) -> Vec<u8> {
    let (m, e) = rust_e.split_once('e').unwrap();
    let exp: i64 = e.parse().unwrap();
    let mut out = m.as_bytes().to_vec();
    out.push(if upper { b'E' } else { b'e' });
    out.push(if exp < 0 { b'-' } else { b'+' });
    out.extend_from_slice(exp.unsigned_abs().to_string().as_bytes());
    out
}

/// php `%g` / `%G`: `zend_gcvt` with the given significant-digit precision.
fn gcvt(f: f64, precision: usize, upper: bool) -> Vec<u8> {
    let prec = precision.max(1);
    let negative = f < 0.0;
    let sign: &[u8] = if negative { b"-" } else { b"" };
    let abs = f.abs();
    if abs == 0.0 {
        return [sign, b"0"].concat();
    }
    let s = format!("{:.*e}", prec - 1, abs);
    let (mant, exp) = s.split_once('e').unwrap();
    let exp10: i32 = exp.parse().unwrap();
    let digits: Vec<u8> = mant.bytes().filter(|&b| b != b'.').collect();
    // trim trailing zeros, keep >= 1
    let mut end = digits.len();
    while end > 1 && digits[end - 1] == b'0' {
        end -= 1;
    }
    let digits = &digits[..end];

    let cutoff = prec as i32;
    let body = if exp10 < -4 || exp10 >= cutoff {
        // exponential, always >= 1 fractional digit
        let mut out = Vec::new();
        out.push(digits[0]);
        out.push(b'.');
        if digits.len() > 1 {
            out.extend_from_slice(&digits[1..]);
        } else {
            out.push(b'0');
        }
        out.push(if upper { b'E' } else { b'e' });
        out.push(if exp10 < 0 { b'-' } else { b'+' });
        out.extend_from_slice(exp10.unsigned_abs().to_string().as_bytes());
        out
    } else if exp10 >= 0 {
        let int_len = (exp10 as usize) + 1;
        let mut out = Vec::new();
        if digits.len() <= int_len {
            out.extend_from_slice(digits);
            out.extend(std::iter::repeat_n(b'0', int_len - digits.len()));
        } else {
            out.extend_from_slice(&digits[..int_len]);
            out.push(b'.');
            out.extend_from_slice(&digits[int_len..]);
        }
        out
    } else {
        let mut out = Vec::new();
        out.extend_from_slice(b"0.");
        out.extend(std::iter::repeat_n(b'0', (-exp10 - 1) as usize));
        out.extend_from_slice(digits);
        out
    };
    [sign, &body].concat()
}

/// Format one non-special float conversion (f/F/e/E). Returns (sign, magnitude).
fn float_parts(f: f64, spec: u8, prec: usize) -> (Vec<u8>, Vec<u8>) {
    let negative = f < 0.0;
    let abs = f.abs();
    let mag = match spec {
        b'f' | b'F' => format!("{abs:.prec$}").into_bytes(),
        b'e' | b'E' => {
            let r = format!("{:.*e}", prec, abs);
            reformat_exp(&r, spec == b'E')
        }
        _ => unreachable!(),
    };
    let sign: Vec<u8> = if negative { b"-".to_vec() } else { Vec::new() };
    (sign, mag)
}

fn render_conv(c: &Conv, arg: &Value) -> Vec<u8> {
    let v = deref(arg);
    match c.spec {
        b's' => {
            let mut s = conv::to_string_bytes(&v);
            if let Some(p) = c.prec {
                s.truncate(p);
            }
            pad_field(s, c.width, c.pad, c.left)
        }
        b'c' => {
            // single byte, width/precision/flags ignored (php behavior)
            let n = conv::to_int(&v);
            vec![(n & 0xff) as u8]
        }
        b'd' => {
            let n = conv::to_int(&v);
            let neg = n < 0;
            let mag = n.unsigned_abs().to_string().into_bytes();
            let sign: &[u8] = if neg {
                b"-"
            } else if c.plus {
                b"+"
            } else {
                b""
            };
            pad_numeric(sign, &mag, c.width, c.pad, c.left)
        }
        b'u' => {
            let n = conv::to_int(&v) as u64;
            pad_numeric(b"", n.to_string().as_bytes(), c.width, c.pad, c.left)
        }
        b'o' => {
            let n = conv::to_int(&v) as u64;
            pad_numeric(b"", format!("{n:o}").as_bytes(), c.width, c.pad, c.left)
        }
        b'x' => {
            let n = conv::to_int(&v) as u64;
            pad_numeric(b"", format!("{n:x}").as_bytes(), c.width, c.pad, c.left)
        }
        b'X' => {
            let n = conv::to_int(&v) as u64;
            pad_numeric(b"", format!("{n:X}").as_bytes(), c.width, c.pad, c.left)
        }
        b'b' => {
            let n = conv::to_int(&v) as u64;
            pad_numeric(b"", format!("{n:b}").as_bytes(), c.width, c.pad, c.left)
        }
        b'f' | b'F' | b'e' | b'E' => {
            let f = conv::to_float(&v);
            if f.is_nan() {
                return b"NaN".to_vec();
            }
            if f.is_infinite() {
                return if f < 0.0 {
                    b"-INF".to_vec()
                } else {
                    b"INF".to_vec()
                };
            }
            let prec = c.prec.unwrap_or(6);
            let (mut sign, mag) = float_parts(f, c.spec, prec);
            if sign.is_empty() && c.plus {
                sign = b"+".to_vec();
            }
            pad_numeric(&sign, &mag, c.width, c.pad, c.left)
        }
        b'g' | b'G' => {
            let f = conv::to_float(&v);
            if f.is_nan() {
                return b"NaN".to_vec();
            }
            if f.is_infinite() {
                return if f < 0.0 {
                    b"-INF".to_vec()
                } else {
                    b"INF".to_vec()
                };
            }
            let prec = c.prec.unwrap_or(6);
            let body = gcvt(f, prec, c.spec == b'G');
            // split sign for numeric padding
            if let Some(stripped) = body.strip_prefix(b"-") {
                pad_numeric(b"-", stripped, c.width, c.pad, c.left)
            } else {
                let sign: &[u8] = if c.plus { b"+" } else { b"" };
                pad_numeric(sign, &body, c.width, c.pad, c.left)
            }
        }
        _ => Vec::new(),
    }
}

/// Core of sprintf/vsprintf: `fmt` + value args (not including the format).
/// `given` is the total argument count (incl. the format) for error messages.
pub(crate) fn do_sprintf(
    fmt: &[u8],
    value_args: &[Value],
    given: usize,
) -> Result<Vec<u8>, Thrown> {
    let pieces = parse_format(fmt)?;

    // Resolve argument indices (positional does not advance the sequential ptr).
    let mut seq = 1usize;
    let mut resolved: Vec<usize> = Vec::new();
    let mut max_idx = 0usize;
    for p in &pieces {
        if let Piece::Conv(c) = p {
            let idx = match c.arg {
                Some(n) => n,
                None => {
                    let v = seq;
                    seq += 1;
                    v
                }
            };
            resolved.push(idx);
            max_idx = max_idx.max(idx);
        }
    }
    if max_idx > value_args.len() {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!("{} arguments are required, {} given", max_idx + 1, given),
        ));
    }

    let mut out = Vec::new();
    let mut ci = 0;
    for p in &pieces {
        match p {
            Piece::Lit(b) => out.extend_from_slice(b),
            Piece::Conv(c) => {
                let idx = resolved[ci];
                ci += 1;
                out.extend_from_slice(&render_conv(c, &value_args[idx - 1]));
            }
        }
    }
    Ok(out)
}

fn fn_sprintf(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = bytes_of(&args[0]);
    let out = do_sprintf(&fmt, &args[1..], args.len())?;
    Ok(Value::str(out))
}

fn fn_printf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = bytes_of(&args[0]);
    let out = do_sprintf(&fmt, &args[1..], args.len())?;
    let len = out.len() as i64;
    host.write_out(&out);
    Ok(Value::Int(len))
}

fn fn_vsprintf(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = bytes_of(&args[0]);
    let Value::Array(arr) = deref(&args[1]) else {
        return Err(Thrown::type_error(
            "vsprintf(): Argument #2 ($values) must be of type array",
        ));
    };
    let values: Vec<Value> = arr.iter().map(|(_, v)| v.clone()).collect();
    // `given` mimics the equivalent sprintf() call: format + each value.
    let out = do_sprintf(&fmt, &values, values.len() + 1)?;
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
    let s = bytes_of(&args[0]);
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
    let s = bytes_of(&args[0]);
    let mask = charmask(&bytes_of(&args[1]));
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
    let s = bytes_of(&args[0]);
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
    let s = bytes_of(&args[0]);
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
            ("number_format", fn_number_format),
            ("sprintf", fn_sprintf),
            ("printf", fn_printf),
            ("vsprintf", fn_vsprintf),
            ("soundex", fn_soundex),
            ("addcslashes", fn_addcslashes),
            ("stripcslashes", fn_stripcslashes),
            ("count_chars", fn_count_chars),
        ],
    );
}

#[cfg(test)]
mod tests;
