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
    let (h, _) = crate::ext::standard::file::stream_param(host, args, 0, "fprintf", "stream")?;
    let fmt = conv::to_string_bytes(&args[1]);
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
    let (h, _) = crate::ext::standard::file::stream_param(host, args, 0, "vfprintf", "stream")?;
    let fmt = conv::to_string_bytes(&args[1]);
    let values = array_values(&args[2], "vfprintf", 3)?;
    let out = do_sprintf(&fmt, &values, values.len() + 1)?;
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

struct Conv {
    arg: Option<usize>, // explicit `N$` (1-based)
    left: bool,
    plus: bool,
    pad: u8,
    width: usize,
    prec: Option<usize>,
    spec: u8,
}

enum Piece {
    Lit(Vec<u8>),
    Conv(Conv),
}

fn parse_format(fmt: &[u8]) -> Result<Vec<Piece>, Thrown> {
    let mut pieces = Vec::new();
    let mut lit = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' {
            lit.push(fmt[i]);
            i += 1;
            continue;
        }
        // '%' found
        i += 1;
        if i < fmt.len() && fmt[i] == b'%' {
            lit.push(b'%');
            i += 1;
            continue;
        }
        if !lit.is_empty() {
            pieces.push(Piece::Lit(std::mem::take(&mut lit)));
        }

        // optional argnum: digits followed by '$'
        let mut arg = None;
        {
            let mut j = i;
            while j < fmt.len() && fmt[j].is_ascii_digit() {
                j += 1;
            }
            if j > i && j < fmt.len() && fmt[j] == b'$' {
                let n: usize = std::str::from_utf8(&fmt[i..j])
                    .unwrap()
                    .parse()
                    .unwrap_or(0);
                arg = Some(n);
                i = j + 1;
            }
        }

        // flags
        let mut left = false;
        let mut plus = false;
        let mut pad = b' ';
        loop {
            if i >= fmt.len() {
                break;
            }
            match fmt[i] {
                b'-' => left = true,
                b'+' => plus = true,
                b' ' => pad = b' ',
                b'0' => pad = b'0',
                b'\'' => {
                    i += 1;
                    if i >= fmt.len() {
                        return Err(Thrown::error(
                            "ValueError",
                            "Missing format specifier at end of string",
                        ));
                    }
                    pad = fmt[i];
                }
                _ => break,
            }
            i += 1;
        }

        // width
        let ws = i;
        while i < fmt.len() && fmt[i].is_ascii_digit() {
            i += 1;
        }
        let width: usize = if i > ws {
            std::str::from_utf8(&fmt[ws..i])
                .unwrap()
                .parse()
                .unwrap_or(0)
        } else {
            0
        };

        // precision
        let mut prec = None;
        if i < fmt.len() && fmt[i] == b'.' {
            i += 1;
            let ps = i;
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                i += 1;
            }
            prec = Some(if i > ps {
                std::str::from_utf8(&fmt[ps..i])
                    .unwrap()
                    .parse()
                    .unwrap_or(0)
            } else {
                0
            });
        }

        // A C length modifier is skipped.
        if i < fmt.len() && fmt[i] == b'l' {
            i += 1;
        }
        if i >= fmt.len() {
            return Err(Thrown::error(
                "ValueError",
                "Missing format specifier at end of string",
            ));
        }
        let spec = fmt[i];
        i += 1;
        if !matches!(
            spec,
            b's' | b'd'
                | b'f'
                | b'F'
                | b'b'
                | b'o'
                | b'x'
                | b'X'
                | b'c'
                | b'u'
                | b'e'
                | b'E'
                | b'g'
                | b'G'
        ) {
            return Err(Thrown::error(
                "ValueError",
                format!("Unknown format specifier \"{}\"", spec as char),
            ));
        }
        pieces.push(Piece::Conv(Conv {
            arg,
            left,
            plus,
            pad,
            width,
            prec,
            spec,
        }));
    }
    if !lit.is_empty() {
        pieces.push(Piece::Lit(lit));
    }
    Ok(pieces)
}

/// Field-pad a string conversion (pad char used as-is on either side).
fn pad_field(body: Vec<u8>, width: usize, pad: u8, left: bool) -> Vec<u8> {
    if body.len() >= width {
        return body;
    }
    let fill = width - body.len();
    let mut out = Vec::with_capacity(width);
    if left {
        out.extend_from_slice(&body);
        out.extend(std::iter::repeat_n(pad, fill));
    } else {
        out.extend(std::iter::repeat_n(pad, fill));
        out.extend_from_slice(&body);
    }
    out
}

