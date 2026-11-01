//! The `gmp` extension: arbitrary-precision integer arithmetic.
//!
//! Reimplemented in Rust under the differential-test framework and proven
//! byte-identical to php 8.5.8's `ext/gmp` (which binds the C GMP library).
//! The big-integer math comes from `num-bigint` / `num-integer` (a faithful
//! 1:1 for the operations GMP exposes); the PHP-specific layer — the `GMP`
//! object wrapper, base-0 operand parsing, `gmp_strval` radix formatting (with
//! GMP's upper/lower-case rules and negative bases), the division rounding
//! modes, and `gmp_intval`'s `mpz_get_si` truncation — is hand-rolled here.
//!
//! A `GMP` value is a PHP object of class `GMP` (defined in the compiler
//! prelude) carrying a public `num` property that holds the decimal string, so
//! `var_dump` renders `object(GMP)#N (1) { ["num"]=> string(..) ".." }`, exactly
//! like php. Results are built through [`Host::instantiate`]; operands that are
//! `GMP` objects are read back through [`Host::object_prop`].

use std::collections::HashMap;

use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// value helpers
// ---------------------------------------------------------------------------

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => deref(&r.borrow()),
        o => o.clone(),
    }
}

/// Parse a GMP operand/`gmp_init` string with GMP's base-0 grammar: optional
/// surrounding ASCII whitespace, an optional leading `-` (but not `+`), then a
/// magnitude whose base is auto-detected from a `0x`/`0X`, `0b`/`0B` or leading
/// `0` prefix (hex / binary / octal), else decimal. Returns `None` if the
/// string is not a well-formed integer.
fn parse_str_auto(s: &[u8]) -> Option<BigInt> {
    let s = trim_ascii_ws(s);
    if s.is_empty() {
        return None;
    }
    let (neg, body) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        _ => (false, s),
    };
    if body.is_empty() {
        return None;
    }
    let (base, digits): (u32, &[u8]) = if body.len() >= 2 && body[0] == b'0' {
        match body[1] {
            b'x' | b'X' => (16, &body[2..]),
            b'b' | b'B' => (2, &body[2..]),
            b'0'..=b'7' => (8, &body[1..]),
            _ if body.iter().all(|&c| c.is_ascii_digit()) => (10, body),
            _ => (8, &body[1..]),
        }
    } else {
        (10, body)
    };
    let mag = parse_in_base(digits, base)?;
    Some(if neg { -mag } else { mag })
}

/// Parse `digits` as an unsigned magnitude in an explicit base (2..=62).
/// Case-insensitive for base <= 36; for base 37..=62, `A`-`Z` = 10..35 and
/// `a`-`z` = 36..61.
fn parse_in_base(digits: &[u8], base: u32) -> Option<BigInt> {
    if digits.is_empty() {
        return None;
    }
    let mut acc = BigInt::zero();
    let b = BigInt::from(base);
    for &c in digits {
        let d = digit_value(c, base)?;
        acc = acc * &b + BigInt::from(d);
    }
    Some(acc)
}

fn digit_value(c: u8, base: u32) -> Option<u32> {
    let v = match c {
        b'0'..=b'9' => (c - b'0') as u32,
        b'a'..=b'z' => {
            if base > 36 {
                (c - b'a') as u32 + 36
            } else {
                (c - b'a') as u32 + 10
            }
        }
        b'A'..=b'Z' => (c - b'A') as u32 + 10,
        _ => return None,
    };
    if v < base {
        Some(v)
    } else {
        None
    }
}

fn trim_ascii_ws(s: &[u8]) -> &[u8] {
    let mut a = 0;
    let mut b = s.len();
    while a < b && s[a].is_ascii_whitespace() {
        a += 1;
    }
    while b > a && s[b - 1].is_ascii_whitespace() {
        b -= 1;
    }
    &s[a..b]
}

/// Convert a finite `f64` to a `BigInt` truncating toward zero, like
/// `mpz_set_d`.
fn bigint_from_f64(f: f64) -> BigInt {
    let t = f.trunc();
    BigInt::from_f64_trunc(t)
}

trait FromF64Trunc {
    fn from_f64_trunc(f: f64) -> BigInt;
}
impl FromF64Trunc for BigInt {
    fn from_f64_trunc(f: f64) -> BigInt {
        if !f.is_finite() {
            return BigInt::zero();
        }
        // f is already an integral value here; convert exactly.
        num_traits::FromPrimitive::from_f64(f).unwrap_or_else(BigInt::zero)
    }
}

fn value_err(fname: &str, argnum: usize, argname: &str, what: &str) -> Thrown {
    Thrown::error(
        "ValueError",
        format!("{fname}(): Argument #{argnum} (${argname}) {what}"),
    )
}

