//! `is_iterable` / `is_countable`.
//!
//! (The `ctype_*` tests moved to `ext::ctype`; the JSON builtins live in
//! `json_fns`.) All behavior verified byte-for-byte against the local php 8.5.8.

use phrust_values::Value;

use crate::{Builtin, Host, Thrown};

/// Local deref: unwrap a `Ref` to an owned clone of its target.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
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
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn arr(items: &[Value]) -> Value {
        let mut a = phrust_values::PhpArray::new();
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
