//! PHP operator semantics (arithmetic, comparison, bitwise, concat) and the
//! value helpers the VM, the compiler's constant folding and the builtins
//! share (`deref`, `(array)` casts, `++`/`--`). Operators can raise
//! `Throwable`s (`DivisionByZeroError`, `TypeError`). All rules verified
//! against php 8.5.8 (see `docs/design/php-semantics.md`).

use std::cmp::Ordering;

use crate::conv::{self, Num};
use crate::Thrown;
use crate::{PhpArray, PhpStr, Value};

/// Deref a possible `Value::Ref` to the underlying value (clone).
#[inline]
pub fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        other => other.clone(),
    }
}

/// A copy of an array element as php makes one (zend_array_dup_element,
/// and the builtins that build arrays from other arrays' elements): a
/// reference nothing else holds (refcount 1) becomes its value.
#[inline]
pub fn copy_elem(v: &Value) -> Value {
    match v {
        Value::Ref(r) if std::rc::Rc::strong_count(r) == 1 => r.borrow().clone(),
        other => other.clone(),
    }
}

/// `(array)` cast: arrays pass through, null → empty, scalars → single element.
pub fn cast_to_array(v: Value) -> Value {
    match deref(&v) {
        Value::Array(a) => Value::Array(a),
        Value::Null => Value::Array(PhpArray::new()),
        // `(array)$object`: public properties become array elements keyed by
        // name (visibility name-mangling for private/protected is not modelled).
        Value::Object(h) => {
            let mut a = PhpArray::new();
            for (name, val) in h.borrow().props() {
                a.insert(crate::ArrayKey::from_php_str(name), deref(val));
            }
            Value::Array(a)
        }
        scalar => {
            let mut a = PhpArray::new();
            a.push(scalar);
            Value::Array(a)
        }
    }
}

/// PHP `++`/`--`: null++ => 1, null-- => null, numeric steps numerically;
/// non-numeric string increment (Perl-style) deferred (returns unchanged).
pub fn inc_dec_value(v: &Value, delta: i64) -> Value {
    match deref(v) {
        Value::Null => {
            if delta > 0 {
                Value::Int(1)
            } else {
                Value::Null
            }
        }
        Value::Int(n) => match n.checked_add(delta) {
            Some(r) => Value::Int(r),
            None => Value::Float(n as f64 + delta as f64),
        },
        Value::Float(f) => Value::Float(f + delta as f64),
        Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
            Some(conv::Num::Int(n)) => match n.checked_add(delta) {
                Some(r) => Value::Int(r),
                None => Value::Float(n as f64 + delta as f64),
            },
            Some(conv::Num::Float(f)) => Value::Float(f + delta as f64),
            None => Value::Str(s),
        },
        other => other,
    }
}

/// Coerce an operand to a number for arithmetic without an engine: only
/// operands php converts silently. Anything that would warn or throw (a
/// leading-numeric or non-numeric string, an array, a resource, an object)
/// is an error; the VM's slow path (`compare.rs`) handles those with php's
/// diagnostics, and constant folding leaves them to run time.
fn arith_num(v: &Value, op: &str) -> Result<Num, Thrown> {
    match v {
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) => Ok(conv::to_number(v).0),
        Value::Str(s) if s.as_bytes().first().is_none_or(|&c| c <= b'9') => {
            match conv::numeric_string(s.as_bytes()) {
                Some(num) => Ok(num),
                None => Err(unsupported(op)),
            }
        }
        Value::Ref(r) => arith_num(&r.borrow(), op),
        _ => Err(unsupported(op)),
    }
}

fn unsupported(op: &str) -> Thrown {
    Thrown::type_error(format!("Unsupported operand types for {op}"))
}

/// An operand as an int for `%`, `<<`, `>>`, `|`, `&`, `^` without an
/// engine: only conversions php does silently (see [`arith_num`]).
fn long_num(v: &Value, op: &str) -> Result<i64, Thrown> {
    let f = match arith_num(v, op)? {
        Num::Int(n) => return Ok(n),
        Num::Float(f) => f,
    };
    // zend_dval_to_lval_safe would warn or deprecate.
    if !f.is_finite()
        || f.fract() != 0.0
        || f >= 9_223_372_036_854_775_808.0
        || f < -9_223_372_036_854_775_808.0
    {
        return Err(unsupported(op));
    }
    Ok(f as i64)
}

/// `+` `-` `*` on two numbers, with int-overflow → float promotion.
fn arith_add_sub_mul(
    a: Num,
    b: Num,
    f_int: fn(i64, i64) -> Option<i64>,
    f_flt: fn(f64, f64) -> f64,
) -> Value {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => match f_int(x, y) {
            Some(n) => Value::Int(n),
            None => Value::Float(f_flt(x as f64, y as f64)), // overflow → float
        },
        _ => Value::Float(f_flt(a.as_f64(), b.as_f64())),
    }
}

