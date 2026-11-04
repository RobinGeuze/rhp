//! ext/standard/versioning.c: `version_compare`.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::zend::zpp::arg_bytes;
use crate::{Host, Thrown};

// --- version_compare ---------------------------------------------------------

/// `php_canonicalize_version` (ext/standard/versioning.c).
fn canonicalize_version(v: &[u8]) -> Vec<u8> {
    if v.is_empty() {
        return Vec::new();
    }
    let isdig = |c: u8| c.is_ascii_digit();
    let isndig = |c: u8| !c.is_ascii_digit() && c != b'.';
    let mut q: Vec<u8> = Vec::with_capacity(v.len() * 2);
    let mut lp = v[0];
    q.push(lp);
    for &c in &v[1..] {
        let lq = *q.last().unwrap();
        if matches!(c, b'-' | b'_' | b'+') {
            if lq != b'.' {
                q.push(b'.');
            }
        } else if (isndig(lp) && isdig(c)) || (isdig(lp) && isndig(c)) {
            if lq != b'.' {
                q.push(b'.');
            }
            q.push(c);
        } else if !c.is_ascii_alphanumeric() {
            if lq != b'.' {
                q.push(b'.');
            }
        } else {
            q.push(c);
        }
        lp = c;
    }
    if q.last() == Some(&b'.') {
        q.pop();
    }
    q
}

fn special_version_form(s: &[u8]) -> i32 {
    const FORMS: [(&[u8], i32); 10] = [
        (b"dev", 0),
        (b"alpha", 1),
        (b"a", 1),
        (b"beta", 2),
        (b"b", 2),
        (b"RC", 3),
        (b"rc", 3),
        (b"#", 4),
        (b"pl", 5),
        (b"p", 5),
    ];
    FORMS
        .iter()
        .find(|(n, _)| s.starts_with(n))
        .map_or(-1, |(_, o)| *o)
}

fn compare_special_version_forms(a: &[u8], b: &[u8]) -> i32 {
    (special_version_form(a) - special_version_form(b)).signum()
}

fn strtol_prefix(s: &[u8]) -> i128 {
    let mut n: i128 = 0;
    for &c in s {
        if !c.is_ascii_digit() {
            break;
        }
        n = (n * 10 + (c - b'0') as i128).min(i64::MAX as i128);
    }
    n
}

/// `php_version_compare` (ext/standard/versioning.c), ported step for step.
fn version_cmp(a: &[u8], b: &[u8]) -> i32 {
    let cut = |s: &[u8]| -> Vec<u8> {
        match s.iter().position(|&c| c == 0) {
            Some(i) => s[..i].to_vec(),
            None => s.to_vec(),
        }
    };
    let (a, b) = (cut(a), cut(b));
    if a.is_empty() || b.is_empty() {
        return if a.is_empty() && b.is_empty() {
            0
        } else if !a.is_empty() {
            1
        } else {
            -1
        };
    }
    let v1 = if a[0] == b'#' {
        a.clone()
    } else {
        canonicalize_version(&a)
    };
    let v2 = if b[0] == b'#' {
        b.clone()
    } else {
        canonicalize_version(&b)
    };
    let at = |v: &[u8], i: usize| v.get(i).copied().unwrap_or(0);
    let find_dot = |v: &[u8], p: usize| {
        v[p.min(v.len())..]
            .iter()
            .position(|&c| c == b'.')
            .map(|i| i + p)
    };
    let (mut p1, mut p2) = (0usize, 0usize);
    let (mut n1, mut n2): (Option<usize>, Option<usize>) = (Some(0), Some(0));
    let mut compare = 0;
    while at(&v1, p1) != 0 && at(&v2, p2) != 0 && n1.is_some() && n2.is_some() {
        n1 = find_dot(&v1, p1);
        n2 = find_dot(&v2, p2);
        let s1 = &v1[p1..n1.unwrap_or(v1.len())];
        let s2 = &v2[p2..n2.unwrap_or(v2.len())];
        let d1 = at(&v1, p1).is_ascii_digit();
        let d2 = at(&v2, p2).is_ascii_digit();
        compare = if d1 && d2 {
            (strtol_prefix(s1) - strtol_prefix(s2)).signum() as i32
        } else if !d1 && !d2 {
            compare_special_version_forms(s1, s2)
        } else if d1 {
            compare_special_version_forms(b"#N#", s2)
        } else {
            compare_special_version_forms(s1, b"#N#")
        };
        if compare != 0 {
            break;
        }
        if let Some(n) = n1 {
            p1 = n + 1;
        }
        if let Some(n) = n2 {
            p2 = n + 1;
        }
    }
    if compare == 0 {
        if n1.is_some() {
            compare = if at(&v1, p1).is_ascii_digit() {
                1
            } else {
                version_cmp(&v1[p1.min(v1.len())..], b"#N#")
            };
        } else if n2.is_some() {
            compare = if at(&v2, p2).is_ascii_digit() {
                -1
            } else {
                version_cmp(b"#N#", &v2[p2.min(v2.len())..])
            };
        }
    }
    compare
}

fn fn_version_compare(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = arg_bytes(args, 0);
    let b = arg_bytes(args, 1);
    let c = version_cmp(&a, &b);
    match args.get(2).map(deref) {
        None | Some(Value::Null) => Ok(Value::Int(c as i64)),
        Some(op) => {
            let op = conv::to_string_bytes(&op);
            let r = match op.as_slice() {
                b"<" | b"lt" => c == -1,
                b"<=" | b"le" => c != 1,
                b">" | b"gt" => c == 1,
                b">=" | b"ge" => c != -1,
                b"==" | b"=" | b"eq" => c == 0,
                b"!=" | b"<>" | b"ne" => c != 0,
                _ => {
                    return Err(Thrown::error(
                        "ValueError",
                        "version_compare(): Argument #3 ($operator) must be a valid comparison operator",
                    ))
                }
            };
            Ok(Value::Bool(r))
        }
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("version_compare", fn_version_compare)]);
}
