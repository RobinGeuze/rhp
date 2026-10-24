//! PHP operator + value-operation semantics, shared by both execution tiers
//! (the bytecode VM and the tree-walking oracle) so neither reimplements them.
//!
//! Depends only on `rhp-values` (which now owns `Thrown`), so it sits below
//! both tiers and below `rhp-builtins`.

use rhp_values::conv;
use rhp_values::{PhpArray, Value};

pub mod ops;

/// Deref a possible `Value::Ref` to the underlying value (clone).
pub fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        other => other.clone(),
    }
}

/// `(array)` cast: arrays pass through, null → empty, scalars → single element.
pub fn cast_to_array(v: Value) -> Value {
    match deref(&v) {
        Value::Array(a) => Value::Array(a),
        Value::Null => Value::Array(PhpArray::new()),
        // `(array)$object`: public properties become array elements keyed by
        // name (visibility name-mangling for private/protected is not modelled).
        Value::Object(h) => {
            let mut a = PhpArray::new();
            for (name, val) in &h.borrow().properties {
                a.insert(
                    rhp_values::ArrayKey::from_bytes(name.as_bytes()),
                    deref(val),
                );
            }
            Value::Array(a)
        }
        scalar => {
            let mut a = PhpArray::new();
            a.push(scalar);
            Value::Array(a)
        }
    }
}

/// PHP `++`/`--`: null++ => 1, null-- => null, numeric steps numerically;
/// non-numeric string increment (Perl-style) deferred (returns unchanged).
pub fn inc_dec_value(v: &Value, delta: i64) -> Value {
    match deref(v) {
        Value::Null => {
            if delta > 0 {
                Value::Int(1)
            } else {
                Value::Null
            }
        }
        Value::Int(n) => match n.checked_add(delta) {
            Some(r) => Value::Int(r),
            None => Value::Float(n as f64 + delta as f64),
        },
        Value::Float(f) => Value::Float(f + delta as f64),
        Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
            Some(conv::Num::Int(n)) => Value::Int(n + delta),
            Some(conv::Num::Float(f)) => Value::Float(f + delta as f64),
            None => Value::Str(s),
        },
        other => other,
    }
}
