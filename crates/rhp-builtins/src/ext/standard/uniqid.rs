//! ext/standard/uniqid.c: `uniqid`.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::php_fns::rand_range;
use crate::{Host, Thrown};

fn fn_uniqid(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let prefix = args
        .first()
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_default();
    let more = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let mut s = prefix;
    // php: 8 hex of seconds + 5 hex of microseconds.
    s.extend_from_slice(
        format!(
            "{:08x}{:05x}",
            now.as_secs() & 0xffff_ffff,
            now.subsec_micros() & 0xf_ffff
        )
        .as_bytes(),
    );
    if more {
        s.extend_from_slice(format!(".{:08}", rand_range(0, 99999999)).as_bytes());
    }
    Ok(Value::str(s))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("uniqid", fn_uniqid)]);
}
