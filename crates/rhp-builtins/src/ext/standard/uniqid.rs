//! ext/standard/uniqid.c: `uniqid`.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::ext::random::rand_range;
use crate::{Host, Thrown};

thread_local! {
    /// The last time handed out: uniqid waits for the microsecond to change.
    static PREV: std::cell::Cell<(u64, u32)> = const { std::cell::Cell::new((0, 0)) };
}

/// `uniqid(string $prefix = "", bool $more_entropy = false)`: "%s%08x%05x",
/// plus "%.8F" of a random seed in [0, 10] with more entropy.
fn fn_uniqid(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let prefix = args
        .first()
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_default();
    let more = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    let (sec, usec) = loop {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let t = (now.as_secs(), now.subsec_micros());
        if t != PREV.with(|p| p.get()) {
            PREV.with(|p| p.set(t));
            break t;
        }
    };
    let mut s = prefix;
    s.extend_from_slice(format!("{:08x}{:05x}", sec as u32, usec % 0x10_0000).as_bytes());
    if more {
        let bytes = rand_range(0, u32::MAX as i64) as u32;
        let seed = (bytes as f64 / u32::MAX as f64) * 10.0;
        s.extend_from_slice(format!("{seed:.8}").as_bytes());
    }
    Ok(Value::str(s))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("uniqid", fn_uniqid)]);
}
