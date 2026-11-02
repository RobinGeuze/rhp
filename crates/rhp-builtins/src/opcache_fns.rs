//! Zend OPcache, built in but not started (php-cli's default, and php-cgi
//! run with `opcache.enable=0`): the functions exist, so feature detection
//! (`function_exists('opcache_invalidate')`) sees them, and each reports
//! the "not started" result php gives.

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

fn fn_opcache_get_status(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    bool_param(host, args, 0, "opcache_get_status", "include_scripts", true)?;
    Ok(Value::Bool(false))
}

fn fn_opcache_reset(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(false))
}

fn fn_opcache_invalidate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    str_param(host, args, 0, "opcache_invalidate", "filename")?;
    bool_param(host, args, 1, "opcache_invalidate", "force", false)?;
    Ok(Value::Bool(false))
}

fn fn_opcache_is_script_cached(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    str_param(host, args, 0, "opcache_is_script_cached", "filename")?;
    Ok(Value::Bool(false))
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
    Ok(Value::Bool(false))
}

fn fn_opcache_compile_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    str_param(host, args, 0, "opcache_compile_file", "filename")?;
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
    out.insert(
        ArrayKey::from_bytes(b"blacklist"),
        Value::Array(PhpArray::new()),
    );
    Ok(Value::Array(out))
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
