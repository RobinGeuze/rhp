//! ext/standard/math.c: the math functions.
//!
//! Every function's behaviour and output formatting is matched byte-for-byte
//! against the local `php` 8.5.8 binary (var_dump floats use shortest
//! round-trip; echo uses precision 14). See `math/tests.rs` for the
//! verified expectations.
//!
//! Notes on fidelity:
//! - `floor`/`ceil`/`round`/`sqrt`/trig/… always return `float`, as in PHP.
//! - `round()` ports php 8.5's `_php_math_round` with every rounding mode;
//!   the `RoundingMode` enum is PHP source ([`PRELUDE_PHP`]).
//! - `pow()` is the `**` operator (`Host::pow_op`).
//! - `hexdec`/`bindec`/`octdec`/`base_convert` reproduce PHP's `_php_math_*`
//!   routines exactly, including the int→float switch on overflow and the
//!   lossy float→base rendering used by `base_convert` for large values.
//! - `log2()` is intentionally NOT provided: PHP has no `log2()` function
//!   (use `log($x, 2)`), so defining it would diverge from the oracle.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

// Deref helper matching the crate-wide pattern; the `conv::*` helpers already
// recurse through `Value::Ref`, so we only need an owned copy for `match`.

fn f(v: &Value) -> f64 {
    conv::to_float(v)
}

// ---------------------------------------------------------------------------
// rounding family
// ---------------------------------------------------------------------------

fn fn_floor(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(f(&args[0]).floor()))
}

fn fn_ceil(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(f(&args[0]).ceil()))
}

/// math.c's `RoundingMode` enum.
pub const PRELUDE_PHP: &str = include_str!("math/prelude.php");

/// round()'s `RoundingMode|int $mode` (`Z_PARAM_OBJ_OF_CLASS_OR_LONG`):
/// `php_math_round_mode_from_enum` for the enum, else a weak int.
pub(crate) fn rounding_mode(host: &mut dyn Host, v: &Value) -> Result<i64, Thrown> {
    let v = deref(v);
    let type_error = |host: &mut dyn Host, v: &Value| {
        Thrown::type_error(format!(
            "round(): Argument #3 ($mode) must be of type RoundingMode|int, {} given",
            crate::zend::zpp::zval_value_name(host, v)
        ))
    };
    match &v {
        Value::Object(_) => {
            let is_mode = host
                .object_class_name(&v)
                .is_some_and(|c| c.eq_ignore_ascii_case(b"RoundingMode"));
            if !is_mode {
                return Err(type_error(host, &v));
            }
            let name = host
                .object_prop(&v, b"name")
                .map(|n| conv::to_string_bytes(&n))
                .unwrap_or_default();
            Ok(match name.as_slice() {
                b"HalfAwayFromZero" => PHP_ROUND_HALF_UP,
                b"HalfTowardsZero" => PHP_ROUND_HALF_DOWN,
                b"HalfEven" => PHP_ROUND_HALF_EVEN,
                b"HalfOdd" => PHP_ROUND_HALF_ODD,
                b"TowardsZero" => PHP_ROUND_TOWARD_ZERO,
                b"AwayFromZero" => PHP_ROUND_AWAY_FROM_ZERO,
                b"NegativeInfinity" => PHP_ROUND_FLOOR,
                _ => PHP_ROUND_CEILING,
            })
        }
        Value::Null => {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                b"round(): Passing null to parameter #3 ($mode) of type RoundingMode|int is deprecated",
            )?;
            Ok(0)
        }
        _ => match crate::zend::zpp::zpp_long(host, std::slice::from_ref(&v), 0, "round", "mode") {
            Ok(n) => Ok(n),
            Err(_) => Err(type_error(host, &v)),
        },
    }
}

fn fn_round(host: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    let value = deref(&args[0]);
    let places = args
        .get(1)
        .map(conv::to_int)
        .unwrap_or(0)
        .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let mode = match args.get(2) {
        Some(v) => rounding_mode(host, v)?,
        None => PHP_ROUND_HALF_UP,
    };
    if !(1..=8).contains(&mode) {
        return Err(Thrown::error(
            "ValueError",
            "round(): Argument #3 ($mode) must be a valid rounding mode (RoundingMode::*)",
        ));
    }
    Ok(Value::Float(match value {
        // A long that needs no rounding.
        Value::Int(n) if places >= 0 => n as f64,
        v => php_math_round(conv::to_float(&v), places, mode),
    }))
}