/// Read a function argument as a `BigInt`. Accepts a `GMP` object (reads its
/// `num` property), an int, a float (truncated), a bool, or a numeric string
/// (base-0). On a malformed string raises the exact php `ValueError`.
fn operand(
    host: &dyn Host,
    args: &[Value],
    idx: usize,
    fname: &str,
    argnum: usize,
    argname: &str,
) -> Result<BigInt, Thrown> {
    let v = &args[idx];
    // A GMP object round-trips through its "num" property.
    if let Some(Value::Str(num)) = host.object_prop(v, b"num") {
        if let Some(bi) = BigInt::parse_bytes(num.as_bytes(), 10) {
            return Ok(bi);
        }
    }
    match deref(v) {
        Value::Int(i) => Ok(BigInt::from(i)),
        Value::Bool(b) => Ok(BigInt::from(b as i64)),
        Value::Float(f) => Ok(bigint_from_f64(f)),
        Value::Str(s) => parse_str_auto(s.as_bytes())
            .ok_or_else(|| value_err(fname, argnum, argname, "is not an integer string")),
        _ => Err(value_err(
            fname,
            argnum,
            argname,
            "is not an integer string",
        )),
    }
}

/// Read a plain integer argument (`$exponent`, `$index`, bit flags, …).
fn int_arg(args: &[Value], idx: usize) -> i64 {
    conv::to_int(&deref(&args[idx]))
}

/// Build a `GMP` result object wrapping `n`.
fn gmp_obj(host: &mut dyn Host, n: BigInt) -> Result<Value, Thrown> {
    let dec = n.to_string();
    host.instantiate(b"GMP", vec![(b"num".to_vec(), Value::str(dec))])
        .ok_or_else(|| Thrown::unsupported("gmp", "no object model in this host"))
}

// ---------------------------------------------------------------------------
// radix formatting (gmp_strval)
// ---------------------------------------------------------------------------

/// Format `n` in `base` following GMP's `mpz_get_str` case rules.
fn to_radix(n: &BigInt, base: i64) -> Vec<u8> {
    let abs_base = base.unsigned_abs() as u32;
    // 10..35 use upper-case for negative bases or bases > 36, lower-case for 2..36.
    let upper_10_35 = !(0..=36).contains(&base);
    if n.is_zero() {
        return vec![b'0'];
    }
    let neg = n.sign() == Sign::Minus;
    let mut mag = n.abs();
    let b = BigInt::from(abs_base);
    let mut out: Vec<u8> = Vec::new();
    while !mag.is_zero() {
        let (q, r) = mag.div_rem(&b);
        let d = r.to_u32().unwrap();
        out.push(radix_digit(d, upper_10_35));
        mag = q;
    }
    if neg {
        out.push(b'-');
    }
    out.reverse();
    out
}

fn radix_digit(d: u32, upper_10_35: bool) -> u8 {
    if d < 10 {
        b'0' + d as u8
    } else if d < 36 {
        if upper_10_35 {
            b'A' + (d - 10) as u8
        } else {
            b'a' + (d - 10) as u8
        }
    } else {
        // Only reachable for bases 37..=62: 36..61 -> lower-case a..z.
        b'a' + (d - 36) as u8
    }
}

// ---------------------------------------------------------------------------
// gmp_init / gmp_intval / gmp_strval
// ---------------------------------------------------------------------------

fn fn_gmp_init(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = if args.len() >= 2 { int_arg(args, 1) } else { 0 };
    if base != 0 && !(2..=62).contains(&base) {
        return Err(value_err(
            "gmp_init",
            2,
            "base",
            "must be 0 or between 2 and 62",
        ));
    }
    let v = &args[0];
    let n = match deref(v) {
        Value::Int(i) => BigInt::from(i),
        Value::Bool(b) => BigInt::from(b as i64),
        Value::Float(f) => bigint_from_f64(f),
        Value::Str(s) => {
            let parsed = if base == 0 {
                parse_str_auto(s.as_bytes())
            } else {
                parse_signed_in_base(s.as_bytes(), base as u32)
            };
            parsed.ok_or_else(|| value_err("gmp_init", 1, "num", "is not an integer string"))?
        }
        _ => return Err(value_err("gmp_init", 1, "num", "is not an integer string")),
    };
    gmp_obj(host, n)
}

/// Parse a signed string in an explicit base (trim whitespace, optional `-`).
fn parse_signed_in_base(s: &[u8], base: u32) -> Option<BigInt> {
    let s = trim_ascii_ws(s);
    if s.is_empty() {
        return None;
    }
    let (neg, body) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mag = parse_in_base(body, base)?;
    Some(if neg { -mag } else { mag })
}

fn fn_gmp_intval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = operand(host, args, 0, "gmp_intval", 1, "num")?;
    // mpz_get_si: take the low limb (64 bits) of the magnitude, then
    //   positive: zl & LONG_MAX
    //   negative: -1 - ((zl - 1) & LONG_MAX)
    // which handles i64::MIN exactly.
    let low64 = (&n.abs()) & &((BigInt::one() << 64u32) - 1);
    let zl = low64.to_u64().unwrap_or(0);
    const LONG_MAX: u64 = i64::MAX as u64;
    let i = if n.sign() == Sign::Minus {
        -1 - ((zl.wrapping_sub(1) & LONG_MAX) as i64)
    } else {
        (zl & LONG_MAX) as i64
    };
    Ok(Value::Int(i))
}

