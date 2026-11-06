//! ext/standard/formatted_print.c: the `printf` family over one sprintf
//! core ([`do_sprintf`]). `fprintf`/`vfprintf` write to the stream argument.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

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
    let fmt = conv::to_string_bytes(&args[0]);
    let values = array_values(&args[1], "vprintf", 2)?;
    let out = do_sprintf(host, &fmt, &values, None)?;
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
    let (h, _) = crate::ext::standard::file::stream_param(host, args, 0, "fprintf", "stream")?;
    let fmt = conv::to_string_bytes(&args[1]);
    let out = do_sprintf(host, &fmt, &args[2..], Some(2))?;
    let len = out.len() as i64;
    host.stream_write(&h, &out);
    Ok(Value::Int(len))
}

/// `vfprintf(resource $stream, string $format, array $values): int` — array-arg
/// form of `fprintf`.
fn fn_vfprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, _) = crate::ext::standard::file::stream_param(host, args, 0, "vfprintf", "stream")?;
    let fmt = conv::to_string_bytes(&args[1]);
    let values = array_values(&args[2], "vfprintf", 3)?;
    let out = do_sprintf(host, &fmt, &values, None)?;
    let len = out.len() as i64;
    host.stream_write(&h, &out);
    Ok(Value::Int(len))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// number_format
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// sprintf / printf / vsprintf
// ---------------------------------------------------------------------------

const ALIGN_LEFT: bool = true;
const FLOAT_PRECISION: i64 = 6;
const MAX_FLOAT_PRECISION: i64 = 53;
const INT_MAX: i64 = i32::MAX as i64;

/// "Field width %zd is too long": a fatal error.
fn field_too_long(host: &mut dyn Host, width: usize) -> Result<(), Thrown> {
    host.emit_diagnostic(
        crate::E_ERROR,
        format!("Field width {width} is too long").as_bytes(),
    )?;
    Err(Thrown::Exit(255))
}

fn value_error(msg: impl Into<String>) -> Thrown {
    Thrown::error("ValueError", msg.into())
}

/// `php_sprintf_appendstring`: pad `add` to `min_width`; with `expprec`
/// only the first `max_width` bytes are kept. A right-aligned zero-padded
/// number keeps its sign in front of the padding.
#[allow(clippy::too_many_arguments)]
fn append_string(
    out: &mut Vec<u8>,
    add: &[u8],
    min_width: usize,
    max_width: usize,
    padding: u8,
    left: bool,
    neg: bool,
    expprec: bool,
    always_sign: bool,
) -> Result<(), usize> {
    let mut add = add;
    let mut copy_len = if expprec {
        max_width.min(add.len())
    } else {
        add.len()
    };
    let npad = min_width.saturating_sub(copy_len);
    let m_width = min_width.max(copy_len);
    if m_width as u64 > (i32::MAX as u64).saturating_sub(out.len() as u64 + 1) {
        return Err(m_width);
    }
    if !left {
        if (neg || always_sign) && padding == b'0' {
            out.push(if neg { b'-' } else { b'+' });
            add = &add[1.min(add.len())..];
            copy_len = copy_len.saturating_sub(1);
        }
        out.extend(std::iter::repeat_n(padding, npad));
    }
    out.extend_from_slice(&add[..copy_len.min(add.len())]);
    if left {
        out.extend(std::iter::repeat_n(padding, npad));
    }
    Ok(())
}

/// `php_sprintf_appendint`.
fn append_int(
    out: &mut Vec<u8>,
    number: i64,
    width: usize,
    padding: u8,
    left: bool,
    always_sign: bool,
) -> Result<(), usize> {
    // Can't right-pad 0's on integers.
    let padding = if left && padding == b'0' {
        b' '
    } else {
        padding
    };
    let mut buf = Vec::with_capacity(21);
    if number < 0 {
        buf.push(b'-');
    } else if always_sign {
        buf.push(b'+');
    }
    buf.extend_from_slice(number.unsigned_abs().to_string().as_bytes());
    append_string(
        out,
        &buf,
        width,
        0,
        padding,
        left,
        number < 0,
        false,
        always_sign,
    )
}

/// `php_sprintf_appenduint`.
fn append_uint(
    out: &mut Vec<u8>,
    number: u64,
    width: usize,
    padding: u8,
    left: bool,
) -> Result<(), usize> {
    let padding = if left && padding == b'0' {
        b' '
    } else {
        padding
    };
    let buf = number.to_string();
    append_string(
        out,
        buf.as_bytes(),
        width,
        0,
        padding,
        left,
        false,
        false,
        false,
    )
}

