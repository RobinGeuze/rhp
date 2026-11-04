//! The `bcmath` extension: arbitrary-precision decimal arithmetic.
//!
//! Reimplemented in Rust under the differential-test framework
//! (`crates/rhp-difftest`) and proven byte-identical to php 8.5.8's
//! `ext/bcmath`. The arbitrary-precision integers come from `num-bigint` (a
//! faithful 1:1); the *decimal* layer on top — PHP's scale propagation,
//! truncate-toward-zero semantics, and exact string formatting — is
//! PHP-specific and hand-rolled here.
//!
//! The default scale is the `bcmath.scale` ini setting, as in php: `bcscale()`
//! changes it like `ini_set()` does, and it resets with the request's ini.

use std::cell::Cell;
use std::collections::HashMap;

use num_bigint::BigInt;
use num_traits::{One, Signed, Zero};
use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// value helpers
// ---------------------------------------------------------------------------

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn bytes_of(v: &Value) -> Vec<u8> {
    conv::to_string_bytes(&deref(v))
}

fn ten_pow(n: usize) -> BigInt {
    BigInt::from(10).pow(n as u32)
}

// ---------------------------------------------------------------------------
// a bc number: value = mant / 10^scale
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Dec {
    mant: BigInt,
    scale: usize,
}

impl Dec {
    fn zero() -> Self {
        Dec {
            mant: BigInt::zero(),
            scale: 0,
        }
    }

    fn one() -> Self {
        Dec {
            mant: BigInt::one(),
            scale: 0,
        }
    }

    /// Parse a PHP bc-number string. PHP accepts `[+-]?[0-9]*(\.[0-9]*)?` (an
    /// empty or sign-only string is 0); anything else is "not well-formed".
    fn parse(s: &[u8]) -> Option<Dec> {
        let mut i = 0;
        let mut neg = false;
        if let Some(&c) = s.first() {
            if c == b'+' {
                i = 1;
            } else if c == b'-' {
                neg = true;
                i = 1;
            }
        }
        let body = &s[i..];
        let mut int_digits: Vec<u8> = Vec::new();
        let mut frac_digits: Vec<u8> = Vec::new();
        let mut seen_dot = false;
        for &c in body {
            match c {
                b'0'..=b'9' => {
                    if seen_dot {
                        frac_digits.push(c);
                    } else {
                        int_digits.push(c);
                    }
                }
                b'.' if !seen_dot => seen_dot = true,
                _ => return None,
            }
        }
        let scale = frac_digits.len();
        let mut all = int_digits;
        all.extend_from_slice(&frac_digits);
        let mant = if all.is_empty() {
            BigInt::zero()
        } else {
            // safe: all bytes are ascii digits
            BigInt::parse_bytes(&all, 10)?
        };
        let mant = if neg && !mant.is_zero() { -mant } else { mant };
        Some(Dec { mant, scale })
    }

    /// Re-scale to exactly `target` fractional digits: pad with zeros, or
    /// truncate toward zero (dropping digits, never rounding).
    fn rescale(&self, target: usize) -> Dec {
        if self.scale == target {
            self.clone()
        } else if self.scale < target {
            Dec {
                mant: &self.mant * ten_pow(target - self.scale),
                scale: target,
            }
        } else {
            // BigInt division truncates toward zero — exactly bc's behavior.
            Dec {
                mant: &self.mant / ten_pow(self.scale - target),
                scale: target,
            }
        }
    }

    fn add(&self, other: &Dec) -> Dec {
        let s = self.scale.max(other.scale);
        Dec {
            mant: self.rescale(s).mant + other.rescale(s).mant,
            scale: s,
        }
    }

    fn sub(&self, other: &Dec) -> Dec {
        let s = self.scale.max(other.scale);
        Dec {
            mant: self.rescale(s).mant - other.rescale(s).mant,
            scale: s,
        }
    }

    fn mul(&self, other: &Dec) -> Dec {
        Dec {
            mant: &self.mant * &other.mant,
            scale: self.scale + other.scale,
        }
    }

    fn is_zero(&self) -> bool {
        self.mant.is_zero()
    }

    /// The integer part toward zero as a `BigInt` (drops the fraction).
    fn trunc_int(&self) -> BigInt {
        &self.mant / ten_pow(self.scale)
    }