pub fn add(a: &Value, b: &Value) -> Result<Value, Thrown> {
    // array + array = union (left wins); only defined for arrays.
    if let (Value::Array(x), Value::Array(y)) = (&deref(a), &deref(b)) {
        let mut out = x.clone();
        for (k, v) in y.iter() {
            if out.get(&k).is_none() {
                out.insert(k.clone(), v.clone());
            }
        }
        return Ok(Value::Array(out));
    }
    let (x, y) = (arith_num(a, "+")?, arith_num(b, "+")?);
    Ok(arith_add_sub_mul(x, y, i64::checked_add, |p, q| p + q))
}

pub fn sub(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let (x, y) = (arith_num(a, "-")?, arith_num(b, "-")?);
    Ok(arith_add_sub_mul(x, y, i64::checked_sub, |p, q| p - q))
}

pub fn mul(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let (x, y) = (arith_num(a, "*")?, arith_num(b, "*")?);
    Ok(arith_add_sub_mul(x, y, i64::checked_mul, |p, q| p * q))
}

pub fn div(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let (x, y) = (arith_num(a, "/")?, arith_num(b, "/")?);
    if y.as_f64() == 0.0 {
        return Err(Thrown::error("DivisionByZeroError", "Division by zero"));
    }
    match (x, y) {
        // i64::MIN / -1 overflows: a float, as php's div_function_base.
        (Num::Int(p), Num::Int(q)) if q != 0 && p.checked_rem(q) == Some(0) => {
            Ok(match p.checked_div(q) {
                Some(n) => Value::Int(n),
                None => Value::Float(p as f64 / q as f64),
            })
        }
        _ => Ok(Value::Float(x.as_f64() / y.as_f64())),
    }
}

pub fn modulo(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let p = long_num(a, "%")?;
    let q = long_num(b, "%")?;
    long_binop(BitOp::Mod, p, q)
}

/// `**` without an engine. `0 ** negative` is an error here: php deprecates
/// it at run time, so constant folding must leave it alone.
pub fn pow(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let x = arith_num(a, "**")?;
    let y = arith_num(b, "**")?;
    if pow_base0_neg_exp(x, y) {
        return Err(unsupported("**"));
    }
    Ok(pow_num(x, y))
}

/// Both operands of `op` as numbers, without an engine (see [`arith_num`]).
pub fn arith_pair(a: &Value, b: &Value, op: &str) -> Result<(Num, Num), Thrown> {
    Ok((arith_num(a, op)?, arith_num(b, op)?))
}

/// `safe_pow`'s condition for "Power of base 0 and negative exponent is
/// deprecated".
pub fn pow_base0_neg_exp(x: Num, y: Num) -> bool {
    match (x, y) {
        (Num::Int(_), Num::Int(e)) if e >= 0 => false,
        _ => x.as_f64() == 0.0 && y.as_f64() < 0.0,
    }
}

/// pow_function_base on two numbers: an int power by squaring while it fits,
/// else a float.
pub fn pow_num(x: Num, y: Num) -> Value {
    if let (Num::Int(base), Num::Int(exp)) = (x, y) {
        if exp >= 0 {
            let (mut l1, mut l2, mut i) = (1i64, base, exp);
            if i == 0 {
                return Value::Int(1);
            } else if l2 == 0 {
                return Value::Int(0);
            }
            while i >= 1 {
                if i % 2 != 0 {
                    i -= 1;
                    match l1.checked_mul(l2) {
                        Some(v) => l1 = v,
                        None => {
                            let d = l1 as f64 * l2 as f64;
                            return Value::Float(d * (l2 as f64).powf(i as f64));
                        }
                    }
                } else {
                    i /= 2;
                    match l2.checked_mul(l2) {
                        Some(v) => l2 = v,
                        None => {
                            let d = l2 as f64 * l2 as f64;
                            return Value::Float(l1 as f64 * d.powf(i as f64));
                        }
                    }
                }
            }
            return Value::Int(l1);
        }
    }
    Value::Float(x.as_f64().powf(y.as_f64()))
}

/// `| & ^ << >> %` on two plain values (no diagnostics: see [`arith_num`]).
/// Two strings combine bytewise for `| & ^`.
pub fn bitwise(a: &Value, b: &Value, op: BitOp) -> Result<Value, Thrown> {
    let (a, b) = (deref(a), deref(b));
    if let (Value::Str(x), Value::Str(y)) = (&a, &b) {
        if let Some(r) = string_bitwise(x.as_bytes(), y.as_bytes(), &op) {
            return Ok(Value::str(r));
        }
    }
    let sigil = op.sigil();
    long_binop(op, long_num(&a, sigil)?, long_num(&b, sigil)?)
}

/// `bitwise_or/and/xor_function` on two strings: `|` keeps the longer
/// string's tail, `&` and `^` stop at the shorter one. `None` for shifts.
pub fn string_bitwise(x: &[u8], y: &[u8], op: &BitOp) -> Option<Vec<u8>> {
    let f: fn(u8, u8) -> u8 = match op {
        BitOp::Or => |p, q| p | q,
        BitOp::And => |p, q| p & q,
        BitOp::Xor => |p, q| p ^ q,
        _ => return None,
    };
    let (longer, shorter) = if x.len() >= y.len() { (x, y) } else { (y, x) };
    let mut out: Vec<u8> = shorter.iter().zip(longer).map(|(&p, &q)| f(q, p)).collect();
    if matches!(op, BitOp::Or) {
        out.extend_from_slice(&longer[shorter.len()..]);
    }
    Some(out)
}

