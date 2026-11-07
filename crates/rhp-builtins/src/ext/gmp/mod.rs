//! The `gmp` extension: a port of php-src's ext/gmp/gmp.c onto libgmp (the
//! library php links; [`sys`]).
//!
//! A `GMP` object (the final class in [`PRELUDE_PHP`]) carries its number as
//! its payload ([`GmpNum`], php's `gmp_object.num`). Its handlers are
//! gmp.c's: operator overloading, comparison, casts, a debug view with a
//! `num` entry, and clone. `__construct`, `__serialize` and `__unserialize`
//! call `__gmp_construct` / `__gmp_serialize` / `__gmp_unserialize`.
//!
//! libgmp aborts the process when a number outgrows an `mpz_t` (`INT_MAX`
//! limbs) or memory runs out, and php calls it unchecked. With one VM per
//! thread that would end every request, so a result that cannot fit throws
//! a `ValueError` ("is too large") instead. Factorials and binomials are
//! sized from an estimate, with a margin.

pub(crate) mod sys;

use std::cell::{Ref, RefCell};
use std::ops::Deref;
use std::rc::Rc;

use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, PhpStr, Value};

use crate::zend::object_handlers;
use crate::zend::object_handlers::{
    CastTarget, ClassHandlers, CloneObj, ObjectHandlers, Op, PropPurpose,
};
use crate::zend::zpp;
use crate::zend::zpp::deref;
use crate::zend::zpp::zval_value_name;
use crate::{Host, Thrown};
use sys::Mpz;

const GMP_ROUND_ZERO: i64 = 0;
const GMP_ROUND_PLUSINF: i64 = 1;
const GMP_ROUND_MINUSINF: i64 = 2;
const GMP_MSW_FIRST: i64 = 1 << 0;
const GMP_LSW_FIRST: i64 = 1 << 1;
const GMP_LITTLE_ENDIAN: i64 = 1 << 2;
const GMP_BIG_ENDIAN: i64 = 1 << 3;
const GMP_NATIVE_ENDIAN: i64 = 1 << 4;
const GMP_MAX_BASE: i64 = 62;
const GMP_NUMB_BITS: i64 = 64;
const INT_MAX: i64 = i32::MAX as i64;
/// The arginfo type of the `GMP|int|string` parameters, as php prints it.
const NUM_TYPE: &str = "GMP|string|int";

// ---------------------------------------------------------------------------
// the GMP object
// ---------------------------------------------------------------------------

/// A GMP object's number.
struct GmpNum(RefCell<Mpz>);

/// `GET_GMP_FROM_ZVAL`: the number of a GMP object. One created without a
/// constructor (by `unserialize`) gets 0, as php's `create_object` gives it.
fn gmp_num(h: &ObjectHandle) -> Rc<GmpNum> {
    if let Some(n) = object_handlers::payload_of::<GmpNum>(h) {
        return n;
    }
    let n = Rc::new(GmpNum(RefCell::new(Mpz::new())));
    object_handlers::set_payload(h, n.clone());
    n
}

/// `IS_GMP`: an object of class GMP (which is final).
fn gmp_object(host: &dyn Host, v: &Value) -> Option<ObjectHandle> {
    let h = object_handlers::object_of(v)?;
    object_handlers::has_handlers(host, v, &GMP_HANDLERS).then_some(h)
}

/// `gmp_create`: a new GMP object holding `n`.
fn gmp_new(host: &mut dyn Host, n: Mpz) -> Result<Value, Thrown> {
    let v = host
        .instantiate(b"GMP", Vec::new())
        .ok_or_else(|| Thrown::unsupported("gmp", "no object model in this host"))?;
    if let Some(h) = object_handlers::object_of(&v) {
        object_handlers::set_payload(&h, Rc::new(GmpNum(RefCell::new(n))));
    }
    Ok(v)
}

fn pair(host: &mut dyn Host, a: Mpz, b: Mpz) -> Result<Value, Thrown> {
    let mut arr = PhpArray::new();
    arr.push(gmp_new(host, a)?);
    arr.push(gmp_new(host, b)?);
    Ok(Value::Array(arr))
}

/// A parsed number argument: a GMP object's own number, or a temporary
/// (php's `GMPG(zpp_arg)`).
enum Arg {
    Obj(Rc<GmpNum>),
    Tmp(Mpz),
}

enum ArgRef<'a> {
    Obj(Ref<'a, Mpz>),
    Tmp(&'a Mpz),
}

impl Arg {
    fn get(&self) -> ArgRef<'_> {
        match self {
            Arg::Obj(n) => ArgRef::Obj(n.0.borrow()),
            Arg::Tmp(m) => ArgRef::Tmp(m),
        }
    }
}

impl Deref for ArgRef<'_> {
    type Target = Mpz;
    fn deref(&self) -> &Mpz {
        match self {
            ArgRef::Obj(r) => r,
            ArgRef::Tmp(m) => m,
        }
    }
}

/// Where an argument comes from: a function's parameter (its errors name
/// it), or an operator's operand (`None`).
#[derive(Clone, Copy)]
struct At<'a> {
    func: &'a str,
    num: usize,
    name: &'a str,
}

fn arg_value_error(at: At, msg: &str) -> Thrown {
    zpp::value_error(at.func, at.num, at.name, msg)
}

/// The size guard's error (see the module docs).
fn too_large(at: Option<At>, what: &str) -> Thrown {
    match at {
        Some(at) => arg_value_error(at, "is too large"),
        None => Thrown::error("ValueError", format!("{what} is too large")),
    }
}

/// Whether a result of `limbs` limbs fits an `mpz_t`.
fn fits(limbs: u128) -> bool {
    limbs <= INT_MAX as u128
}

