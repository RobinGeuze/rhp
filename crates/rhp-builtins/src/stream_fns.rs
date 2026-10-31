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
    Ok(host
        .stream_write(&res, &data)
        .map(|n| Value::Int(n as i64))
        .unwrap_or(Value::Bool(false)))
}

fn fn_fread(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    let len = conv::to_int(&deref(&args[1]));
    Ok(host
        .stream_read(&res, len)
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn fn_fgets(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    let line = match args.get(1).map(deref) {
        None | Some(Value::Null) => host.stream_gets(&res),
        Some(v) => {
            let n = conv::to_int(&v);
            if n <= 0 {
                return Err(Thrown::error(
                    "ValueError",
                    "fgets(): Argument #2 ($length) must be greater than 0",
                ));
            }
            host.stream_gets_max(&res, n as usize)
        }
    };
    Ok(line.map(Value::str).unwrap_or(Value::Bool(false)))
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
    let maxlen = match args.get(1).map(deref) {
        None | Some(Value::Null) => -1,
        Some(v) => conv::to_int(&v),
    };
    // `$offset` (>= 0): seek there first; php returns false when it cannot.
    if let Some(off) = args.get(2).map(|v| conv::to_int(&deref(v))) {
        if off >= 0 && !host.stream_seek(&res, off, 0) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(host
        .stream_read(&res, maxlen)
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn fn_fseek(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    let off = conv::to_int(&deref(&args[1]));
    let whence = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    Ok(Value::Int(if host.stream_seek(&res, off, whence) {
        0
    } else {
        -1
    }))
}

fn fn_ftell(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    Ok(host
        .stream_tell(&res)
        .map(Value::Int)
        .unwrap_or(Value::Bool(false)))
}

fn fn_stream_set_blocking(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    let on = conv::to_bool(&deref(&args[1]));
    Ok(Value::Bool(host.stream_set_blocking(&res, on)))
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

fn fn_stream_context_create(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let options = args
        .first()
        .map(deref)
        .unwrap_or(Value::Array(rhp_values::PhpArray::new()));
    Ok(host.create_stream_context(options))
}

fn fn_stream_context_get_options(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match host.stream_context_options(&deref(&args[0])) {
        Value::Null => Ok(Value::Array(rhp_values::PhpArray::new())),
        v => Ok(v),
    }
}

fn fn_stream_context_set_option(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // Two forms: ($ctx, $wrapper, $option, $value) or ($ctx, $options_array).
    if args.len() >= 4 {
        let wrapper = conv::to_string_bytes(&deref(&args[1]));
        let option = conv::to_string_bytes(&deref(&args[2]));
        host.stream_context_set_option(&deref(&args[0]), &wrapper, &option, deref(&args[3]));
    } else if let Value::Array(opts) = deref(&args[1]) {
        for (wk, wv) in opts.iter() {
            if let Value::Array(inner) = wv {
                let wrapper = key_bytes(wk);
                for (ok, ov) in inner.iter() {
                    host.stream_context_set_option(
                        &deref(&args[0]),
                        &wrapper,
                        &key_bytes(ok),
                        ov.clone(),
                    );
                }
            }
        }
    }
    Ok(Value::Bool(true))
}

fn key_bytes(k: &rhp_values::ArrayKey) -> Vec<u8> {
    match k {
        rhp_values::ArrayKey::Int(n) => n.to_string().into_bytes(),
        rhp_values::ArrayKey::Str(s) => s.as_bytes().to_vec(),
    }
}

fn fn_http_get_last_response_headers(
    host: &mut dyn Host,
    _args: &[Value],
) -> Result<Value, Thrown> {
    let headers = host.http_response_headers();
    if headers.is_empty() {
        return Ok(Value::Null);
    }
    let mut a = rhp_values::PhpArray::new();
    for h in headers {
        a.push(Value::str(h));
    }
    Ok(Value::Array(a))
}

fn fn_http_clear_last_response_headers(
    host: &mut dyn Host,
    _args: &[Value],
) -> Result<Value, Thrown> {
    host.clear_http_response_headers();
    Ok(Value::Null)
}

fn fn_stream_copy_to_stream(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let src = deref(&args[0]);
    let dst = deref(&args[1]);
    let mut total = 0usize;
    loop {
        match host.stream_read(&src, 8192) {
            Some(b) if !b.is_empty() => {
                host.stream_write(&dst, &b);
                total += b.len();
            }
            _ => break,
        }
    }
    Ok(Value::Int(total as i64))
}

fn fn_stream_get_wrappers(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = rhp_values::PhpArray::new();
    for scheme in host.stream_wrapper_schemes() {
        a.push(Value::str(scheme));
    }
    Ok(Value::Array(a))
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
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
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
    put(m, "stream_get_wrappers", fn_stream_get_wrappers, 0, Some(0));
    put(
        m,
        "stream_copy_to_stream",
        fn_stream_copy_to_stream,
        2,
        Some(4),
    );
    put(
        m,
        "stream_context_create",
        fn_stream_context_create,
        0,
        Some(2),
    );
    put(
        m,
        "stream_context_get_options",
        fn_stream_context_get_options,
        1,
        Some(1),
    );
    put(
        m,
        "stream_context_set_option",
        fn_stream_context_set_option,
        2,
        Some(4),
    );
    put(
        m,
        "http_get_last_response_headers",
        fn_http_get_last_response_headers,
        0,
        Some(0),
    );
    put(
        m,
        "http_clear_last_response_headers",
        fn_http_clear_last_response_headers,
        0,
        Some(0),
    );
    // No-op stream knobs (accepted so framework output code runs).
    put(m, "stream_set_blocking", fn_stream_set_blocking, 2, Some(2));
    put(m, "stream_set_timeout", fn_true, 2, Some(3));
    put(m, "stream_set_write_buffer", fn_zero, 2, Some(2));
    put(m, "set_file_buffer", fn_zero, 2, Some(2));
    put(m, "fseek", fn_fseek, 2, Some(3));
    put(m, "ftell", fn_ftell, 1, Some(1));
    // TTY detection: `stream_isatty` lives with the process functions
    // (proc_fns), `posix_isatty` in ext/posix.
    put(m, "sapi_windows_vt100_support", fn_false, 1, Some(2));
    put(m, "stream_get_meta_data", fn_meta, 1, Some(1));
}

fn fn_meta(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // Minimal metadata array (framework code reads a few keys defensively).
    let mut a = rhp_values::PhpArray::new();
    a.insert(
        rhp_values::ArrayKey::from_bytes(b"timed_out"),
        Value::Bool(false),
    );
    a.insert(
        rhp_values::ArrayKey::from_bytes(b"blocked"),
        Value::Bool(true),
    );
    a.insert(rhp_values::ArrayKey::from_bytes(b"eof"), Value::Bool(false));
    a.insert(
        rhp_values::ArrayKey::from_bytes(b"stream_type"),
        Value::str("STDIO"),
    );
    Ok(Value::Array(a))
}
