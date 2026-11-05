//! ext/standard/microtime.c: `microtime`, `gettimeofday` and `getrusage`.

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::zend::zpp::{bool_param, int_param};
use crate::{Host, Thrown};

/// `microtime(bool $as_float = false)` — current time with microseconds.
fn fn_microtime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let as_float = args
        .first()
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    if as_float {
        Ok(Value::Float(now.as_secs_f64()))
    } else {
        // php format: "0.xxxxxxxx sssssssss" (microsecond fraction, then seconds).
        Ok(Value::str(
            format!(
                "{:.8} {}",
                now.subsec_micros() as f64 / 1_000_000.0,
                now.as_secs()
            )
            .into_bytes(),
        ))
    }
}

fn fn_gettimeofday(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let as_float = bool_param(host, args, 0, "gettimeofday", "as_float", false)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    if as_float {
        return Ok(Value::Float(
            now.as_secs() as f64 + now.subsec_micros() as f64 / 1_000_000.0,
        ));
    }
    let mut a = PhpArray::new();
    a.insert(
        ArrayKey::from_bytes(b"sec"),
        Value::Int(now.as_secs() as i64),
    );
    a.insert(
        ArrayKey::from_bytes(b"usec"),
        Value::Int(now.subsec_micros() as i64),
    );
    // The default zone's offset at that second.
    let (west, dst) = crate::ext::date::get_timezone_info().map_or((0, 0), |tz| {
        let o = tz.offset_at(now.as_secs() as i64);
        (-(o.offset as i64) / 60, o.is_dst as i64)
    });
    a.insert(ArrayKey::from_bytes(b"minuteswest"), Value::Int(west));
    a.insert(ArrayKey::from_bytes(b"dsttime"), Value::Int(dst));
    Ok(Value::Array(a))
}

// --- process introspection ---------------------------------------------------

fn fn_getrusage(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mode = int_param(host, args, 0, "getrusage", "mode", 0)?;
    let who = if mode == 1 {
        libc::RUSAGE_CHILDREN
    } else {
        libc::RUSAGE_SELF
    };
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(who, &mut u) } == -1 {
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: i64| a.insert(ArrayKey::from_bytes(k.as_bytes()), Value::Int(v));
    put("ru_oublock", u.ru_oublock as i64);
    put("ru_inblock", u.ru_inblock as i64);
    put("ru_msgsnd", u.ru_msgsnd as i64);
    put("ru_msgrcv", u.ru_msgrcv as i64);
    put("ru_maxrss", u.ru_maxrss as i64);
    put("ru_ixrss", u.ru_ixrss as i64);
    put("ru_idrss", u.ru_idrss as i64);
    put("ru_minflt", u.ru_minflt as i64);
    put("ru_majflt", u.ru_majflt as i64);
    put("ru_nsignals", u.ru_nsignals as i64);
    put("ru_nvcsw", u.ru_nvcsw as i64);
    put("ru_nivcsw", u.ru_nivcsw as i64);
    put("ru_nswap", u.ru_nswap as i64);
    put("ru_utime.tv_usec", u.ru_utime.tv_usec as i64);
    put("ru_utime.tv_sec", u.ru_utime.tv_sec as i64);
    put("ru_stime.tv_usec", u.ru_stime.tv_usec as i64);
    put("ru_stime.tv_sec", u.ru_stime.tv_sec as i64);
    Ok(Value::Array(a))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("microtime", fn_microtime),
            ("gettimeofday", fn_gettimeofday),
            ("getrusage", fn_getrusage),
        ],
    );
}
