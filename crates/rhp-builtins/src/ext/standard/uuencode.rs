//! ext/standard/uuencode.c: `convert_uuencode` and `convert_uudecode`.

use rhp_values::{conv, Value};

use crate::{Host, Thrown};

fn enc(c: u8) -> u8 {
    if c != 0 {
        (c & 0o77) + b' '
    } else {
        b'`'
    }
}

fn dec(c: u8) -> u8 {
    c.wrapping_sub(b' ') & 0o77
}

/// The byte at `i`, or the terminating NUL past the end.
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `php_uuencode`: lines of 45 bytes, each prefixed by its length.
fn uuencode(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() / 2 * 3 + 46);
    let e = src.len();
    let mut s = 0;
    let mut len = 45usize;
    let enc_c2 = |s: usize| enc(((at(src, s) << 4) & 0o60) | ((at(src, s + 1) >> 4) & 0o17));
    let enc_c3 = |s: usize| enc(((at(src, s + 1) << 2) & 0o74) | ((at(src, s + 2) >> 6) & 0o3));
    while s + 3 < e {
        let mut ee = s + len;
        if ee > e {
            ee = e;
            len = ee - s;
            if len % 3 != 0 {
                ee = s + len / 3 * 3;
            }
        }
        out.push(enc(len as u8));
        while s < ee {
            out.push(enc(at(src, s) >> 2));
            out.push(enc_c2(s));
            out.push(enc_c3(s));
            out.push(enc(at(src, s + 2) & 0o77));
            s += 3;
        }
        if len == 45 {
            out.push(b'\n');
        }
    }
    if s < e {
        if len == 45 {
            out.push(enc((e - s) as u8));
            len = 0;
        }
        out.push(enc(at(src, s) >> 2));
        out.push(enc_c2(s));
        out.push(if e - s > 1 { enc_c3(s) } else { enc(0) });
        out.push(if e - s > 2 {
            enc(at(src, s + 2) & 0o77)
        } else {
            enc(0)
        });
    }
    if len < 45 {
        out.push(b'\n');
    }
    out.push(enc(0));
    out.push(b'\n');
    out
}

/// `php_uudecode`: `None` for malformed input.
fn uudecode(src: &[u8]) -> Option<Vec<u8>> {
    if src.is_empty() {
        return None;
    }
    let e = src.len();
    let mut out = Vec::with_capacity(e * 3 / 4 + 1);
    let mut total_len = 0usize;
    let mut s = 0usize;
    while s < e {
        let len = dec(src[s]) as usize;
        s += 1;
        if len == 0 {
            break;
        }
        if len > src.len() {
            return None;
        }
        total_len += len;
        let ee = s + if len == 45 {
            60
        } else {
            (len as f64 * 1.33).floor() as usize
        };
        if ee > e {
            return None;
        }
        while s < ee {
            if s + 4 > e {
                return None;
            }
            out.push(dec(src[s]) << 2 | dec(src[s + 1]) >> 4);
            out.push(dec(src[s + 1]) << 4 | dec(src[s + 2]) >> 2);
            out.push(dec(src[s + 2]) << 6 | dec(src[s + 3]));
            s += 4;
        }
        if len < 45 {
            break;
        }
        s += 1; // the newline
    }
    if total_len > out.len() {
        out.push(dec(at(src, s)) << 2 | dec(at(src, s + 1)) >> 4);
        if total_len > 1 {
            out.push(dec(at(src, s + 1)) << 4 | dec(at(src, s + 2)) >> 2);
            if total_len > 2 {
                out.push(dec(at(src, s + 2)) << 6 | dec(at(src, s + 3)));
            }
        }
    }
    out.resize(total_len, 0);
    Some(out)
}

fn fn_convert_uuencode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(uuencode(&conv::to_string_bytes(&args[0]))))
}

fn fn_convert_uudecode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match uudecode(&conv::to_string_bytes(&args[0])) {
        Some(out) => Ok(Value::str(out)),
        None => {
            host.docref_diagnostic(
                crate::E_WARNING,
                b"Argument #1 ($data) is not a valid uuencoded string",
            )?;
            Ok(Value::Bool(false))
        }
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("convert_uuencode", fn_convert_uuencode),
            ("convert_uudecode", fn_convert_uudecode),
        ],
    );
}