fn fn_gmp_strval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = operand(host, args, 0, "gmp_strval", 1, "num")?;
    let base = if args.len() >= 2 {
        int_arg(args, 1)
    } else {
        10
    };
    let ok = (2..=62).contains(&base) || (-36..=-2).contains(&base);
    if !ok {
        return Err(value_err(
            "gmp_strval",
            2,
            "base",
            "must be between 2 and 62, or -2 and -36",
        ));
    }
    Ok(Value::str(to_radix(&n, base)))
}

// ---------------------------------------------------------------------------
// arithmetic
// ---------------------------------------------------------------------------

macro_rules! binop {
    ($name:ident, $fname:literal, $op:expr) => {
        fn $name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            let a = operand(host, args, 0, $fname, 1, "num1")?;
            let b = operand(host, args, 1, $fname, 2, "num2")?;
            let f: fn(BigInt, BigInt) -> BigInt = $op;
            gmp_obj(host, f(a, b))
        }
    };
}

binop!(fn_gmp_add, "gmp_add", |a, b| a + b);
binop!(fn_gmp_sub, "gmp_sub", |a, b| a - b);
binop!(fn_gmp_mul, "gmp_mul", |a, b| a * b);
binop!(fn_gmp_and, "gmp_and", |a, b| a & b);
binop!(fn_gmp_or, "gmp_or", |a, b| a | b);
binop!(fn_gmp_xor, "gmp_xor", |a, b| a ^ b);

fn fn_gmp_neg(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_neg", 1, "num")?;
    gmp_obj(host, -a)
}

fn fn_gmp_abs(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_abs", 1, "num")?;
    gmp_obj(host, a.abs())
}

fn fn_gmp_com(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_com", 1, "num")?;
    // ~a == -a - 1 in two's complement.
    gmp_obj(host, -a - 1)
}

/// q and r for `a / b` under the given rounding mode: 0 = toward zero,
/// 1 = toward +inf, 2 = toward -inf.
fn div_round(a: &BigInt, b: &BigInt, mode: i64) -> (BigInt, BigInt) {
    let q0 = a / b;
    let r0 = a - &q0 * b;
    let q = if r0.is_zero() {
        q0
    } else {
        match mode {
            1 => {
                // ceil: bump up when the true quotient is positive.
                if (r0.sign() == Sign::Plus) == (b.sign() == Sign::Plus) {
                    q0 + 1
                } else {
                    q0
                }
            }
            2 => {
                // floor: bump down when the true quotient is negative.
                if (r0.sign() == Sign::Plus) != (b.sign() == Sign::Plus) {
                    q0 - 1
                } else {
                    q0
                }
            }
            _ => q0,
        }
    };
    let r = a - &q * b;
    (q, r)
}

fn div_zero(fname: &str) -> Thrown {
    Thrown::error(
        "DivisionByZeroError",
        format!("{fname}(): Argument #2 ($num2) Division by zero"),
    )
}

fn fn_gmp_div_q(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_div_q", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_div_q", 2, "num2")?;
    if b.is_zero() {
        return Err(div_zero("gmp_div_q"));
    }
    let mode = if args.len() >= 3 { int_arg(args, 2) } else { 0 };
    let (q, _) = div_round(&a, &b, mode);
    gmp_obj(host, q)
}

fn fn_gmp_div(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // alias of gmp_div_q (message still names the real callee in php: gmp_div_q)
    let a = operand(host, args, 0, "gmp_div_q", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_div_q", 2, "num2")?;
    if b.is_zero() {
        return Err(div_zero("gmp_div_q"));
    }
    let mode = if args.len() >= 3 { int_arg(args, 2) } else { 0 };
    let (q, _) = div_round(&a, &b, mode);
    gmp_obj(host, q)
}

fn fn_gmp_div_r(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_div_r", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_div_r", 2, "num2")?;
    if b.is_zero() {
        return Err(div_zero("gmp_div_r"));
    }
    let mode = if args.len() >= 3 { int_arg(args, 2) } else { 0 };
    let (_, r) = div_round(&a, &b, mode);
    gmp_obj(host, r)
}

fn fn_gmp_div_qr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_div_qr", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_div_qr", 2, "num2")?;
    if b.is_zero() {
        return Err(div_zero("gmp_div_qr"));
    }
    let mode = if args.len() >= 3 { int_arg(args, 2) } else { 0 };
    let (q, r) = div_round(&a, &b, mode);
    let mut arr = rhp_values::PhpArray::new();
    arr.insert(rhp_values::ArrayKey::Int(0), gmp_obj(host, q)?);
    arr.insert(rhp_values::ArrayKey::Int(1), gmp_obj(host, r)?);
    Ok(Value::Array(arr))
}

fn fn_gmp_mod(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_mod", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_mod", 2, "num2")?;
    if b.is_zero() {
        return Err(Thrown::error(
            "DivisionByZeroError",
            "gmp_mod(): Argument #2 ($num2) Modulo by zero",
        ));
    }
    // Euclidean remainder in [0, |b|).
    let m = b.abs();
    let mut r = &a % &m;
    if r.sign() == Sign::Minus {
        r += &m;
    }
    gmp_obj(host, r)
}

fn fn_gmp_divexact(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_divexact", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_divexact", 2, "num2")?;
    if b.is_zero() {
        return Err(div_zero("gmp_divexact"));
    }
    gmp_obj(host, a / b)
}

