//! Native string-comparison + related builtins (pure, non-callback).
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see `strcmp_fns/tests.rs` — expected values are hardcoded from
//! `php -r '...'` output).
//!
//! Note: on this platform php's `strcmp`/`strncmp`/`strcasecmp`/`strncasecmp`
//! family returns the *raw* byte difference of the first differing byte (e.g.
//! `strcmp("Hello","hello") === -32`), NOT a clamped -1/0/1 — only the
//! "one string is a prefix of the other" (equal-prefix) path returns a clamped
//! sign. This mirrors glibc/macOS `memcmp` behaviour, which php relies on.
//!
//! Wire into `lib.rs`:
//! ```ignore
//! mod strcmp_fns;                 // near the other `mod` lines
//! // ...inside registry(), after `m` is built, before it is returned:
//! strcmp_fns::register(&mut m);
//! ```

use std::collections::HashMap;

use rhp_semantics::deref;
use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

/// `(string)` bytes of a value (after deref).
fn bytes_of(v: &Value) -> Vec<u8> {
    conv::to_string_bytes(&deref(v))
}

/// Raw byte-difference `memcmp` over the first `n` bytes: returns the signed
/// difference of the first differing (unsigned) byte, or 0. Matches the
/// platform `memcmp` php returns directly.
fn memcmp_raw(a: &[u8], b: &[u8], n: usize) -> i64 {
    for i in 0..n {
        let d = a[i] as i64 - b[i] as i64;
        if d != 0 {
            return d;
        }
    }
    0
}

/// Case-insensitive variant: compares `tolower` of each byte.
fn memcmp_raw_ci(a: &[u8], b: &[u8], n: usize) -> i64 {
    for i in 0..n {
        let d = a[i].to_ascii_lowercase() as i64 - b[i].to_ascii_lowercase() as i64;
        if d != 0 {
            return d;
        }
    }
    0
}

/// Normalize to -1 / 0 / 1 (the equal-prefix / length-difference path).
fn norm(x: i64) -> i64 {
    match x.cmp(&0) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// `zend_binary_strcmp`: raw diff on first differing byte, clamped sign on the
/// equal-prefix (length-difference) path.
fn binary_strcmp(s1: &[u8], s2: &[u8]) -> i64 {
    let n = s1.len().min(s2.len());
    let r = memcmp_raw(s1, s2, n);
    if r != 0 {
        r
    } else {
        norm(s1.len() as i64 - s2.len() as i64)
    }
}

fn binary_strcasecmp(s1: &[u8], s2: &[u8]) -> i64 {
    let n = s1.len().min(s2.len());
    let r = memcmp_raw_ci(s1, s2, n);
    if r != 0 {
        r
    } else {
        norm(s1.len() as i64 - s2.len() as i64)
    }
}

/// `zend_binary_strncmp`: like strcmp but bounded to `length` bytes. The
/// equal-prefix path compares `min(length,len)` of each side.
fn binary_strncmp(s1: &[u8], s2: &[u8], length: usize) -> i64 {
    let n = length.min(s1.len()).min(s2.len());
    let r = memcmp_raw(s1, s2, n);
    if r != 0 {
        r
    } else {
        norm(length.min(s1.len()) as i64 - length.min(s2.len()) as i64)
    }
}

fn binary_strncasecmp(s1: &[u8], s2: &[u8], length: usize) -> i64 {
    let n = length.min(s1.len()).min(s2.len());
    let r = memcmp_raw_ci(s1, s2, n);
    if r != 0 {
        r
    } else {
        norm(length.min(s1.len()) as i64 - length.min(s2.len()) as i64)
    }
}

// ---------------------------------------------------------------------------
// strcmp / strcasecmp / strncmp / strncasecmp
// ---------------------------------------------------------------------------

fn fn_strcmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(binary_strcmp(
        &bytes_of(&args[0]),
        &bytes_of(&args[1]),
    )))
}

fn fn_strcasecmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(binary_strcasecmp(
        &bytes_of(&args[0]),
        &bytes_of(&args[1]),
    )))
}

fn fn_strncmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = conv::to_int(&deref(&args[2]));
    if n < 0 {
        return Err(Thrown::error(
            "ValueError",
            "strncmp(): Argument #3 ($length) must be greater than or equal to 0",
        ));
    }
    Ok(Value::Int(binary_strncmp(
        &bytes_of(&args[0]),
        &bytes_of(&args[1]),
        n as usize,
    )))
}

fn fn_strncasecmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = conv::to_int(&deref(&args[2]));
    if n < 0 {
        return Err(Thrown::error(
            "ValueError",
            "strncasecmp(): Argument #3 ($length) must be greater than or equal to 0",
        ));
    }
    Ok(Value::Int(binary_strncasecmp(
        &bytes_of(&args[0]),
        &bytes_of(&args[1]),
        n as usize,
    )))
}

// ---------------------------------------------------------------------------
// substr_compare
// ---------------------------------------------------------------------------

fn fn_substr_compare(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let main = bytes_of(&args[0]);
    let needle = bytes_of(&args[1]);
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
        &bytes_of(&args[0]),
        &bytes_of(&args[1]),
        false,
    )))
}

fn fn_strnatcasecmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(strnatcmp_ex(
        &bytes_of(&args[0]),
        &bytes_of(&args[1]),
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
    crate::str_fns::str_replace_common(host, args, "str_ireplace", ireplace_all)
}

// ---------------------------------------------------------------------------
// strpbrk
// ---------------------------------------------------------------------------

fn fn_strpbrk(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let chars = bytes_of(&args[1]);
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
    let subject = bytes_of(&args[0]);
    let mask = bytes_of(&args[1]);
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
    let subject = bytes_of(&args[0]);
    let mask = bytes_of(&args[1]);
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
    let mut b = bytes_of(&args[0]);
    for c in b.iter_mut() {
        *c = match *c {
            b'A'..=b'Z' => b'A' + (*c - b'A' + 13) % 26,
            b'a'..=b'z' => b'a' + (*c - b'a' + 13) % 26,
            other => other,
        };
    }
    Ok(Value::str(b))
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
    put(m, "strcmp", fn_strcmp, 2, Some(2));
    put(m, "strcasecmp", fn_strcasecmp, 2, Some(2));
    put(m, "strncmp", fn_strncmp, 3, Some(3));
    put(m, "strncasecmp", fn_strncasecmp, 3, Some(3));
    put(m, "substr_compare", fn_substr_compare, 3, Some(5));
    put(m, "strnatcmp", fn_strnatcmp, 2, Some(2));
    put(m, "strnatcasecmp", fn_strnatcasecmp, 2, Some(2));
    put(m, "str_ireplace", fn_str_ireplace, 3, Some(4));
    put(m, "strpbrk", fn_strpbrk, 2, Some(2));
    put(m, "strspn", fn_strspn, 2, Some(4));
    put(m, "strcspn", fn_strcspn, 2, Some(4));
    put(m, "str_rot13", fn_str_rot13, 1, Some(1));
}

#[cfg(test)]
mod tests;
