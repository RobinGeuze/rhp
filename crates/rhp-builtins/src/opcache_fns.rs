//! Zend OPcache's functions. The compiled-script cache itself lives in the
//! host (`Host::opcache_op`); when it isn't started (php-cli without
//! `opcache.enable_cli`, or `opcache.enable=0`) each function reports what
//! php reports then, so feature detection still sees them.

use std::collections::HashMap;

use rhp_values::{ArrayKey, PhpArray, Value};

use crate::argparse::{bool_param, str_param};
use crate::{Builtin, Host, Thrown};

/// How `opcache_get_configuration()` types a directive.
#[derive(Clone, Copy)]
enum Ty {
    Bool,
    Int,
    /// Megabytes in the ini, bytes in the report.
    Mb,
    /// A size with a k/m/g suffix.
    Quantity,
    /// Percent in the ini, a fraction in the report.
    Percent,
    Float,
    Str,
}

/// `opcache_get_configuration()['directives']`, in php's order.
const DIRECTIVES: &[(&str, Ty)] = &[
    ("opcache.enable", Ty::Bool),
    ("opcache.enable_cli", Ty::Bool),
    ("opcache.use_cwd", Ty::Bool),
    ("opcache.validate_timestamps", Ty::Bool),
    ("opcache.validate_permission", Ty::Bool),
    ("opcache.validate_root", Ty::Bool),
    ("opcache.dups_fix", Ty::Bool),
    ("opcache.revalidate_path", Ty::Bool),
    ("opcache.log_verbosity_level", Ty::Int),
    ("opcache.memory_consumption", Ty::Mb),
    ("opcache.interned_strings_buffer", Ty::Int),
    ("opcache.max_accelerated_files", Ty::Int),
    ("opcache.max_wasted_percentage", Ty::Percent),
    ("opcache.force_restart_timeout", Ty::Int),
    ("opcache.revalidate_freq", Ty::Int),
    ("opcache.preferred_memory_model", Ty::Str),
    ("opcache.blacklist_filename", Ty::Str),
    ("opcache.max_file_size", Ty::Int),
    ("opcache.error_log", Ty::Str),
    ("opcache.protect_memory", Ty::Bool),
    ("opcache.save_comments", Ty::Bool),
    ("opcache.record_warnings", Ty::Bool),
    ("opcache.enable_file_override", Ty::Bool),
    ("opcache.optimization_level", Ty::Quantity),
    ("opcache.lockfile_path", Ty::Str),
    ("opcache.file_cache", Ty::Str),
    ("opcache.file_cache_read_only", Ty::Bool),
    ("opcache.file_cache_only", Ty::Bool),
    ("opcache.file_cache_consistency_checks", Ty::Bool),
    ("opcache.file_update_protection", Ty::Int),
    ("opcache.opt_debug_level", Ty::Quantity),
    ("opcache.restrict_api", Ty::Str),
    ("opcache.huge_code_pages", Ty::Bool),
    ("opcache.preload", Ty::Str),
    ("opcache.preload_user", Ty::Str),
    ("opcache.jit", Ty::Str),
    ("opcache.jit_buffer_size", Ty::Quantity),
    ("opcache.jit_debug", Ty::Quantity),
    ("opcache.jit_bisect_limit", Ty::Int),
    ("opcache.jit_blacklist_root_trace", Ty::Int),
    ("opcache.jit_blacklist_side_trace", Ty::Int),
    ("opcache.jit_hot_func", Ty::Int),
    ("opcache.jit_hot_loop", Ty::Int),
    ("opcache.jit_hot_return", Ty::Int),
    ("opcache.jit_hot_side_exit", Ty::Int),
    ("opcache.jit_max_exit_counters", Ty::Int),
    ("opcache.jit_max_loop_unrolls", Ty::Int),
    ("opcache.jit_max_polymorphic_calls", Ty::Int),
    ("opcache.jit_max_recursive_calls", Ty::Int),
    ("opcache.jit_max_recursive_returns", Ty::Int),
    ("opcache.jit_max_root_traces", Ty::Int),
    ("opcache.jit_max_side_traces", Ty::Int),
    ("opcache.jit_prof_threshold", Ty::Float),
    ("opcache.jit_max_trace_length", Ty::Int),
];

fn ini_bool(v: &[u8]) -> bool {
    let l = v.to_ascii_lowercase();
    matches!(&l[..], b"on" | b"yes" | b"true") || crate::runtime_fns::c_atol(&l) != 0
}