// php_math_round_mode.h; 5..8 are reachable as ints too.
const PHP_ROUND_HALF_UP: i64 = 1;
const PHP_ROUND_HALF_DOWN: i64 = 2;
const PHP_ROUND_HALF_EVEN: i64 = 3;
const PHP_ROUND_HALF_ODD: i64 = 4;
const PHP_ROUND_CEILING: i64 = 5;
const PHP_ROUND_FLOOR: i64 = 6;
const PHP_ROUND_TOWARD_ZERO: i64 = 7;
const PHP_ROUND_AWAY_FROM_ZERO: i64 = 8;

/// `php_intpow10`: an exact table for 0..=22, else C `pow`.
fn php_intpow10(power: i32) -> f64 {
    const P: [f64; 23] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
        1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
    ];
    if (0..=22).contains(&power) {
        P[power as usize]
    } else {
        10f64.powf(power as f64)
    }
}

/// `php_round_helper`: round the integral part `integral` of `value`
/// (scaled by `exponent`) in `mode`.
fn php_round_helper(integral: f64, value: f64, exponent: f64, places: i32, mode: i64) -> f64 {
    let scale = |x: f64| {
        if places > 0 {
            (x / exponent).abs()
        } else {
            (x * exponent).abs()
        }
    };
    let basic_edge = || scale(integral + 0.5f64.copysign(integral));
    let zero_edge = || scale(integral);
    let away = integral + 1f64.copysign(integral);
    let value_abs = value.abs();
    let even = || integral % 2.0 == 0.0;
    match mode {
        PHP_ROUND_HALF_UP if value_abs >= basic_edge() => away,
        PHP_ROUND_HALF_DOWN if value_abs > basic_edge() => away,
        PHP_ROUND_CEILING if value > 0.0 && value_abs > zero_edge() => integral + 1.0,
        PHP_ROUND_FLOOR if value < 0.0 && value_abs > zero_edge() => integral - 1.0,
        PHP_ROUND_HALF_EVEN | PHP_ROUND_HALF_ODD => {
            let edge = basic_edge();
            let odd_mode = mode == PHP_ROUND_HALF_ODD;
            if value_abs > edge || (value_abs == edge && even() == odd_mode) {
                away
            } else {
                integral
            }
        }
        PHP_ROUND_HALF_UP
        | PHP_ROUND_HALF_DOWN
        | PHP_ROUND_CEILING
        | PHP_ROUND_FLOOR
        | PHP_ROUND_TOWARD_ZERO => integral,
        // PHP_ROUND_AWAY_FROM_ZERO
        _ if value_abs > zero_edge() => away,
        _ => integral,
    }
}

/// Port of php 8.5's `_php_math_round`.
#[inline]
pub(crate) fn php_math_round(value: f64, places: i32, mode: i64) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    if places == 0 && value == value.trunc() {
        return value;
    }
    let places = places.max(i32::MIN + 1);
    let exponent = php_intpow10(places.abs());
    let scaled = if places > 0 {
        value * exponent
    } else {
        value / exponent
    };
    // The integral part may be one too small in magnitude through float
    // error (0.285 * 1e10 = 2849999999.9999995): take the next one when it
    // scales back to exactly `value`.
    let (mut tmp, next) = if value >= 0.0 {
        let t = scaled.floor();
        (t, t + 1.0)
    } else {
        let t = scaled.ceil();
        (t, t - 1.0)
    };
    let back = if places > 0 {
        next / exponent
    } else {
        next * exponent
    };
    if back == value {
        tmp = next;
    }
    // Beyond the precision: rounding is pointless.
    if tmp.abs() >= 1e16 {
        return value;
    }
    tmp = php_round_helper(tmp, value, exponent, places, mode);
    if places.abs() < 23 {
        if places > 0 {
            tmp / exponent
        } else {
            tmp * exponent
        }
    } else {
        // snprintf("%15fe%d") then zend_strtod; php's snprintf prints no
        // sign for -0.0.
        let tmp = if tmp == 0.0 { 0.0 } else { tmp };
        let s = format!("{tmp:15.6}e{}", -(places as i64));
        match s.trim().parse::<f64>() {
            Ok(v) if v.is_finite() => v,
            _ => value,
        }
    }
}

// ---------------------------------------------------------------------------
// powers / roots / exponentials
// ---------------------------------------------------------------------------

fn fn_sqrt(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(f(&args[0]).sqrt()))
}

fn fn_exp(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(f(&args[0]).exp()))
}