/// C's `isspace` in the C locale.
fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `convert_zstr_to_gmp`: php's prefixes (`0x`, `0o`, `0b`) after leading
/// white space, then `mpz_set_str`. The error names the argument, or is
/// "Number is not an integer string" for an operand.
fn convert_str(z: &mut Mpz, s: &[u8], base: i64, at: Option<At>) -> Result<(), Thrown> {
    let start = s.iter().position(|&c| !is_c_space(c)).unwrap_or(s.len());
    let rest = &s[start..];
    let mut base = base;
    let mut skip = false;
    if rest.len() >= 2 && rest[0] == b'0' {
        let prefix = match rest[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if prefix != 0 && (base == 0 || base == prefix) {
            base = prefix;
            skip = true;
        }
    }
    if z.set_str(if skip { &rest[2..] } else { rest }, base as i32) {
        return Ok(());
    }
    Err(match at {
        Some(at) => arg_value_error(at, "is not an integer string"),
        None => Thrown::error("ValueError", "Number is not an integer string"),
    })
}

/// `zend_incompatible_double_to_long_error`.
fn float_precision_deprecation(host: &mut dyn Host, f: f64) -> Result<(), Thrown> {
    let shown = String::from_utf8_lossy(&rhp_values::fmt::float_serialize(f)).into_owned();
    host.emit_diagnostic(
        crate::E_DEPRECATED,
        format!("Implicit conversion from float {shown} to int loses precision").as_bytes(),
    )
}

/// `zend_parse_arg_long_weak` for the values gmp hands it (not strings,
/// ints or objects): `None` when it fails.
fn long_weak(
    host: &mut dyn Host,
    v: &Value,
    at: Option<At>,
    ty: &str,
) -> Result<Option<i64>, Thrown> {
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    Ok(match v {
        Value::Float(f) if (-TWO_63..TWO_63).contains(f) => {
            if f.fract() != 0.0 {
                float_precision_deprecation(host, *f)?;
            }
            Some(*f as i64)
        }
        Value::Bool(b) => Some(*b as i64),
        Value::Null => {
            // zend_null_arg_deprecated, with the parameter's arginfo type.
            if let Some(at) = at {
                host.emit_diagnostic(
                    crate::E_DEPRECATED,
                    format!(
                        "{}(): Passing null to parameter #{} (${}) of type {ty} is deprecated",
                        at.func, at.num, at.name
                    )
                    .as_bytes(),
                )?;
            }
            Some(0)
        }
        _ => None,
    })
}

/// `gmp_zend_parse_arg_into_mpz_ex`: a GMP object, an integer string
/// (base 0), an int, or in coercive mode a bool, an integral float or (for
/// a function) null. `Ok(None)` when `v` is none of these and nothing was
/// thrown.
fn parse_arg(
    host: &mut dyn Host,
    v: &Value,
    at: Option<At>,
    strict: bool,
) -> Result<Option<Arg>, Thrown> {
    let v = deref(v);
    match &v {
        Value::Object(_) => {
            return Ok(gmp_object(host, &v).map(|h| Arg::Obj(gmp_num(&h))));
        }
        Value::Closure(_) => return Ok(None),
        Value::Str(s) => {
            let mut z = Mpz::new();
            convert_str(&mut z, s.as_bytes(), 0, at)?;
            return Ok(Some(Arg::Tmp(z)));
        }
        Value::Int(i) => return Ok(Some(Arg::Tmp(Mpz::from_i64(*i)))),
        _ => {}
    }
    if strict || (at.is_none() && matches!(v, Value::Null)) {
        return Ok(None);
    }
    Ok(long_weak(host, &v, at, NUM_TYPE)?.map(|i| Arg::Tmp(Mpz::from_i64(i))))
}

/// `GMP_Z_PARAM_INTO_MPZ_PTR`: argument `i` of a function as a number.
fn num_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    name: &str,
) -> Result<Arg, Thrown> {
    let v = args.get(i).cloned().unwrap_or(Value::Null);
    let at = At {
        func,
        num: i + 1,
        name,
    };
    let strict = host.caller_strict_types();
    match parse_arg(host, &v, Some(at), strict)? {
        Some(a) => Ok(a),
        None => Err(Thrown::type_error(format!(
            "{func}(): Argument #{} (${name}) must be of type {NUM_TYPE}, {} given",
            i + 1,
            zval_value_name(host, &v)
        ))),
    }
}

/// `Z_PARAM_LONG`.
fn long_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    name: &str,
) -> Result<i64, Thrown> {
    zpp::long_arg(host, args, i, func, name)
}

/// `Z_PARAM_OPTIONAL Z_PARAM_LONG` with a default.
fn opt_long_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    name: &str,
    default: i64,
) -> Result<i64, Thrown> {
    if i < args.len() {
        long_arg(host, args, i, func, name)
    } else {
        Ok(default)
    }
}

/// `Z_PARAM_STR_OR_LONG`: a string or an int.
fn str_or_long_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    name: &str,
) -> Result<Result<Vec<u8>, i64>, Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Int(0));
    let at = At {
        func,
        num: i + 1,
        name,
    };
    let wrong = |host: &dyn Host, v: &Value| {
        Thrown::type_error(format!(
            "{func}(): Argument #{} (${name}) must be of type string|int, {} given",
            i + 1,
            zval_value_name(host, v)
        ))
    };
    match &v {
        Value::Int(n) => return Ok(Err(*n)),
        Value::Str(s) => return Ok(Ok(s.as_bytes().to_vec())),
        Value::Object(_) | Value::Closure(_) => {
            // zend_parse_arg_str_weak: the object's string cast.
            if !host.caller_strict_types() {
                if let Some(s) = object_handlers::to_string(host, &v)? {
                    return Ok(Ok(s));
                }
            }
            return Err(wrong(host, &v));
        }
        Value::Array(_) | Value::Resource(_) => return Err(wrong(host, &v)),
        _ => {}
    }
    if host.caller_strict_types() {
        return Err(wrong(host, &v));
    }
    match long_weak(host, &v, Some(at), "string|int")? {
        Some(n) => Ok(Err(n)),
        // zend_parse_arg_str_weak of a float out of the int range.
        None => Ok(Ok(conv::to_string_bytes(&v))),
    }
}

/// `Z_PARAM_STR`.
fn str_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    name: &str,
) -> Result<Vec<u8>, Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    let wrong = |host: &dyn Host, v: &Value| {
        Thrown::type_error(format!(
            "{func}(): Argument #{} (${name}) must be of type string, {} given",
            i + 1,
            zval_value_name(host, v)
        ))
    };
    if let Value::Str(s) = &v {
        return Ok(s.as_bytes().to_vec());
    }
    if host.caller_strict_types() {
        return Err(wrong(host, &v));
    }
    match &v {
        Value::Null => {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                format!(
                    "{func}(): Passing null to parameter #{} (${name}) of type string is deprecated",
                    i + 1
                ).as_bytes(),
            )?;
            Ok(Vec::new())
        }
        Value::Bool(_) | Value::Int(_) | Value::Float(_) => Ok(conv::to_string_bytes(&v)),
        Value::Object(_) | Value::Closure(_) => match object_handlers::to_string(host, &v)? {
            Some(s) => Ok(s),
            None => Err(wrong(host, &v)),
        },
        _ => Err(wrong(host, &v)),
    }
}

/// `Z_PARAM_BOOL` with a default.
fn bool_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    name: &str,
    default: bool,
) -> Result<bool, Thrown> {
    let Some(v) = args.get(i).map(deref) else {
        return Ok(default);
    };
    let wrong = |host: &dyn Host, v: &Value| {
        Thrown::type_error(format!(
            "{func}(): Argument #{} (${name}) must be of type bool, {} given",
            i + 1,
            zval_value_name(host, v)
        ))
    };
    match &v {
        Value::Bool(b) => Ok(*b),
        _ if host.caller_strict_types() => Err(wrong(host, &v)),
        Value::Null => {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                format!(
                    "{func}(): Passing null to parameter #{} (${name}) of type bool is deprecated",
                    i + 1
                )
                .as_bytes(),
            )?;
            Ok(false)
        }
        Value::Int(_) | Value::Float(_) | Value::Str(_) => Ok(conv::to_bool(&v)),
        _ => Err(wrong(host, &v)),
    }
}

/// `gmp_verify_base`.
fn verify_base(base: i64, at: At) -> Result<(), Thrown> {
    if base != 0 && !(2..=GMP_MAX_BASE).contains(&base) {
        return Err(arg_value_error(
            at,
            &format!("must be 0 or between 2 and {GMP_MAX_BASE}"),
        ));
    }
    Ok(())
}

/// `gmp_initialize_number`.
fn initialize_number(arg: Result<Vec<u8>, i64>, base: i64, at: At) -> Result<Mpz, Thrown> {
    match arg {
        Ok(s) => {
            let mut z = Mpz::new();
            convert_str(&mut z, &s, base, Some(at))?;
            Ok(z)
        }
        Err(n) => Ok(Mpz::from_i64(n)),
    }
}

