//! `md5`, `sha1`, `crc32`, `base64_encode`/`decode` and `bin2hex`/`hex2bin`.
//!
//! md5 and sha1 come from RustCrypto, crc32 from the linked zlib. base64 and
//! hex are php's own code, ported. Registered via `super::register`.

use std::collections::HashMap;

use digest::Digest;

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

// ---------------------------------------------------------------------------
// builtin wrappers
// ---------------------------------------------------------------------------

fn fn_md5(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let digest = md5::Md5::digest(&s);
    let binary = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    Ok(Value::str(if binary {
        digest.to_vec()
    } else {
        to_hex(&digest)
    }))
}

fn fn_sha1(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let digest = sha1::Sha1::digest(&s);
    let binary = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    Ok(Value::str(if binary {
        digest.to_vec()
    } else {
        to_hex(&digest)
    }))
}

fn fn_crc32(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    Ok(Value::Int(crate::ext::zlib::zraw::crc32(&s) as i64))
}

fn fn_base64_encode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    Ok(Value::str(base64_encode(&s)))
}

fn fn_base64_decode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let strict = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    match base64_decode(&s, strict) {
        Some(bytes) => Ok(Value::str(bytes)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_bin2hex(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    Ok(Value::str(to_hex(&s)))
}

fn fn_hex2bin(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
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
        let hi = hex_val(pair[0]);
        let lo = hex_val(pair[1]);
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

fn hex_val(b: u8) -> Option<u8> {
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

/// Register all hashing + binary-encoding builtins into the shared table.
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
    put(m, "md5", fn_md5, 1, Some(2));
    put(m, "sha1", fn_sha1, 1, Some(2));
    put(m, "crc32", fn_crc32, 1, Some(1));
    put(m, "base64_encode", fn_base64_encode, 1, Some(1));
    put(m, "base64_decode", fn_base64_decode, 1, Some(2));
    put(m, "bin2hex", fn_bin2hex, 1, Some(1));
    put(m, "hex2bin", fn_hex2bin, 1, Some(1));
}

// ---------------------------------------------------------------------------
// tests: expected values from php
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
    fn calls(f: crate::NativeFn, args: &[Value]) -> String {
        String::from_utf8(bytes_of(&call(f, args))).unwrap()
    }
    fn call_int(f: crate::NativeFn, args: &[Value]) -> i64 {
        match call(f, args) {
            Value::Int(n) => n,
            other => panic!("expected int, got {}", other.type_name()),
        }
    }
    /// True when the result is php `false` (used for failure returns).
    fn is_false(f: crate::NativeFn, args: &[Value]) -> bool {
        matches!(call(f, args), Value::Bool(false))
    }

    #[test]
    fn md5_vectors() {
        assert_eq!(calls(fn_md5, &[s("")]), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(
            calls(fn_md5, &[s("abc")]),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            calls(fn_md5, &[s("message digest")]),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(
            calls(fn_md5, &[s("The quick brown fox jumps over the lazy dog")]),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        // binary form: bin2hex(md5($s, true)) == md5($s)
        let bin = call(fn_md5, &[s("abc"), Value::Bool(true)]);
        assert_eq!(bytes_of(&bin).len(), 16);
        assert_eq!(
            String::from_utf8(to_hex(&bytes_of(&bin))).unwrap(),
            "900150983cd24fb0d6963f7d28e17f72"
        );
    }

    #[test]
    fn sha1_vectors() {
        assert_eq!(
            calls(fn_sha1, &[s("")]),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            calls(fn_sha1, &[s("abc")]),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            calls(fn_sha1, &[s("The quick brown fox jumps over the lazy dog")]),
            "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
        );
        assert_eq!(
            calls(
                fn_sha1,
                &[s(
                    "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
                )]
            ),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        let bin = call(fn_sha1, &[s("abc"), Value::Bool(true)]);
        assert_eq!(bytes_of(&bin).len(), 20);
    }

    #[test]
    fn crc32_vectors() {
        assert_eq!(call_int(fn_crc32, &[s("")]), 0);
        assert_eq!(call_int(fn_crc32, &[s("abc")]), 891568578);
        assert_eq!(
            call_int(
                fn_crc32,
                &[s("The quick brown fox jumps over the lazy dog")]
            ),
            1095738169
        );
    }

    #[test]
    fn base64_encode_vectors() {
        assert_eq!(calls(fn_base64_encode, &[s("")]), "");
        assert_eq!(calls(fn_base64_encode, &[s("f")]), "Zg==");
        assert_eq!(calls(fn_base64_encode, &[s("fo")]), "Zm8=");
        assert_eq!(calls(fn_base64_encode, &[s("foo")]), "Zm9v");
        assert_eq!(calls(fn_base64_encode, &[s("foob")]), "Zm9vYg==");
        assert_eq!(calls(fn_base64_encode, &[s("fooba")]), "Zm9vYmE=");
        assert_eq!(calls(fn_base64_encode, &[s("foobar")]), "Zm9vYmFy");
        assert_eq!(
            calls(fn_base64_encode, &[s("Hello, World!")]),
            "SGVsbG8sIFdvcmxkIQ=="
        );
        assert_eq!(
            calls(
                fn_base64_encode,
                &[Value::str(vec![0x00, 0xff, 0x80, 0x7f])]
            ),
            "AP+Afw=="
        );
    }

    #[test]
    fn base64_decode_vectors() {
        assert_eq!(calls(fn_base64_decode, &[s("Zm9vYmFy")]), "foobar");
        assert_eq!(
            calls(fn_base64_decode, &[s("SGVsbG8sIFdvcmxkIQ==")]),
            "Hello, World!"
        );
        assert_eq!(calls(fn_base64_decode, &[s("")]), "");
        assert_eq!(calls(fn_base64_decode, &[s("====")]), "");
        // non-strict: skip invalid chars / whitespace
        assert_eq!(calls(fn_base64_decode, &[s("Zm9v!!")]), "foo");
        assert_eq!(calls(fn_base64_decode, &[s("Zm 9v")]), "foo");
        assert_eq!(calls(fn_base64_decode, &[s("QQ")]), "A");
        // strict: invalid chars fail, whitespace ok
        assert!(is_false(
            fn_base64_decode,
            &[s("Zm9v!!"), Value::Bool(true)]
        ));
        assert_eq!(
            calls(fn_base64_decode, &[s("Zm 9v"), Value::Bool(true)]),
            "foo"
        );
        assert_eq!(
            calls(fn_base64_decode, &[s("Zm9vYg=="), Value::Bool(true)]),
            "foob"
        );
        assert!(is_false(
            fn_base64_decode,
            &[s("Zm9vYg="), Value::Bool(true)]
        ));
        assert_eq!(
            calls(fn_base64_decode, &[s("Zm9vYg"), Value::Bool(true)]),
            "foob"
        );
        assert!(is_false(fn_base64_decode, &[s("Z==="), Value::Bool(true)]));
        assert!(is_false(fn_base64_decode, &[s("A"), Value::Bool(true)]));
        assert_eq!(calls(fn_base64_decode, &[s("QQ"), Value::Bool(true)]), "A");
    }

    #[test]
    fn base64_roundtrip_binary() {
        // decode non-strict of "ab=c" -> bytes 69 b7
        let v = call(fn_base64_decode, &[s("ab=c")]);
        assert_eq!(bytes_of(&v), vec![0x69, 0xb7]);
        let v = call(fn_base64_decode, &[s("YQ==YQ==")]);
        assert_eq!(bytes_of(&v), vec![0x61, 0x06, 0x10]);
    }

    #[test]
    fn bin2hex_hex2bin_vectors() {
        assert_eq!(calls(fn_bin2hex, &[s("abc")]), "616263");
        assert_eq!(calls(fn_bin2hex, &[s("")]), "");
        assert_eq!(
            calls(fn_bin2hex, &[Value::str(vec![0xff, 0x00, 0x10])]),
            "ff0010"
        );
        assert_eq!(calls(fn_hex2bin, &[s("616263")]), "abc");
        assert_eq!(calls(fn_hex2bin, &[s("")]), "");
        assert_eq!(
            bytes_of(&call(fn_hex2bin, &[s("ff0010")])),
            vec![0xff, 0x00, 0x10]
        );
        // odd length -> false
        assert!(is_false(fn_hex2bin, &[s("abc")]));
        // non-hex -> false
        assert!(is_false(fn_hex2bin, &[s("xy")]));
    }
}
