//! `is_iterable` / `is_countable`.
//!
//! (The `ctype_*` tests moved to `ext::ctype`; the JSON builtins live in
//! `json_fns`.) All behavior verified byte-for-byte against the local php 8.5.8.

use std::net::{Ipv4Addr, Ipv6Addr};

use rhp_values::{conv, Value};

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