fn fn_gmp_pow(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = operand(host, args, 0, "gmp_pow", 1, "num")?;
    let exp = int_arg(args, 1);
    if exp < 0 {
        return Err(value_err(
            "gmp_pow",
            2,
            "exponent",
            "must be greater than or equal to 0",
        ));
    }
    let e = u32::try_from(exp).map_err(|_| value_err("gmp_pow", 2, "exponent", "is too large"))?;
    gmp_obj(host, base.pow(e))
}

fn fn_gmp_powm(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let base = operand(host, args, 0, "gmp_powm", 1, "num")?;
    let exp = operand(host, args, 1, "gmp_powm", 2, "exponent")?;
    let modulus = operand(host, args, 2, "gmp_powm", 3, "modulus")?;
    if modulus.is_zero() {
        return Err(Thrown::error(
            "DivisionByZeroError",
            "gmp_powm(): Argument #3 ($modulus) Modulo by zero",
        ));
    }
    if exp.sign() == Sign::Minus {
        return Err(value_err(
            "gmp_powm",
            2,
            "exponent",
            "must be greater than or equal to 0",
        ));
    }
    let m = modulus.abs();
    gmp_obj(host, base.modpow(&exp, &m))
}

// ---------------------------------------------------------------------------
// roots
// ---------------------------------------------------------------------------

fn fn_gmp_sqrt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_sqrt", 1, "num")?;
    if a.sign() == Sign::Minus {
        return Err(value_err(
            "gmp_sqrt",
            1,
            "num",
            "must be greater than or equal to 0",
        ));
    }
    gmp_obj(host, a.sqrt())
}

fn fn_gmp_sqrtrem(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_sqrtrem", 1, "num")?;
    if a.sign() == Sign::Minus {
        return Err(value_err(
            "gmp_sqrtrem",
            1,
            "num",
            "must be greater than or equal to 0",
        ));
    }
    let s = a.sqrt();
    let rem = &a - &s * &s;
    let mut arr = rhp_values::PhpArray::new();
    arr.insert(rhp_values::ArrayKey::Int(0), gmp_obj(host, s)?);
    arr.insert(rhp_values::ArrayKey::Int(1), gmp_obj(host, rem)?);
    Ok(Value::Array(arr))
}

fn root_check(a: &BigInt, nth: i64, fname: &str) -> Result<u32, Thrown> {
    if nth <= 0 {
        return Err(value_err(fname, 2, "nth", "must be greater than 0"));
    }
    if a.sign() == Sign::Minus && nth % 2 == 0 {
        return Err(value_err(
            fname,
            2,
            "nth",
            "must be odd if argument #1 ($a) is negative",
        ));
    }
    u32::try_from(nth).map_err(|_| value_err(fname, 2, "nth", "is too large"))
}

fn fn_gmp_root(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_root", 1, "a")?;
    let nth = int_arg(args, 1);
    let e = root_check(&a, nth, "gmp_root")?;
    let r = a.abs().nth_root(e);
    let r = if a.sign() == Sign::Minus { -r } else { r };
    gmp_obj(host, r)
}

fn fn_gmp_rootrem(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_rootrem", 1, "a")?;
    let nth = int_arg(args, 1);
    let e = root_check(&a, nth, "gmp_rootrem")?;
    let mut r = a.abs().nth_root(e);
    if a.sign() == Sign::Minus {
        r = -r;
    }
    let rem = &a - r.pow(e);
    let mut arr = rhp_values::PhpArray::new();
    arr.insert(rhp_values::ArrayKey::Int(0), gmp_obj(host, r)?);
    arr.insert(rhp_values::ArrayKey::Int(1), gmp_obj(host, rem)?);
    Ok(Value::Array(arr))
}

// ---------------------------------------------------------------------------
// comparison / sign
// ---------------------------------------------------------------------------

fn fn_gmp_cmp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_cmp", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_cmp", 2, "num2")?;
    Ok(Value::Int(match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

fn fn_gmp_sign(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_sign", 1, "num")?;
    Ok(Value::Int(match a.sign() {
        Sign::Minus => -1,
        Sign::NoSign => 0,
        Sign::Plus => 1,
    }))
}

// ---------------------------------------------------------------------------
// gcd / lcm / inverse
// ---------------------------------------------------------------------------

fn fn_gmp_gcd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_gcd", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_gcd", 2, "num2")?;
    gmp_obj(host, a.gcd(&b))
}

fn fn_gmp_lcm(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_lcm", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_lcm", 2, "num2")?;
    if a.is_zero() || b.is_zero() {
        return gmp_obj(host, BigInt::zero());
    }
    gmp_obj(host, a.lcm(&b))
}

fn fn_gmp_gcdext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_gcdext", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_gcdext", 2, "num2")?;
    let (g, s, t) = ext_gcd(&a, &b);
    let mut arr = rhp_values::PhpArray::new();
    arr.insert(rhp_values::ArrayKey::from_bytes(b"g"), gmp_obj(host, g)?);
    arr.insert(rhp_values::ArrayKey::from_bytes(b"s"), gmp_obj(host, s)?);
    arr.insert(rhp_values::ArrayKey::from_bytes(b"t"), gmp_obj(host, t)?);
    Ok(Value::Array(arr))
}