/// `% << >> | & ^` on two ints (mod_function, shift_left_function, …).
pub fn long_binop(op: BitOp, a: i64, b: i64) -> Result<Value, Thrown> {
    Ok(Value::Int(match op {
        BitOp::Mod if b == 0 => return Err(Thrown::error("DivisionByZeroError", "Modulo by zero")),
        BitOp::Mod if b == -1 => 0,
        BitOp::Mod => a % b,
        BitOp::Shl | BitOp::Shr if b < 0 => {
            return Err(Thrown::error(
                "ArithmeticError",
                "Bit shift by negative number",
            ))
        }
        BitOp::Shl if b >= 64 => 0,
        BitOp::Shl => ((a as u64) << b) as i64,
        BitOp::Shr if b >= 64 => {
            if a < 0 {
                -1
            } else {
                0
            }
        }
        BitOp::Shr => a >> b,
        BitOp::And => a & b,
        BitOp::Or => a | b,
        BitOp::Xor => a ^ b,
    }))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BitOp {
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Mod,
}

impl BitOp {
    pub fn sigil(self) -> &'static str {
        match self {
            BitOp::And => "&",
            BitOp::Or => "|",
            BitOp::Xor => "^",
            BitOp::Shl => "<<",
            BitOp::Shr => ">>",
            BitOp::Mod => "%",
        }
    }
}

/// `~` on a plain value: ints, whole floats in range, strings bytewise;
/// the rest (which php warns about or rejects) is an error.
pub fn bitwise_not(a: &Value) -> Result<Value, Thrown> {
    match deref(a) {
        Value::Int(n) => Ok(Value::Int(!n)),
        f @ Value::Float(_) => Ok(Value::Int(!long_num(&f, "~")?)),
        Value::Str(s) => Ok(Value::str(
            s.as_bytes().iter().map(|b| !b).collect::<Vec<u8>>(),
        )),
        _ => Err(unsupported("~")),
    }
}

pub fn negate(a: &Value) -> Result<Value, Thrown> {
    match arith_num(a, "-")? {
        Num::Int(n) => match n.checked_neg() {
            Some(v) => Ok(Value::Int(v)),
            None => Ok(Value::Float(-(n as f64))), // -i64::MIN → float
        },
        Num::Float(f) => Ok(Value::Float(-f)),
    }
}

pub fn concat(a: &Value, b: &Value) -> Value {
    // One buffer of the result's size, as php's concat allocates the result
    // once (no copies of the operands, no growth slack counted against
    // memory_limit).
    let (a, b) = (deref(a), deref(b));
    let (mut ia, mut ib) = ([0u8; 20], [0u8; 20]);
    let a = match &a {
        Value::Str(s) => std::borrow::Cow::Borrowed(s.as_bytes()),
        Value::Int(n) => std::borrow::Cow::Borrowed(conv::int_digits(*n, &mut ia)),
        v => std::borrow::Cow::Owned(conv::to_string_bytes(v)),
    };
    let b = match &b {
        Value::Str(s) => std::borrow::Cow::Borrowed(s.as_bytes()),
        Value::Int(n) => std::borrow::Cow::Borrowed(conv::int_digits(*n, &mut ib)),
        v => std::borrow::Cow::Owned(conv::to_string_bytes(v)),
    };
    let len = a.len() + b.len();
    // php's zend_string_alloc for the result: past the hard limit the VM
    // stops at its next check, so don't allocate.
    if !crate::mem::alloc_check(crate::mem::zstr_size(len)) {
        return Value::Str(PhpStr::from_bytes(Vec::new()));
    }
    Value::Str(PhpStr::concat(&a, &b))
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// The type an object's `cast_object` handler converts it to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CastTarget {
    Bool,
    Int,
    Float,
    Str,
    Array,
    Resource,
    /// `_IS_NUMBER`: an int or a float (arithmetic operands, `++`).
    Number,
}

