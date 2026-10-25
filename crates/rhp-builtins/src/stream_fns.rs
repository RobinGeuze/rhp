//! Stream-resource builtins (`fopen`/`fwrite`/`fread`/`fgets`/`fclose`/…) over the
//! `Host` stream primitives, plus `is_resource`/`get_resource_type`. The resource
//! state lives in the VM; these just marshal arguments.

use std::collections::HashMap;

use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn fn_fopen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = conv::to_string_bytes(&deref(&args[0]));
    let mode = conv::to_string_bytes(&deref(&args[1]));
    Ok(host.stream_open(&path, &mode).unwrap_or(Value::Bool(false)))
}

fn fn_fwrite(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    let mut data = conv::to_string_bytes(&deref(&args[1]));
    if let Some(len) = args.get(2) {
        let n = conv::to_int(&deref(len));
        if n >= 0 && (n as usize) < data.len() {
            data.truncate(n as usize);
        }
    }
    Ok(host.stream_write(&res, &data).map(|n| Value::Int(n as i64)).unwrap_or(Value::Bool(false)))
}

fn fn_fread(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    let len = conv::to_int(&deref(&args[1]));
    Ok(host.stream_read(&res, len).map(Value::str).unwrap_or(Value::Bool(false)))
}

fn fn_fgets(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    Ok(host.stream_gets(&res).map(Value::str).unwrap_or(Value::Bool(false)))
}

fn fn_fgetc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    match host.stream_read(&res, 1) {
        Some(b) if !b.is_empty() => Ok(Value::str(b)),
        _ => Ok(Value::Bool(false)),
    }
}

fn fn_feof(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_eof(&deref(&args[0]))))
}

fn fn_fclose(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_close(&deref(&args[0]))))
}

fn fn_fflush(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_flush(&deref(&args[0]))))
}

fn fn_rewind(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_rewind(&deref(&args[0]))))
}

fn fn_stream_get_contents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    let maxlen = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(-1);
    Ok(host.stream_read(&res, maxlen).map(Value::str).unwrap_or(Value::Bool(false)))
}

fn fn_is_resource(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(matches!(deref(&args[0]), Value::Resource(_))))
}

fn fn_get_resource_type(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Resource(r) => Ok(Value::str(r.type_name)),
        _ => Ok(Value::str("Unknown")),
    }
}

fn fn_true(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

fn fn_zero(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(0))
}

fn fn_false(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(false))
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    }
    put(m, "fopen", fn_fopen, 2, Some(4));
    put(m, "fclose", fn_fclose, 1, Some(1));
    put(m, "fwrite", fn_fwrite, 2, Some(3));
    put(m, "fputs", fn_fwrite, 2, Some(3));
    put(m, "fread", fn_fread, 2, Some(2));
    put(m, "fgets", fn_fgets, 1, Some(2));
    put(m, "fgetc", fn_fgetc, 1, Some(1));
    put(m, "feof", fn_feof, 1, Some(1));
    put(m, "fflush", fn_fflush, 1, Some(1));
    put(m, "rewind", fn_rewind, 1, Some(1));
    put(m, "stream_get_contents", fn_stream_get_contents, 1, Some(3));
    put(m, "is_resource", fn_is_resource, 1, Some(1));
    put(m, "get_resource_type", fn_get_resource_type, 1, Some(1));
    // No-op stream knobs (accepted so framework output code runs).
    put(m, "stream_set_blocking", fn_true, 2, Some(2));
    put(m, "stream_set_timeout", fn_true, 2, Some(3));
    put(m, "stream_set_write_buffer", fn_zero, 2, Some(2));
    put(m, "set_file_buffer", fn_zero, 2, Some(2));
    put(m, "fseek", fn_zero, 2, Some(3));
    put(m, "ftell", fn_zero, 1, Some(1));
    // TTY / terminal detection — not a terminal under this SAPI.
    put(m, "stream_isatty", fn_false, 1, Some(1));
    put(m, "posix_isatty", fn_false, 1, Some(1));
    put(m, "sapi_windows_vt100_support", fn_false, 1, Some(2));
    put(m, "stream_get_meta_data", fn_meta, 1, Some(1));
}

fn fn_meta(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // Minimal metadata array (framework code reads a few keys defensively).
    let mut a = rhp_values::PhpArray::new();
    a.insert(rhp_values::ArrayKey::from_bytes(b"timed_out"), Value::Bool(false));
    a.insert(rhp_values::ArrayKey::from_bytes(b"blocked"), Value::Bool(true));
    a.insert(rhp_values::ArrayKey::from_bytes(b"eof"), Value::Bool(false));
    a.insert(rhp_values::ArrayKey::from_bytes(b"stream_type"), Value::str("STDIO"));
    Ok(Value::Array(a))
}
