//! PHP runtime / environment introspection (php's `ext/standard` "info" group):
//! `extension_loaded`, `function_exists`, `phpversion`, `php_sapi_name`,
//! `version_compare`, the error/exception-handler setters (stored, not invoked —
//! this engine emits no diagnostics), and a handful of no-op-ish runtime knobs.
//! Enough to boot a real framework / Composer.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

pub(crate) fn ob(host: &mut dyn Host, call: crate::ObCall) -> Result<Value, Thrown> {
    host.output_control(call)
}

fn fn_ob_start(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let callback = args.first().map(deref).unwrap_or(Value::Null);
    let chunk_size = crate::zend::zpp::int_param(host, args, 1, "ob_start", "chunk_size", 0)?;
    let flags = crate::zend::zpp::int_param(host, args, 2, "ob_start", "flags", 0x70)?;
    ob(
        host,
        crate::ObCall::Start {
            callback,
            chunk_size,
            flags,
        },
    )
}

fn fn_ob_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::Flush)
}

fn fn_ob_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::Clean)
}

fn fn_ob_end_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::EndFlush)
}

fn fn_ob_end_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::EndClean)
}

fn fn_ob_get_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetFlush)
}

fn fn_ob_get_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetClean)
}

fn fn_ob_get_contents(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetContents)
}

fn fn_ob_get_length(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetLength)
}

fn fn_ob_get_level(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetLevel)
}

fn fn_ob_list_handlers(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::ListHandlers)
}

fn fn_ob_get_status(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let full = crate::zend::zpp::bool_param(host, args, 0, "ob_get_status", "full_status", false)?;
    ob(host, crate::ObCall::GetStatus { full })
}

fn fn_ob_implicit_flush(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let on = crate::zend::zpp::bool_param(host, args, 0, "ob_implicit_flush", "enable", true)?;
    ob(host, crate::ObCall::ImplicitFlush(on))
}

fn fn_set_time_limit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let secs = crate::zend::zpp::int_param(host, args, 0, "set_time_limit", "seconds", 0)?;
    crate::runtime_fns::ini_store(b"max_execution_time", secs.to_string().into_bytes());
    Ok(Value::Bool(true))
}

// --- randomness --------------------------------------------------------------

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    u64::from_le_bytes(b)
}

/// A uniform integer in `[min, max]` (inclusive) via rejection sampling.
pub(crate) fn rand_range(min: i64, max: i64) -> i64 {
    if min >= max {
        return min;
    }
    let span = (max as i128 - min as i128 + 1) as u128;
    // Rejection-sample to avoid modulo bias.
    let limit = u128::MAX - (u128::MAX % span);
    loop {
        let r = ((rand_u64() as u128) << 64) | rand_u64() as u128;
        if r < limit {
            return (min as i128 + (r % span) as i128) as i64;
        }
    }
}

fn fn_random_bytes(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let len = conv::to_int(&deref(&args[0]));
    if len <= 0 {
        return Err(Thrown::error(
            "ValueError",
            "random_bytes(): Argument #1 ($length) must be greater than 0",
        ));
    }
    let mut b = vec![0u8; len as usize];
    if getrandom::getrandom(&mut b).is_err() {
        return Err(Thrown::error(
            "Random\\RandomException",
            "Could not gather sufficient random data",
        ));
    }
    Ok(Value::str(b))
}

fn fn_random_int(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let min = conv::to_int(&deref(&args[0]));
    let max = conv::to_int(&deref(&args[1]));
    if min > max {
        return Err(Thrown::error(
            "ValueError",
            "random_int(): Argument #1 ($min) must be less than or equal to argument #2 ($max)",
        ));
    }
    Ok(Value::Int(rand_range(min, max)))
}

fn fn_getrandmax(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(2147483647))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("ob_start", fn_ob_start),
            ("ob_flush", fn_ob_flush),
            ("ob_clean", fn_ob_clean),
            ("ob_end_flush", fn_ob_end_flush),
            ("ob_end_clean", fn_ob_end_clean),
            ("ob_get_flush", fn_ob_get_flush),
            ("ob_get_clean", fn_ob_get_clean),
            ("ob_get_contents", fn_ob_get_contents),
            ("ob_get_length", fn_ob_get_length),
            ("ob_get_level", fn_ob_get_level),
            ("ob_list_handlers", fn_ob_list_handlers),
            ("ob_get_status", fn_ob_get_status),
            ("ob_implicit_flush", fn_ob_implicit_flush),
            ("set_time_limit", fn_set_time_limit),
            ("random_bytes", fn_random_bytes),
            ("random_int", fn_random_int),
            ("mt_getrandmax", fn_getrandmax),
            ("getrandmax", fn_getrandmax),
        ],
    );
}