/// Extended gcd matching GMP's `mpz_gcdext`: returns `(g, s, t)` with
/// `g = s*a + t*b`, `g >= 0`.
fn ext_gcd(a: &BigInt, b: &BigInt) -> (BigInt, BigInt, BigInt) {
    // Iterative extended Euclid on magnitudes, then fix up signs.
    let (mut old_r, mut r) = (a.abs(), b.abs());
    let (mut old_s, mut s) = (BigInt::one(), BigInt::zero());
    let (mut old_t, mut t) = (BigInt::zero(), BigInt::one());
    while !r.is_zero() {
        let q = &old_r / &r;
        let nr = &old_r - &q * &r;
        old_r = std::mem::replace(&mut r, nr);
        let ns = &old_s - &q * &s;
        old_s = std::mem::replace(&mut s, ns);
        let nt = &old_t - &q * &t;
        old_t = std::mem::replace(&mut t, nt);
    }
    // old_s, old_t are cofactors for |a|, |b|; restore original signs.
    if a.sign() == Sign::Minus {
        old_s = -old_s;
    }
    if b.sign() == Sign::Minus {
        old_t = -old_t;
    }
    (old_r, old_s, old_t)
}

fn mod_invert(a: &BigInt, m: &BigInt) -> Option<BigInt> {
    let m = m.abs();
    if m.is_one() {
        return Some(BigInt::zero());
    }
    if m.is_zero() {
        return None;
    }
    let (g, s, _) = ext_gcd(a, &m);
    if !g.is_one() {
        return None;
    }
    let mut inv = s % &m;
    if inv.sign() == Sign::Minus {
        inv += &m;
    }
    Some(inv)
}

fn fn_gmp_invert(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_invert", 1, "num1")?;
    let m = operand(host, args, 1, "gmp_invert", 2, "num2")?;
    match mod_invert(&a, &m) {
        Some(inv) => gmp_obj(host, inv),
        None => Ok(Value::Bool(false)),
    }
}

// ---------------------------------------------------------------------------
// factorial / binomial
// ---------------------------------------------------------------------------

fn fn_gmp_fact(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = operand(host, args, 0, "gmp_fact", 1, "num")?;
    if n.sign() == Sign::Minus {
        return Err(value_err(
            "gmp_fact",
            1,
            "num",
            "must be greater than or equal to 0",
        ));
    }
    let mut acc = BigInt::one();
    let mut i = BigInt::from(2);
    while i <= n {
        acc *= &i;
        i += 1;
    }
    gmp_obj(host, acc)
}

fn fn_gmp_binomial(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = operand(host, args, 0, "gmp_binomial", 1, "n")?;
    let k = int_arg(args, 1);
    if k < 0 {
        return Err(value_err(
            "gmp_binomial",
            2,
            "k",
            "must be greater than or equal to 0",
        ));
    }
    // C(n,k) via the falling factorial n*(n-1)*...*(n-k+1)/k!; works for any
    // integer n and yields 0 when 0 <= n < k.
    let mut num = BigInt::one();
    let mut den = BigInt::one();
    let mut term = n.clone();
    for i in 0..k {
        num *= &term;
        term -= 1;
        den *= BigInt::from(i + 1);
    }
    gmp_obj(host, num / den)
}

// ---------------------------------------------------------------------------
// bit operations
// ---------------------------------------------------------------------------

/// Two's-complement test of bit `i` (handles negatives).
fn test_bit(n: &BigInt, i: u64) -> bool {
    n.bit(i)
}

fn fn_gmp_testbit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_testbit", 1, "num")?;
    let index = int_arg(args, 1);
    if index < 0 {
        return Err(value_err(
            "gmp_testbit",
            2,
            "index",
            "must be greater than or equal to 0",
        ));
    }
    Ok(Value::Bool(test_bit(&a, index as u64)))
}

/// Shared implementation of `gmp_setbit`/`gmp_clrbit`: mutate the `GMP` object's
/// `num` property in place (GMP objects are PHP handles, so this reaches the
/// caller's variable).
fn set_or_clear_bit(obj: &Value, index: u64, set: bool) {
    // Locate the ObjectData handle through any Ref.
    fn handle(v: &Value) -> Option<rhp_values::ObjectHandle> {
        match v {
            Value::Object(h) => Some(h.clone()),
            Value::Ref(r) => handle(&r.borrow()),
            _ => None,
        }
    }
    let Some(h) = handle(obj) else { return };
    let key = rhp_values::PhpStr::from_bytes(b"num".to_vec());
    let cur = {
        let b = h.borrow();
        match b.properties.get(&key) {
            Some(Value::Str(s)) => {
                BigInt::parse_bytes(s.as_bytes(), 10).unwrap_or_else(BigInt::zero)
            }
            _ => BigInt::zero(),
        }
    };
    let bit = BigInt::one() << (index as u32);
    let next = if set {
        cur | bit
    } else {
        // clear: AND with ~bit (= -bit-1 in two's complement).
        cur & (-bit - 1)
    };
    h.borrow_mut()
        .properties
        .insert(key, Value::str(next.to_string()));
}