// ---------------------------------------------------------------------------
// size guards
// ---------------------------------------------------------------------------

/// php's `zend_string_safe_alloc(n, m, 0)`, which cannot fail: false when
/// php would stop with a fatal error instead (recorded for the VM).
fn string_alloc_ok(n: usize, m: usize) -> bool {
    use rhp_values::mem;
    match n
        .checked_mul(m)
        .and_then(|b| b.checked_add(mem::zstr_size(0)))
    {
        Some(bytes) => mem::alloc_check(bytes),
        None => {
            // Records php's "Possible integer overflow in memory allocation".
            mem::reserve_safe(n, m, mem::zstr_size(0));
            false
        }
    }
}

/// `a + b`, `a - b` and the bitwise operators need at most one limb more
/// than the larger operand.
fn check_grow(a: &Mpz, b: &Mpz, at: Option<At>, what: &str) -> Result<(), Thrown> {
    if fits(a.limbs().max(b.limbs()) as u128 + 1) {
        Ok(())
    } else {
        Err(too_large(at, what))
    }
}

/// `mpz_mul` allocates the sum of the operands' sizes (as does `mpz_lcm`).
fn check_mul(a: &Mpz, b: &Mpz, at: Option<At>) -> Result<(), Thrown> {
    if fits(a.limbs() as u128 + b.limbs() as u128) {
        Ok(())
    } else {
        Err(too_large(at, "Number"))
    }
}

/// `mpz_pow_ui`, sized as libgmp sizes its result: the bits of |b| times
/// `e`, plus the low zero bits again, plus a few limbs; a power of two only
/// needs its shifted bit.
fn check_pow(b: &Mpz, e: u64, at: Option<At>) -> Result<(), Thrown> {
    if e == 0 || b.cmp_si(1) <= 0 && b.cmp_si(-1) >= 0 {
        return Ok(());
    }
    let bits = b.bits() as u128;
    let twos = b.scan1(0) as u128;
    let e = e as u128;
    let limbs = if bits - twos == 1 {
        twos * e / 64 + 2
    } else {
        (bits + twos) * e / 64 + 8
    };
    if fits(limbs) {
        Ok(())
    } else {
        Err(too_large(at, "Exponent"))
    }
}

/// `mpz_mul_2exp`: the operand's limbs, the shift's, and one more.
fn check_shl(x: &Mpz, shift: u64) -> Result<(), Thrown> {
    if x.sgn() == 0 || fits(x.limbs() as u128 + (shift / 64) as u128 + 1) {
        Ok(())
    } else {
        Err(too_large(None, "Shift"))
    }
}

/// Factorials and binomials are estimated; refuse at half the limit, so
/// libgmp's own (larger) estimates stay within it.
fn check_estimate(bits: f64, at: At) -> Result<(), Thrown> {
    if bits / 64.0 + 64.0 <= (INT_MAX / 2) as f64 {
        Ok(())
    } else {
        Err(too_large(Some(at), ""))
    }
}

/// log2(n!) by Stirling's formula, rounded up.
fn fact_bits(n: u64) -> f64 {
    if n < 2 {
        return 1.0;
    }
    let n = n as f64;
    n * n.log2() - n * std::f64::consts::LOG2_E
        + 0.5 * (2.0 * std::f64::consts::PI * n).log2()
        + 2.0
}

/// An upper bound on the bits of binomial(n, k) as `mpz_bin_ui` computes it:
/// binomial(ni + k', k') with ni = n - k (or -n - 1 for negative n) and
/// k' = min(k, ni).
fn bin_bits(n: &Mpz, k: u64) -> f64 {
    let ni = if n.sgn() >= 0 {
        if n.cmp(&Mpz::from_u64(k)) < 0 {
            return 1.0;
        }
        Mpz::binop(sys::mpz_sub, n, &Mpz::from_u64(k))
    } else {
        Mpz::unop(sys::mpz_neg, &n.add_ui(1))
    };
    let small = if ni.cmp(&Mpz::from_u64(k)) < 0 {
        ni.get_ui()
    } else {
        k
    };
    let top = ni.bits().max(64) + 1;
    small as f64 * top as f64 + 1.0
}

// ---------------------------------------------------------------------------
// gmp_init, gmp_import / gmp_export, gmp_intval, gmp_strval
// ---------------------------------------------------------------------------

fn fn_gmp_init(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_init";
    let num = str_or_long_arg(host, args, 0, f, "num")?;
    let base = opt_long_arg(host, args, 1, f, "base", 0)?;
    verify_base(
        base,
        At {
            func: f,
            num: 2,
            name: "base",
        },
    )?;
    let at = At {
        func: f,
        num: 1,
        name: "num",
    };
    let z = initialize_number(num, base, at)?;
    gmp_new(host, z)
}

/// `GMP::__construct` (`__gmp_construct($this, $num, $base)`).
fn fn_gmp_construct(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "GMP::__construct";
    let this = object_handlers::object_of(&args[0]).expect("GMP::__construct passes $this");
    let rest = &args[1..];
    let num = str_or_long_arg(host, rest, 0, f, "num")?;
    let base = opt_long_arg(host, rest, 1, f, "base", 0)?;
    verify_base(
        base,
        At {
            func: f,
            num: 2,
            name: "base",
        },
    )?;
    let at = At {
        func: f,
        num: 1,
        name: "num",
    };
    let z = initialize_number(num, base, at)?;
    *gmp_num(&this).0.borrow_mut() = z;
    Ok(Value::Null)
}

/// `gmp_import_export_validate`: (order, endian).
fn import_export_validate(func: &str, size: i64, options: i64) -> Result<(i32, i32), Thrown> {
    if size < 1 {
        return Err(zpp::value_error(
            func,
            2,
            "word_size",
            "must be greater than or equal to 1",
        ));
    }
    let order = match options & (GMP_LSW_FIRST | GMP_MSW_FIRST) {
        GMP_LSW_FIRST => -1,
        GMP_MSW_FIRST | 0 => 1,
        _ => {
            return Err(zpp::value_error(
                func,
                3,
                "flags",
                "cannot use multiple word order options",
            ))
        }
    };
    let endian = match options & (GMP_LITTLE_ENDIAN | GMP_BIG_ENDIAN | GMP_NATIVE_ENDIAN) {
        GMP_LITTLE_ENDIAN => -1,
        GMP_BIG_ENDIAN => 1,
        GMP_NATIVE_ENDIAN | 0 => 0,
        _ => {
            return Err(zpp::value_error(
                func,
                3,
                "flags",
                "cannot use multiple endian options",
            ))
        }
    };
    Ok((order, endian))
}

fn fn_gmp_import(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_import";
    let data = str_arg(host, args, 0, f, "data")?;
    let size = opt_long_arg(host, args, 1, f, "word_size", 1)?;
    let options = opt_long_arg(host, args, 2, f, "flags", GMP_MSW_FIRST | GMP_NATIVE_ENDIAN)?;
    let (order, endian) = import_export_validate(f, size, options)?;
    if !(data.len() as u64).is_multiple_of(size as u64) {
        return Err(zpp::value_error(
            f,
            1,
            "data",
            "must be a multiple of argument #2 ($word_size)",
        ));
    }
    if !fits(data.len() as u128 / 8 + 1) {
        return Err(zpp::value_error(f, 1, "data", "is too large"));
    }
    let z = Mpz::import(&data, order, size as usize, endian);
    gmp_new(host, z)
}

