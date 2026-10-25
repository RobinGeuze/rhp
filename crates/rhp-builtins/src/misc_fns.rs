//! `is_iterable` / `is_countable`.
//!
//! (The `ctype_*` tests moved to `ext::ctype`; the JSON builtins live in
//! `json_fns`.) All behavior verified byte-for-byte against the local php 8.5.8.

use std::net::{Ipv4Addr, Ipv6Addr};

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

/// Local deref: unwrap a `Ref` to an owned clone of its target.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

// ---------------------------------------------------------------------------
// inet_pton / inet_ntop
// ---------------------------------------------------------------------------

/// `inet_pton(string $ip): string|false` — IPv4/IPv6 text to packed binary.
fn fn_inet_pton(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ip = conv::to_string_bytes(&deref(&args[0]));
    let s = match std::str::from_utf8(&ip) {
        Ok(s) => s,
        Err(_) => return Ok(Value::Bool(false)),
    };
    if let Ok(v4) = s.parse::<Ipv4Addr>() {
        return Ok(Value::str(v4.octets().to_vec()));
    }
    if let Ok(v6) = s.parse::<Ipv6Addr>() {
        return Ok(Value::str(v6.octets().to_vec()));
    }
    Ok(Value::Bool(false))
}

/// `inet_ntop(string $binary): string|false` — packed binary to IP text.
fn fn_inet_ntop(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let b = conv::to_string_bytes(&deref(&args[0]));
    match b.len() {
        4 => {
            let o: [u8; 4] = [b[0], b[1], b[2], b[3]];
            Ok(Value::str(Ipv4Addr::from(o).to_string().into_bytes()))
        }
        16 => {
            let mut o = [0u8; 16];
            o.copy_from_slice(&b);
            Ok(Value::str(Ipv6Addr::from(o).to_string().into_bytes()))
        }
        _ => Ok(Value::Bool(false)),
    }
}

// ---------------------------------------------------------------------------
// is_iterable / is_countable
// ---------------------------------------------------------------------------

// Only arrays qualify until Traversable/Countable objects land (phase 7); php
// also returns true for objects implementing those interfaces.
fn fn_is_iterable(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(matches!(deref(&args[0]), Value::Array(_))))
}

fn fn_is_countable(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(matches!(deref(&args[0]), Value::Array(_))))
}

// ---------------------------------------------------------------------------
// pack / unpack  (subset of format codes — enough for Composer)
// ---------------------------------------------------------------------------

/// Read `n` bytes at `off` as an unsigned integer, big- or little-endian.
fn read_uint(data: &[u8], off: usize, n: usize, be: bool) -> Option<u64> {
    if off + n > data.len() {
        return None;
    }
    let mut v: u64 = 0;
    if be {
        for i in 0..n {
            v = (v << 8) | data[off + i] as u64;
        }
    } else {
        for i in 0..n {
            v |= (data[off + i] as u64) << (8 * i);
        }
    }
    Some(v)
}

fn sign_extend(v: u64, n: usize) -> i64 {
    let bits = n * 8;
    let shift = 64 - bits;
    ((v << shift) as i64) >> shift
}

/// `unpack(string $format, string $data, int $offset = 0): array|false`
fn fn_unpack(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let format = conv::to_string_bytes(&deref(&args[0]));
    let data = conv::to_string_bytes(&deref(&args[1]));
    let mut pos = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0).max(0) as usize;

    let mut out = PhpArray::new();
    let fmt = String::from_utf8_lossy(&format);
    for part in fmt.split('/') {
        if part.is_empty() {
            continue;
        }
        let bytes = part.as_bytes();
        let code = bytes[0] as char;
        // optional repeat count (digits or '*'), then the name.
        let mut i = 1;
        let mut star = false;
        let mut count_str = String::new();
        if i < bytes.len() && bytes[i] == b'*' {
            star = true;
            i += 1;
        } else {
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                count_str.push(bytes[i] as char);
                i += 1;
            }
        }
        let name = &part[i..];
        let count: usize = if star {
            0 // resolved per-code below
        } else if count_str.is_empty() {
            1
        } else {
            count_str.parse().unwrap_or(1)
        };

        // size (bytes) + endianness + signedness per code.
        let (size, be, signed) = match code {
            'C' => (1usize, false, false),
            'c' => (1, false, true),
            'n' => (2, true, false),
            'v' => (2, false, false),
            's' => (2, false, true),
            'S' => (2, false, false),
            'N' => (4, true, false),
            'V' => (4, false, false),
            'l' => (4, false, true),
            'L' | 'I' => (4, false, false),
            'i' => (4, false, true),
            'J' => (8, true, false),
            'P' => (8, false, false),
            'q' => (8, false, true),
            'Q' => (8, false, false),
            'a' | 'A' | 'Z' => {
                // string: count = length (or rest for '*').
                let len = if star { data.len().saturating_sub(pos) } else { count };
                let end = (pos + len).min(data.len());
                let mut s = data[pos..end].to_vec();
                pos = end;
                if code == 'A' {
                    while matches!(s.last(), Some(b' ') | Some(0)) {
                        s.pop();
                    }
                } else if code == 'Z' {
                    if let Some(z) = s.iter().position(|&b| b == 0) {
                        s.truncate(z);
                    }
                }
                let key = if name.is_empty() { "1".to_string() } else { name.to_string() };
                out.insert(ArrayKey::from_bytes(key.as_bytes()), Value::str(s));
                continue;
            }
            _ => return Ok(Value::Bool(false)),
        };

        let n = if star {
            (data.len().saturating_sub(pos)) / size
        } else {
            count
        };
        for idx in 0..n {
            let raw = match read_uint(&data, pos, size, be) {
                Some(v) => v,
                None => break,
            };
            pos += size;
            let val = if signed { Value::Int(sign_extend(raw, size)) } else { Value::Int(raw as i64) };
            let key = if name.is_empty() {
                (idx + 1).to_string()
            } else if star || count > 1 {
                format!("{}{}", name, idx + 1)
            } else {
                name.to_string()
            };
            out.insert(ArrayKey::from_bytes(key.as_bytes()), val);
        }
    }
    Ok(Value::Array(out))
}