// php's math functions call the platform libm directly. Rust's std implements
// `asinh`/`acosh`/`atanh` in pure Rust (not via libm), which can differ from php
// in the last ULP, so every function here is routed through the C libm (`libc`)
// to stay byte-identical to the oracle.
extern "C" {
    fn expm1(x: f64) -> f64;
    fn log1p(x: f64) -> f64;
    fn sinh(x: f64) -> f64;
    fn cosh(x: f64) -> f64;
    fn tanh(x: f64) -> f64;
    fn asinh(x: f64) -> f64;
    fn acosh(x: f64) -> f64;
    fn atanh(x: f64) -> f64;
}

fn fn_expm1(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(unsafe { expm1(f(&args[0])) }))
}

fn fn_log1p(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(unsafe { log1p(f(&args[0])) }))
}

fn fn_sinh(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(unsafe { sinh(f(&args[0])) }))
}

fn fn_cosh(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(unsafe { cosh(f(&args[0])) }))
}

fn fn_tanh(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(unsafe { tanh(f(&args[0])) }))
}

fn fn_asinh(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(unsafe { asinh(f(&args[0])) }))
}

fn fn_acosh(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(unsafe { acosh(f(&args[0])) }))
}

fn fn_atanh(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(unsafe { atanh(f(&args[0])) }))
}

/// `pow()` is `pow_function`, the `**` operator.
fn fn_pow(host: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    host.pow_op(&args[0], &args[1])
}

/// `fpow()`: C `pow` on two floats, no deprecation.
fn fn_fpow(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(f(&args[0]).powf(f(&args[1]))))
}

// ---------------------------------------------------------------------------
// logarithms
// ---------------------------------------------------------------------------

fn fn_log(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    let num = f(&args[0]);
    match args.get(1) {
        None => Ok(Value::Float(num.ln())),
        Some(base_v) => {
            let base = f(base_v);
            if base <= 0.0 {
                return Err(Thrown::error(
                    "ValueError",
                    "log(): Argument #2 ($base) must be greater than 0",
                ));
            }
            // PHP special-cases base 10 and 2 to the dedicated C functions,
            // which give exact results (e.g. log(8, 2) == 3.0).
            let r = if base == 10.0 {
                num.log10()
            } else if base == 2.0 {
                num.log2()
            } else if base == 1.0 {
                f64::NAN
            } else {
                num.ln() / base.ln()
            };
            Ok(Value::Float(r))
        }
    }
}

fn fn_log10(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(f(&args[0]).log10()))
}

// ---------------------------------------------------------------------------
// trigonometry
// ---------------------------------------------------------------------------

macro_rules! unary_float {
    ($name:ident, $method:ident) => {
        fn $name(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
            Ok(Value::Float(f(&args[0]).$method()))
        }
    };
}
unary_float!(fn_sin, sin);
unary_float!(fn_cos, cos);
unary_float!(fn_tan, tan);
unary_float!(fn_asin, asin);
unary_float!(fn_acos, acos);
unary_float!(fn_atan, atan);

fn fn_atan2(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    // PHP: atan2($y, $x)
    Ok(Value::Float(f(&args[0]).atan2(f(&args[1]))))
}

fn fn_pi(_: &mut dyn crate::Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(std::f64::consts::PI))
}

// ---------------------------------------------------------------------------
// fmod / fdiv / hypot / angle conversion
// ---------------------------------------------------------------------------

fn fn_fmod(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    // Rust `%` on f64 is C fmod (truncated remainder).
    Ok(Value::Float(f(&args[0]) % f(&args[1])))
}

fn fn_fdiv(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    // IEEE division: x/0 -> ±INF / NAN, never an error.
    Ok(Value::Float(f(&args[0]) / f(&args[1])))
}

fn fn_hypot(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float(f(&args[0]).hypot(f(&args[1]))))
}

// math.c's operation order; `x * PI / 180` rounds differently on about a third
// of inputs.
fn fn_deg2rad(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float((f(&args[0]) / 180.0) * std::f64::consts::PI))
}

fn fn_rad2deg(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Float((f(&args[0]) / std::f64::consts::PI) * 180.0))
}

// ---------------------------------------------------------------------------
// base conversion
// ---------------------------------------------------------------------------