fn fn_gmp_export(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_export";
    let num = num_arg(host, args, 0, f, "num")?;
    let size = opt_long_arg(host, args, 1, f, "word_size", 1)?;
    let options = opt_long_arg(host, args, 2, f, "flags", GMP_MSW_FIRST | GMP_NATIVE_ENDIAN)?;
    let (order, endian) = import_export_validate(f, size, options)?;
    let z = num.get();
    if z.sgn() == 0 {
        return Ok(Value::str(Vec::new()));
    }
    let bits = z.sizeinbase(2) as u64;
    if size > i64::MAX / 4 || bits > u64::MAX - (size as u64) * 8 + 1 {
        return Err(zpp::value_error(
            f,
            2,
            "word_size",
            "is too large for argument #1 ($num)",
        ));
    }
    let word_bits = size as u64 * 8;
    let count = bits.div_ceil(word_bits) as usize;
    if !string_alloc_ok(count, size as usize) {
        return Ok(Value::Null);
    }
    Ok(Value::str(z.export(order, size as usize, endian, count)))
}

fn fn_gmp_intval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = num_arg(host, args, 0, "gmp_intval", "num")?;
    let i = n.get().get_si();
    Ok(Value::Int(i))
}

fn fn_gmp_strval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_strval";
    let n = num_arg(host, args, 0, f, "num")?;
    let base = opt_long_arg(host, args, 1, f, "base", 10)?;
    // mpz_get_str takes negative bases only down to -36.
    if (-1..=1).contains(&base) || !(-36..=GMP_MAX_BASE).contains(&base) {
        return Err(zpp::value_error(
            f,
            2,
            "base",
            &format!("must be between 2 and {GMP_MAX_BASE}, or -2 and -36"),
        ));
    }
    let z = n.get();
    // zend_string_alloc
    let len = z.sizeinbase(base.abs() as i32) + 1;
    if !rhp_values::mem::alloc_check(rhp_values::mem::zstr_size(len)) {
        return Ok(Value::Null);
    }
    Ok(Value::str(z.to_str(base as i32)))
}

// ---------------------------------------------------------------------------
// arithmetic
// ---------------------------------------------------------------------------

fn unary(host: &mut dyn Host, args: &[Value], func: &str, op: sys::UnOp) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, func, "num")?;
    let r = {
        let a = a.get();
        if !fits(a.limbs() as u128 + 1) {
            return Err(too_large(
                Some(At {
                    func,
                    num: 1,
                    name: "num",
                }),
                "",
            ));
        }
        Mpz::unop(op, &a)
    };
    gmp_new(host, r)
}

fn fn_gmp_neg(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    unary(host, args, "gmp_neg", sys::mpz_neg)
}

fn fn_gmp_abs(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    unary(host, args, "gmp_abs", sys::mpz_abs)
}

fn fn_gmp_com(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    unary(host, args, "gmp_com", sys::mpz_com)
}

fn fn_gmp_nextprime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    unary(host, args, "gmp_nextprime", sys::mpz_nextprime)
}

/// `GMP_BINARY_OP_FUNCTION`.
fn binary(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    op: sys::BinOp,
    mul: bool,
) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, func, "num1")?;
    let b = num_arg(host, args, 1, func, "num2")?;
    let at = Some(At {
        func,
        num: 2,
        name: "num2",
    });
    let r = {
        let (a, b) = (a.get(), b.get());
        if mul {
            check_mul(&a, &b, at)?;
        } else {
            check_grow(&a, &b, at, "")?;
        }
        Mpz::binop(op, &a, &b)
    };
    gmp_new(host, r)
}

fn fn_gmp_add(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    binary(host, args, "gmp_add", sys::mpz_add, false)
}

fn fn_gmp_sub(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    binary(host, args, "gmp_sub", sys::mpz_sub, false)
}

fn fn_gmp_mul(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    binary(host, args, "gmp_mul", sys::mpz_mul, true)
}

fn fn_gmp_gcd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    binary(host, args, "gmp_gcd", sys::mpz_gcd, false)
}

fn fn_gmp_lcm(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    binary(host, args, "gmp_lcm", sys::mpz_lcm, true)
}

fn fn_gmp_and(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    binary(host, args, "gmp_and", sys::mpz_and, false)
}

fn fn_gmp_or(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    binary(host, args, "gmp_or", sys::mpz_ior, false)
}

fn fn_gmp_xor(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    binary(host, args, "gmp_xor", sys::mpz_xor, false)
}

/// The dividend, a non-zero divisor and the rounding mode of `gmp_div_*`.
fn div_args(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    zero: &str,
) -> Result<(Arg, Arg, i64), Thrown> {
    let a = num_arg(host, args, 0, func, "num1")?;
    let b = num_arg(host, args, 1, func, "num2")?;
    let round = opt_long_arg(host, args, 2, func, "rounding_mode", GMP_ROUND_ZERO)?;
    if b.get().sgn() == 0 {
        return Err(Thrown::error(
            "DivisionByZeroError",
            format!("{func}(): Argument #2 ($num2) {zero}"),
        ));
    }
    Ok((a, b, round))
}

fn bad_round(func: &str) -> Thrown {
    zpp::value_error(
        func,
        3,
        "rounding_mode",
        "must be one of GMP_ROUND_ZERO, GMP_ROUND_PLUSINF, or GMP_ROUND_MINUSINF",
    )
}

fn fn_gmp_div_qr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_div_qr";
    let (a, b, round) = div_args(host, args, f, "Division by zero")?;
    let op: sys::DivQr = match round {
        GMP_ROUND_ZERO => sys::mpz_tdiv_qr,
        GMP_ROUND_PLUSINF => sys::mpz_cdiv_qr,
        GMP_ROUND_MINUSINF => sys::mpz_fdiv_qr,
        _ => return Err(bad_round(f)),
    };
    let (q, r) = Mpz::divqr(op, &a.get(), &b.get());
    pair(host, q, r)
}

fn div_one(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    ops: [sys::BinOp; 3],
) -> Result<Value, Thrown> {
    let (a, b, round) = div_args(host, args, func, "Division by zero")?;
    let op = match round {
        GMP_ROUND_ZERO => ops[0],
        GMP_ROUND_PLUSINF => ops[1],
        GMP_ROUND_MINUSINF => ops[2],
        _ => return Err(bad_round(func)),
    };
    let r = Mpz::binop(op, &a.get(), &b.get());
    gmp_new(host, r)
}

fn fn_gmp_div_r(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    div_one(
        host,
        args,
        "gmp_div_r",
        [sys::mpz_tdiv_r, sys::mpz_cdiv_r, sys::mpz_fdiv_r],
    )
}

fn fn_gmp_div_q(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    div_one(
        host,
        args,
        "gmp_div_q",
        [sys::mpz_tdiv_q, sys::mpz_cdiv_q, sys::mpz_fdiv_q],
    )
}

/// `gmp_div`, an alias of `gmp_div_q` under its own name.
fn fn_gmp_div(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    div_one(
        host,
        args,
        "gmp_div",
        [sys::mpz_tdiv_q, sys::mpz_cdiv_q, sys::mpz_fdiv_q],
    )
}