/// `validate_api_restriction`: with `opcache.restrict_api` set, only a
/// script whose path starts with it may use the API.
fn api_allowed(host: &mut dyn Host) -> Result<bool, Thrown> {
    let restrict = host.ini_get(b"opcache.restrict_api").unwrap_or_default();
    if restrict.is_empty() {
        return Ok(true);
    }
    let main = host.running_main_path().unwrap_or_default();
    if main.starts_with(&restrict) {
        return Ok(true);
    }
    host.emit_diagnostic(
        crate::E_WARNING,
        "Zend OPcache API is restricted by \"restrict_api\" configuration directive",
    )?;
    Ok(false)
}

fn fn_opcache_get_status(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scripts = bool_param(host, args, 0, "opcache_get_status", "include_scripts", true)?;
    if !api_allowed(host)? {
        return Ok(Value::Bool(false));
    }
    host.opcache_op("get_status", &[Value::Bool(scripts)])
        .unwrap_or(Ok(Value::Bool(false)))
}

fn fn_opcache_reset(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    if !api_allowed(host)? {
        return Ok(Value::Bool(false));
    }
    host.opcache_op("reset", &[])
        .unwrap_or(Ok(Value::Bool(false)))
}

fn fn_opcache_invalidate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "opcache_invalidate", "filename")?;
    let force = bool_param(host, args, 1, "opcache_invalidate", "force", false)?;
    if !api_allowed(host)? {
        return Ok(Value::Bool(false));
    }
    host.opcache_op("invalidate", &[Value::str(name), Value::Bool(force)])
        .unwrap_or(Ok(Value::Bool(false)))
}

fn fn_opcache_is_script_cached(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "opcache_is_script_cached", "filename")?;
    if !api_allowed(host)? {
        return Ok(Value::Bool(false));
    }
    host.opcache_op("is_script_cached", &[Value::str(name)])
        .unwrap_or(Ok(Value::Bool(false)))
}

fn fn_opcache_is_script_cached_in_file_cache(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    str_param(
        host,
        args,
        0,
        "opcache_is_script_cached_in_file_cache",
        "filename",
    )?;
    if !api_allowed(host)? {
        return Ok(Value::Bool(false));
    }
    // No file cache.
    Ok(Value::Bool(false))
}

fn fn_opcache_compile_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "opcache_compile_file", "filename")?;
    if let Some(r) = host.opcache_op("compile_file", &[Value::str(name)]) {
        return r;
    }
    host.emit_diagnostic(
        crate::E_NOTICE,
        "Zend OPcache has not been properly started, can't compile file",
    )?;
    Ok(Value::Bool(false))
}

fn fn_opcache_jit_blacklist(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Null)
}

fn fn_opcache_get_configuration(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    if !api_allowed(host)? {
        return Ok(Value::Bool(false));
    }
    let mut d = PhpArray::new();
    for &(name, ty) in DIRECTIVES {
        let raw = host.ini_get(name.as_bytes()).unwrap_or_default();
        let atol = || crate::runtime_fns::c_atol(&raw);
        let v = match ty {
            Ty::Bool => Value::Bool(ini_bool(&raw)),
            Ty::Int => Value::Int(atol()),
            Ty::Mb => Value::Int(atol().saturating_mul(1024 * 1024)),
            Ty::Quantity => Value::Int(crate::runtime_fns::ini_parse_quantity(&raw)),
            Ty::Percent => Value::Float(atol() as f64 / 100.0),
            Ty::Float => Value::Float(
                String::from_utf8_lossy(&raw)
                    .trim()
                    .parse::<f64>()
                    .unwrap_or(0.0),
            ),
            Ty::Str => Value::str(raw),
        };
        d.insert(ArrayKey::from_bytes(name.as_bytes()), v);
    }
    let mut version = PhpArray::new();
    version.insert(
        ArrayKey::from_bytes(b"version"),
        Value::str(crate::PHP_VERSION),
    );
    version.insert(
        ArrayKey::from_bytes(b"opcache_product_name"),
        Value::str("Zend OPcache"),
    );
    let mut out = PhpArray::new();
    out.insert(ArrayKey::from_bytes(b"directives"), Value::Array(d));
    out.insert(ArrayKey::from_bytes(b"version"), Value::Array(version));
    let blacklist = host
        .opcache_op("blacklist", &[])
        .transpose()?
        .unwrap_or_else(|| Value::Array(PhpArray::new()));
    out.insert(ArrayKey::from_bytes(b"blacklist"), blacklist);
    Ok(Value::Array(out))
}