const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// `dechex`/`decbin`/`decoct`: convert an i64 (reinterpreted as unsigned 64-bit,
/// like PHP's `_php_math_longtobase`) to a base-`base` string.
fn long_to_base(arg: i64, base: u64) -> Vec<u8> {
    let mut value = arg as u64;
    if value == 0 {
        return b"0".to_vec();
    }
    let mut out = Vec::new();
    while value != 0 {
        out.push(DIGITS[(value % base) as usize]);
        value /= base;
    }
    out.reverse();
    out
}

/// PHP's `_php_math_basetozval`: parse `s` in `base` (surrounding
/// whitespace and a 0x/0o/0b prefix skipped), switching from i64 to f64 on
/// overflow. Invalid characters are skipped with one deprecation.
fn base_to_value(host: &mut dyn Host, s: &[u8], base: i64) -> Result<Value, Thrown> {
    let mut num: i64 = 0;
    let mut fnum: f64 = 0.0;
    let mut float_mode = false;
    let cutoff = i64::MAX / base;
    let cutlim = i64::MAX % base;
    let is_space = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
    let start = s.iter().position(|c| !is_space(c)).unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|c| !is_space(c))
        .map_or(start, |e| e + 1);
    let mut s = &s[start..end.max(start)];
    if s.len() >= 2 && s[0] == b'0' {
        let p = s[1].to_ascii_lowercase();
        if (base == 16 && p == b'x') || (base == 8 && p == b'o') || (base == 2 && p == b'b') {
            s = &s[2..];
        }
    }
    let mut invalid = false;
    for &ch in s {
        let c = match ch {
            b'0'..=b'9' => (ch - b'0') as i64,
            b'A'..=b'Z' => (ch - b'A' + 10) as i64,
            b'a'..=b'z' => (ch - b'a' + 10) as i64,
            _ => {
                invalid = true;
                continue;
            }
        };
        if c >= base {
            invalid = true;
            continue;
        }
        if !float_mode {
            if num < cutoff || (num == cutoff && c <= cutlim) {
                num = num * base + c;
                continue;
            }
            fnum = num as f64;
            float_mode = true;
        }
        fnum = fnum * base as f64 + c as f64;
    }
    if invalid {
        host.emit_diagnostic(
            crate::E_DEPRECATED,
            b"Invalid characters passed for attempted conversion, these have been ignored",
        )?;
    }
    Ok(if float_mode {
        Value::Float(fnum)
    } else {
        Value::Int(num)
    })
}

/// PHP's `_php_math_zvaltobase`: render a (non-negative) numeric Value in
/// `base`. Integers convert exactly; floats use PHP's lossy floor/fmod loop.
fn value_to_base(v: &Value, base: i64) -> Vec<u8> {
    match v {
        Value::Int(n) => {
            let mut value = *n;
            if value == 0 {
                return b"0".to_vec();
            }
            let mut out = Vec::new();
            while value != 0 {
                out.push(DIGITS[(value % base) as usize]);
                value /= base;
            }
            out.reverse();
            out
        }
        Value::Float(fv) => {
            let base_f = base as f64;
            let mut fvalue = fv.floor();
            let mut out = Vec::new();
            loop {
                let idx = (fvalue % base_f) as usize;
                out.push(DIGITS[idx]);
                fvalue /= base_f;
                if fvalue.abs() < 1.0 {
                    break;
                }
            }
            out.reverse();
            out
        }
        _ => b"0".to_vec(),
    }
}

fn fn_dechex(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(long_to_base(conv::to_int(&args[0]), 16)))
}

fn fn_decbin(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(long_to_base(conv::to_int(&args[0]), 2)))
}

fn fn_decoct(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(long_to_base(conv::to_int(&args[0]), 8)))
}

fn fn_hexdec(host: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    base_to_value(host, &conv::to_string_bytes(&deref(&args[0])), 16)
}

fn fn_bindec(host: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    base_to_value(host, &conv::to_string_bytes(&deref(&args[0])), 2)
}

fn fn_octdec(host: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    base_to_value(host, &conv::to_string_bytes(&deref(&args[0])), 8)
}