fn fn_gmp_setbit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // Validate that arg 0 is a GMP object (reads back its num).
    let _ = operand(host, args, 0, "gmp_setbit", 1, "num")?;
    let index = int_arg(args, 1);
    if index < 0 {
        return Err(value_err(
            "gmp_setbit",
            2,
            "index",
            "must be greater than or equal to 0",
        ));
    }
    let set = if args.len() >= 3 {
        conv::to_bool(&deref(&args[2]))
    } else {
        true
    };
    set_or_clear_bit(&args[0], index as u64, set);
    Ok(Value::Null)
}

fn fn_gmp_clrbit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let _ = operand(host, args, 0, "gmp_clrbit", 1, "num")?;
    let index = int_arg(args, 1);
    if index < 0 {
        return Err(value_err(
            "gmp_clrbit",
            2,
            "index",
            "must be greater than or equal to 0",
        ));
    }
    set_or_clear_bit(&args[0], index as u64, false);
    Ok(Value::Null)
}

fn fn_gmp_popcount(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_popcount", 1, "num")?;
    // GMP returns ULONG_MAX (-1 as a signed long) for negatives.
    if a.sign() == Sign::Minus {
        return Ok(Value::Int(-1));
    }
    let ones = a.to_biguint().map(|u| u.count_ones()).unwrap_or(0);
    Ok(Value::Int(ones as i64))
}

fn fn_gmp_hamdist(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_hamdist", 1, "num1")?;
    let b = operand(host, args, 1, "gmp_hamdist", 2, "num2")?;
    // GMP requires matching signs; otherwise returns ULONG_MAX (-1).
    if (a.sign() == Sign::Minus) != (b.sign() == Sign::Minus) {
        return Ok(Value::Int(-1));
    }
    let x = a ^ b; // same-sign XOR is always non-negative
    let ones = x.to_biguint().map(|u| u.count_ones()).unwrap_or(0);
    Ok(Value::Int(ones as i64))
}

fn scan(a: &BigInt, start: u64, looking_for_one: bool) -> i64 {
    let bound = a.bits().max(start + 1) + 2;
    let mut i = start;
    while i < bound {
        if a.bit(i) == looking_for_one {
            return i as i64;
        }
        i += 1;
    }
    -1
}

fn fn_gmp_scan0(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_scan0", 1, "num1")?;
    let start = int_arg(args, 1);
    if start < 0 {
        return Err(value_err(
            "gmp_scan0",
            2,
            "start",
            "must be greater than or equal to 0",
        ));
    }
    Ok(Value::Int(scan(&a, start as u64, false)))
}

fn fn_gmp_scan1(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_scan1", 1, "num1")?;
    let start = int_arg(args, 1);
    if start < 0 {
        return Err(value_err(
            "gmp_scan1",
            2,
            "start",
            "must be greater than or equal to 0",
        ));
    }
    Ok(Value::Int(scan(&a, start as u64, true)))
}

// ---------------------------------------------------------------------------
// number theory
// ---------------------------------------------------------------------------

fn fn_gmp_perfect_square(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_perfect_square", 1, "num")?;
    if a.sign() == Sign::Minus {
        return Ok(Value::Bool(false));
    }
    let s = a.sqrt();
    Ok(Value::Bool(&s * &s == a))
}

fn fn_gmp_perfect_power(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_perfect_power", 1, "num")?;
    Ok(Value::Bool(is_perfect_power(&a)))
}

fn is_perfect_power(n: &BigInt) -> bool {
    // 0, 1, -1 are perfect powers by GMP's definition.
    if n.is_zero() || n.is_one() || *n == BigInt::from(-1) {
        return true;
    }
    let neg = n.sign() == Sign::Minus;
    let mag = n.abs();
    let maxexp = mag.bits(); // |n| < 2^bits, so any exponent <= bits
    let mut e = 2u64;
    while e <= maxexp {
        if neg && e.is_multiple_of(2) {
            e += 1;
            continue;
        }
        let r = mag.nth_root(e as u32);
        if r.pow(e as u32) == mag {
            return true;
        }
        e += 1;
    }
    false
}

/// Kronecker symbol (a|n), per GMP's `mpz_kronecker`. `gmp_jacobi` and
/// `gmp_legendre` route here too (they coincide for valid — odd, positive —
/// moduli).
fn kronecker(a0: &BigInt, n0: &BigInt) -> i64 {
    const TAB2: [i64; 8] = [0, 1, 0, -1, 0, -1, 0, 1];
    let mut a = a0.clone();
    let mut n = n0.clone();
    if n.is_zero() {
        return if a.abs().is_one() { 1 } else { 0 };
    }
    if a.is_even() && n.is_even() {
        return 0;
    }
    let mut k = 1i64;
    // Factor twos out of n.
    let mut v = 0u32;
    while n.is_even() {
        n /= 2;
        v += 1;
    }
    if v % 2 == 1 {
        k = TAB2[(low3(&a)) as usize];
    }
    if n.sign() == Sign::Minus {
        n = -n;
        if a.sign() == Sign::Minus {
            k = -k;
        }
    }
    while !a.is_zero() {
        let mut v = 0u32;
        while a.is_even() {
            a /= 2;
            v += 1;
        }
        if v % 2 == 1 {
            k *= TAB2[(low3(&n)) as usize];
        }
        // reciprocity: both a and n ≡ 3 (mod 4)
        if a.bit(1) && n.bit(1) {
            k = -k;
        }
        let r = a.abs();
        a = &n % &r;
        n = r;
    }
    if n.is_one() {
        k
    } else {
        0
    }
}

