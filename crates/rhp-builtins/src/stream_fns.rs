//! Stream-resource builtins over the `Host` stream primitives: contexts,
//! `is_resource`/`get_resource_type`, copy/wrapper listing and the no-op
//! knobs. The per-handle file functions (`fopen`/`fread`/`fwrite`/`fgets`/
//! `fclose`/`fseek`/…) live in `fs_fns` with php's argument validation. The
//! resource state lives in the VM.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

fn set_blocking_impl(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let (res, _) = crate::fs_fns::stream_param(host, args, 0, fname, "stream")?;
    let on = crate::zend::zpp::bool_param(host, args, 1, fname, "enable", true)?;
    Ok(Value::Bool(host.stream_set_blocking(&res, on)))
}

fn fn_stream_set_blocking(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_blocking_impl(host, args, "stream_set_blocking")
}

pub(crate) fn fn_socket_set_blocking(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_blocking_impl(host, args, "socket_set_blocking")
}

/// `stream_set_write_buffer` / `set_file_buffer`: php's WRITE_BUFFER option
/// is only implemented by stdio streams backed by a `FILE*` (popen, the
/// CLI's STDIN/STDOUT/STDERR); every other stream (plain files, memory,
/// sockets, wrappers) answers -1.
fn write_buffer_impl(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let (res, _) = crate::fs_fns::stream_param(host, args, 0, fname, "stream")?;
    crate::proc_fns::zpp_long(host, args, 1, fname, "size")?;
    let file_backed = host.stream_sock_info(&res).is_none()
        && host.stream_meta(&res).is_some_and(|m| {
            m.stream_type == "STDIO" && (m.wrapper_type.is_empty() || m.wrapper_type == "PHP")
        });
    Ok(Value::Int(if file_backed { 0 } else { -1 }))
}

fn fn_stream_set_write_buffer(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    write_buffer_impl(host, args, "stream_set_write_buffer")
}

fn fn_set_file_buffer(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    write_buffer_impl(host, args, "set_file_buffer")
}

fn fn_stream_context_create(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ctx = host.create_stream_context(Value::Array(rhp_values::PhpArray::new()));
    if let Some(Value::Array(options)) = args.first().map(deref) {
        parse_context_options(host, &ctx, &options)?;
    }
    if let Some(Value::Array(params)) = args.get(1).map(deref) {
        parse_context_params(host, "stream_context_create", &ctx, &params)?;
    }
    Ok(ctx)
}

/// php's `parse_context_options`: each `[wrapper][option]` value copied
/// into the context (a list of options is skipped, as php skips a packed
/// array).
fn parse_context_options(
    host: &mut dyn Host,
    ctx: &Value,
    options: &rhp_values::PhpArray,
) -> Result<(), Thrown> {
    use rhp_values::ArrayKey;
    for (wk, wv) in options.iter() {
        let wv = deref(wv);
        let (ArrayKey::Str(w), Value::Array(inner)) = (wk, &wv) else {
            return Err(Thrown::error(
                "ValueError",
                "Options should have the form [\"wrappername\"][\"optionname\"] = $value",
            ));
        };
        if !inner.is_empty() && inner.iter().all(|(k, _)| matches!(k, ArrayKey::Int(_))) {
            continue;
        }
        for (ok, ov) in inner.iter() {
            if let ArrayKey::Str(o) = ok {
                host.stream_context_set_option(ctx, w.as_bytes(), o.as_bytes(), deref(ov));
            }
        }
    }
    Ok(())
}

/// php's `parse_context_params`: the `notification` callback, then
/// `options`.
fn parse_context_params(
    host: &mut dyn Host,
    fname: &str,
    ctx: &Value,
    params: &rhp_values::PhpArray,
) -> Result<(), Thrown> {
    use rhp_values::ArrayKey;
    if let Some(cb) = params
        .get(&ArrayKey::from_bytes(b"notification"))
        .map(deref)
    {
        host.set_stream_context_notifier(ctx, None);
        if let Some(err) = host.callable_error(&cb) {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #1 ($context) must be an array with valid callbacks as values, {err}"
            )));
        }
        host.set_stream_context_notifier(ctx, Some(cb));
        crate::ext::standard::http_fopen_wrapper::reset_progress(host, ctx);
    }
    if let Some(opts) = params.get(&ArrayKey::from_bytes(b"options")).map(deref) {
        let Value::Array(opts) = opts else {
            return Err(Thrown::type_error("Invalid stream/context parameter"));
        };
        parse_context_options(host, ctx, &opts)?;
    }
    Ok(())
}

/// `decode_context_param` for a function's first argument.
fn context_param(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    if !matches!(v, Value::Resource(_)) {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($context) must be of type resource, {} given",
            crate::zend::zpp::zval_value_name(host, &v)
        )));
    }
    host.resolve_stream_context(&v).ok_or_else(|| {
        Thrown::type_error(format!(
            "{fname}(): Argument #1 ($context) must be a valid stream/context"
        ))
    })
}

fn fn_stream_context_set_params(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_context_set_params";
    let ctx = context_param(host, args, fname)?;
    let params = crate::zend::zpp::array_param(host, args, 1, fname)?;
    parse_context_params(host, fname, &ctx, &params)?;
    Ok(Value::Bool(true))
}

fn fn_stream_context_get_params(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ctx = context_param(host, args, "stream_context_get_params")?;
    let mut a = rhp_values::PhpArray::new();
    if let Some(cb) = host.stream_context_notifier(&ctx) {
        a.insert(rhp_values::ArrayKey::from_bytes(b"notification"), cb);
    }
    let opts = match host.stream_context_options(&ctx) {
        Value::Null => Value::Array(rhp_values::PhpArray::new()),
        v => v,
    };
    a.insert(rhp_values::ArrayKey::from_bytes(b"options"), opts);
    Ok(Value::Array(a))
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
    if pos > 0
        && !crate::fs_fns::stream_seek(host, "stream_copy_to_stream", &src, pos, libc::SEEK_SET)?
    {
        crate::warn(
            host,
            "stream_copy_to_stream",
            &format!("Failed to seek to position {pos} in the stream"),
        )?;
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

/// TTY detection lives elsewhere: `stream_isatty` with the process
/// functions (proc_fns), `posix_isatty` in ext/posix;
/// sapi_windows_vt100_support exists on Windows only.
pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("stream_get_wrappers", fn_stream_get_wrappers),
            ("stream_copy_to_stream", fn_stream_copy_to_stream),
            ("stream_context_create", fn_stream_context_create),
            ("stream_context_get_options", fn_stream_context_get_options),
            ("stream_context_set_params", fn_stream_context_set_params),
            ("stream_context_get_params", fn_stream_context_get_params),
            ("stream_context_set_option", fn_stream_context_set_option),
            ("stream_set_blocking", fn_stream_set_blocking),
            ("stream_set_write_buffer", fn_stream_set_write_buffer),
            ("set_file_buffer", fn_set_file_buffer),
        ],
    );
}