/// The engine services object comparison needs: class facts, `cast_object`
/// (`__toString`, internal casts), internal `compare` handlers, and notices.
/// [`NoCx`] is the context-free fallback (no user code can run).
pub trait CompareCx {
    /// Whether the object's class is an enum (enums are uncomparable).
    fn is_enum(&self, _class: crate::ClassId) -> bool {
        false
    }
    /// The class's declared (default) property names, in slot order; `None`
    /// when unknown (every property is then treated as declared).
    fn declared_props(&self, _class: crate::ClassId) -> Option<Vec<PhpStr>> {
        None
    }
    /// Engine-internal state of a built-in class, invisible to comparison
    /// (php keeps it outside the property table).
    fn is_internal_prop(&self, _class: crate::ClassId, _name: &PhpStr) -> bool {
        false
    }
    /// The object whose properties a comparison reads (php's
    /// get_properties: a lazy object initializes, a proxy forwards).
    fn props_object(&mut self, obj: &crate::ObjectHandle) -> Result<crate::ObjectHandle, Thrown> {
        Ok(obj.clone())
    }
    /// The class name (for the conversion notice).
    fn class_name(&self, _class: crate::ClassId) -> Vec<u8> {
        b"object".to_vec()
    }
    /// `cast_object`: `Ok(None)` is FAILURE.
    fn cast_object(
        &mut self,
        _obj: &crate::ObjectHandle,
        target: CastTarget,
    ) -> Result<Option<Value>, Thrown> {
        Ok(match target {
            CastTarget::Bool => Some(Value::Bool(true)),
            _ => None,
        })
    }
    /// An internal class's own `compare` handler (GMP, DateTimeInterface),
    /// called with the operands in order when `obj` (one of them) has one.
    fn compare_handler(
        &mut self,
        _obj: &crate::ObjectHandle,
        _a: &Value,
        _b: &Value,
    ) -> Option<Result<i32, Thrown>> {
        None
    }
    /// Raise an `E_NOTICE`.
    fn notice(&mut self, _msg: &str) {}
}

/// A comparison context with no engine behind it.
pub struct NoCx;
impl CompareCx for NoCx {}