fn fn_gmp_mod(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_mod";
    let a = num_arg(host, args, 0, f, "num1")?;
    let b = num_arg(host, args, 1, f, "num2")?;
    if b.get().sgn() == 0 {
        return Err(Thrown::error(
            "DivisionByZeroError",
            "gmp_mod(): Argument #2 ($num2) Modulo by zero",
        ));
    }
    let r = Mpz::binop(sys::mpz_mod, &a.get(), &b.get());
    gmp_new(host, r)
}

fn fn_gmp_divexact(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_divexact";
    let a = num_arg(host, args, 0, f, "num1")?;
    let b = num_arg(host, args, 1, f, "num2")?;
    if b.get().sgn() == 0 {
        return Err(Thrown::error(
            "DivisionByZeroError",
            "gmp_divexact(): Argument #2 ($num2) Division by zero",
        ));
    }
    let r = Mpz::binop(sys::mpz_divexact, &a.get(), &b.get());
    gmp_new(host, r)
}

fn fn_gmp_fact(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_fact";
    let n = num_arg(host, args, 0, f, "num")?;
    let at = At {
        func: f,
        num: 1,
        name: "num",
    };
    let n = {
        let n = n.get();
        if n.sgn() < 0 {
            return Err(arg_value_error(at, "must be greater than or equal to 0"));
        }
        // php passes mpz_get_ui: the low limb.
        n.get_ui()
    };
    check_estimate(fact_bits(n), at)?;
    gmp_new(host, Mpz::fac_ui(n))
}

fn fn_gmp_binomial(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_binomial";
    let n = num_arg(host, args, 0, f, "n")?;
    let k = long_arg(host, args, 1, f, "k")?;
    let at = At {
        func: f,
        num: 2,
        name: "k",
    };
    if k < 0 {
        return Err(arg_value_error(at, "must be greater than or equal to 0"));
    }
    let r = {
        let n = n.get();
        check_estimate(bin_bits(&n, k as u64), at)?;
        n.bin_ui(k as u64)
    };
    gmp_new(host, r)
}

fn fn_gmp_pow(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_pow";
    let base = num_arg(host, args, 0, f, "num")?;
    let exp = long_arg(host, args, 1, f, "exponent")?;
    let at = At {
        func: f,
        num: 2,
        name: "exponent",
    };
    if exp < 0 {
        return Err(arg_value_error(at, "must be greater than or equal to 0"));
    }
    let r = {
        let b = base.get();
        check_pow(&b, exp as u64, Some(at))?;
        Mpz::uiop(sys::mpz_pow_ui, &b, exp as u64)
    };
    gmp_new(host, r)
}

fn fn_gmp_powm(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_powm";
    let base = num_arg(host, args, 0, f, "num")?;
    let exp = num_arg(host, args, 1, f, "exponent")?;
    let m = num_arg(host, args, 2, f, "modulus")?;
    if exp.get().sgn() < 0 {
        return Err(zpp::value_error(
            f,
            2,
            "exponent",
            "must be greater than or equal to 0",
        ));
    }
    if m.get().sgn() == 0 {
        return Err(Thrown::error("DivisionByZeroError", "Modulo by zero"));
    }
    let r = base.get().powm(&exp.get(), &m.get());
    gmp_new(host, r)
}

// ---------------------------------------------------------------------------
// roots
// ---------------------------------------------------------------------------

fn non_negative(n: &Arg, func: &str) -> Result<(), Thrown> {
    if n.get().sgn() < 0 {
        return Err(zpp::value_error(
            func,
            1,
            "num",
            "must be greater than or equal to 0",
        ));
    }
    Ok(())
}

fn fn_gmp_sqrt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, "gmp_sqrt", "num")?;
    non_negative(&a, "gmp_sqrt")?;
    let r = Mpz::unop(sys::mpz_sqrt, &a.get());
    gmp_new(host, r)
}

fn fn_gmp_sqrtrem(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, "gmp_sqrtrem", "num")?;
    non_negative(&a, "gmp_sqrtrem")?;
    let (s, r) = a.get().sqrtrem();
    pair(host, s, r)
}

/// The number and a valid `$nth` of `gmp_root` / `gmp_rootrem`.
fn root_args(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    min_msg: &str,
) -> Result<(Arg, u64), Thrown> {
    let a = num_arg(host, args, 0, func, "num")?;
    let nth = long_arg(host, args, 1, func, "nth")?;
    if nth <= 0 {
        return Err(zpp::value_error(func, 2, "nth", min_msg));
    }
    if nth % 2 == 0 && a.get().sgn() < 0 {
        return Err(zpp::value_error(
            func,
            2,
            "nth",
            "must be odd if argument #1 ($a) is negative",
        ));
    }
    Ok((a, nth as u64))
}

fn fn_gmp_root(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (a, nth) = root_args(host, args, "gmp_root", "must be greater than 0")?;
    let r = a.get().root(nth);
    gmp_new(host, r)
}

fn fn_gmp_rootrem(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (a, nth) = root_args(
        host,
        args,
        "gmp_rootrem",
        "must be greater than or equal to 1",
    )?;
    let (r, rem) = a.get().rootrem(nth);
    pair(host, r, rem)
}

// ---------------------------------------------------------------------------
// number theory
// ---------------------------------------------------------------------------

fn fn_gmp_perfect_square(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, "gmp_perfect_square", "num")?;
    let r = a.get().perfect_square();
    Ok(Value::Bool(r))
}

fn fn_gmp_perfect_power(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, "gmp_perfect_power", "num")?;
    let r = a.get().perfect_power();
    Ok(Value::Bool(r))
}

fn fn_gmp_prob_prime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_prob_prime";
    let a = num_arg(host, args, 0, f, "num")?;
    let reps = opt_long_arg(host, args, 1, f, "repetitions", 10)?;
    // php casts to int.
    let r = a.get().probab_prime(reps as i32);
    Ok(Value::Int(r as i64))
}

fn fn_gmp_gcdext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_gcdext";
    let a = num_arg(host, args, 0, f, "num1")?;
    let b = num_arg(host, args, 1, f, "num2")?;
    let (g, s, t) = Mpz::gcdext(&a.get(), &b.get());
    let mut arr = PhpArray::new();
    arr.insert(ArrayKey::from_bytes(b"g"), gmp_new(host, g)?);
    arr.insert(ArrayKey::from_bytes(b"s"), gmp_new(host, s)?);
    arr.insert(ArrayKey::from_bytes(b"t"), gmp_new(host, t)?);
    Ok(Value::Array(arr))
}

fn fn_gmp_invert(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_invert";
    let a = num_arg(host, args, 0, f, "num1")?;
    let b = num_arg(host, args, 1, f, "num2")?;
    if b.get().sgn() == 0 {
        return Err(Thrown::error("DivisionByZeroError", "Division by zero"));
    }
    let r = a.get().invert(&b.get());
    match r {
        Some(r) => gmp_new(host, r),
        None => Ok(Value::Bool(false)),
    }
}

/// `mpz_jacobi`, which gmp.h also names `mpz_legendre` and `mpz_kronecker`.
fn symbol(host: &mut dyn Host, args: &[Value], func: &str) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, func, "num1")?;
    let b = num_arg(host, args, 1, func, "num2")?;
    let r = a.get().jacobi(&b.get());
    Ok(Value::Int(r as i64))
}

fn fn_gmp_jacobi(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    symbol(host, args, "gmp_jacobi")
}

fn fn_gmp_legendre(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    symbol(host, args, "gmp_legendre")
}

