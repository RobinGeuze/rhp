//! PHP runtime / environment introspection (php's `ext/standard` "info" group):
//! `extension_loaded`, `function_exists`, `phpversion`, `php_sapi_name`,
//! `version_compare`, the error/exception-handler setters (stored, not invoked —
//! this engine emits no diagnostics), and a handful of no-op-ish runtime knobs.
//! Enough to boot a real framework / Composer.

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, Value};

use crate::{Host, Thrown};

fn ob(host: &mut dyn Host, call: crate::ObCall) -> Result<Value, Thrown> {
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

fn fn_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::SapiFlush)
}

/// One `putenv` change: name and new value (`None` = unset).
pub type EnvChange = (Vec<u8>, Option<Vec<u8>>);

thread_local! {
    /// `putenv` changes for this request, layered over the process
    /// environment (`None` = unset). The process environment itself is never
    /// mutated: that would race with other threads' `getenv` (the test
    /// harness runs engines on parallel threads) and leak across engines.
    static ENV_OVERLAY: std::cell::RefCell<Vec<EnvChange>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) fn reset_state() {
    ENV_OVERLAY.with(|e| e.borrow_mut().clear());
}

/// The request's `putenv` changes in order (`None` = unset). A spawner of
/// child processes must apply these (`Command::env` / `env_remove`) so the
/// child sees the same environment as `getenv`, as in php.
pub fn env_overrides() -> Vec<EnvChange> {
    ENV_OVERLAY.with(|e| e.borrow().clone())
}

/// Apply [`env_overrides`] to a child-process command.
pub fn apply_env_overrides(cmd: &mut std::process::Command) {
    for (k, v) in env_overrides() {
        let k = String::from_utf8_lossy(&k).into_owned();
        match v {
            Some(v) => {
                cmd.env(k, String::from_utf8_lossy(&v).into_owned());
            }
            None => {
                cmd.env_remove(k);
            }
        }
    }
}

/// `getenv(name)` as php sees it: the request's `putenv` changes, then the
/// process environment.
pub(crate) fn env_get(name: &[u8]) -> Option<Vec<u8>> {
    if let Some(v) = ENV_OVERLAY.with(|e| {
        e.borrow()
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }) {
        return v;
    }
    std::env::var_os(String::from_utf8_lossy(name).as_ref())
        .map(|v| v.to_string_lossy().into_owned().into_bytes())
}

/// `$local_only` (arg 2) skips the SAPI's own environment. rhp's SAPIs
/// keep the request environment in the process one, so both lookups agree.
fn fn_getenv(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first() {
        Some(v) if !matches!(deref(v), Value::Null) => {
            let name = conv::to_string_bytes(&deref(v));
            match env_get(&name) {
                Some(val) => Ok(Value::str(val)),
                None => Ok(Value::Bool(false)),
            }
        }
        _ => {
            let mut a = PhpArray::new();
            for (k, v) in std::env::vars_os() {
                a.insert(
                    rhp_values::ArrayKey::from_bytes(k.to_string_lossy().as_bytes()),
                    Value::str(v.to_string_lossy().into_owned().into_bytes()),
                );
            }
            for (k, v) in env_overrides() {
                let key = rhp_values::ArrayKey::from_bytes(&k);
                match v {
                    Some(v) => {
                        a.insert(key, Value::str(v));
                    }
                    None => {
                        a.remove(&key);
                    }
                }
            }
            Ok(Value::Array(a))
        }
    }
}

/// `putenv("NAME=value")` sets, `putenv("NAME")` unsets (in the request's
/// environment overlay: `getenv` sees it; see [`env_overrides`]).
fn fn_putenv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = crate::zend::zpp::str_param(host, args, 0, "putenv", "assignment")?;
    if a.is_empty() || a[0] == b'=' {
        return Err(Thrown::error(
            "ValueError",
            "putenv(): Argument #1 ($assignment) must have a valid syntax",
        ));
    }
    let (name, value) = match a.iter().position(|&b| b == b'=') {
        Some(p) => (&a[..p], Some(&a[p + 1..])),
        None => (&a[..], None),
    };
    if name.contains(&0) || value.is_some_and(|v| v.contains(&0)) {
        return Ok(Value::Bool(false));
    }
    let entry = (name.to_vec(), value.map(|v| v.to_vec()));
    ENV_OVERLAY.with(|e| e.borrow_mut().push(entry));
    Ok(Value::Bool(true))
}

fn fn_set_time_limit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let secs = crate::zend::zpp::int_param(host, args, 0, "set_time_limit", "seconds", 0)?;
    crate::runtime_fns::ini_store(b"max_execution_time", secs.to_string().into_bytes());
    Ok(Value::Bool(true))
}

fn fn_error_get_last(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(match host.last_error() {
        None => Value::Null,
        Some((ty, msg, file, line)) => {
            let mut a = rhp_values::PhpArray::new();
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"type".to_vec())),
                Value::Int(ty),
            );
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"message".to_vec())),
                Value::Str(rhp_values::PhpStr::from_bytes(msg)),
            );
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"file".to_vec())),
                Value::Str(rhp_values::PhpStr::from_bytes(file)),
            );
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"line".to_vec())),
                Value::Int(line),
            );
            Value::Array(a)
        }
    })
}

fn fn_error_clear_last(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.clear_last_error();
    Ok(Value::Null)
}

/// `register_shutdown_function(callable $callback, mixed ...$args): void`.
fn fn_register_shutdown_function(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cb = deref(&args[0]);
    if let Some(why) = host.check_callable(&cb)? {
        return Err(Thrown::type_error(format!(
            "register_shutdown_function(): Argument #1 ($callback) must be a valid callback, {why}"
        )));
    }
    host.register_shutdown_function(cb, args[1..].iter().map(deref).collect())?;
    Ok(Value::Null)
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
            ("flush", fn_flush),
            ("getenv", fn_getenv),
            ("putenv", fn_putenv),
            ("error_get_last", fn_error_get_last),
            ("error_clear_last", fn_error_clear_last),
            ("register_shutdown_function", fn_register_shutdown_function),
            ("set_time_limit", fn_set_time_limit),
            ("random_bytes", fn_random_bytes),
            ("random_int", fn_random_int),
            ("mt_getrandmax", fn_getrandmax),
            ("getrandmax", fn_getrandmax),
        ],
    );
}