    /// Format with exactly `scale` fractional digits, PHP-style (no negative
    /// zero, always at least one integer digit).
    fn format(&self, scale: usize) -> Vec<u8> {
        let d = self.rescale(scale);
        let neg = d.mant.is_negative();
        let mut digits = d.mant.abs().to_str_radix(10).into_bytes();
        // ensure at least scale+1 digits so there is an integer part
        if digits.len() < scale + 1 {
            let mut padded = vec![b'0'; scale + 1 - digits.len()];
            padded.extend_from_slice(&digits);
            digits = padded;
        }
        let split = digits.len() - scale;
        let mut out = Vec::with_capacity(digits.len() + 2);
        if neg && !d.mant.is_zero() {
            out.push(b'-');
        }
        out.extend_from_slice(&digits[..split]);
        if scale > 0 {
            out.push(b'.');
            out.extend_from_slice(&digits[split..]);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// operations with scale
// ---------------------------------------------------------------------------

fn bc_div(a: &Dec, b: &Dec, scale: usize) -> Result<Dec, Thrown> {
    if b.is_zero() {
        return Err(Thrown::error("DivisionByZeroError", "Division by zero"));
    }
    // a/b at `scale` digits, truncated toward zero:
    // q = trunc( a.mant * 10^(b.scale+scale) / (b.mant * 10^a.scale) )
    let num = &a.mant * ten_pow(b.scale + scale);
    let den = &b.mant * ten_pow(a.scale);
    Ok(Dec {
        mant: num / den,
        scale,
    })
}

fn bc_mod(a: &Dec, b: &Dec, scale: usize) -> Result<Dec, Thrown> {
    if b.is_zero() {
        return Err(Thrown::error("DivisionByZeroError", "Modulo by zero"));
    }
    // r = a - b * trunc(a/b); quotient is integer (toward zero).
    let q = bc_div(a, b, 0)?;
    let r = a.sub(&b.mul(&q));
    Ok(r.rescale(scale))
}

fn bc_pow(base: &Dec, exp: &Dec, scale: usize) -> Result<Dec, Thrown> {
    // exponent is truncated to an integer.
    let e = exp.trunc_int();
    if e.is_zero() {
        return Ok(Dec::one().rescale(scale));
    }
    let neg_exp = e.is_negative();
    let mut n = e.abs();
    // exponentiation by squaring on Dec
    let mut result = Dec::one();
    let mut b = base.clone();
    let two = BigInt::from(2);
    while !n.is_zero() {
        if (&n % &two).is_one() {
            result = result.mul(&b);
        }
        n /= &two;
        if !n.is_zero() {
            b = b.mul(&b);
        }
    }
    if neg_exp {
        bc_div(&Dec::one(), &result, scale)
    } else {
        Ok(result.rescale(scale))
    }
}

fn bc_sqrt(a: &Dec, scale: usize) -> Result<Dec, Thrown> {
    if a.mant.is_negative() {
        return Err(Thrown::error(
            "ValueError",
            "bcsqrt(): Argument #1 ($num) must be greater than or equal to 0",
        ));
    }
    if a.is_zero() {
        return Ok(Dec::zero().rescale(scale));
    }
    // floor(sqrt(value) * 10^scale) = floor(sqrt(value * 10^(2*scale)))
    // value * 10^(2*scale) = a.mant * 10^(2*scale - a.scale)
    let shift = 2 * scale as i64 - a.scale as i64;
    let radicand = if shift >= 0 {
        &a.mant * ten_pow(shift as usize)
    } else {
        &a.mant / ten_pow((-shift) as usize)
    };
    Ok(Dec {
        mant: radicand.sqrt(),
        scale,
    })
}

/// `bcround` with half-away-from-zero (PHP's default `RoundingMode::HalfAwayFromZero`).
fn bc_round(a: &Dec, places: i64) -> Dec {
    if places >= 0 && (places as usize) >= a.scale {
        return a.rescale(places as usize);
    }
    // number of fractional+integer digits to drop
    let drop = a.scale as i64 - places;
    debug_assert!(drop > 0);
    let divisor = ten_pow(drop as usize);
    let abs = a.mant.abs();
    let q = &abs / &divisor;
    let rem = &abs % &divisor;
    let mut q = q;
    if &rem * 2 >= divisor {
        q += 1;
    }
    // result lives at scale max(places,0); for negative places scale back up.
    let (mant, out_scale) = if places >= 0 {
        (q, places as usize)
    } else {
        (q * ten_pow((-places) as usize), 0)
    };
    let mant = if a.mant.is_negative() { -mant } else { mant };
    Dec {
        mant,
        scale: out_scale,
    }
}

// ---------------------------------------------------------------------------
// argument parsing
// ---------------------------------------------------------------------------

fn operand(
    args: &[Value],
    idx: usize,
    func: &str,
    argnum: usize,
    name: &str,
) -> Result<Dec, Thrown> {
    let raw = bytes_of(&args[idx]);
    Dec::parse(&raw).ok_or_else(|| {
        Thrown::error(
            "ValueError",
            format!("{func}(): Argument #{argnum} (${name}) is not well-formed"),
        )
    })
}

/// The `?int $scale` argument at position `idx` (zpp's
/// `Z_PARAM_LONG_OR_NULL`), or the default scale, checked against the
/// 0..=INT_MAX range. php parses it before it reads the operands.
fn scale_arg(
    host: &mut dyn Host,
    args: &[Value],
    idx: usize,
    func: &str,
    argnum: usize,
) -> Result<usize, Thrown> {
    let s = crate::ext::zpp::opt_long_arg(host, args, idx, func, "scale")?
        .unwrap_or_else(default_scale);
    if !(0..=2147483647).contains(&s) {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): Argument #{argnum} ($scale) must be between 0 and 2147483647"),
        ));
    }
    Ok(s as usize)
}

