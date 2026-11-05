//! ext/standard/math.c: the math functions.
//!
//! Every function's behaviour and output formatting is matched byte-for-byte
//! against the local `php` 8.5.8 binary (var_dump floats use shortest
//! round-trip; echo uses precision 14). See `math/tests.rs` for the
//! verified expectations.
//!
//! Notes on fidelity:
//! - `floor`/`ceil`/`round`/`sqrt`/trig/… always return `float`, as in PHP.
//! - `round()` ports php 8.5's `_php_math_round` with every rounding mode
//!   (RoundingMode itself is not implemented, its int values are).
//! - `pow()` returns `int` only for an int base with a non-negative int
//!   exponent that does not overflow `i64`; otherwise `float`.
//! - `hexdec`/`bindec`/`octdec`/`base_convert` reproduce PHP's `_php_math_*`
//!   routines exactly, including the int→float switch on overflow and the
//!   lossy float→base rendering used by `base_convert` for large values.
//! - `log2()` is intentionally NOT provided: PHP has no `log2()` function
//!   (use `log($x, 2)`), so defining it would diverge from the oracle.

use rhp_values::ops::deref;
use rhp_values::{conv, fmt, Value};

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

fn fn_round(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    let value = deref(&args[0]);
    let places = args
        .get(1)
        .map(conv::to_int)
        .unwrap_or(0)
        .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let mode = args.get(2).map(conv::to_int).unwrap_or(PHP_ROUND_HALF_UP);
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

/// Integer exponentiation by squaring with overflow detection.
fn ipow(mut base: i64, mut exp: u64) -> Option<i64> {
    let mut result: i64 = 1;
    while exp > 0 {
        if exp & 1 == 1 {
            result = result.checked_mul(base)?;
        }
        exp >>= 1;
        if exp > 0 {
            base = base.checked_mul(base)?;
        }
    }
    Some(result)
}

fn fn_pow(host: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    // pow_function: ZEND_TRY_BINARY_OBJECT_OPERATION(ZEND_POW), op1's
    // handler, then op2's.
    for v in [&args[0], &args[1]] {
        if let Some(f) =
            crate::zend::object_handlers::handlers_of(host, v).and_then(|t| t.do_operation)
        {
            if let Some(r) = f(
                host,
                crate::zend::object_handlers::Op::Pow,
                &args[0],
                Some(&args[1]),
            )? {
                return Ok(r);
            }
        }
    }
    let base = conv::to_number(&deref(&args[0])).0;
    let exp = conv::to_number(&deref(&args[1])).0;
    match (base, exp) {
        (conv::Num::Int(b), conv::Num::Int(e)) if e >= 0 => match ipow(b, e as u64) {
            Some(r) => Ok(Value::Int(r)),
            None => Ok(Value::Float((b as f64).powf(e as f64))),
        },
        _ => Ok(Value::Float(base.as_f64().powf(exp.as_f64()))),
    }
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

/// PHP's `_php_math_basetozval`: parse `s` in `base`, ignoring invalid chars,
/// switching from i64 to f64 accumulation on overflow. Returns a numeric Value.
fn base_to_value(s: &[u8], base: i64) -> Value {
    let mut num: i64 = 0;
    let mut fnum: f64 = 0.0;
    let mut mode = 0u8; // 0 = integer, 1 = float
    let cutoff = i64::MAX / base;
    let cutlim = i64::MAX % base;

    for &ch in s {
        let c = match ch {
            b'0'..=b'9' => (ch - b'0') as i64,
            b'A'..=b'Z' => (ch - b'A' + 10) as i64,
            b'a'..=b'z' => (ch - b'a' + 10) as i64,
            _ => continue,
        };
        if c >= base {
            continue;
        }
        if mode == 0 {
            if num < cutoff || (num == cutoff && c <= cutlim) {
                num = num * base + c;
                continue;
            } else {
                fnum = num as f64;
                mode = 1;
            }
        }
        fnum = fnum * base as f64 + c as f64;
    }

    if mode == 0 {
        Value::Int(num)
    } else {
        Value::Float(fnum)
    }
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

fn fn_hexdec(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(base_to_value(&conv::to_string_bytes(&deref(&args[0])), 16))
}

fn fn_bindec(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(base_to_value(&conv::to_string_bytes(&deref(&args[0])), 2))
}

fn fn_octdec(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(base_to_value(&conv::to_string_bytes(&deref(&args[0])), 8))
}

fn fn_base_convert(_: &mut dyn crate::Host, args: &[Value]) -> Result<Value, Thrown> {
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
    let num = base_to_value(&s, from_base);
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

/// Increment a big-endian ascii-digit string (`"199"` -> `"200"`),
/// growing at the front on carry (`"99"` -> `"100"`).
fn increment_digits(d: &mut Vec<u8>) {
    let mut i = d.len();
    loop {
        if i == 0 {
            d.insert(0, b'1');
            return;
        }
        i -= 1;
        if d[i] == b'9' {
            d[i] = b'0';
        } else {
            d[i] += 1;
            return;
        }
    }
}

/// Round |value| (given as decimal digit strings) to `dec` decimal places using
/// half-away-from-zero, operating on the shortest decimal representation.
/// Returns (int_digits, frac_digits) where frac has length `max(dec,0)`.
fn round_decimal(int_digits: &[u8], frac_digits: &[u8], dec: i64) -> (Vec<u8>, Vec<u8>) {
    let int_len = int_digits.len() as i64;
    let mut all: Vec<u8> = Vec::with_capacity(int_digits.len() + frac_digits.len());
    all.extend_from_slice(int_digits);
    all.extend_from_slice(frac_digits);

    let keep = int_len + dec; // number of leading digits to retain
    let round_up = if keep < 0 || keep as usize >= all.len() {
        false
    } else {
        all[keep as usize] >= b'5'
    };

    let kept_len = keep.max(0) as usize;
    let mut kept: Vec<u8> = all.iter().take(kept_len).copied().collect();
    while kept.len() < kept_len {
        kept.push(b'0'); // value had fewer digits than requested
    }
    if round_up {
        increment_digits(&mut kept);
    }
    // normalize kept to a plain integer (strip leading zeros, keep >= "0")
    let first_nonzero = kept.iter().position(|&b| b != b'0').unwrap_or(kept.len());
    let kept_int: Vec<u8> = if first_nonzero == kept.len() {
        vec![b'0']
    } else {
        kept[first_nonzero..].to_vec()
    };
    let is_zero = kept_int == *b"0";

    if dec <= 0 {
        // No fractional output. For dec<0 the kept integer scales by 10^(-dec).
        let mut int_str = kept_int;
        if !is_zero && dec < 0 {
            int_str.extend(std::iter::repeat_n(b'0', (-dec) as usize));
        }
        (int_str, Vec::new())
    } else {
        let dec = dec as usize;
        let mut kint = kept_int;
        while kint.len() <= dec {
            kint.insert(0, b'0'); // ensure at least one integer digit
        }
        let split = kint.len() - dec;
        let int_str = kint[..split].to_vec();
        let frac_str = kint[split..].to_vec();
        // strip leading zeros of int part (keep one)
        let fnz = int_str
            .iter()
            .position(|&b| b != b'0')
            .unwrap_or(int_str.len());
        let int_str = if fnz == int_str.len() {
            vec![b'0']
        } else {
            int_str[fnz..].to_vec()
        };
        (int_str, frac_str)
    }
}

pub(crate) fn fn_number_format(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let num = conv::to_float(&deref(&args[0]));
    let dec = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let dec_point = match args.get(2) {
        Some(v) => conv::to_string_bytes(v),
        None => b".".to_vec(),
    };
    let thousands = match args.get(3) {
        Some(v) => conv::to_string_bytes(v),
        None => b",".to_vec(),
    };

    if !num.is_finite() {
        // php emits "nan"/"inf"; rare, keep it simple.
        return Ok(Value::str(fmt::float_echo(num)));
    }

    let negative = num < 0.0;
    let abs = num.abs();
    // Shortest decimal (Rust's Display is always non-exponential for f64).
    let s = format!("{abs}");
    let (int_part, frac_part) = match s.split_once('.') {
        Some((a, b)) => (a.as_bytes().to_vec(), b.as_bytes().to_vec()),
        None => (s.as_bytes().to_vec(), Vec::new()),
    };

    let (int_digits, frac_digits) = round_decimal(&int_part, &frac_part, dec);

    let value_is_zero = int_digits == *b"0" && frac_digits.iter().all(|&b| b == b'0');

    // Group the integer part with the thousands separator.
    let mut grouped: Vec<u8> = Vec::new();
    let n = int_digits.len();
    for (i, &d) in int_digits.iter().enumerate() {
        if i > 0 && (n - i) % 3 == 0 {
            grouped.extend_from_slice(&thousands);
        }
        grouped.push(d);
    }

    let mut out = Vec::new();
    if negative && !value_is_zero {
        out.push(b'-');
    }
    out.extend_from_slice(&grouped);
    if dec > 0 {
        out.extend_from_slice(&dec_point);
        out.extend_from_slice(&frac_digits);
    }
    Ok(Value::str(out))
}

/// math.c's constants: `M_*`, `NAN`, `INF`, `PHP_ROUND_HALF_*`.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("M_PI", Value::Float(std::f64::consts::PI)),
        ("M_E", Value::Float(std::f64::consts::E)),
        ("M_SQRT2", Value::Float(std::f64::consts::SQRT_2)),
        ("NAN", Value::Float(f64::NAN)),
        ("INF", Value::Float(f64::INFINITY)),
        ("PHP_ROUND_HALF_UP", Value::Int(PHP_ROUND_HALF_UP)),
        ("PHP_ROUND_HALF_DOWN", Value::Int(PHP_ROUND_HALF_DOWN)),
        ("PHP_ROUND_HALF_EVEN", Value::Int(PHP_ROUND_HALF_EVEN)),
        ("PHP_ROUND_HALF_ODD", Value::Int(PHP_ROUND_HALF_ODD)),
    ]
}
