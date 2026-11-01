//! Stream-resource builtins over the `Host` stream primitives: contexts,
//! `is_resource`/`get_resource_type`, copy/wrapper listing and the no-op
//! knobs. The per-handle file functions (`fopen`/`fread`/`fwrite`/`fgets`/
//! `fclose`/`fseek`/…) live in `fs_fns` with php's argument validation. The
//! resource state lives in the VM.

use std::collections::HashMap;

use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
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

/// `stream_copy_to_stream($from, $to, ?int $length = null, int $offset = 0)`:
/// seek `$from` to `$offset` (when > 0), then copy up to `$length` bytes
/// (null or negative = everything; php casts it to size_t).
fn fn_stream_copy_to_stream(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (src, _) = crate::fs_fns::stream_param(host, args, 0, "stream_copy_to_stream", "from")?;
    let (dst, _) = crate::fs_fns::stream_param(host, args, 1, "stream_copy_to_stream", "to")?;
    let maxlen = match args.get(2).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = conv::to_int(&v);
            (n >= 0).then_some(n as usize)
        }
    };
    let pos = args.get(3).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if pos > 0 && !host.stream_seek(&src, pos, 0) {
        return Ok(Value::Bool(false));
    }
    if maxlen == Some(0) {
        return Ok(Value::Int(0));
    }
    let mut total = 0usize;
    loop {
        let chunk = match maxlen {
            Some(m) => (m - total).min(8192),
            None => 8192,
        };
        if chunk == 0 {
            break;
        }
        match host.stream_read(&src, chunk as i64) {
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
    put(m, "stream_set_blocking", fn_true, 2, Some(2));
    put(m, "stream_set_timeout", fn_true, 2, Some(3));
    put(m, "stream_set_write_buffer", fn_zero, 2, Some(2));
    put(m, "set_file_buffer", fn_zero, 2, Some(2));
    // TTY / terminal detection — not a terminal under this SAPI.
    put(m, "stream_isatty", fn_false, 1, Some(1));
    put(m, "posix_isatty", fn_false, 1, Some(1));
    put(m, "sapi_windows_vt100_support", fn_false, 1, Some(2));
}