thread_local! {
    /// Objects currently being compared as the left operand (php protects
    /// `o1` against recursion with `Z_PROTECT_RECURSION`).
    static CMP_GUARD: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn nesting_error() -> Thrown {
    Thrown::error("Error", "Nesting level too deep - recursive dependency?")
}

struct Guard(usize);
impl Guard {
    fn enter(p: usize) -> Result<Guard, Thrown> {
        CMP_GUARD.with(|g| {
            let mut g = g.borrow_mut();
            if g.contains(&p) {
                Err(nesting_error())
            } else {
                g.push(p);
                Ok(Guard(p))
            }
        })
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        CMP_GUARD.with(|g| {
            let mut g = g.borrow_mut();
            if let Some(i) = g.iter().rposition(|&x| x == self.0) {
                g.remove(i);
            }
        });
    }
}

fn ord(n: i32) -> Ordering {
    n.cmp(&0)
}

/// PHP loose comparison (`<`, `<=`, `<=>`, and `==` via `== Equal`) without an
/// engine: objects can't run `__toString`, and errors collapse to "uncomparable".
/// Uncomparable pairs report `Greater` (php's `ZEND_UNCOMPARABLE` = 1); callers
/// evaluate `a > b` as `b < a`.
pub fn loose_compare(a: &Value, b: &Value) -> Ordering {
    compare(a, b, &mut NoCx).unwrap_or(Ordering::Greater)
}

/// `zend_compare`, ported: verified against php 8.5.8.
pub fn compare(a: &Value, b: &Value, cx: &mut dyn CompareCx) -> Result<Ordering, Thrown> {
    let a = deref(a);
    let b = deref(b);
    use Value::*;
    Ok(match (&a, &b) {
        (Int(x), Int(y)) => x.cmp(y),
        (Int(_) | Float(_), Int(_) | Float(_)) => {
            cmp_num(conv::to_number(&a).0, conv::to_number(&b).0)
        }
        (Array(x), Array(y)) => return compare_arrays(x, y, cx),
        (Null, Null) | (Null, Bool(false)) | (Bool(false), Null) => Ordering::Equal,
        (Bool(x), Bool(y)) => x.cmp(y),
        (Null, Bool(true)) => Ordering::Less,
        (Bool(true), Null) => Ordering::Greater,
        (Str(x), Str(y)) => smart_strcmp(x.as_bytes(), y.as_bytes()),
        (Null, Str(s)) => {
            if s.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        }
        (Str(s), Null) => {
            if s.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        }
        (Int(_) | Float(_), Str(s)) => num_vs_str(&a, s.as_bytes()),
        (Str(s), Int(_) | Float(_)) => num_vs_str(&b, s.as_bytes()).reverse_unless_nan(&b),
        (Object(_) | Closure(_), Null) => Ordering::Greater,
        (Null, Object(_) | Closure(_)) => Ordering::Less,
        _ if is_obj(&a) || is_obj(&b) => return compare_object_operand(&a, &b, cx),
        // NaN: "NAN used be cast to TRUE so handle this manually".
        _ if is_nan(&a) || is_nan(&b) => {
            if lt_true(&a) {
                Ordering::Less
            } else if matches!(a, Bool(true)) || matches!(b, Bool(true)) {
                Ordering::Equal
            } else if lt_true(&b) {
                Ordering::Greater
            } else if matches!(a, Array(_)) {
                // An array stays an array after the conversion: greater.
                Ordering::Greater
            } else if matches!(b, Array(_)) {
                Ordering::Less
            } else {
                cmp_num(conv::to_number(&a).0, conv::to_number(&b).0)
            }
        }
        _ if lt_true(&a) => {
            if conv::to_bool(&b) {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        }
        (Bool(true), _) => {
            if conv::to_bool(&b) {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        }
        _ if lt_true(&b) => {
            if conv::to_bool(&a) {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        (_, Bool(true)) => {
            if conv::to_bool(&a) {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        }
        (Array(_), _) => Ordering::Greater,
        (_, Array(_)) => Ordering::Less,
        // remaining scalars (resources, …): compared as numbers.
        _ => cmp_num(conv::to_number(&a).0, conv::to_number(&b).0),
    })
}

trait ReverseUnlessNan {
    fn reverse_unless_nan(self, num: &Value) -> Ordering;
}
impl ReverseUnlessNan for Ordering {
    /// `-compare_*_to_string(..)`, except NaN which is uncomparable both ways.
    fn reverse_unless_nan(self, num: &Value) -> Ordering {
        if is_nan(num) {
            Ordering::Greater
        } else {
            self.reverse()
        }
    }
}

fn is_obj(v: &Value) -> bool {
    matches!(v, Value::Object(_) | Value::Closure(_))
}

fn is_nan(v: &Value) -> bool {
    matches!(v, Value::Float(f) if f.is_nan())
}

/// `Z_TYPE_P(v) < IS_TRUE`: null or false.
fn lt_true(v: &Value) -> bool {
    matches!(v, Value::Null | Value::Bool(false))
}

/// `compare_long_to_string` / `compare_double_to_string` (number on the left).
fn num_vs_str(n: &Value, s: &[u8]) -> Ordering {
    if is_nan(n) {
        return Ordering::Greater;
    }
    match conv::numeric_string(s) {
        Some(m) => cmp_num(conv::to_number(n).0, m),
        None => cmp_bytes(&conv::to_string_bytes(n), s),
    }
}

/// `zendi_smart_strcmp`: numeric strings compare as numbers.
fn smart_strcmp(x: &[u8], y: &[u8]) -> Ordering {
    match (conv::numeric_string(x), conv::numeric_string(y)) {
        (Some(Num::Int(a)), Some(Num::Int(b))) => a.cmp(&b),
        // An integer string past the int range against an int: its side.
        (Some(nx @ Num::Int(_)), Some(ny)) => match int_overflow(y, ny) {
            0 => cmp_num(nx, ny),
            o => (-o).cmp(&0),
        },
        (Some(nx), Some(ny @ Num::Int(_))) => match int_overflow(x, nx) {
            0 => cmp_num(nx, ny),
            o => o.cmp(&0),
        },
        // Equal as doubles but overflowed integers (or infinities): the
        // double compare lost the difference.
        (Some(nx), Some(ny)) if nx.as_f64() == ny.as_f64() => {
            let o1 = int_overflow(x, nx);
            if (o1 != 0 && o1 == int_overflow(y, ny)) || !nx.as_f64().is_finite() {
                cmp_bytes(x, y)
            } else {
                Ordering::Equal
            }
        }
        (Some(nx), Some(ny)) => cmp_num(nx, ny),
        _ => cmp_bytes(x, y),
    }
}

/// `is_numeric_string_ex`'s `oflow`: 1 or -1 when `s` (parsed as the float
/// `n`) has 20 or more integer digits after leading zeros, or is an integer
/// string past the int range; else 0.
fn int_overflow(s: &[u8], n: Num) -> i8 {
    let Num::Float(f) = n else { return 0 };
    let ws = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c');
    let mut it = s.iter().skip_while(|c| ws(c)).peekable();
    if matches!(it.peek(), Some(b'-' | b'+')) {
        it.next();
    }
    while it.peek() == Some(&&b'0') {
        it.next();
    }
    let sign = if f < 0.0 { -1 } else { 1 };
    let mut digits = 0;
    while let Some(c) = it.peek() {
        if !c.is_ascii_digit() {
            break;
        }
        it.next();
        digits += 1;
        // MAX_LENGTH_OF_LONG digits: php stops and parses a double.
        if digits >= 20 {
            return sign;
        }
    }
    // Fewer digits followed by '.' or 'e' is a plain double.
    if digits == 0 || !it.all(ws) {
        return 0;
    }
    sign
}

thread_local! {
    /// Arrays being compared as the left operand (zend_hash_compare's
    /// GC_PROTECT_RECURSION).
    static COMPARING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
    /// zend_call_stack_overflowed, installed by the engine.
    static STACK_OVERFLOWED: std::cell::Cell<Option<fn() -> bool>> = const { std::cell::Cell::new(None) };
}

/// Install the native stack limit check comparisons start with.
pub fn set_stack_overflow_check(f: fn() -> bool) {
    STACK_OVERFLOWED.with(|c| c.set(Some(f)));
}

fn stack_overflowed() -> bool {
    STACK_OVERFLOWED.with(|c| c.get()).is_some_and(|f| f())
}

/// zend_hash_compare's checks before comparing `x` with another array:
/// the stack limit, then `x` met again inside itself.
fn hash_compare_guard(x: &crate::PhpArray) -> Result<(), Thrown> {
    if stack_overflowed() {
        return Err(Thrown::error(
            "Error",
            "Maximum call stack size reached during comparison",
        ));
    }
    let p = x.data_ptr();
    if COMPARING.with(|c| c.borrow().contains(&p)) {
        return Err(nesting_error());
    }
    Ok(())
}

/// `zend_hash_compare(.., ordered=0)` over two arrays.
fn compare_arrays(
    x: &crate::PhpArray,
    y: &crate::PhpArray,
    cx: &mut dyn CompareCx,
) -> Result<Ordering, Thrown> {
    // zend_hash_compare: the same table is equal to itself; one met again
    // inside itself is a recursive dependency.
    if x.data_ptr() == y.data_ptr() {
        return Ok(Ordering::Equal);
    }
    // Checked before the sizes, as zend_hash_compare does.
    hash_compare_guard(x)?;
    let p = x.data_ptr();
    if x.len() != y.len() {
        return Ok(x.len().cmp(&y.len()));
    }
    COMPARING.with(|c| c.borrow_mut().push(p));
    let r = (|| {
        for (k, xv) in x.iter() {
            match y.get(&k) {
                Some(yv) => match compare(xv, yv, cx)? {
                    Ordering::Equal => continue,
                    other => return Ok(other),
                },
                None => return Ok(Ordering::Greater), // uncomparable
            }
        }
        Ok(Ordering::Equal)
    })();
    COMPARING.with(|c| c.borrow_mut().pop());
    r
}

/// The object branch of `zend_compare`: at least one operand is an object.
fn compare_object_operand(
    a: &Value,
    b: &Value,
    cx: &mut dyn CompareCx,
) -> Result<Ordering, Thrown> {
    use Value::*;
    let (object, other, object_lhs) = if is_obj(a) {
        (a, b, true)
    } else {
        (b, a, false)
    };
    if is_obj(other) && same_object(object, other) {
        return Ok(Ordering::Equal);
    }
    if let Bool(_) = other {
        let casted = match object {
            Object(h) => cx.cast_object(h, CastTarget::Bool)?,
            _ => Some(Bool(true)),
        };
        let Some(casted) = casted else {
            return Ok(side(object_lhs));
        };
        return if object_lhs {
            compare(&casted, other, cx)
        } else {
            compare(other, &casted, cx)
        };
    }
    // Z_OBJ_HANDLER(object, compare)(a, b)
    let Object(h) = object else {
        // Closures: zend_closure_compare. Against another object (which is
        // op2 here) its fallback, zend_std_compare_objects, finds them
        // uncomparable; against scalars the std object path applies.
        return if is_obj(other) {
            Ok(Ordering::Greater)
        } else {
            std_compare_scalar(None, b"Closure", object_lhs, other, cx)
        };
    };
    if let Some(r) = cx.compare_handler(h, a, b) {
        return Ok(ord(r?));
    }
    let class = h.borrow().class;
    if cx.is_enum(class) {
        // zend_objects_not_comparable (identical instances returned 0 above).
        return Ok(Ordering::Greater);
    }
    match (a, b) {
        (Object(x), Object(y)) => std_compare_objects(x, y, cx),
        (Object(_), Closure(_)) | (Closure(_), Object(_)) => Ok(Ordering::Greater),
        _ => {
            let name = cx.class_name(class);
            std_compare_scalar(Some(h), &name, object_lhs, other, cx)
        }
    }
}

fn side(object_lhs: bool) -> Ordering {
    if object_lhs {
        Ordering::Greater
    } else {
        Ordering::Less
    }
}

fn same_object(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => std::rc::Rc::ptr_eq(x, y),
        (Value::Closure(x), Value::Closure(y)) => {
            std::rc::Rc::ptr_eq(x, y) || fake_closures_equal(x, y)
        }
        _ => false,
    }
}

/// php's `zend_closure_compare` for two distinct closure objects: only
/// first-class-callable ("fake") closures over the same function, `$this` and
/// scope are equal.
fn fake_closures_equal(x: &crate::ClosureData, y: &crate::ClosureData) -> bool {
    let (Some(wx), Some(wy)) = (&x.wrapped, &y.wrapped) else {
        return false;
    };
    let same_this = match (&x.bound_this, &y.bound_this) {
        (None, None) => true,
        (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
        _ => false,
    };
    same_this && x.bound_class == y.bound_class && callables_equal(wx, wy)
}

fn callables_equal(a: &Value, b: &Value) -> bool {
    let a = deref(a);
    let b = deref(b);
    match (&a, &b) {
        (Value::Str(x), Value::Str(y)) => x.as_bytes().eq_ignore_ascii_case(y.as_bytes()),
        (Value::Array(x), Value::Array(y)) if x.len() == 2 && y.len() == 2 => {
            let xs: Vec<Value> = x.values().map(deref).collect();
            let ys: Vec<Value> = y.values().map(deref).collect();
            let target = match (&xs[0], &ys[0]) {
                (Value::Object(p), Value::Object(q)) => std::rc::Rc::ptr_eq(p, q),
                (Value::Str(p), Value::Str(q)) => p.as_bytes().eq_ignore_ascii_case(q.as_bytes()),
                _ => false,
            };
            target && callables_equal(&xs[1], &ys[1])
        }
        (Value::Closure(p), Value::Closure(q)) => std::rc::Rc::ptr_eq(p, q),
        // `$invokable(...)`: the object's __invoke.
        (Value::Object(p), Value::Object(q)) => std::rc::Rc::ptr_eq(p, q),
        (Value::Object(o), Value::Array(arr)) | (Value::Array(arr), Value::Object(o))
            if arr.len() == 2 =>
        {
            let v: Vec<Value> = arr.values().map(deref).collect();
            matches!(&v[0], Value::Object(p) if std::rc::Rc::ptr_eq(p, o))
                && matches!(&v[1], Value::Str(m) if m.as_bytes().eq_ignore_ascii_case(b"__invoke"))
        }
        _ => false,
    }
}

/// zend_std_compare_objects with one non-object operand: cast the object to
/// the other operand's type.
fn std_compare_scalar(
    obj: Option<&crate::ObjectHandle>,
    class_name: &[u8],
    object_lhs: bool,
    value: &Value,
    cx: &mut dyn CompareCx,
) -> Result<Ordering, Thrown> {
    let target = match value {
        Value::Int(_) => CastTarget::Int,
        Value::Float(_) => CastTarget::Float,
        Value::Str(_) => CastTarget::Str,
        Value::Array(_) => CastTarget::Array,
        Value::Bool(_) => CastTarget::Bool,
        _ => CastTarget::Resource,
    };
    let casted = match obj {
        Some(h) => cx.cast_object(h, target)?,
        None => None,
    };
    let casted = match casted {
        Some(v) => v,
        None => match target {
            CastTarget::Int | CastTarget::Float => {
                let tn = if target == CastTarget::Int {
                    "int"
                } else {
                    "float"
                };
                cx.notice(&format!(
                    "Object of class {} could not be converted to {tn}",
                    String::from_utf8_lossy(class_name)
                ));
                if target == CastTarget::Int {
                    Value::Int(1)
                } else {
                    Value::Float(1.0)
                }
            }
            _ => return Ok(side(object_lhs)),
        },
    };
    if object_lhs {
        compare(&casted, value, cx)
    } else {
        compare(value, &casted, cx)
    }
}

/// zend_std_compare_objects for two objects.
fn std_compare_objects(
    x: &crate::ObjectHandle,
    y: &crate::ObjectHandle,
    cx: &mut dyn CompareCx,
) -> Result<Ordering, Thrown> {
    if stack_overflowed() {
        return Err(Thrown::error(
            "Error",
            "Maximum call stack size reached during object comparison",
        ));
    }
    let (xc, yc) = (x.borrow().class, y.borrow().class);
    if xc != yc {
        return Ok(Ordering::Greater); // different classes: uncomparable
    }
    // A lazy object initializes (a proxy compares its instance).
    let lazy;
    let (x, y, xc, yc) = if x.borrow().is_lazy() || y.borrow().is_lazy() {
        lazy = (cx.props_object(x)?, cx.props_object(y)?);
        let (xc, yc) = (lazy.0.borrow().class, lazy.1.borrow().class);
        (&lazy.0, &lazy.1, xc, yc)
    } else {
        (x, y, xc, yc)
    };
    // Snapshot the property tables: comparing nested values may run user code
    // (`__toString`) that touches these objects.
    if let Some(pairs) = slot_pairs(x, y, xc, cx) {
        // No dynamic properties: slot by slot.
        let _guard = Guard::enter(std::rc::Rc::as_ptr(x) as *const u8 as usize)?;
        for (p1, p2) in pairs {
            match (p1.is_undef(), p2.is_undef()) {
                (false, false) => match compare(&p1, &p2, cx)? {
                    Ordering::Equal => {}
                    o => return Ok(o),
                },
                (true, true) => {}
                _ => return Ok(Ordering::Greater),
            }
        }
        return Ok(Ordering::Equal);
    }
    let xp: Vec<(PhpStr, Value)> = x
        .borrow()
        .props()
        .iter()
        .filter(|(k, _)| !cx.is_internal_prop(xc, k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let yp: crate::PhpArray = {
        let mut a = crate::PhpArray::new();
        for (k, v) in y.borrow().props().iter() {
            if !cx.is_internal_prop(yc, k) {
                a.insert(crate::ArrayKey::Str(k.clone()), v.clone());
            }
        }
        a
    };
    let get_y = |k: &PhpStr| yp.get(&crate::ArrayKey::Str(k.clone())).cloned();
    let declared: Vec<PhpStr> = cx
        .declared_props(xc)
        .unwrap_or_else(|| xp.iter().map(|(k, _)| k.clone()).collect())
        .into_iter()
        .filter(|k| !cx.is_internal_prop(xc, k))
        .collect();
    let is_declared = |k: &PhpStr| declared.iter().any(|d| d == k);
    let x_dyn: Vec<&(PhpStr, Value)> = xp.iter().filter(|(k, _)| !is_declared(k)).collect();
    let y_dyn = y
        .borrow()
        .props()
        .keys()
        .filter(|k| !is_declared(k) && !cx.is_internal_prop(yc, k))
        .count();
    let get_x = |k: &PhpStr| xp.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let _guard = Guard::enter(std::rc::Rc::as_ptr(x) as *const u8 as usize)?;
    if x_dyn.is_empty() && y_dyn == 0 {
        // Slot comparison over the declared properties.
        for k in &declared {
            match (get_x(k), get_y(k)) {
                (Some(p1), Some(p2)) => match compare(&p1, &p2, cx)? {
                    Ordering::Equal => {}
                    o => return Ok(o),
                },
                (None, None) => {}
                _ => return Ok(Ordering::Greater),
            }
        }
        return Ok(Ordering::Equal);
    }
    // zend_compare_symbol_tables: declared slots (incl. UNDEF) + dynamic props.
    let (c1, c2) = (declared.len() + x_dyn.len(), declared.len() + y_dyn);
    if c1 != c2 {
        return Ok(c1.cmp(&c2));
    }
    for k in &declared {
        match (get_x(k), get_y(k)) {
            (Some(p1), Some(p2)) => match compare(&p1, &p2, cx)? {
                Ordering::Equal => {}
                o => return Ok(o),
            },
            (None, None) => {}
            (None, Some(_)) => return Ok(Ordering::Less),
            (Some(_), None) => return Ok(Ordering::Greater),
        }
    }
    for (k, p1) in x_dyn {
        match get_y(k) {
            Some(p2) => match compare(p1, &p2, cx)? {
                Ordering::Equal => {}
                o => return Ok(o),
            },
            None => return Ok(Ordering::Greater),
        }
    }
    Ok(Ordering::Equal)
}

/// The declared slots of two objects of class `class` side by side (empty
/// ones kept), when neither has dynamic properties.
fn slot_pairs(
    x: &crate::ObjectHandle,
    y: &crate::ObjectHandle,
    class: crate::ClassId,
    cx: &mut dyn CompareCx,
) -> Option<Vec<(Value, Value)>> {
    let (bx, by) = (x.borrow(), y.borrow());
    let (px, py) = (bx.props(), by.props());
    let n = px.pos_len();
    if px.has_dynamic() || py.has_dynamic() || py.pos_len() != n {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let ((k, v1), (_, v2)) = (px.get_index(i)?, py.get_index(i)?);
        if !cx.is_internal_prop(class, k) {
            out.push((v1.copy_slot(), v2.copy_slot()));
        }
    }
    Some(out)
}

fn cmp_num(a: Num, b: Num) -> Ordering {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => x.cmp(&y),
        _ => a
            .as_f64()
            .partial_cmp(&b.as_f64())
            // ZEND_THREEWAY_COMPARE: NaN is uncomparable (1).
            .unwrap_or(Ordering::Greater),
    }
}

fn cmp_bytes(a: &[u8], b: &[u8]) -> Ordering {
    a.cmp(b)
}

pub fn loose_eq(a: &Value, b: &Value) -> bool {
    loose_compare(a, b) == Ordering::Equal
}

/// PHP strict comparison (`===`): same type and value; identity for objects.
/// A comparison that throws (see [`identical`]) is false.
pub fn strict_eq(a: &Value, b: &Value) -> bool {
    identical(a, b).unwrap_or(false)
}

/// zend_is_identical: [`strict_eq`], with zend_hash_compare's errors for a
/// recursive array (`$a = [&$a]`) and for nesting past the stack limit.
pub fn identical(a: &Value, b: &Value) -> Result<bool, Thrown> {
    let a = deref(a);
    let b = deref(b);
    use Value::*;
    Ok(match (&a, &b) {
        (Null, Null) => true,
        (Bool(x), Bool(y)) => x == y,
        (Int(x), Int(y)) => x == y,
        (Float(x), Float(y)) => x == y, // NaN !== NaN, as PHP
        (Str(x), Str(y)) => x == y,
        (Array(x), Array(y)) => return identical_arrays(x, y),
        (Object(x), Object(y)) => std::rc::Rc::ptr_eq(x, y),
        (Closure(x), Closure(y)) => std::rc::Rc::ptr_eq(x, y),
        (Resource(x), Resource(y)) => x.id == y.id,
        _ => false,
    })
}

/// `zend_hash_compare(.., ordered=1)` with hash_zval_identical_function.
fn identical_arrays(x: &crate::PhpArray, y: &crate::PhpArray) -> Result<bool, Thrown> {
    if x.data_ptr() == y.data_ptr() {
        return Ok(true);
    }
    hash_compare_guard(x)?;
    if x.len() != y.len() {
        return Ok(false);
    }
    let p = x.data_ptr();
    COMPARING.with(|c| c.borrow_mut().push(p));
    let r = (|| {
        for ((xk, xv), (yk, yv)) in x.iter().zip(y.iter()) {
            if xk != yk || !identical(xv, yv)? {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    COMPARING.with(|c| c.borrow_mut().pop());
    r
}

/// The `<=>` result as a PHP int.
pub fn spaceship(a: &Value, b: &Value) -> Value {
    Value::Int(match loose_compare(a, b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    })
}
