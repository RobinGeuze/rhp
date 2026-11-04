//! ext/standard/hrtime.c: `hrtime`.

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, Value};

use crate::{Host, Thrown};

/// `hrtime(bool $as_number = false)` — high-resolution time. As a monotonic-ish
/// value derived from the wall clock (no separate monotonic source needed here).
fn fn_hrtime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let as_number = args
        .first()
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    if as_number {
        Ok(Value::Int(now.as_nanos() as i64))
    } else {
        let mut a = PhpArray::new();
        a.push(Value::Int(now.as_secs() as i64));
        a.push(Value::Int(now.subsec_nanos() as i64));
        Ok(Value::Array(a))
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("hrtime", fn_hrtime)]);
}