fn fn_base_convert(host: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    let from_base = conv::to_int(&args[1]);
    let to_base = conv::to_int(&args[2]);
    if !(2..=36).contains(&from_base) {
        return Err(Thrown::error(
            "ValueError",
            "base_convert(): Argument #2 ($from_base) must be between 2 and 36 (inclusive)",
        ));
    }
    if !(2..=36).contains(&to_base) {
        return Err(Thrown::error(
            "ValueError",
            "base_convert(): Argument #3 ($to_base) must be between 2 and 36 (inclusive)",
        ));
    }
    let s = conv::to_string_bytes(&deref(&args[0]));
    let num = base_to_value(host, &s, from_base)?;
    if matches!(num, Value::Float(f) if f.is_infinite()) {
        return Err(Thrown::error(
            "ValueError",
            format!("An infinite value cannot be converted to base {to_base}"),
        ));
    }
    Ok(Value::str(value_to_base(&num, to_base)))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("floor", fn_floor),
            ("ceil", fn_ceil),
            ("round", fn_round),
            ("sqrt", fn_sqrt),
            ("pow", fn_pow),
            ("fpow", fn_fpow),
            ("exp", fn_exp),
            ("expm1", fn_expm1),
            ("log1p", fn_log1p),
            ("sinh", fn_sinh),
            ("cosh", fn_cosh),
            ("tanh", fn_tanh),
            ("asinh", fn_asinh),
            ("acosh", fn_acosh),
            ("atanh", fn_atanh),
            ("log", fn_log),
            ("log10", fn_log10),
            ("sin", fn_sin),
            ("cos", fn_cos),
            ("tan", fn_tan),
            ("asin", fn_asin),
            ("acos", fn_acos),
            ("atan", fn_atan),
            ("atan2", fn_atan2),
            ("pi", fn_pi),
            ("fmod", fn_fmod),
            ("fdiv", fn_fdiv),
            ("hypot", fn_hypot),
            ("deg2rad", fn_deg2rad),
            ("rad2deg", fn_rad2deg),
            ("dechex", fn_dechex),
            ("hexdec", fn_hexdec),
            ("decbin", fn_decbin),
            ("bindec", fn_bindec),
            ("decoct", fn_decoct),
            ("octdec", fn_octdec),
            ("base_convert", fn_base_convert),
            ("intdiv", fn_intdiv),
            ("abs", fn_abs),
            ("is_nan", fn_is_nan),
            ("is_infinite", fn_is_infinite),
            ("is_finite", fn_is_finite),
            ("number_format", fn_number_format),
        ],
    );
}

#[cfg(test)]
mod tests;

fn fn_is_nan(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        matches!(deref(&args[0]), Value::Float(f) if f.is_nan()),
    ))
}

fn fn_is_infinite(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        matches!(deref(&args[0]), Value::Float(f) if f.is_infinite()),
    ))
}

fn fn_is_finite(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        matches!(deref(&args[0]), Value::Float(f) if f.is_finite()),
    ))
}

fn fn_intdiv(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = conv::to_int(&args[0]);
    let b = conv::to_int(&args[1]);
    if b == 0 {
        return Err(Thrown::error("DivisionByZeroError", "Division by zero"));
    }
    if a == i64::MIN && b == -1 {
        return Err(Thrown::error(
            "ArithmeticError",
            "Division of PHP_INT_MIN by -1 is not an integer",
        ));
    }
    Ok(Value::Int(a / b))
}

fn fn_abs(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match conv::to_number(&deref(&args[0])).0 {
        conv::Num::Int(n) => Ok(match n.checked_abs() {
            Some(v) => Value::Int(v),
            None => Value::Float((n as f64).abs()),
        }),
        conv::Num::Float(f) => Ok(Value::Float(f.abs())),
    }
}

/// Group `digits` by three with `sep` (the integral part of number_format).
fn group_thousands(out: &mut Vec<u8>, digits: &[u8], sep: &[u8]) {
    let n = digits.len();
    for (i, &d) in digits.iter().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.extend_from_slice(sep);
        }
        out.push(d);
    }
}

/// `_php_math_number_format_ex`: round half up, print with `%.*F` (at most
/// NDIG - 2 = 318 places, the rest zero-padded), then group.
fn number_format_double(d: f64, dec: i32, dec_point: &[u8], thousand_sep: &[u8]) -> Vec<u8> {
    let mut is_negative = d < 0.0;
    let d = php_math_round(d.abs(), dec, PHP_ROUND_HALF_UP);
    let dec = dec.max(0) as usize;
    if d.is_nan() {
        return b"nan".to_vec();
    }
    if d.is_infinite() {
        return b"inf".to_vec();
    }
    let tmp = format!("{d:.*}", dec.min(318));
    if is_negative && d == 0.0 {
        is_negative = false;
    }
    let (integral, frac) = match tmp.split_once('.') {
        Some((i, f)) => (i.as_bytes(), f.as_bytes()),
        None => (tmp.as_bytes(), &b""[..]),
    };
    let mut out = Vec::with_capacity(integral.len() * 2 + dec + dec_point.len() + 1);
    if is_negative {
        out.push(b'-');
    }
    group_thousands(&mut out, integral, thousand_sep);
    if dec > 0 {
        out.extend_from_slice(dec_point);
        out.extend_from_slice(frac);
        out.extend(std::iter::repeat_n(b'0', dec - frac.len()));
    }
    out
}