/// `asctime(localtime(t))` without its newline (`opcache_get_status()`'s
/// `last_used`): the process's local time zone, not `date.timezone`.
pub fn asctime_local(t: i64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // SAFETY: localtime_r only writes the tm it is given.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let tt = t as libc::time_t;
        if libc::localtime_r(&tt, &mut tm).is_null() {
            return String::new();
        }
        tm
    };
    format!(
        "{} {}{:3} {:02}:{:02}:{:02} {}",
        DAYS[tm.tm_wday.rem_euclid(7) as usize],
        MONTHS[tm.tm_mon.rem_euclid(12) as usize],
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        tm.tm_year + 1900
    )
}

/// Whether php's JIT is enabled (`JIT_G(enabled)`): opcache started,
/// `opcache.jit` not "disable" at startup, and a JIT buffer.
pub(crate) fn jit_enabled() -> bool {
    let startup = |n: &[u8]| crate::ini_startup_value(n).unwrap_or_default();
    let cli = crate::sapi::sapi_name() == "cli";
    let started = ini_bool(&startup(b"opcache.enable"))
        && (!cli || ini_bool(&startup(b"opcache.enable_cli")));
    let mode = startup(b"opcache.jit").to_ascii_lowercase();
    started
        && mode != b"disable"
        && jit_mode_valid(&mode)
        && crate::runtime_fns::ini_parse_quantity(&startup(b"opcache.jit_buffer_size")) > 0
}

/// `zend_accel_error` levels.
pub(crate) const LOG_WARNING: i64 = 2;

thread_local! {
    /// Messages for opcache's own log, raised before the request starts.
    /// (the directive's position in php's registration order, level, message)
    static ACCEL_LOG: std::cell::RefCell<Vec<(usize, i64, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Queue a message from a directive's startup handler for opcache's log
/// (written by [`take_accel_log`]'s caller once the settings are known, in
/// php's registration order).
pub(crate) fn accel_log(name: &[u8], level: i64, msg: String) {
    let pos = DIRECTIVES
        .iter()
        .position(|(n, _)| n.as_bytes() == name)
        .unwrap_or(usize::MAX);
    ACCEL_LOG.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|e| e.0 != pos);
        l.push((pos, level, msg));
    });
}

/// The queued opcache log lines `opcache.log_verbosity_level` lets through,
/// formatted as `zend_accel_error` writes them, with where they go
/// (`opcache.error_log`; empty for stderr).
pub fn take_accel_log() -> (Vec<String>, Vec<u8>) {
    let mut msgs = ACCEL_LOG.with(|l| std::mem::take(&mut *l.borrow_mut()));
    msgs.sort_by_key(|m| m.0);
    let ini = |n: &[u8]| crate::ini_startup_value(n).unwrap_or_default();
    let verbosity = crate::c_atol(&ini(b"opcache.log_verbosity_level"));
    let now = crate::zip_ext::now();
    let lines = msgs
        .into_iter()
        .filter(|(_, level, _)| *level <= verbosity)
        .map(|(_, level, msg)| {
            let kind = match level {
                0 => "Fatal Error",
                1 => "Error",
                2 => "Warning",
                3 => "Message",
                _ => "Debug",
            };
            format!(
                "{} ({}): {kind} {msg}\n",
                asctime_local(now),
                std::process::id()
            )
        })
        .collect();
    let dest = ini(b"opcache.error_log");
    (lines, if dest == b"stderr" { Vec::new() } else { dest })
}

/// accel_finish_startup_preload_subprocess: `opcache.preload_user` only
/// matters when php runs as root (it then preloads as that user).
pub fn preload_user_check() {
    let user = crate::ini_startup_value(b"opcache.preload_user").unwrap_or_default();
    // SAFETY: geteuid has no preconditions.
    if !user.is_empty() && unsafe { libc::geteuid() } != 0 {
        accel_log(
            b"opcache.preload_user",
            LOG_WARNING,
            "\"opcache.preload_user\" is ignored because the current user is not \"root\"".into(),
        );
    }
}

