//! ext/standard/quot_print.c: `quoted_printable_decode` and
//! `quoted_printable_encode`.

use rhp_values::{conv, Value};

use crate::{Host, Thrown};

/// The byte at `i`, or the terminating NUL past the end (php reads the
/// string as a C string).
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

fn hex2int(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'A'..=b'F' => c - b'A' + 10,
        _ => c - b'a' + 10,
    }
}

/// `quoted_printable_decode(string $string): string`; stops at a NUL byte.
fn fn_quoted_printable_decode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while at(&s, i) != 0 {
        if s[i] == b'='
            && at(&s, i + 1) != 0
            && at(&s, i + 2) != 0
            && s[i + 1].is_ascii_hexdigit()
            && s[i + 2].is_ascii_hexdigit()
        {
            out.push((hex2int(s[i + 1]) << 4) + hex2int(s[i + 2]));
            i += 3;
        } else if s[i] == b'=' {
            // A soft line break (RFC 2045), after optional spaces and tabs.
            let mut k = 1;
            while matches!(at(&s, i + k), b' ' | b'\t') {
                k += 1;
            }
            match at(&s, i + k) {
                0 => i += k,
                b'\r' if at(&s, i + k + 1) == b'\n' => i += k + 2,
                b'\r' | b'\n' => i += k + 1,
                _ => {
                    out.push(s[i]);
                    i += 1;
                }
            }
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    Ok(Value::str(out))
}

const PHP_QPRINT_MAXL: u64 = 75;

/// `php_quot_print_encode`: soft breaks keep lines at 75 bytes and never
/// split a UTF-8 sequence.
fn fn_quoted_printable_encode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let s = conv::to_string_bytes(&args[0]);
    let mut out = Vec::with_capacity(s.len() * 3);
    let mut lp: u64 = 0;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        i += 1;
        let next = at(&s, i);
        if c == b'\r' && next == b'\n' && i < s.len() {
            out.extend_from_slice(b"\r\n");
            i += 1;
            lp = 0;
        } else if c.is_ascii_control() || c & 0x80 != 0 || c == b'=' || (c == b' ' && next == b'\r')
        {
            lp += 3;
            if (lp > PHP_QPRINT_MAXL && c <= 0x7f)
                || ((0x80..=0xdf).contains(&c) && lp + 3 > PHP_QPRINT_MAXL)
                || ((0xe0..=0xef).contains(&c) && lp + 6 > PHP_QPRINT_MAXL)
                || ((0xf0..=0xf4).contains(&c) && lp + 9 > PHP_QPRINT_MAXL)
            {
                out.extend_from_slice(b"=\r\n");
                lp = 3;
            }
            out.extend_from_slice(&[b'=', HEX[(c >> 4) as usize], HEX[(c & 0xf) as usize]]);
        } else {
            lp += 1;
            if lp > PHP_QPRINT_MAXL {
                out.extend_from_slice(b"=\r\n");
                lp = 1;
            }
            out.push(c);
        }
    }
    Ok(Value::str(out))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("quoted_printable_decode", fn_quoted_printable_decode),
            ("quoted_printable_encode", fn_quoted_printable_encode),
        ],
    );
}
