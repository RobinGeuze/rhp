//! `vprintf`, `fprintf` and `vfprintf`, over the sprintf core in
//! [`crate::str_fns::do_sprintf`] (`sprintf`, `printf` and `vsprintf` live
//! in `str_fns`). `fprintf`/`vfprintf` write to the stream argument.

use rhp_semantics::deref;
use rhp_values::{conv, Value};

use crate::str_fns::do_sprintf;
use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

/// `(string)` bytes of a value (after deref).
fn bytes_of(v: &Value) -> Vec<u8> {
    conv::to_string_bytes(&deref(v))
}

/// Pull the values out of an `array` argument (the `$values` of a `v*printf`),
/// or raise the php `TypeError` naming the function and argument position.
fn array_values(v: &Value, fname: &str, argno: usize) -> Result<Vec<Value>, Thrown> {
    match deref(v) {
        Value::Array(arr) => Ok(arr.iter().map(|(_, v)| v.clone()).collect()),
        _ => Err(Thrown::type_error(format!(
            "{fname}(): Argument #{argno} ($values) must be of type array"
        ))),
    }
}

// ---------------------------------------------------------------------------
// vprintf
// ---------------------------------------------------------------------------

/// `vprintf(string $format, array $values): int` — array-arg form of `printf`.
fn fn_vprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = bytes_of(&args[0]);
    let values = array_values(&args[1], "vprintf", 2)?;
    // `given` mimics the equivalent printf() call: format + each value.
    let out = do_sprintf(&fmt, &values, values.len() + 1)?;
    let len = out.len() as i64;
    host.write_out(&out);
    Ok(Value::Int(len))
}

// ---------------------------------------------------------------------------
// fprintf / vfprintf
// ---------------------------------------------------------------------------

/// `fprintf(resource $stream, string $format, mixed ...$values): int` — the
/// formatted string is written to `$stream`; the formatted length is returned
/// whether or not the write succeeds (php_stream_write's result is ignored).
fn fn_fprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = crate::fs_fns::stream_param(host, args, 0, "fprintf", "stream")?;
    let fmt = bytes_of(&args[1]);
    // Argument numbering follows php: the format is arg 2, values start at arg 3,
    // so the error-reporting `given` excludes the stream argument.
    let out = do_sprintf(&fmt, &args[2..], args.len() - 1)?;
    let len = out.len() as i64;
    host.stream_write(&h, &out);
    Ok(Value::Int(len))
}

/// `vfprintf(resource $stream, string $format, array $values): int` — array-arg
/// form of `fprintf`.
fn fn_vfprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = crate::fs_fns::stream_param(host, args, 0, "vfprintf", "stream")?;
    let fmt = bytes_of(&args[1]);
    let values = array_values(&args[2], "vfprintf", 3)?;
    let out = do_sprintf(&fmt, &values, values.len() + 1)?;
    let len = out.len() as i64;
    host.stream_write(&h, &out);
    Ok(Value::Int(len))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register the printf output-family builtins into the shared table.
///
/// `sprintf`/`printf`/`vsprintf` are registered by `str_fns` and must NOT be
/// re-registered here; this module adds `vprintf`, `fprintf` and `vfprintf`.
pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("vprintf", fn_vprintf),
            ("fprintf", fn_fprintf),
            ("vfprintf", fn_vfprintf),
        ],
    );
}

#[cfg(test)]
mod tests;