/// `pack(string $format, mixed ...$values): string` — subset of codes.
fn fn_pack(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let format = conv::to_string_bytes(&deref(&args[0]));
    let vals = &args[1..];
    let mut vi = 0usize;
    let mut out: Vec<u8> = Vec::new();
    let fmt = String::from_utf8_lossy(&format).into_owned();
    let chars: Vec<char> = fmt.chars().collect();
    let mut ci = 0;
    while ci < chars.len() {
        let code = chars[ci];
        ci += 1;
        let mut star = false;
        let mut count_str = String::new();
        if ci < chars.len() && chars[ci] == '*' {
            star = true;
            ci += 1;
        } else {
            while ci < chars.len() && chars[ci].is_ascii_digit() {
                count_str.push(chars[ci]);
                ci += 1;
            }
        }
        let count: usize = if count_str.is_empty() { 1 } else { count_str.parse().unwrap_or(1) };
        let push_int = |out: &mut Vec<u8>, v: u64, size: usize, be: bool| {
            if be {
                for i in (0..size).rev() {
                    out.push((v >> (8 * i)) as u8);
                }
            } else {
                for i in 0..size {
                    out.push((v >> (8 * i)) as u8);
                }
            }
        };
        let (size, be) = match code {
            'C' | 'c' => (1, false),
            'n' => (2, true),
            'v' => (2, false),
            's' | 'S' => (2, false),
            'N' => (4, true),
            'V' => (4, false),
            'l' | 'L' | 'i' | 'I' => (4, false),
            'J' => (8, true),
            'P' | 'q' | 'Q' => (8, false),
            'a' | 'A' | 'Z' => {
                let s = vals.get(vi).map(|v| conv::to_string_bytes(&deref(v))).unwrap_or_default();
                vi += 1;
                let len = if star { s.len() + if code == 'Z' { 1 } else { 0 } } else { count };
                let pad = if code == 'A' { b' ' } else { 0u8 };
                for i in 0..len {
                    out.push(*s.get(i).unwrap_or(&pad));
                }
                continue;
            }
            _ => return Ok(Value::Bool(false)),
        };
        let reps = if star { vals.len().saturating_sub(vi) } else { count };
        for _ in 0..reps {
            let v = vals.get(vi).map(|x| conv::to_int(&deref(x))).unwrap_or(0) as u64;
            vi += 1;
            push_int(&mut out, v, size, be);
        }
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register this module's builtins into the engine's table.
pub fn register(m: &mut std::collections::HashMap<&'static str, Builtin>) {
    macro_rules! reg {
        ($name:literal, $func:path, $min:literal, $max:expr) => {
            m.insert($name, Builtin { name: $name, func: $func, min_args: $min, max_args: $max });
        };
    }
    reg!("is_iterable", fn_is_iterable, 1, Some(1));
    reg!("is_countable", fn_is_countable, 1, Some(1));
    reg!("inet_pton", fn_inet_pton, 1, Some(1));
    reg!("inet_ntop", fn_inet_ntop, 1, Some(1));
    reg!("unpack", fn_unpack, 2, Some(3));
    reg!("pack", fn_pack, 1, None);
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn arr(items: &[Value]) -> Value {
        let mut a = rhp_values::PhpArray::new();
        for v in items {
            a.push(v.clone());
        }
        Value::Array(a)
    }

    struct NoopHost;
    impl Host for NoopHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    #[test]
    fn iterable_countable() {
        assert!(matches!(fn_is_iterable(&mut NoopHost, &[arr(&[])]).unwrap(), Value::Bool(true)));
        assert!(matches!(
            fn_is_iterable(&mut NoopHost, &[Value::str("abc")]).unwrap(),
            Value::Bool(false)
        ));
        assert!(matches!(
            fn_is_countable(&mut NoopHost, &[arr(&[Value::Int(1)])]).unwrap(),
            Value::Bool(true)
        ));
        assert!(matches!(
            fn_is_countable(&mut NoopHost, &[Value::Int(5)]).unwrap(),
            Value::Bool(false)
        ));
    }
}