/// `php_sprintf_append2n`: binary, octal or hex digits of the unsigned
/// value. With a precision (`expprec`) the maximum width is 0, so nothing
/// is printed but the padding, as in php.
#[allow(clippy::too_many_arguments)]
fn append_2n(
    out: &mut Vec<u8>,
    number: i64,
    width: usize,
    padding: u8,
    left: bool,
    n: u32,
    upper: bool,
    expprec: bool,
) -> Result<(), usize> {
    let chars: &[u8; 16] = if upper {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mask = (1u64 << n) - 1;
    let mut num = number as u64;
    let mut buf = Vec::with_capacity(64);
    loop {
        buf.push(chars[(num & mask) as usize]);
        num >>= n;
        if num == 0 {
            break;
        }
    }
    buf.reverse();
    append_string(out, &buf, width, 0, padding, left, false, expprec, false)
}

/// `php_conv_fp` for `F`, `e` and `E` on a finite number: the digits with
/// `precision` places, without the sign.
fn conv_fp(format: u8, num: f64, precision: usize) -> Vec<u8> {
    let abs = num.abs();
    match format {
        b'F' => format!("{abs:.precision$}").into_bytes(),
        _ => {
            let r = format!("{abs:.precision$e}");
            let (m, e) = r.split_once('e').unwrap_or((&r, "0"));
            let exp: i64 = e.parse().unwrap_or(0);
            let mut out = m.as_bytes().to_vec();
            out.push(format);
            out.push(if exp < 0 { b'-' } else { b'+' });
            out.extend_from_slice(exp.unsigned_abs().to_string().as_bytes());
            out
        }
    }
}

/// `php_sprintf_appenddouble`.
#[allow(clippy::too_many_arguments)]
fn append_double(
    host: &mut dyn Host,
    out: &mut Vec<u8>,
    number: f64,
    width: usize,
    padding: u8,
    left: bool,
    precision: i64,
    adjust_precision: bool,
    fmt: u8,
    always_sign: bool,
) -> Result<(), Thrown> {
    let mut precision = precision;
    if !adjust_precision {
        precision = FLOAT_PRECISION;
    } else if precision > MAX_FLOAT_PRECISION {
        host.docref_diagnostic(
            crate::E_NOTICE,
            format!(
                "Requested precision of {precision} digits was truncated to PHP maximum of {MAX_FLOAT_PRECISION} digits"
            )
            .as_bytes(),
        )?;
        precision = MAX_FLOAT_PRECISION;
    }
    if number.is_nan() {
        return append_string(out, b"NaN", 3, 0, padding, left, false, false, always_sign)
            .or_else(|w| field_too_long(host, w));
    }
    if number.is_infinite() {
        let neg = number < 0.0;
        let s: &[u8] = if neg { b"-INF" } else { b"INF" };
        return append_string(out, s, s.len(), 0, padding, left, neg, false, always_sign)
            .or_else(|w| field_too_long(host, w));
    }
    let mut s = Vec::new();
    let is_negative;
    match fmt {
        b'e' | b'E' | b'f' | b'F' => {
            let f = if fmt == b'f' { b'F' } else { fmt };
            is_negative = number < 0.0;
            if is_negative {
                s.push(b'-');
            } else if always_sign {
                s.push(b'+');
            }
            let mut fp = conv_fp(f, number, precision.max(0) as usize);
            if fmt == b'f' {
                let dp = crate::ext::standard::string::locale_decimal_point();
                if dp != b'.' {
                    fp.iter_mut().filter(|c| **c == b'.').for_each(|c| *c = dp);
                }
            }
            s.extend_from_slice(&fp);
        }
        _ => {
            // g G h H
            if precision == 0 {
                precision = 1;
            }
            let exp_char = if fmt == b'G' || fmt == b'H' {
                b'E'
            } else {
                b'e'
            };
            let mut g = rhp_values::fmt::zend_gcvt(number, precision, exp_char);
            if fmt == b'g' || fmt == b'G' {
                let dp = crate::ext::standard::string::locale_decimal_point();
                if dp != b'.' {
                    g.iter_mut().filter(|c| **c == b'.').for_each(|c| *c = dp);
                }
            }
            is_negative = g.first() == Some(&b'-');
            if !is_negative && always_sign {
                s.push(b'+');
            }
            s.extend_from_slice(&g);
        }
    }
    append_string(
        out,
        &s,
        width,
        0,
        padding,
        left,
        is_negative,
        false,
        always_sign,
    )
    .or_else(|w| field_too_long(host, w))
}

/// `php_sprintf_getnumber`: the digits at `*i` as a number, -1 when it
/// does not fit below INT_MAX.
fn get_number(fmt: &[u8], i: &mut usize) -> i64 {
    let start = *i;
    let mut num: i64 = 0;
    let mut overflow = false;
    while *i < fmt.len() && fmt[*i].is_ascii_digit() {
        match num
            .checked_mul(10)
            .and_then(|n| n.checked_add((fmt[*i] - b'0') as i64))
        {
            Some(n) => num = n,
            None => overflow = true,
        }
        *i += 1;
    }
    if *i == start {
        return 0;
    }
    if overflow || num >= INT_MAX {
        -1
    } else {
        num
    }
}

/// `php_sprintf_get_argnum`: `Ok(None)` when no `N$` follows (the next
/// argument), else the 0-based argument number.
fn get_argnum(fmt: &[u8], i: &mut usize) -> Result<Option<usize>, Thrown> {
    let mut t = *i;
    while t < fmt.len() && fmt[t].is_ascii_digit() {
        t += 1;
    }
    if fmt.get(t) != Some(&b'$') {
        return Ok(None);
    }
    let argnum = get_number(fmt, i);
    if argnum <= 0 {
        return Err(value_error(format!(
            "Argument number specifier must be greater than zero and less than {INT_MAX}"
        )));
    }
    *i += 1; // the '$'
    Ok(Some(argnum as usize - 1))
}

/// `php_formatted_print`: `args` are the values after the format;
/// `nb_additional` is how many arguments precede them (format included)
/// for the ArgumentCountError, or `None` for the `v*` functions' ValueError.
pub(crate) fn do_sprintf(
    host: &mut dyn Host,
    fmt: &[u8],
    args: &[Value],
    nb_additional: Option<usize>,
) -> Result<Vec<u8>, Thrown> {
    let argc = args.len();
    let mut out = Vec::with_capacity(fmt.len() + 16);
    let mut currarg = 0usize;
    let mut max_missing: Option<usize> = None;
    let at = |i: usize| fmt.get(i).copied().unwrap_or(0);
    let mut i = 0usize;
    while i < fmt.len() {
        let Some(p) = fmt[i..].iter().position(|&c| c == b'%') else {
            out.extend_from_slice(&fmt[i..]);
            break;
        };
        out.extend_from_slice(&fmt[i..i + p]);
        i += p + 1; // skip the '%'
        if at(i) == b'%' {
            out.push(b'%');
            i += 1;
            continue;
        }
        let mut left = false;
        let mut adjust_precision = false;
        let mut padding = b' ';
        let mut always_sign = false;
        let mut expprec = false;
        let mut width: i64 = 0;
        let mut precision: i64 = 0;
        let mut argnum: Option<usize> = None;
        if !at(i).is_ascii_alphabetic() {
            argnum = get_argnum(fmt, &mut i)?;
            // modifiers
            loop {
                match at(i) {
                    c @ (b' ' | b'0') => padding = c,
                    b'-' => left = ALIGN_LEFT,
                    b'+' => always_sign = true,
                    b'\'' => {
                        if fmt.len() - i > 1 {
                            i += 1;
                            padding = at(i);
                        } else {
                            return Err(value_error("Missing padding character"));
                        }
                    }
                    _ => break,
                }
                i += 1;
            }
            // width
            if at(i) == b'*' {
                i += 1;
                let wa = match get_argnum(fmt, &mut i)? {
                    Some(n) => n,
                    None => {
                        currarg += 1;
                        currarg - 1
                    }
                };
                if wa >= argc {
                    max_missing = max_missing.max(Some(wa));
                    continue;
                }
                let Value::Int(w) = deref(&args[wa]) else {
                    return Err(value_error("Width must be an integer"));
                };
                if !(0..=INT_MAX).contains(&w) {
                    return Err(value_error(format!(
                        "Width must be between 0 and {INT_MAX}"
                    )));
                }
                width = w;
            } else if at(i).is_ascii_digit() {
                width = get_number(fmt, &mut i);
                if width < 0 {
                    return Err(value_error(format!(
                        "Width must be between 0 and {INT_MAX}"
                    )));
                }
            }
            // precision
            if at(i) == b'.' {
                i += 1;
                adjust_precision = true;
                if at(i) == b'*' {
                    i += 1;
                    let pa = match get_argnum(fmt, &mut i)? {
                        Some(n) => n,
                        None => {
                            currarg += 1;
                            currarg - 1
                        }
                    };
                    if pa >= argc {
                        max_missing = max_missing.max(Some(pa));
                        continue;
                    }
                    let Value::Int(p) = deref(&args[pa]) else {
                        return Err(value_error("Precision must be an integer"));
                    };
                    if !(-1..=INT_MAX).contains(&p) {
                        return Err(value_error(format!(
                            "Precision must be between -1 and {INT_MAX}"
                        )));
                    }
                    precision = p;
                    expprec = true;
                } else if at(i).is_ascii_digit() {
                    precision = get_number(fmt, &mut i);
                    if precision < 0 {
                        return Err(value_error(format!(
                            "Precision must be between 0 and {INT_MAX}"
                        )));
                    }
                    expprec = true;
                }
            }
        }
        if at(i) == b'l' {
            i += 1;
        }
        let argnum = match argnum {
            Some(n) => n,
            None => {
                currarg += 1;
                currarg - 1
            }
        };
        if argnum >= argc {
            max_missing = max_missing.max(Some(argnum));
            continue;
        }
        let spec = at(i);
        if expprec && precision == -1 && !matches!(spec, b'g' | b'G' | b'h' | b'H') {
            return Err(value_error(
                "Precision -1 is only supported for %g, %G, %h and %H",
            ));
        }
        let width = width as usize;
        let tmp = &args[argnum];
        match spec {
            b's' => {
                let s = crate::zend::zpp::zval_get_string(host, tmp)?;
                append_string(
                    &mut out,
                    &s,
                    width,
                    precision.max(0) as usize,
                    padding,
                    left,
                    false,
                    expprec,
                    false,
                )
                .or_else(|w| field_too_long(host, w))?;
            }
            b'd' => {
                let n = crate::zend::zpp::zval_get_long(host, tmp)?;
                append_int(&mut out, n, width, padding, left, always_sign)
                    .or_else(|w| field_too_long(host, w))?;
            }
            b'u' => {
                let n = crate::zend::zpp::zval_get_long(host, tmp)?;
                append_uint(&mut out, n as u64, width, padding, left)
                    .or_else(|w| field_too_long(host, w))?;
            }
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'h' | b'H' => {
                let d = zval_get_double(host, tmp)?;
                append_double(
                    host,
                    &mut out,
                    d,
                    width,
                    padding,
                    left,
                    precision,
                    adjust_precision,
                    spec,
                    always_sign,
                )?;
            }
            b'c' => {
                let n = crate::zend::zpp::zval_get_long(host, tmp)?;
                out.push(n as u8);
            }
            b'o' | b'x' | b'X' | b'b' => {
                let n = crate::zend::zpp::zval_get_long(host, tmp)?;
                let bits = match spec {
                    b'o' => 3,
                    b'b' => 1,
                    _ => 4,
                };
                append_2n(
                    &mut out,
                    n,
                    width,
                    padding,
                    left,
                    bits,
                    spec == b'X',
                    expprec,
                )
                .or_else(|w| field_too_long(host, w))?;
            }
            b'%' => out.push(b'%'),
            0 if i >= fmt.len() => {
                return Err(value_error("Missing format specifier at end of string"));
            }
            c => {
                return Err(value_error(format!(
                    "Unknown format specifier \"{}\"",
                    char::from(c)
                )));
            }
        }
        i += 1;
    }
    if let Some(missing) = max_missing {
        return Err(match nb_additional {
            None => value_error(format!(
                "The arguments array must contain {} items, {argc} given",
                missing + 1
            )),
            Some(nb) => Thrown::error(
                "ArgumentCountError",
                format!(
                    "{} arguments are required, {} given",
                    missing + nb + 1,
                    argc + nb
                ),
            ),
        });
    }
    Ok(out)
}

/// zval_get_double: an object's cast_object, else a warning and 1.0.
pub(crate) fn zval_get_double(host: &mut dyn Host, v: &Value) -> Result<f64, Thrown> {
    let v = deref(v);
    if let Some(n) = crate::zend::object_handlers::object_to_number(
        host,
        &v,
        crate::zend::object_handlers::CastTarget::Float,
    )? {
        return Ok(conv::to_float(&n));
    }
    Ok(conv::to_float(&v))
}

fn fn_sprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = conv::to_string_bytes(&args[0]);
    let out = do_sprintf(host, &fmt, &args[1..], Some(1))?;
    Ok(Value::str(out))
}

fn fn_printf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = conv::to_string_bytes(&args[0]);
    let out = do_sprintf(host, &fmt, &args[1..], Some(1))?;
    let len = out.len() as i64;
    host.write_out(&out);
    Ok(Value::Int(len))
}

fn fn_vsprintf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = conv::to_string_bytes(&args[0]);
    let Value::Array(arr) = deref(&args[1]) else {
        return Err(Thrown::type_error(
            "vsprintf(): Argument #2 ($values) must be of type array",
        ));
    };
    let values: Vec<Value> = arr.iter().map(|(_, v)| v.clone()).collect();
    let out = do_sprintf(host, &fmt, &values, None)?;
    Ok(Value::str(out))
}

/// Register the printf family into the shared table.
pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("vprintf", fn_vprintf),
            ("fprintf", fn_fprintf),
            ("vfprintf", fn_vfprintf),
            ("sprintf", fn_sprintf),
            ("printf", fn_printf),
            ("vsprintf", fn_vsprintf),
        ],
    );
}