// ---------------------------------------------------------------------------
// builtin wrappers
// ---------------------------------------------------------------------------

fn fn_bcadd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scale = scale_arg(host, args, 2, "bcadd", 3)?;
    let a = operand(args, 0, "bcadd", 1, "num1")?;
    let b = operand(args, 1, "bcadd", 2, "num2")?;
    Ok(Value::str(a.add(&b).format(scale)))
}

fn fn_bcsub(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scale = scale_arg(host, args, 2, "bcsub", 3)?;
    let a = operand(args, 0, "bcsub", 1, "num1")?;
    let b = operand(args, 1, "bcsub", 2, "num2")?;
    Ok(Value::str(a.sub(&b).format(scale)))
}

fn fn_bcmul(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scale = scale_arg(host, args, 2, "bcmul", 3)?;
    let a = operand(args, 0, "bcmul", 1, "num1")?;
    let b = operand(args, 1, "bcmul", 2, "num2")?;
    Ok(Value::str(a.mul(&b).format(scale)))
}

fn fn_bcdiv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scale = scale_arg(host, args, 2, "bcdiv", 3)?;
    let a = operand(args, 0, "bcdiv", 1, "num1")?;
    let b = operand(args, 1, "bcdiv", 2, "num2")?;
    Ok(Value::str(bc_div(&a, &b, scale)?.format(scale)))
}

fn fn_bcmod(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scale = scale_arg(host, args, 2, "bcmod", 3)?;
    let a = operand(args, 0, "bcmod", 1, "num1")?;
    let b = operand(args, 1, "bcmod", 2, "num2")?;
    Ok(Value::str(bc_mod(&a, &b, scale)?.format(scale)))
}

fn fn_bcdivmod(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scale = scale_arg(host, args, 2, "bcdivmod", 3)?;
    let a = operand(args, 0, "bcdivmod", 1, "num1")?;
    let b = operand(args, 1, "bcdivmod", 2, "num2")?;
    if b.is_zero() {
        return Err(Thrown::error("DivisionByZeroError", "Division by zero"));
    }
    let q = bc_div(&a, &b, 0)?;
    let r = a.sub(&b.mul(&q)).rescale(scale);
    let mut arr = rhp_values::PhpArray::new();
    arr.insert(rhp_values::ArrayKey::Int(0), Value::str(q.format(0)));
    arr.insert(rhp_values::ArrayKey::Int(1), Value::str(r.format(scale)));
    Ok(Value::Array(arr))
}

fn fn_bcpow(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scale = scale_arg(host, args, 2, "bcpow", 3)?;
    let base = operand(args, 0, "bcpow", 1, "num")?;
    let exp = operand(args, 1, "bcpow", 2, "exponent")?;
    Ok(Value::str(bc_pow(&base, &exp, scale)?.format(scale)))
}

fn fn_bcpowmod(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scale = scale_arg(host, args, 3, "bcpowmod", 4)?;
    let base = operand(args, 0, "bcpowmod", 1, "num")?;
    let exp = operand(args, 1, "bcpowmod", 2, "exponent")?;
    let modulus = operand(args, 2, "bcpowmod", 3, "modulus")?;
    let e = exp.trunc_int();
    if e.is_negative() {
        return Err(Thrown::error(
            "ValueError",
            "bcpowmod(): Argument #2 ($exponent) must be greater than or equal to 0",
        ));
    }
    let m = modulus.trunc_int();
    if m.is_zero() {
        return Err(Thrown::error("DivisionByZeroError", "Modulo by zero"));
    }
    let base_i = base.trunc_int();
    let result = base_i.modpow(&e, &m.abs());
    Ok(Value::str(
        Dec {
            mant: result,
            scale: 0,
        }
        .format(scale),
    ))
}

fn fn_bcsqrt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scale = scale_arg(host, args, 1, "bcsqrt", 2)?;
    let a = operand(args, 0, "bcsqrt", 1, "num")?;
    Ok(Value::str(bc_sqrt(&a, scale)?.format(scale)))
}