/// The low 3 bits of a two's-complement integer (its value mod 8 in 0..8).
fn low3(x: &BigInt) -> u8 {
    let mut b = 0u8;
    if x.bit(0) {
        b |= 1;
    }
    if x.bit(1) {
        b |= 2;
    }
    if x.bit(2) {
        b |= 4;
    }
    b
}

fn fn_gmp_jacobi(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_jacobi", 1, "a")?;
    let b = operand(host, args, 1, "gmp_jacobi", 2, "b")?;
    Ok(Value::Int(kronecker(&a, &b)))
}

fn fn_gmp_legendre(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_legendre", 1, "a")?;
    let p = operand(host, args, 1, "gmp_legendre", 2, "p")?;
    Ok(Value::Int(kronecker(&a, &p)))
}

fn fn_gmp_kronecker(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = operand(host, args, 0, "gmp_kronecker", 1, "a")?;
    let b = operand(host, args, 1, "gmp_kronecker", 2, "b")?;
    Ok(Value::Int(kronecker(&a, &b)))
}

/// Largest value for which GMP 6.3.0's `mpz_probab_prime_p` returns 2 (proven
/// prime) rather than 1 (probably prime). Determined by differential probing of
/// the local `php`/GMP build.
const PROVEN_PRIME_MAX: &str = "2462906046218231";

fn fn_gmp_prob_prime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = operand(host, args, 0, "gmp_prob_prime", 1, "num")?;
    let mag = n.abs();
    if !is_prime(&mag) {
        return Ok(Value::Int(0));
    }
    let proven = BigInt::parse_bytes(PROVEN_PRIME_MAX.as_bytes(), 10).unwrap();
    Ok(Value::Int(if mag <= proven { 2 } else { 1 }))
}

fn fn_gmp_nextprime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = operand(host, args, 0, "gmp_nextprime", 1, "num")?;
    let mut c = n + BigInt::one();
    let two = BigInt::from(2);
    if c <= two {
        return gmp_obj(host, two);
    }
    if c.is_even() {
        c += 1;
    }
    while !is_prime(&c) {
        c += 2;
    }
    gmp_obj(host, c)
}