fn fn_gmp_kronecker(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    symbol(host, args, "gmp_kronecker")
}

fn fn_gmp_cmp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, "gmp_cmp", "num1")?;
    let b = num_arg(host, args, 1, "gmp_cmp", "num2")?;
    let r = a.get().cmp(&b.get());
    Ok(Value::Int(r.signum() as i64))
}

fn fn_gmp_sign(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, "gmp_sign", "num")?;
    let r = a.get().sgn();
    Ok(Value::Int(r as i64))
}

// ---------------------------------------------------------------------------
// random numbers
// ---------------------------------------------------------------------------

thread_local! {
    /// `GMPG(rand_state)`: per request (cleared by [`reset_state`]).
    static RAND: RefCell<Option<sys::RandState>> = const { RefCell::new(None) };
}

/// Forget the random state, as php's RSHUTDOWN does.
pub fn reset_state() {
    let old = RAND.with(|r| r.borrow_mut().take());
    drop(old);
}

/// `gmp_init_random`, then `f` on the state.
fn with_rand<R>(f: impl FnOnce(&mut sys::RandState) -> R) -> R {
    RAND.with(|r| {
        let mut r = r.borrow_mut();
        let state = r.get_or_insert_with(|| {
            let mut s = sys::RandState::new_mt();
            let mut seed = [0u8; 8];
            if getrandom::getrandom(&mut seed).is_err() {
                // php_random_generate_fallback_seed
                let t = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                seed = (t ^ std::process::id() as u64).to_ne_bytes();
            }
            s.seed_ui(u64::from_ne_bytes(seed));
            s
        });
        f(state)
    })
}

fn fn_gmp_random_seed(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let seed = num_arg(host, args, 0, "gmp_random_seed", "seed")?;
    let seed = seed.get();
    with_rand(|s| s.seed(&seed));
    Ok(Value::Null)
}

fn fn_gmp_random_bits(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_random_bits";
    let bits = long_arg(host, args, 0, f, "bits")?;
    if bits <= 0 || bits > INT_MAX {
        return Err(zpp::value_error(
            f,
            1,
            "bits",
            &format!("must be between 1 and {INT_MAX}"),
        ));
    }
    let r = with_rand(|s| Mpz::urandomb(s, bits as u64));
    gmp_new(host, r)
}

fn fn_gmp_random_range(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_random_range";
    let min = num_arg(host, args, 0, f, "min")?;
    let max = num_arg(host, args, 1, f, "max")?;
    with_rand(|_| ());
    let r = {
        let (min, max) = (min.get(), max.get());
        if max.cmp(&min) <= 0 {
            return Err(zpp::value_error(
                f,
                1,
                "min",
                "must be less than argument #2 ($maximum)",
            ));
        }
        let range = Mpz::binop(sys::mpz_sub, &max, &min).add_ui(1);
        let r = with_rand(|s| Mpz::urandomm(s, &range));
        Mpz::binop(sys::mpz_add, &r, &min)
    };
    gmp_new(host, r)
}

// ---------------------------------------------------------------------------
// bits
// ---------------------------------------------------------------------------

/// `gmp_is_bit_index_valid`, else php's ValueError.
fn bit_index(func: &str, name: &str, index: i64) -> Result<u64, Thrown> {
    if index >= 0 && index / GMP_NUMB_BITS < INT_MAX {
        return Ok(index as u64);
    }
    Err(zpp::value_error(
        func,
        2,
        name,
        &format!("must be between 0 and {INT_MAX} * {GMP_NUMB_BITS}"),
    ))
}

/// zpp `O` with class GMP.
fn gmp_this(host: &dyn Host, args: &[Value], func: &str) -> Result<Rc<GmpNum>, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    match gmp_object(host, &v) {
        Some(h) => Ok(gmp_num(&h)),
        None => Err(Thrown::type_error(format!(
            "{func}(): Argument #1 ($num) must be of type GMP, {} given",
            zval_value_name(host, &v)
        ))),
    }
}

fn fn_gmp_setbit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_setbit";
    let n = gmp_this(host, args, f)?;
    let index = long_arg(host, args, 1, f, "index")?;
    let set = bool_arg(host, args, 2, f, "value", true)?;
    let index = bit_index(f, "index", index)?;
    let mut z = n.0.borrow_mut();
    if set {
        z.setbit(index);
    } else {
        z.clrbit(index);
    }
    Ok(Value::Null)
}

fn fn_gmp_clrbit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_clrbit";
    let n = gmp_this(host, args, f)?;
    let index = long_arg(host, args, 1, f, "index")?;
    let index = bit_index(f, "index", index)?;
    n.0.borrow_mut().clrbit(index);
    Ok(Value::Null)
}

fn fn_gmp_testbit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "gmp_testbit";
    let a = num_arg(host, args, 0, f, "num")?;
    let index = long_arg(host, args, 1, f, "index")?;
    let index = bit_index(f, "index", index)?;
    let r = a.get().tstbit(index);
    Ok(Value::Bool(r))
}

fn fn_gmp_popcount(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, "gmp_popcount", "num")?;
    // ULONG_MAX (a negative number) is -1 as a zend_long.
    let r = a.get().popcount() as i64;
    Ok(Value::Int(r))
}

fn fn_gmp_hamdist(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, "gmp_hamdist", "num1")?;
    let b = num_arg(host, args, 1, "gmp_hamdist", "num2")?;
    let r = a.get().hamdist(&b.get()) as i64;
    Ok(Value::Int(r))
}

fn scan(host: &mut dyn Host, args: &[Value], func: &str, one: bool) -> Result<Value, Thrown> {
    let a = num_arg(host, args, 0, func, "num1")?;
    let start = long_arg(host, args, 1, func, "start")?;
    let start = bit_index(func, "start", start)?;
    let a = a.get();
    let r = if one { a.scan1(start) } else { a.scan0(start) };
    Ok(Value::Int(r as i64))
}

fn fn_gmp_scan0(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    scan(host, args, "gmp_scan0", false)
}

fn fn_gmp_scan1(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    scan(host, args, "gmp_scan1", true)
}

// ---------------------------------------------------------------------------
// __serialize / __unserialize
// ---------------------------------------------------------------------------

/// `GMP::__serialize` (`__gmp_serialize($this)`): the number in base 16,
/// then the properties when there are any.
fn fn_gmp_serialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = object_handlers::object_of(&args[0]).expect("GMP::__serialize passes $this");
    let num = gmp_num(&h).0.borrow().to_str(16);
    let mut arr = PhpArray::new();
    arr.push(Value::str(num));
    if !h.borrow().properties.is_empty() {
        // zend_proptable_to_symtable: numeric names become integer keys.
        let props = host.object_props_mangled(&h);
        let mut table = PhpArray::new();
        for (k, v) in props.iter() {
            let key = match k {
                ArrayKey::Str(s) => ArrayKey::from_php_str(s),
                k => k.clone(),
            };
            table.insert(key, deref(v));
        }
        arr.push(Value::Array(table));
    }
    Ok(Value::Array(arr))
}