/// Field-pad a numeric conversion (sign kept with the magnitude; `'0'` pads
/// after the sign when right-aligned, and degrades to space when left-aligned).
fn pad_numeric(sign: &[u8], mag: &[u8], width: usize, pad: u8, left: bool) -> Vec<u8> {
    let total = sign.len() + mag.len();
    let mut out = Vec::with_capacity(width.max(total));
    if total >= width {
        out.extend_from_slice(sign);
        out.extend_from_slice(mag);
        return out;
    }
    let fill = width - total;
    if left {
        let p = if pad == b'0' { b' ' } else { pad };
        out.extend_from_slice(sign);
        out.extend_from_slice(mag);
        out.extend(std::iter::repeat_n(p, fill));
    } else if pad == b'0' {
        out.extend_from_slice(sign);
        out.extend(std::iter::repeat_n(b'0', fill));
        out.extend_from_slice(mag);
    } else {
        out.extend(std::iter::repeat_n(pad, fill));
        out.extend_from_slice(sign);
        out.extend_from_slice(mag);
    }
    out
}

/// php-style exponent reformat: Rust's `{:e}` gives `1.23e4`; php wants
/// `1.23e+4` (signed, minimal digits). Applies to the already-built mantissa.
fn reformat_exp(rust_e: &str, upper: bool) -> Vec<u8> {
    let (m, e) = rust_e.split_once('e').unwrap();
    let exp: i64 = e.parse().unwrap();
    let mut out = m.as_bytes().to_vec();
    out.push(if upper { b'E' } else { b'e' });
    out.push(if exp < 0 { b'-' } else { b'+' });
    out.extend_from_slice(exp.unsigned_abs().to_string().as_bytes());
    out
}

/// php `%g` / `%G`: `zend_gcvt` with the given significant-digit precision.
fn gcvt(f: f64, precision: usize, upper: bool) -> Vec<u8> {
    let prec = precision.max(1);
    let negative = f < 0.0;
    let sign: &[u8] = if negative { b"-" } else { b"" };
    let abs = f.abs();
    if abs == 0.0 {
        return [sign, b"0"].concat();
    }
    let s = format!("{:.*e}", prec - 1, abs);
    let (mant, exp) = s.split_once('e').unwrap();
    let exp10: i32 = exp.parse().unwrap();
    let digits: Vec<u8> = mant.bytes().filter(|&b| b != b'.').collect();
    // trim trailing zeros, keep >= 1
    let mut end = digits.len();
    while end > 1 && digits[end - 1] == b'0' {
        end -= 1;
    }
    let digits = &digits[..end];

    let cutoff = prec as i32;
    let body = if exp10 < -4 || exp10 >= cutoff {
        // exponential, always >= 1 fractional digit
        let mut out = Vec::new();
        out.push(digits[0]);
        out.push(b'.');
        if digits.len() > 1 {
            out.extend_from_slice(&digits[1..]);
        } else {
            out.push(b'0');
        }
        out.push(if upper { b'E' } else { b'e' });
        out.push(if exp10 < 0 { b'-' } else { b'+' });
        out.extend_from_slice(exp10.unsigned_abs().to_string().as_bytes());
        out
    } else if exp10 >= 0 {
        let int_len = (exp10 as usize) + 1;
        let mut out = Vec::new();
        if digits.len() <= int_len {
            out.extend_from_slice(digits);
            out.extend(std::iter::repeat_n(b'0', int_len - digits.len()));
        } else {
            out.extend_from_slice(&digits[..int_len]);
            out.push(b'.');
            out.extend_from_slice(&digits[int_len..]);
        }
        out
    } else {
        let mut out = Vec::new();
        out.extend_from_slice(b"0.");
        out.extend(std::iter::repeat_n(b'0', (-exp10 - 1) as usize));
        out.extend_from_slice(digits);
        out
    };
    [sign, &body].concat()
}

/// Format one non-special float conversion (f/F/e/E). Returns (sign, magnitude).
fn float_parts(f: f64, spec: u8, prec: usize) -> (Vec<u8>, Vec<u8>) {
    let negative = f < 0.0;
    let abs = f.abs();
    let mag = match spec {
        b'f' | b'F' => format!("{abs:.prec$}").into_bytes(),
        b'e' | b'E' => {
            let r = format!("{:.*e}", prec, abs);
            reformat_exp(&r, spec == b'E')
        }
        _ => unreachable!(),
    };
    let sign: Vec<u8> = if negative { b"-".to_vec() } else { Vec::new() };
    (sign, mag)
}

