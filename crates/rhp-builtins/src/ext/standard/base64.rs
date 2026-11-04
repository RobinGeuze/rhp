//! ext/standard/base64.c: `base64_encode`/`base64_decode`, php's own code,
//! ported.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// base64 (RFC 4648) — implemented by hand
// ---------------------------------------------------------------------------

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize]);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize]);
        if chunk.len() > 1 {
            out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize]);
        } else {
            out.push(b'=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHABET[(n & 0x3f) as usize]);
        } else {
            out.push(b'=');
        }
    }
    out
}

/// php's reverse table: 0..63 = value, -1 = whitespace (skip), -2 = invalid.
/// '=' is handled by the caller before lookup.
fn b64_reverse(byte: u8) -> i16 {
    match byte {
        b'A'..=b'Z' => (byte - b'A') as i16,
        b'a'..=b'z' => (byte - b'a') as i16 + 26,
        b'0'..=b'9' => (byte - b'0') as i16 + 52,
        b'+' => 62,
        b'/' => 63,
        b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r' => -1,
        _ => -2,
    }
}

/// Mirrors php's `php_base64_decode_ex`. Returns `None` for php `false`.
pub(crate) fn base64_decode(input: &[u8], strict: bool) -> Option<Vec<u8>> {
    let mut out = vec![0u8; input.len() + 4];
    let mut i: usize = 0; // count of data (non-pad) chars consumed
    let mut j: usize = 0; // output byte index
    let mut padding: usize = 0;

    for &byte in input {
        if byte == b'=' {
            padding += 1;
            continue;
        }
        let ch = b64_reverse(byte);
        if !strict {
            // skip unknown characters and whitespace
            if ch < 0 {
                continue;
            }
        } else {
            // skip whitespace
            if ch == -1 {
                continue;
            }
            // fail on bad characters or if any data follows padding
            if ch == -2 || padding > 0 {
                return None;
            }
        }
        let ch = ch as u8;
        match i % 4 {
            0 => out[j] = ch << 2,
            1 => {
                out[j] |= ch >> 4;
                j += 1;
                out[j] = (ch & 0x0f) << 4;
            }
            2 => {
                out[j] |= ch >> 2;
                j += 1;
                out[j] = (ch & 0x03) << 6;
            }
            _ => {
                out[j] |= ch;
                j += 1;
            }
        }
        i += 1;
    }

    if strict {
        // truncated final group (a single leftover char) is invalid
        if i % 4 == 1 {
            return None;
        }
        // padding must be well-formed and land the total on a 4-char boundary
        if padding > 0 && (padding > 2 || !(i + padding).is_multiple_of(4)) {
            return None;
        }
    }

    out.truncate(j);
    Some(out)
}

fn fn_base64_encode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    Ok(Value::str(base64_encode(&s)))
}

fn fn_base64_decode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let strict = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    match base64_decode(&s, strict) {
        Some(bytes) => Ok(Value::str(bytes)),
        None => Ok(Value::Bool(false)),
    }
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register all hashing + binary-encoding builtins into the shared table.
pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("base64_encode", fn_base64_encode),
            ("base64_decode", fn_base64_decode),
        ],
    );
}

#[cfg(test)]
mod tests;