/// The startup OnUpdate handlers of opcache's sized directives: the
/// warning when they refuse a value (which then keeps its default).
pub(crate) fn startup_check(name: &[u8], v: &[u8]) -> Option<String> {
    let n = crate::c_atol(v) as i32;
    let msg = match name {
        b"opcache.memory_consumption" if n < 8 => {
            "opcache.memory_consumption is set below the required 8MB.\n".to_string()
        }
        b"opcache.max_accelerated_files" if n < 200 => {
            "opcache.max_accelerated_files is set below the required minimum (200).\n".to_string()
        }
        b"opcache.max_accelerated_files" if n > 1_000_000 => {
            "opcache.max_accelerated_files is set above the limit (1000000).\n".to_string()
        }
        b"opcache.max_wasted_percentage" if n <= 0 || n > 50 => {
            "opcache.max_wasted_percentage must be set between 1 and 50.\n".to_string()
        }
        b"opcache.interned_strings_buffer" => {
            let q = crate::runtime_fns::ini_parse_quantity(v);
            if q < 0 {
                format!("opcache.interned_strings_buffer must be greater than or equal to 0, {q} given.\n")
            } else if q > 32767 {
                format!("opcache.interned_strings_buffer must be less than or equal to 32767, {q} given.\n")
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(msg)
}

/// The runtime OnModify handlers of `opcache.enable` (OnEnable: it may only
/// be switched off) and `opcache.jit` (OnUpdateJit). `Ok(false)`: refused.
pub(crate) fn on_modify(host: &mut dyn Host, name: &[u8], v: &[u8]) -> Result<bool, Thrown> {
    match name {
        b"opcache.enable" if crate::runtime_fns::ini_parse_bool(v) => {
            let cur = host.ini_get(name).unwrap_or_default();
            if crate::runtime_fns::ini_parse_bool(&cur) {
                return Ok(true);
            }
            host.emit_diagnostic(
                crate::E_WARNING,
                "Zend OPcache can't be temporarily enabled (it may be only disabled until the end of request)",
            )?;
            Ok(false)
        }
        b"opcache.jit" if !jit_enabled() => {
            host.emit_diagnostic(
                crate::E_WARNING,
                "Cannot change opcache.jit setting at run-time (JIT is disabled)",
            )?;
            Ok(false)
        }
        b"opcache.jit" if !jit_mode_valid(v) => {
            host.emit_diagnostic(
                crate::E_WARNING,
                "Invalid \"opcache.jit\" setting. Should be \"disable\", \"on\", \"off\", \"tracing\", \"function\" or 4-digit number",
            )?;
            Ok(false)
        }
        _ => Ok(true),
    }
}

/// `zend_jit_config`'s accepted values.
pub(crate) fn jit_mode_valid(mode: &[u8]) -> bool {
    let m = mode.to_ascii_lowercase();
    if matches!(
        &m[..],
        b"disable"
            | b""
            | b"0"
            | b"off"
            | b"no"
            | b"false"
            | b"1"
            | b"on"
            | b"yes"
            | b"true"
            | b"tracing"
            | b"function"
    ) {
        return true;
    }
    let Ok(s) = std::str::from_utf8(&m) else {
        return false;
    };
    let Ok(mut jit) = s.parse::<i64>() else {
        return false;
    };
    if jit < 0 || jit % 10 == 0 || jit % 10 > 5 {
        return false;
    }
    jit /= 10;
    if jit % 10 > 5 || jit % 10 == 4 {
        return false;
    }
    jit /= 10;
    if jit % 10 > 2 {
        return false;
    }
    jit /= 10;
    jit % 10 <= 1 && jit / 10 == 0
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let mut put = |name: &'static str, func: crate::NativeFn, min: usize, max: usize| {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: Some(max),
            },
        );
    };
    put("opcache_get_status", fn_opcache_get_status, 0, 1);
    put("opcache_reset", fn_opcache_reset, 0, 0);
    put("opcache_invalidate", fn_opcache_invalidate, 1, 2);
    put(
        "opcache_is_script_cached",
        fn_opcache_is_script_cached,
        1,
        1,
    );
    put(
        "opcache_is_script_cached_in_file_cache",
        fn_opcache_is_script_cached_in_file_cache,
        1,
        1,
    );
    put("opcache_compile_file", fn_opcache_compile_file, 1, 1);
    put(
        "opcache_get_configuration",
        fn_opcache_get_configuration,
        0,
        0,
    );
    put("opcache_jit_blacklist", fn_opcache_jit_blacklist, 1, 1);
}