fn render_conv(c: &Conv, arg: &Value) -> Vec<u8> {
    let v = deref(arg);
    match c.spec {
        b's' => {
            let mut s = conv::to_string_bytes(&v);
            if let Some(p) = c.prec {
                s.truncate(p);
            }
            pad_field(s, c.width, c.pad, c.left)
        }
        b'c' => {
            // single byte, width/precision/flags ignored (php behavior)
            let n = conv::to_int(&v);
            vec![(n & 0xff) as u8]
        }
        b'd' => {
            let n = conv::to_int(&v);
            let neg = n < 0;
            let mag = n.unsigned_abs().to_string().into_bytes();
            let sign: &[u8] = if neg {
                b"-"
            } else if c.plus {
                b"+"
            } else {
                b""
            };
            pad_numeric(sign, &mag, c.width, c.pad, c.left)
        }
        b'u' => {
            let n = conv::to_int(&v) as u64;
            pad_numeric(b"", n.to_string().as_bytes(), c.width, c.pad, c.left)
        }
        b'o' => {
            let n = conv::to_int(&v) as u64;
            pad_numeric(b"", format!("{n:o}").as_bytes(), c.width, c.pad, c.left)
        }
        b'x' => {
            let n = conv::to_int(&v) as u64;
            pad_numeric(b"", format!("{n:x}").as_bytes(), c.width, c.pad, c.left)
        }
        b'X' => {
            let n = conv::to_int(&v) as u64;
            pad_numeric(b"", format!("{n:X}").as_bytes(), c.width, c.pad, c.left)
        }
        b'b' => {
            let n = conv::to_int(&v) as u64;
            pad_numeric(b"", format!("{n:b}").as_bytes(), c.width, c.pad, c.left)
        }
        b'f' | b'F' | b'e' | b'E' => {
            let f = conv::to_float(&v);
            if f.is_nan() {
                return b"NaN".to_vec();
            }
            if f.is_infinite() {
                return if f < 0.0 {
                    b"-INF".to_vec()
                } else {
                    b"INF".to_vec()
                };
            }
            let prec = c.prec.unwrap_or(6);
            let (mut sign, mag) = float_parts(f, c.spec, prec);
            if sign.is_empty() && c.plus {
                sign = b"+".to_vec();
            }
            pad_numeric(&sign, &mag, c.width, c.pad, c.left)
        }
        b'g' | b'G' => {
            let f = conv::to_float(&v);
            if f.is_nan() {
                return b"NaN".to_vec();
            }
            if f.is_infinite() {
                return if f < 0.0 {
                    b"-INF".to_vec()
                } else {
                    b"INF".to_vec()
                };
            }
            let prec = c.prec.unwrap_or(6);
            let body = gcvt(f, prec, c.spec == b'G');
            // split sign for numeric padding
            if let Some(stripped) = body.strip_prefix(b"-") {
                pad_numeric(b"-", stripped, c.width, c.pad, c.left)
            } else {
                let sign: &[u8] = if c.plus { b"+" } else { b"" };
                pad_numeric(sign, &body, c.width, c.pad, c.left)
            }
        }
        _ => Vec::new(),
    }
}

/// Core of sprintf/vsprintf: `fmt` + value args (not including the format).
/// `given` is the total argument count (incl. the format) for error messages.
pub(crate) fn do_sprintf(
    fmt: &[u8],
    value_args: &[Value],
    given: usize,
) -> Result<Vec<u8>, Thrown> {
    let pieces = parse_format(fmt)?;

    // Resolve argument indices (positional does not advance the sequential ptr).
    let mut seq = 1usize;
    let mut resolved: Vec<usize> = Vec::new();
    let mut max_idx = 0usize;
    for p in &pieces {
        if let Piece::Conv(c) = p {
            let idx = match c.arg {
                Some(n) => n,
                None => {
                    let v = seq;
                    seq += 1;
                    v
                }
            };
            resolved.push(idx);
            max_idx = max_idx.max(idx);
        }
    }
    if max_idx > value_args.len() {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!("{} arguments are required, {} given", max_idx + 1, given),
        ));
    }

    let mut out = Vec::new();
    let mut ci = 0;
    for p in &pieces {
        match p {
            Piece::Lit(b) => out.extend_from_slice(b),
            Piece::Conv(c) => {
                let idx = resolved[ci];
                ci += 1;
                out.extend_from_slice(&render_conv(c, &value_args[idx - 1]));
            }
        }
    }
    Ok(out)
}

fn fn_sprintf(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = conv::to_string_bytes(&args[0]);
    let out = do_sprintf(&fmt, &args[1..], args.len())?;
    Ok(Value::str(out))
}

fn fn_printf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = conv::to_string_bytes(&args[0]);
    let out = do_sprintf(&fmt, &args[1..], args.len())?;
    let len = out.len() as i64;
    host.write_out(&out);
    Ok(Value::Int(len))
}

fn fn_vsprintf(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = conv::to_string_bytes(&args[0]);
    let Value::Array(arr) = deref(&args[1]) else {
        return Err(Thrown::type_error(
            "vsprintf(): Argument #2 ($values) must be of type array",
        ));
    };
    let values: Vec<Value> = arr.iter().map(|(_, v)| v.clone()).collect();
    // `given` mimics the equivalent sprintf() call: format + each value.
    let out = do_sprintf(&fmt, &values, values.len() + 1)?;
    Ok(Value::str(out))
}

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
            ("sprintf", fn_sprintf),
            ("printf", fn_printf),
            ("vsprintf", fn_vsprintf),
        ],
    );
}