/// Deterministic primality for the ranges GMP returns definitive answers in
/// (and a very strong probable-prime test beyond): small-prime trial division
/// then Miller-Rabin over a fixed base set that is deterministic below 3.3e24.
fn is_prime(n: &BigInt) -> bool {
    if *n < BigInt::from(2) {
        return false;
    }
    let small: [u64; 12] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];
    for &p in &small {
        let bp = BigInt::from(p);
        if *n == bp {
            return true;
        }
        if (n % &bp).is_zero() {
            return false;
        }
    }
    // Write n-1 = d * 2^s.
    let one = BigInt::one();
    let n_minus_1 = n - &one;
    let mut d = n_minus_1.clone();
    let mut s = 0u32;
    while d.is_even() {
        d /= 2;
        s += 1;
    }
    'bases: for &a in &small {
        let a = BigInt::from(a);
        let mut x = a.modpow(&d, n);
        if x == one || x == n_minus_1 {
            continue;
        }
        for _ in 0..s.saturating_sub(1) {
            x = x.modpow(&BigInt::from(2), n);
            if x == n_minus_1 {
                continue 'bases;
            }
        }
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register all `gmp_*` builtins.
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
    put(m, "gmp_init", fn_gmp_init, 1, Some(2));
    put(m, "gmp_intval", fn_gmp_intval, 1, Some(1));
    put(m, "gmp_strval", fn_gmp_strval, 1, Some(2));
    put(m, "gmp_add", fn_gmp_add, 2, Some(2));
    put(m, "gmp_sub", fn_gmp_sub, 2, Some(2));
    put(m, "gmp_mul", fn_gmp_mul, 2, Some(2));
    put(m, "gmp_div_q", fn_gmp_div_q, 2, Some(3));
    put(m, "gmp_div_r", fn_gmp_div_r, 2, Some(3));
    put(m, "gmp_div_qr", fn_gmp_div_qr, 2, Some(3));
    put(m, "gmp_div", fn_gmp_div, 2, Some(3));
    put(m, "gmp_mod", fn_gmp_mod, 2, Some(2));
    put(m, "gmp_divexact", fn_gmp_divexact, 2, Some(2));
    put(m, "gmp_pow", fn_gmp_pow, 2, Some(2));
    put(m, "gmp_powm", fn_gmp_powm, 3, Some(3));
    put(m, "gmp_sqrt", fn_gmp_sqrt, 1, Some(1));
    put(m, "gmp_sqrtrem", fn_gmp_sqrtrem, 1, Some(1));
    put(m, "gmp_root", fn_gmp_root, 2, Some(2));
    put(m, "gmp_rootrem", fn_gmp_rootrem, 2, Some(2));
    put(m, "gmp_cmp", fn_gmp_cmp, 2, Some(2));
    put(m, "gmp_sign", fn_gmp_sign, 1, Some(1));
    put(m, "gmp_abs", fn_gmp_abs, 1, Some(1));
    put(m, "gmp_neg", fn_gmp_neg, 1, Some(1));
    put(m, "gmp_gcd", fn_gmp_gcd, 2, Some(2));
    put(m, "gmp_lcm", fn_gmp_lcm, 2, Some(2));
    put(m, "gmp_gcdext", fn_gmp_gcdext, 2, Some(2));
    put(m, "gmp_invert", fn_gmp_invert, 2, Some(2));
    put(m, "gmp_fact", fn_gmp_fact, 1, Some(1));
    put(m, "gmp_binomial", fn_gmp_binomial, 2, Some(2));
    put(m, "gmp_and", fn_gmp_and, 2, Some(2));
    put(m, "gmp_or", fn_gmp_or, 2, Some(2));
    put(m, "gmp_xor", fn_gmp_xor, 2, Some(2));
    put(m, "gmp_com", fn_gmp_com, 1, Some(1));
    put(m, "gmp_setbit", fn_gmp_setbit, 2, Some(3));
    put(m, "gmp_clrbit", fn_gmp_clrbit, 2, Some(2));
    put(m, "gmp_testbit", fn_gmp_testbit, 2, Some(2));
    put(m, "gmp_popcount", fn_gmp_popcount, 1, Some(1));
    put(m, "gmp_hamdist", fn_gmp_hamdist, 2, Some(2));
    put(m, "gmp_scan0", fn_gmp_scan0, 2, Some(2));
    put(m, "gmp_scan1", fn_gmp_scan1, 2, Some(2));
    put(m, "gmp_perfect_square", fn_gmp_perfect_square, 1, Some(1));
    put(m, "gmp_perfect_power", fn_gmp_perfect_power, 1, Some(1));
    put(m, "gmp_jacobi", fn_gmp_jacobi, 2, Some(2));
    put(m, "gmp_legendre", fn_gmp_legendre, 2, Some(2));
    put(m, "gmp_kronecker", fn_gmp_kronecker, 2, Some(2));
    put(m, "gmp_nextprime", fn_gmp_nextprime, 1, Some(1));
    put(m, "gmp_prob_prime", fn_gmp_prob_prime, 1, Some(2));
}

/// The `GMP` wrapper class, owned by this module (was in the compiler prelude).
pub const PRELUDE_PHP: &str = r#"
class GMP {
    public $num = "0";
    public function __construct($num = 0) { $this->num = (string)$num; }
    public function __toString() { return (string)$this->num; }
}
"#;

/// `GMP_*` constants: division rounding modes, (de)serialisation endianness,
/// and the reported GMP library version.
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("GMP_ROUND_ZERO", Value::Int(0)),
        ("GMP_ROUND_PLUSINF", Value::Int(1)),
        ("GMP_ROUND_MINUSINF", Value::Int(2)),
        ("GMP_MSW_FIRST", Value::Int(1)),
        ("GMP_LSW_FIRST", Value::Int(2)),
        ("GMP_LITTLE_ENDIAN", Value::Int(4)),
        ("GMP_BIG_ENDIAN", Value::Int(8)),
        ("GMP_NATIVE_ENDIAN", Value::Int(16)),
        ("GMP_VERSION", Value::str("6.3.0")),
    ]
}

/// One operand of the `GMP` object `compare` handler: a GMP object's decimal
/// `num`, or any other value with its `zend_zval_value_name`.
pub enum CmpOperand {
    Gmp(Vec<u8>),
    Other(Value, String),
}

/// `gmp_compare`: both operands are converted to GMP (`is_operator` rules:
/// base-0 integer strings, ints, weak scalars except null); a failed
/// conversion throws, which php reports as uncomparable.
pub fn compare_handler(a: CmpOperand, b: CmpOperand) -> Result<i32, Thrown> {
    fn conv_op(o: CmpOperand) -> Result<BigInt, Thrown> {
        let bad = |name: &str| {
            Thrown::type_error(format!(
                "Number must be of type GMP|string|int, {name} given"
            ))
        };
        match o {
            CmpOperand::Gmp(n) => Ok(BigInt::parse_bytes(&n, 10).unwrap_or_default()),
            CmpOperand::Other(v, name) => match deref(&v) {
                Value::Int(i) => Ok(BigInt::from(i)),
                Value::Str(s) => parse_str_auto(s.as_bytes())
                    .ok_or_else(|| Thrown::error("ValueError", "Number is not an integer string")),
                Value::Bool(b) => Ok(BigInt::from(b as i64)),
                Value::Float(f) if f.is_finite() && f.abs() < 9.2e18 => Ok(bigint_from_f64(f)),
                _ => Err(bad(&name)),
            },
        }
    }
    let x = conv_op(a)?;
    let y = conv_op(b)?;
    Ok(match x.cmp(&y) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    })
}