/// `_php_math_number_format_long`: exact digits; a negative `dec` rounds
/// half up on the integer.
fn number_format_long(num: i64, dec: i64, dec_point: &[u8], thousand_sep: &[u8]) -> Vec<u8> {
    let mut is_negative = num < 0;
    let mut tmpnum = num.unsigned_abs();
    if dec < 0 {
        if dec < -19 {
            tmpnum = 0;
        } else {
            let power = 10u64.pow((-dec) as u32);
            let rest = tmpnum % power;
            tmpnum /= power;
            tmpnum = if rest >= power / 2 {
                tmpnum.wrapping_mul(power).wrapping_add(power)
            } else {
                tmpnum.wrapping_mul(power)
            };
        }
        if tmpnum == 0 {
            is_negative = false;
        }
    }
    let digits = tmpnum.to_string();
    let mut out = Vec::new();
    if is_negative {
        out.push(b'-');
    }
    group_thousands(&mut out, digits.as_bytes(), thousand_sep);
    if dec > 0 {
        out.extend_from_slice(dec_point);
        out.extend(std::iter::repeat_n(b'0', dec as usize));
    }
    out
}

pub(crate) fn fn_number_format(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let num = deref(&args[0]);
    let dec = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let sep = |i: usize, default: &[u8]| match args.get(i).map(deref) {
        None | Some(Value::Null) => default.to_vec(),
        Some(v) => conv::to_string_bytes(&v),
    };
    let dec_point = sep(2, b".");
    let thousand_sep = sep(3, b",");
    let out = match num {
        Value::Int(n) => number_format_long(n, dec, &dec_point, &thousand_sep),
        v => {
            let d = conv::to_float(&v);
            // A double of 2^52 or more has no fraction left.
            if d.abs() >= 4503599627370496.0
                && (-9223372036854775808.0..9223372036854775808.0).contains(&d)
            {
                number_format_long(d as i64, dec, &dec_point, &thousand_sep)
            } else {
                let dec = dec.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
                number_format_double(d, dec, &dec_point, &thousand_sep)
            }
        }
    };
    Ok(Value::str(out))
}

/// math.c's constants (the stub's values): `M_*`, `NAN`, `INF`,
/// `PHP_ROUND_HALF_*`.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("M_E", Value::Float(2.718281828459045)),
        ("M_LOG2E", Value::Float(1.4426950408889634074)),
        ("M_LOG10E", Value::Float(0.43429448190325182765)),
        ("M_LN2", Value::Float(0.69314718055994530942)),
        ("M_LN10", Value::Float(2.30258509299404568402)),
        ("M_PI", Value::Float(3.14159265358979323846)),
        ("M_PI_2", Value::Float(1.57079632679489661923)),
        ("M_PI_4", Value::Float(0.78539816339744830962)),
        ("M_1_PI", Value::Float(0.31830988618379067154)),
        ("M_2_PI", Value::Float(0.63661977236758134308)),
        ("M_SQRTPI", Value::Float(1.77245385090551602729)),
        ("M_2_SQRTPI", Value::Float(1.12837916709551257390)),
        ("M_LNPI", Value::Float(1.14472988584940017414)),
        ("M_EULER", Value::Float(0.57721566490153286061)),
        ("M_SQRT2", Value::Float(1.41421356237309504880)),
        ("M_SQRT1_2", Value::Float(0.70710678118654752440)),
        ("M_SQRT3", Value::Float(1.73205080756887729352)),
        ("NAN", Value::Float(f64::NAN)),
        ("INF", Value::Float(f64::INFINITY)),
        ("PHP_ROUND_HALF_UP", Value::Int(PHP_ROUND_HALF_UP)),
        ("PHP_ROUND_HALF_DOWN", Value::Int(PHP_ROUND_HALF_DOWN)),
        ("PHP_ROUND_HALF_EVEN", Value::Int(PHP_ROUND_HALF_EVEN)),
        ("PHP_ROUND_HALF_ODD", Value::Int(PHP_ROUND_HALF_ODD)),
    ]
}