/// `GMP::__unserialize` (`__gmp_unserialize($this, $data)`).
fn fn_gmp_unserialize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = object_handlers::object_of(&args[0]).expect("GMP::__unserialize passes $this");
    let data = match args.get(1).map(deref) {
        Some(Value::Array(a)) => a,
        other => {
            let v = other.unwrap_or(Value::Null);
            return Err(Thrown::type_error(format!(
                "GMP::__unserialize(): Argument #1 ($data) must be of type array, {} given",
                zval_value_name(host, &v)
            )));
        }
    };
    let fail = |what: &str| Thrown::error("Exception", format!("Could not unserialize {what}"));
    let mut z = Mpz::new();
    match data.get(&ArrayKey::Int(0)).map(deref) {
        Some(Value::Str(s)) => {
            if let Err(e) = convert_str(&mut z, s.as_bytes(), 16, None) {
                // php throws over the conversion's ValueError.
                return Err(host.chain_thrown(fail("number"), e));
            }
        }
        _ => return Err(fail("number")),
    }
    *gmp_num(&h).0.borrow_mut() = z;
    if let Some(props) = data.get(&ArrayKey::Int(1)).map(deref) {
        let Value::Array(props) = props else {
            return Err(fail("properties"));
        };
        properties_load(host, &h, &props)?;
    }
    Ok(Value::Null)
}

/// `object_properties_load` for GMP, which declares no properties: each
/// entry becomes a dynamic property (deprecated unless it exists).
fn properties_load(host: &mut dyn Host, h: &ObjectHandle, props: &PhpArray) -> Result<(), Thrown> {
    for (k, v) in props.iter() {
        let name = match k {
            ArrayKey::Str(s) => s.as_bytes().to_vec(),
            ArrayKey::Int(i) => i.to_string().into_bytes(),
        };
        let key = PhpStr::from_bytes(name.clone());
        if !h.borrow().properties.contains_key(&key) {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                format!(
                    "Creation of dynamic property GMP::${} is deprecated",
                    String::from_utf8_lossy(&name)
                )
                .as_bytes(),
            )?;
        }
        h.borrow_mut().properties.insert(key, v.clone());
    }
    Ok(())
}

/// Whether class `cid` is GMP, whose `ce->unserialize` reads the `C:`
/// format (see ext/standard/var.rs).
pub(crate) fn is_gmp_class(host: &dyn Host, cid: u32) -> bool {
    host.object_handlers(cid)
        .is_some_and(|t| std::ptr::eq(t, &GMP_HANDLERS))
}

/// gmp_unserialize's `gmp_create`: a GMP object holding 0.
pub(crate) fn unserialize_create(host: &mut dyn Host) -> Result<Value, Thrown> {
    gmp_new(host, Mpz::new())
}

/// gmp_unserialize's number: `convert_zstr_to_gmp(num, s, 10, 0)`.
pub(crate) fn unserialize_number(obj: &Value, s: &[u8]) -> Result<(), Thrown> {
    let h = object_handlers::object_of(obj).expect("a GMP object");
    let mut z = Mpz::new();
    convert_str(&mut z, s, 10, None)?;
    *gmp_num(&h).0.borrow_mut() = z;
    Ok(())
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register all `gmp_*` builtins.
pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("gmp_init", fn_gmp_init),
            ("gmp_import", fn_gmp_import),
            ("gmp_export", fn_gmp_export),
            ("gmp_intval", fn_gmp_intval),
            ("gmp_strval", fn_gmp_strval),
            ("gmp_add", fn_gmp_add),
            ("gmp_sub", fn_gmp_sub),
            ("gmp_mul", fn_gmp_mul),
            ("gmp_div_qr", fn_gmp_div_qr),
            ("gmp_div_q", fn_gmp_div_q),
            ("gmp_div_r", fn_gmp_div_r),
            ("gmp_div", fn_gmp_div),
            ("gmp_mod", fn_gmp_mod),
            ("gmp_divexact", fn_gmp_divexact),
            ("gmp_neg", fn_gmp_neg),
            ("gmp_abs", fn_gmp_abs),
            ("gmp_fact", fn_gmp_fact),
            ("gmp_sqrt", fn_gmp_sqrt),
            ("gmp_sqrtrem", fn_gmp_sqrtrem),
            ("gmp_root", fn_gmp_root),
            ("gmp_rootrem", fn_gmp_rootrem),
            ("gmp_pow", fn_gmp_pow),
            ("gmp_powm", fn_gmp_powm),
            ("gmp_perfect_square", fn_gmp_perfect_square),
            ("gmp_perfect_power", fn_gmp_perfect_power),
            ("gmp_prob_prime", fn_gmp_prob_prime),
            ("gmp_gcd", fn_gmp_gcd),
            ("gmp_gcdext", fn_gmp_gcdext),
            ("gmp_lcm", fn_gmp_lcm),
            ("gmp_invert", fn_gmp_invert),
            ("gmp_jacobi", fn_gmp_jacobi),
            ("gmp_legendre", fn_gmp_legendre),
            ("gmp_kronecker", fn_gmp_kronecker),
            ("gmp_cmp", fn_gmp_cmp),
            ("gmp_sign", fn_gmp_sign),
            ("gmp_random_seed", fn_gmp_random_seed),
            ("gmp_random_bits", fn_gmp_random_bits),
            ("gmp_random_range", fn_gmp_random_range),
            ("gmp_and", fn_gmp_and),
            ("gmp_or", fn_gmp_or),
            ("gmp_com", fn_gmp_com),
            ("gmp_xor", fn_gmp_xor),
            ("gmp_setbit", fn_gmp_setbit),
            ("gmp_clrbit", fn_gmp_clrbit),
            ("gmp_testbit", fn_gmp_testbit),
            ("gmp_scan0", fn_gmp_scan0),
            ("gmp_scan1", fn_gmp_scan1),
            ("gmp_popcount", fn_gmp_popcount),
            ("gmp_hamdist", fn_gmp_hamdist),
            ("gmp_nextprime", fn_gmp_nextprime),
            ("gmp_binomial", fn_gmp_binomial),
        ],
    );
    crate::register_helpers(
        m,
        &[
            ("__gmp_construct", fn_gmp_construct, 1, Some(3)),
            ("__gmp_serialize", fn_gmp_serialize, 1, Some(1)),
            ("__gmp_unserialize", fn_gmp_unserialize, 2, Some(2)),
        ],
    );
}

/// The `GMP` class. Its parameters are parsed natively (php's zpp, not a
/// user function's type coercion).
pub const PRELUDE_PHP: &str = include_str!("prelude.php");

/// `GMP_*` constants.
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("GMP_ROUND_ZERO", Value::Int(GMP_ROUND_ZERO)),
        ("GMP_ROUND_PLUSINF", Value::Int(GMP_ROUND_PLUSINF)),
        ("GMP_ROUND_MINUSINF", Value::Int(GMP_ROUND_MINUSINF)),
        ("GMP_MSW_FIRST", Value::Int(GMP_MSW_FIRST)),
        ("GMP_LSW_FIRST", Value::Int(GMP_LSW_FIRST)),
        ("GMP_LITTLE_ENDIAN", Value::Int(GMP_LITTLE_ENDIAN)),
        ("GMP_BIG_ENDIAN", Value::Int(GMP_BIG_ENDIAN)),
        ("GMP_NATIVE_ENDIAN", Value::Int(GMP_NATIVE_ENDIAN)),
        ("GMP_VERSION", Value::str(sys::version())),
    ]
}

// ---------------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------------

