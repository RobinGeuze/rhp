//! Native string-comparison + related builtins (pure, non-callback).
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see the `#[cfg(test)]` block — expected values are hardcoded from
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

use rhp_values::{conv, Value};

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
// strnatcmp / strnatcasecmp: a Rust translation, modified for rhp, of php-src's
// ext/standard/strnatcmp.c, Copyright (C) 2000 by Martin Pool (zlib licence;
// see THIRD-PARTY-NOTICES, section 15).
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

// ---------------------------------------------------------------------------
// tests (expected values hardcoded from local php 8.5.8)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rhp_values::PhpArray;

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
    fn call_int(f: crate::NativeFn, args: &[Value]) -> i64 {
        match call(f, args) {
            Value::Int(n) => n,
            other => panic!("expected int, got {}", other.type_name()),
        }
    }
    fn calls(f: crate::NativeFn, args: &[Value]) -> String {
        String::from_utf8(bytes_of(&call(f, args))).unwrap()
    }
    fn throws(f: crate::NativeFn, args: &[Value]) -> bool {
        f(&mut NullHost, args).is_err()
    }

    #[test]
    fn strcmp_cases() {
        assert_eq!(call_int(fn_strcmp, &[s("a"), s("b")]), -1);
        assert_eq!(call_int(fn_strcmp, &[s("b"), s("a")]), 1);
        assert_eq!(call_int(fn_strcmp, &[s("a"), s("a")]), 0);
        assert_eq!(call_int(fn_strcmp, &[s("abc"), s("abd")]), -1);
        assert_eq!(call_int(fn_strcmp, &[s("Hello"), s("hello")]), -32);
        assert_eq!(call_int(fn_strcmp, &[s("a"), s("ab")]), -1);
        assert_eq!(call_int(fn_strcmp, &[s("abc"), s("abcde")]), -1);
        assert_eq!(call_int(fn_strcmp, &[s("abcde"), s("abc")]), 1);
        assert_eq!(call_int(fn_strcmp, &[s("10"), s("9")]), -8);
        // raw byte diff on single high byte: 0xff - 0x01 = 254
        assert_eq!(
            call_int(
                fn_strcmp,
                &[Value::str(vec![0xffu8]), Value::str(vec![0x01u8])]
            ),
            254
        );
    }

    #[test]
    fn strcasecmp_cases() {
        assert_eq!(call_int(fn_strcasecmp, &[s("Hello"), s("hello")]), 0);
        assert_eq!(call_int(fn_strcasecmp, &[s("a"), s("B")]), -1);
        assert_eq!(call_int(fn_strcasecmp, &[s("B"), s("a")]), 1);
        assert_eq!(call_int(fn_strcasecmp, &[s("A"), s("z")]), -25);
        assert_eq!(call_int(fn_strcasecmp, &[s("Z"), s("a")]), 25);
        assert_eq!(call_int(fn_strcasecmp, &[s("ABC"), s("abcd")]), -1);
    }

    #[test]
    fn strncmp_cases() {
        assert_eq!(
            call_int(fn_strncmp, &[s("Hello"), s("Help"), Value::Int(3)]),
            0
        );
        assert_eq!(
            call_int(fn_strncmp, &[s("Hello"), s("Help"), Value::Int(4)]),
            -4
        );
        assert_eq!(
            call_int(fn_strncmp, &[s("abc"), s("abd"), Value::Int(0)]),
            0
        );
        assert_eq!(
            call_int(fn_strncmp, &[s("abc"), s("abcde"), Value::Int(10)]),
            -1
        );
        assert_eq!(
            call_int(fn_strncmp, &[s("abcde"), s("abc"), Value::Int(10)]),
            1
        );
        assert_eq!(
            call_int(fn_strncmp, &[s("abc"), s("abcde"), Value::Int(3)]),
            0
        );
        assert!(throws(fn_strncmp, &[s("a"), s("b"), Value::Int(-1)]));
    }

    #[test]
    fn strncasecmp_cases() {
        assert_eq!(
            call_int(fn_strncasecmp, &[s("Hello"), s("hello"), Value::Int(5)]),
            0
        );
        assert_eq!(
            call_int(fn_strncasecmp, &[s("Hello"), s("help"), Value::Int(3)]),
            0
        );
        assert_eq!(
            call_int(fn_strncasecmp, &[s("AX"), s("aZ"), Value::Int(2)]),
            -2
        );
        assert!(throws(fn_strncasecmp, &[s("a"), s("b"), Value::Int(-1)]));
    }

    #[test]
    fn substr_compare_cases() {
        assert_eq!(
            call_int(
                fn_substr_compare,
                &[s("Hello World"), s("World"), Value::Int(6)]
            ),
            0
        );
        assert_eq!(
            call_int(
                fn_substr_compare,
                &[
                    s("Hello World"),
                    s("world"),
                    Value::Int(6),
                    Value::Int(5),
                    Value::Bool(true)
                ]
            ),
            0
        );
        assert_eq!(
            call_int(fn_substr_compare, &[s("abcde"), s("bc"), Value::Int(1)]),
            1
        );
        assert_eq!(
            call_int(
                fn_substr_compare,
                &[s("abcde"), s("bc"), Value::Int(1), Value::Int(2)]
            ),
            0
        );
        assert_eq!(
            call_int(fn_substr_compare, &[s("abcde"), s("fg"), Value::Int(-2)]),
            -2
        );
        assert_eq!(
            call_int(
                fn_substr_compare,
                &[s("abcde"), s("abcXY"), Value::Int(0), Value::Int(5)]
            ),
            12
        );
        assert_eq!(
            call_int(fn_substr_compare, &[s("abc"), s("abcde"), Value::Int(0)]),
            -1
        );
        assert_eq!(
            call_int(
                fn_substr_compare,
                &[s("abcde"), s("abc"), Value::Int(0), Value::Int(10)]
            ),
            1
        );
        assert_eq!(
            call_int(
                fn_substr_compare,
                &[s("abcde"), s("XYZ"), Value::Int(1), Value::Int(0)]
            ),
            0
        );
        assert_eq!(
            call_int(fn_substr_compare, &[s("abc"), s("b"), Value::Int(3)]),
            -1
        );
        assert_eq!(
            call_int(fn_substr_compare, &[s("abc"), s("a"), Value::Int(-10)]),
            1
        );
        assert_eq!(
            call_int(
                fn_substr_compare,
                &[
                    s("Hello"),
                    s("hel"),
                    Value::Int(0),
                    Value::Int(3),
                    Value::Bool(true)
                ]
            ),
            0
        );
        assert_eq!(
            call_int(
                fn_substr_compare,
                &[
                    s("Hello"),
                    s("hel"),
                    Value::Int(0),
                    Value::Int(5),
                    Value::Bool(true)
                ]
            ),
            1
        );
        // errors
        assert!(throws(
            fn_substr_compare,
            &[s("abc"), s("b"), Value::Int(5)]
        ));
        assert!(throws(
            fn_substr_compare,
            &[s("abcde"), s("XYZ"), Value::Int(1), Value::Int(-1)]
        ));
    }

    #[test]
    fn strnatcmp_cases() {
        assert_eq!(call_int(fn_strnatcmp, &[s("img10"), s("img2")]), 1);
        assert_eq!(call_int(fn_strnatcmp, &[s("img2"), s("img10")]), -1);
        assert_eq!(call_int(fn_strnatcmp, &[s("img12"), s("img12")]), 0);
        assert_eq!(call_int(fn_strnatcmp, &[s("1.10"), s("1.2")]), 1);
        assert_eq!(call_int(fn_strnatcmp, &[s("10"), s("9")]), 1);
        assert_eq!(call_int(fn_strnatcmp, &[s("a0"), s("a00")]), -1);
        assert_eq!(call_int(fn_strnatcmp, &[s("a10"), s("a10b")]), -1);
        assert_eq!(call_int(fn_strnatcmp, &[s("  a"), s("a")]), 0);
        assert_eq!(call_int(fn_strnatcmp, &[s("0001"), s("1")]), 0);
        assert_eq!(call_int(fn_strnatcmp, &[s("file1"), s("file10")]), -1);
        assert_eq!(call_int(fn_strnatcmp, &[s("ver1.2.3"), s("ver1.2.10")]), -1);
        assert_eq!(call_int(fn_strnatcmp, &[s("x"), s("X")]), 1);
        assert_eq!(call_int(fn_strnatcmp, &[s("100"), s("99")]), 1);
        assert_eq!(call_int(fn_strnatcmp, &[s("T-5"), s("T-50")]), -1);
        assert_eq!(call_int(fn_strnatcmp, &[s("a1b2"), s("a1b10")]), -1);
        assert_eq!(call_int(fn_strnatcmp, &[s("z"), s("z1")]), -1);
        assert_eq!(call_int(fn_strnatcmp, &[s(""), s("a")]), -1);
        assert_eq!(call_int(fn_strnatcmp, &[s("a"), s("")]), 1);
        assert_eq!(call_int(fn_strnatcmp, &[s(""), s("")]), 0);
    }

    #[test]
    fn strnatcasecmp_cases() {
        assert_eq!(call_int(fn_strnatcasecmp, &[s("IMG10"), s("img2")]), 1);
        assert_eq!(call_int(fn_strnatcasecmp, &[s("Img10"), s("img10")]), 0);
        assert_eq!(call_int(fn_strnatcasecmp, &[s("x"), s("X")]), 0);
    }

    #[test]
    fn str_ireplace_cases() {
        assert_eq!(
            calls(fn_str_ireplace, &[s("WORLD"), s("PHP"), s("Hello world")]),
            "Hello PHP"
        );
        assert_eq!(
            calls(fn_str_ireplace, &[s("o"), s("0"), s("FOO Boo")]),
            "F00 B00"
        );
        assert_eq!(calls(fn_str_ireplace, &[s("aa"), s("b"), s("aaaa")]), "bb");
        assert_eq!(calls(fn_str_ireplace, &[s(""), s("X"), s("abc")]), "abc");
        // array search + array replace
        let se = {
            let mut a = PhpArray::new();
            a.push(s("a"));
            a.push(s("B"));
            Value::Array(a)
        };
        let re = {
            let mut a = PhpArray::new();
            a.push(s("X"));
            a.push(s("Y"));
            Value::Array(a)
        };
        assert_eq!(calls(fn_str_ireplace, &[se, re, s("aAbB")]), "XXYY");
        // array search + scalar replace
        let se2 = {
            let mut a = PhpArray::new();
            a.push(s("a"));
            a.push(s("b"));
            Value::Array(a)
        };
        assert_eq!(calls(fn_str_ireplace, &[se2, s("Z"), s("abAB")]), "ZZZZ");
        // scalar search + array replace -> TypeError
        let re2 = {
            let mut a = PhpArray::new();
            a.push(s("b"));
            Value::Array(a)
        };
        assert!(throws(fn_str_ireplace, &[s("a"), re2, s("abc")]));
        // array subject
        let subj = {
            let mut a = PhpArray::new();
            a.push(s("aA"));
            a.push(s("bca"));
            Value::Array(a)
        };
        let out = call(fn_str_ireplace, &[s("a"), s("X"), subj]);
        let Value::Array(arr) = out else {
            panic!("want array")
        };
        let got: Vec<String> = arr
            .iter()
            .map(|(_, v)| String::from_utf8(bytes_of(v)).unwrap())
            .collect();
        assert_eq!(got, vec!["XX".to_string(), "bcX".to_string()]);
    }

    #[test]
    fn strpbrk_cases() {
        assert_eq!(calls(fn_strpbrk, &[s("Hello World"), s("oW")]), "o World");
        assert_eq!(
            calls(fn_strpbrk, &[s("This is a test"), s("st")]),
            "s is a test"
        );
        assert_eq!(calls(fn_strpbrk, &[s("abcdef"), s("a")]), "abcdef");
        assert!(matches!(
            call(fn_strpbrk, &[s("Hello"), s("xyz")]),
            Value::Bool(false)
        ));
        assert!(throws(fn_strpbrk, &[s("abc"), s("")]));
    }

    #[test]
    fn strspn_cases() {
        assert_eq!(
            call_int(fn_strspn, &[s("42 is the answer"), s("1234567890")]),
            2
        );
        assert_eq!(call_int(fn_strspn, &[s("abcdef"), s("abc")]), 3);
        assert_eq!(
            call_int(fn_strspn, &[s("foo123bar"), s("0123456789"), Value::Int(3)]),
            3
        );
        assert_eq!(
            call_int(
                fn_strspn,
                &[
                    s("foo123bar"),
                    s("0123456789"),
                    Value::Int(3),
                    Value::Int(2)
                ]
            ),
            2
        );
        assert_eq!(
            call_int(
                fn_strspn,
                &[s("12345"), s("123"), Value::Int(0), Value::Int(10)]
            ),
            3
        );
        assert_eq!(
            call_int(fn_strspn, &[s("abcdef"), s("abc"), Value::Int(-3)]),
            0
        );
        assert_eq!(
            call_int(fn_strspn, &[s("abc"), s("abc"), Value::Int(10)]),
            0
        );
        assert_eq!(
            call_int(
                fn_strspn,
                &[s("aaa"), s("a"), Value::Int(1), Value::Int(10)]
            ),
            2
        );
    }

    #[test]
    fn strcspn_cases() {
        assert_eq!(call_int(fn_strcspn, &[s("hello, world"), s("o")]), 4);
        assert_eq!(call_int(fn_strcspn, &[s("hello"), s("xyz")]), 5);
        assert_eq!(
            call_int(
                fn_strcspn,
                &[s("hello world"), s("lo"), Value::Int(2), Value::Int(5)]
            ),
            0
        );
        assert_eq!(
            call_int(fn_strcspn, &[s("hello"), s("xyz"), Value::Int(-2)]),
            2
        );
        assert_eq!(call_int(fn_strcspn, &[s("abcABC"), s("abc")]), 0);
    }

    #[test]
    fn str_rot13_cases() {
        assert_eq!(calls(fn_str_rot13, &[s("Hello World")]), "Uryyb Jbeyq");
        assert_eq!(calls(fn_str_rot13, &[s("PHP 8.5")]), "CUC 8.5");
        assert_eq!(calls(fn_str_rot13, &[s("Az09!~")]), "Nm09!~");
        assert_eq!(calls(fn_str_rot13, &[s("mMnN zZ")]), "zZaA mM");
    }
}
