//! Core hashing + binary-encoding primitives for the `hash` extension
//! (`md5`, `sha1`, `crc32`, `base64_encode`/`decode`, `bin2hex`/`hex2bin`).
//!
//! md5 / sha1 / sha256 / crc32 are implemented by hand (no external crates) and
//! reused by the `hash()` dispatch in the parent module. Every function here is
//! verified byte-for-byte against the local php 8.5.8 binary (see the
//! `#[cfg(test)]` block). Registered via `super::register`.

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
// MD5 (RFC 1321) — implemented by hand
// ---------------------------------------------------------------------------

pub(crate) fn md5(msg: &[u8]) -> [u8; 16] {
    // per-round left-rotate amounts
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, //
        5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, //
        4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, //
        6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    // binary integer parts of the sines of integers (radians)
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    // padding: append 0x80, then zeros, then 64-bit little-endian bit length.
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    let mut data = msg.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in data.as_chunks::<64>().0 {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }

        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }

        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

// ---------------------------------------------------------------------------
// SHA-1 (FIPS 180-1) — implemented by hand
// ---------------------------------------------------------------------------

pub(crate) fn sha1(msg: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];

    // padding: 0x80, zeros, 64-bit big-endian bit length.
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    let mut data = msg.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in data.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// SHA-256 (FIPS 180-4) — implemented by hand
// ---------------------------------------------------------------------------

pub(crate) fn sha256(msg: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (msg.len() as u64).wrapping_mul(8);
    let mut data = msg.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in data.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6];
            v[6] = v[5];
            v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2];
            v[2] = v[1];
            v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }

        for (hi, vi) in h.iter_mut().zip(v.iter()) {
            *hi = hi.wrapping_add(*vi);
        }
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// CRC-32 (IEEE 802.3, reflected) — implemented by hand
// ---------------------------------------------------------------------------

/// Standard CRC-32 (reflected polynomial 0xEDB88320, init/final XOR 0xFFFFFFFF).
/// This is php's `crc32()` value and also `hash("crc32b", ...)`.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// base64 (RFC 4648) — implemented by hand
// ---------------------------------------------------------------------------

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> Vec<u8> {
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
    let digest = md5(&s);
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
    let digest = sha1(&s);
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
    Ok(Value::Int(crc32(&s) as i64))
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
// tests — expected values captured from php 8.5.8
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