fn fn_bccomp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let scale = scale_arg(host, args, 2, "bccomp", 3)?;
    let a = operand(args, 0, "bccomp", 1, "num1")?;
    let b = operand(args, 1, "bccomp", 2, "num2")?;
    let diff = a.rescale(scale).mant - b.rescale(scale).mant;
    let r = if diff.is_positive() {
        1
    } else if diff.is_negative() {
        -1
    } else {
        0
    };
    Ok(Value::Int(r))
}

fn has_fraction(a: &Dec) -> bool {
    a.scale > 0 && !(&a.mant % ten_pow(a.scale)).is_zero()
}

fn fn_bcfloor(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(args, 0, "bcfloor", 1, "num")?;
    let mut q = a.trunc_int();
    // trunc_int rounds toward zero; floor rounds toward -inf.
    if a.mant.is_negative() && has_fraction(&a) {
        q -= 1;
    }
    Ok(Value::str(Dec { mant: q, scale: 0 }.format(0)))
}

fn fn_bcceil(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(args, 0, "bcceil", 1, "num")?;
    let mut q = a.trunc_int();
    // trunc_int rounds toward zero; ceil rounds toward +inf.
    if a.mant.is_positive() && has_fraction(&a) {
        q += 1;
    }
    Ok(Value::str(Dec { mant: q, scale: 0 }.format(0)))
}

fn fn_bcround(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // $mode takes a RoundingMode, which rhp does not implement: anything a
    // php script can pass is a zpp TypeError, except a RoundingMode case.
    if let Some(mode) = args.get(2).map(deref) {
        let name = crate::argparse::value_name(host, &mode);
        if name.eq_ignore_ascii_case("RoundingMode") {
            return Err(Thrown::unsupported(
                "bcround() argument #3 ($mode)",
                "RoundingMode is not implemented",
            ));
        }
        return Err(Thrown::type_error(format!(
            "bcround(): Argument #3 ($mode) must be of type RoundingMode, {name} given"
        )));
    }
    let a = operand(args, 0, "bcround", 1, "num")?;
    let places = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let r = bc_round(&a, places);
    let out_scale = if places > 0 { places as usize } else { 0 };
    Ok(Value::str(r.format(out_scale)))
}

thread_local! {
    /// php's `BCG(bc_precision)`: the `bcmath.scale` setting as a number,
    /// kept by [`set_default_scale`] whenever the setting changes.
    static DEFAULT_SCALE: Cell<i64> = const { Cell::new(0) };
}

/// OnUpdateScale's store: `scale` is the setting's quantity, which its
/// handler only accepts within 0..=INT_MAX.
pub(crate) fn set_default_scale(scale: i64) {
    DEFAULT_SCALE.with(|s| s.set(scale));
}

/// The default scale: the `bcmath.scale` setting.
fn default_scale() -> i64 {
    DEFAULT_SCALE.with(Cell::get)
}

/// `bcscale(?int $scale = null)`: the old default scale; a new one is set
/// through the ini entry (`zend_alter_ini_entry`).
fn fn_bcscale(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let old = default_scale();
    if let Some(new) = crate::ext::zpp::opt_long_arg(host, args, 0, "bcscale", "scale")? {
        if !(0..=2147483647).contains(&new) {
            return Err(Thrown::error(
                "ValueError",
                "bcscale(): Argument #1 ($scale) must be between 0 and 2147483647",
            ));
        }
        let new = new.to_string().into_bytes();
        crate::runtime_fns::alter_ini_entry(host, b"bcmath.scale", new, "bcscale")?;
    }
    Ok(Value::Int(old))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

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
    put(m, "bcadd", fn_bcadd, 2, Some(3));
    put(m, "bcsub", fn_bcsub, 2, Some(3));
    put(m, "bcmul", fn_bcmul, 2, Some(3));
    put(m, "bcdiv", fn_bcdiv, 2, Some(3));
    put(m, "bcmod", fn_bcmod, 2, Some(3));
    put(m, "bcdivmod", fn_bcdivmod, 2, Some(3));
    put(m, "bcpow", fn_bcpow, 2, Some(3));
    put(m, "bcpowmod", fn_bcpowmod, 3, Some(4));
    put(m, "bcsqrt", fn_bcsqrt, 1, Some(2));
    put(m, "bccomp", fn_bccomp, 2, Some(3));
    put(m, "bcfloor", fn_bcfloor, 1, Some(1));
    put(m, "bcceil", fn_bcceil, 1, Some(1));
    put(m, "bcround", fn_bcround, 1, Some(3));
    put(m, "bcscale", fn_bcscale, 0, Some(1));
}