/// gmp.c's `gmp_object_handlers`; `free_obj` is the payload's drop.
static GMP_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(gmp_get_properties_for),
    clone_obj: CloneObj::Custom(gmp_clone_obj),
    compare: Some(gmp_compare),
    do_operation: Some(gmp_do_operation),
    cast_object: Some(gmp_cast_object),
    ..ObjectHandlers::STD
};

pub(crate) static CLASSES: &[ClassHandlers] = &[ClassHandlers {
    class: "GMP",
    handlers: &GMP_HANDLERS,
    create_object: true,
}];

/// `gmp_get_debug_info`: the properties and `num`, for the dumpers.
fn gmp_get_properties_for(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose != PropPurpose::Debug {
        return Ok(None);
    }
    let mut table = PhpArray::new();
    {
        let b = h.borrow();
        for (k, v) in b.properties.iter() {
            table.insert(ArrayKey::Str(host.mangled_prop_name(b.class, k)), v.clone());
        }
    }
    let num = gmp_num(h).0.borrow().to_str(10);
    table.insert(ArrayKey::Str(PhpStr::from("num")), Value::str(num));
    Ok(Some(table))
}

/// `gmp_clone_obj`: the copy gets the number too.
fn gmp_clone_obj(_: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let n = gmp_num(src).0.borrow().clone();
    object_handlers::set_payload(dst, Rc::new(GmpNum(RefCell::new(n))));
    Ok(())
}

/// An operator's operand (`is_operator`), with the strictness php's
/// `ZEND_ARG_USES_STRICT_TYPES()` sees there.
fn operand(host: &mut dyn Host, v: &Value) -> Result<Option<Arg>, Thrown> {
    let strict = host.arg_uses_strict_types();
    parse_arg(host, v, None, strict)
}

/// The operand, or php's "Number must be of type GMP|string|int" TypeError.
fn operand_or_error(host: &mut dyn Host, v: &Value) -> Result<Arg, Thrown> {
    match operand(host, v)? {
        Some(n) => Ok(n),
        None => Err(Thrown::type_error(format!(
            "Number must be of type {NUM_TYPE}, {} given",
            zval_value_name(host, v)
        ))),
    }
}

/// gmp_compare: both operands are converted to GMP; a failed conversion
/// throws, which makes the pair uncomparable.
fn gmp_compare(host: &mut dyn Host, a: &Value, b: &Value) -> Result<Option<i32>, Thrown> {
    let x = operand_or_error(host, a)?;
    let y = operand_or_error(host, b)?;
    let r = x.get().cmp(&y.get());
    Ok(Some(r.signum()))
}

/// gmp_cast_object.
fn gmp_cast_object(
    _: &mut dyn Host,
    h: &ObjectHandle,
    target: CastTarget,
) -> Result<Option<Value>, Thrown> {
    let n = gmp_num(h);
    let n = n.0.borrow();
    Ok(match target {
        CastTarget::Str => Some(Value::str(n.to_str(10))),
        CastTarget::Int => Some(Value::Int(n.get_si())),
        CastTarget::Float => Some(Value::Float(n.get_d())),
        CastTarget::Number => Some(if n.fits_slong() {
            Value::Int(n.get_si())
        } else {
            Value::Float(n.get_d())
        }),
        CastTarget::Bool => Some(Value::Bool(n.sgn() != 0)),
        _ => None,
    })
}

/// gmp_do_operation.
fn gmp_do_operation(
    host: &mut dyn Host,
    op: Op,
    a: &Value,
    b: Option<&Value>,
) -> Result<Option<Value>, Thrown> {
    let r = match (op, b) {
        (Op::BitNot, _) => {
            let x = operand_or_error(host, a)?;
            let x = x.get();
            if !fits(x.limbs() as u128 + 1) {
                return Err(too_large(None, "Number"));
            }
            Mpz::unop(sys::mpz_com, &x)
        }
        (Op::Pow | Op::Shl | Op::Shr, Some(b)) => {
            let Some((x, shift)) = shift_operands(host, op, a, b)? else {
                return Ok(None);
            };
            let x = x.get();
            match op {
                Op::Pow => {
                    check_pow(&x, shift, None)?;
                    Mpz::uiop(sys::mpz_pow_ui, &x, shift)
                }
                Op::Shl => {
                    check_shl(&x, shift)?;
                    Mpz::uiop(sys::mpz_mul_2exp, &x, shift)
                }
                _ => Mpz::uiop(sys::mpz_fdiv_q_2exp, &x, shift),
            }
        }
        (
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::BitOr | Op::BitAnd | Op::BitXor,
            Some(b),
        ) => {
            let x = operand_or_error(host, a)?;
            let y = operand_or_error(host, b)?;
            let (x, y) = (x.get(), y.get());
            let f: sys::BinOp = match op {
                Op::Add => sys::mpz_add,
                Op::Sub => sys::mpz_sub,
                Op::Mul => sys::mpz_mul,
                Op::Div => sys::mpz_tdiv_q,
                Op::Mod => sys::mpz_mod,
                Op::BitOr => sys::mpz_ior,
                Op::BitAnd => sys::mpz_and,
                _ => sys::mpz_xor,
            };
            match op {
                Op::Div | Op::Mod if y.sgn() == 0 => {
                    let msg = if op == Op::Div {
                        "Division by zero"
                    } else {
                        "Modulo by zero"
                    };
                    return Err(Thrown::error("DivisionByZeroError", msg));
                }
                Op::Mul => check_mul(&x, &y, None)?,
                Op::Div | Op::Mod => {}
                _ => check_grow(&x, &y, None, "Number")?,
            }
            Mpz::binop(f, &x, &y)
        }
        _ => return Ok(None),
    };
    gmp_new(host, r).map(Some)
}

/// shift_operator_helper's operands: the number and the shift / exponent.
fn shift_operands(
    host: &mut dyn Host,
    op: Op,
    a: &Value,
    b: &Value,
) -> Result<Option<(Arg, u64)>, Thrown> {
    let sigil = match op {
        Op::Pow => "**",
        Op::Shl => "<<",
        _ => ">>",
    };
    let type_name = crate::zend::zpp::zval_type_name;
    let unsupported = |host: &dyn Host| {
        Thrown::type_error(format!(
            "Unsupported operand types: {} {sigil} {}",
            type_name(host, a),
            type_name(host, b)
        ))
    };
    let shift = match deref(b) {
        Value::Int(n) => n,
        v @ Value::Float(_) => crate::zend::zpp::zval_get_long(host, &v)?,
        Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
            Some(conv::Num::Int(n)) => n,
            _ => {
                return Err(Thrown::error(
                    "ValueError",
                    "Number is not an integer string",
                ))
            }
        },
        // zval_get_long of a GMP object: its cast.
        other => match gmp_object(host, &other) {
            Some(h) => gmp_num(&h).0.borrow().get_si(),
            None => return Err(unsupported(host)),
        },
    };
    if shift < 0 {
        let what = if op == Op::Pow { "Exponent" } else { "Shift" };
        return Err(Thrown::error(
            "ValueError",
            format!("{what} must be greater than or equal to 0"),
        ));
    }
    // A failed conversion of the number is the operand-type error, with
    // the conversion's exception as its previous one.
    let x = match operand(host, a) {
        Ok(Some(x)) => x,
        Ok(None) => return Err(unsupported(host)),
        Err(e) => {
            let t = unsupported(host);
            return Err(host.chain_thrown(t, e));
        }
    };
    Ok(Some((x, shift as u64)))
}

#[cfg(test)]
mod tests;
