//! PHP operator semantics (arithmetic, comparison, bitwise, concat).
//!
//! These can raise `Throwable`s (`DivisionByZeroError`, `TypeError`), so they
//! live here rather than in `rhp-values`. All rules verified against
//! php 8.5.8 (see `docs/design/php-semantics.md`).

use std::cmp::Ordering;

use rhp_values::conv::{self, Num};
use rhp_values::Thrown;
use rhp_values::{PhpStr, Value};

/// Coerce an operand to a number for arithmetic, raising `TypeError` for
/// unsupported types and non-numeric strings (PHP 8 behavior).
fn arith_num(v: &Value, op: &str) -> Result<Num, Thrown> {
    match v {
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) => Ok(conv::to_number(v).0),
        Value::Str(s) => match conv::scan_number(s.as_bytes()) {
            // leading-numeric strings are allowed (PHP warns; we accept)
            Some((num, _)) => Ok(num),
            None => Err(Thrown::type_error(format!(
                "Unsupported operand types: string {op} int"
            ))),
        },
        Value::Array(_) => Err(Thrown::type_error(format!(
            "Unsupported operand types: array {op} int"
        ))),
        Value::Object(_) | Value::Closure(_) => Err(Thrown::type_error(format!(
            "Unsupported operand types: object {op} int"
        ))),
        Value::Resource(r) => Ok(Num::Int(r.id as i64)),
        Value::Ref(r) => arith_num(&r.borrow(), op),
    }
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
    if let (Value::Array(x), Value::Array(y)) = (&crate::deref(a), &crate::deref(b)) {
        let mut out = x.clone();
        for (k, v) in y.iter() {
            if out.get(k).is_none() {
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
        (Num::Int(p), Num::Int(q)) if q != 0 && p % q == 0 => Ok(Value::Int(p / q)),
        _ => Ok(Value::Float(x.as_f64() / y.as_f64())),
    }
}

pub fn modulo(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let p = conv::to_int(&crate::deref(a));
    let q = conv::to_int(&crate::deref(b));
    if q == 0 {
        return Err(Thrown::error("DivisionByZeroError", "Modulo by zero"));
    }
    if q == -1 {
        return Ok(Value::Int(0)); // avoid i64::MIN % -1 overflow panic
    }
    Ok(Value::Int(p % q)) // Rust % has sign of dividend, like PHP
}

pub fn pow(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let x = arith_num(a, "**")?;
    let y = arith_num(b, "**")?;
    if let (Num::Int(base), Num::Int(exp)) = (x, y) {
        if exp >= 0 {
            // integer power with overflow → float
            let mut acc: i64 = 1;
            let mut overflow = false;
            for _ in 0..exp {
                match acc.checked_mul(base) {
                    Some(v) => acc = v,
                    None => {
                        overflow = true;
                        break;
                    }
                }
            }
            if !overflow {
                return Ok(Value::Int(acc));
            }
            return Ok(Value::Float((base as f64).powf(exp as f64)));
        }
    }
    Ok(Value::Float(x.as_f64().powf(y.as_f64())))
}

pub fn bitwise(a: &Value, b: &Value, op: BitOp) -> Value {
    let x = conv::to_int(&crate::deref(a));
    let y = conv::to_int(&crate::deref(b));
    Value::Int(match op {
        BitOp::And => x & y,
        BitOp::Or => x | y,
        BitOp::Xor => x ^ y,
        BitOp::Shl => x.wrapping_shl(y as u32),
        BitOp::Shr => x.wrapping_shr(y as u32),
    })
}

pub enum BitOp {
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

pub fn bitwise_not(a: &Value) -> Value {
    Value::Int(!conv::to_int(&crate::deref(a)))
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
    let mut bytes = conv::to_string_bytes(&crate::deref(a));
    bytes.extend_from_slice(&conv::to_string_bytes(&crate::deref(b)));
    Value::Str(PhpStr::from_bytes(bytes))
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// Predicate over objects (see [`set_not_comparable_hook`]).
pub type ObjectPredicate = fn(&rhp_values::ObjectHandle) -> bool;

thread_local! {
    /// Objects whose class installs `zend_objects_not_comparable` (the curl
    /// handle classes, registered by the extension that creates them).
    static NOT_COMPARABLE: std::cell::Cell<Option<ObjectPredicate>> =
        const { std::cell::Cell::new(None) };
}

/// Install the predicate telling which objects are not comparable: such an
/// object is `==` only to itself and uncomparable (1) to anything else.
pub fn set_not_comparable_hook(f: ObjectPredicate) {
    NOT_COMPARABLE.with(|c| c.set(Some(f)));
}

fn not_comparable(v: &Value) -> bool {
    match v {
        Value::Object(h) => NOT_COMPARABLE.with(|c| c.get()).is_some_and(|f| f(h)),
        _ => false,
    }
}

/// The type an object is cast to when it is compared with a non-object
/// (`cast_object` in zend_std_compare_objects).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CastTarget {
    Bool,
    Int,
    Float,
    Str,
    Array,
    Resource,
}

/// The engine services object comparison needs: class facts, `cast_object`
/// (`__toString`, internal casts), internal `compare` handlers, and notices.
/// [`NoCx`] is the context-free fallback (no user code can run).
pub trait CompareCx {
    /// Whether the object's class is an enum (enums are uncomparable).
    fn is_enum(&self, _class: rhp_values::ClassId) -> bool {
        false
    }
    /// The class's declared (default) property names, in slot order; `None`
    /// when unknown (every property is then treated as declared).
    fn declared_props(&self, _class: rhp_values::ClassId) -> Option<Vec<PhpStr>> {
        None
    }
    /// Engine-internal state of a built-in class, invisible to comparison
    /// (php keeps it outside the property table).
    fn is_internal_prop(&self, _class: rhp_values::ClassId, _name: &PhpStr) -> bool {
        false
    }
    /// The class name (for the conversion notice).
    fn class_name(&self, _class: rhp_values::ClassId) -> Vec<u8> {
        b"object".to_vec()
    }
    /// `cast_object`: `Ok(None)` is FAILURE.
    fn cast_object(
        &mut self,
        _obj: &rhp_values::ObjectHandle,
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
        _obj: &rhp_values::ObjectHandle,
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
    let a = crate::deref(a);
    let b = crate::deref(b);
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
        (Some(nx), Some(ny)) => cmp_num(nx, ny),
        _ => cmp_bytes(x, y),
    }
}

/// `zend_hash_compare(.., ordered=0)` over two arrays.
fn compare_arrays(
    x: &rhp_values::PhpArray,
    y: &rhp_values::PhpArray,
    cx: &mut dyn CompareCx,
) -> Result<Ordering, Thrown> {
    if x.len() != y.len() {
        return Ok(x.len().cmp(&y.len()));
    }
    for (k, xv) in x.iter() {
        match y.get(k) {
            Some(yv) => match compare(xv, yv, cx)? {
                Ordering::Equal => continue,
                other => return Ok(other),
            },
            None => return Ok(Ordering::Greater), // uncomparable
        }
    }
    Ok(Ordering::Equal)
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
    // zend_objects_not_comparable (the curl handle classes): uncomparable to
    // anything but itself, whichever side it is on.
    if not_comparable(object) || not_comparable(other) {
        return Ok(Ordering::Greater);
    }
    // Z_OBJ_HANDLER(object, compare)(a, b)
    let Object(h) = object else {
        // Closures: zend_closure_compare — uncomparable unless identical;
        // against scalars the std object path applies.
        return if is_obj(other) {
            if let Object(oh) = other {
                if let Some(r) = cx.compare_handler(oh, a, b) {
                    return Ok(ord(r?));
                }
                if cx.is_enum(oh.borrow().class) {
                    return Ok(Ordering::Greater);
                }
            }
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
fn fake_closures_equal(x: &rhp_values::ClosureData, y: &rhp_values::ClosureData) -> bool {
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
    let a = crate::deref(a);
    let b = crate::deref(b);
    match (&a, &b) {
        (Value::Str(x), Value::Str(y)) => x.as_bytes().eq_ignore_ascii_case(y.as_bytes()),
        (Value::Array(x), Value::Array(y)) if x.len() == 2 && y.len() == 2 => {
            let xs: Vec<Value> = x.iter().map(|(_, v)| crate::deref(v)).collect();
            let ys: Vec<Value> = y.iter().map(|(_, v)| crate::deref(v)).collect();
            let target = match (&xs[0], &ys[0]) {
                (Value::Object(p), Value::Object(q)) => std::rc::Rc::ptr_eq(p, q),
                (Value::Str(p), Value::Str(q)) => p.as_bytes().eq_ignore_ascii_case(q.as_bytes()),
                _ => false,
            };
            target && callables_equal(&xs[1], &ys[1])
        }
        (Value::Closure(p), Value::Closure(q)) => std::rc::Rc::ptr_eq(p, q),
        _ => false,
    }
}

/// zend_std_compare_objects with one non-object operand: cast the object to
/// the other operand's type.
fn std_compare_scalar(
    obj: Option<&rhp_values::ObjectHandle>,
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
    x: &rhp_values::ObjectHandle,
    y: &rhp_values::ObjectHandle,
    cx: &mut dyn CompareCx,
) -> Result<Ordering, Thrown> {
    let (xc, yc) = (x.borrow().class, y.borrow().class);
    if xc != yc {
        return Ok(Ordering::Greater); // different classes: uncomparable
    }
    // Snapshot the property tables: comparing nested values may run user code
    // (`__toString`) that touches these objects.
    let xp: Vec<(PhpStr, Value)> = x
        .borrow()
        .properties
        .iter()
        .filter(|(k, _)| !cx.is_internal_prop(xc, k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let yp: rhp_values::PhpArray = {
        let mut a = rhp_values::PhpArray::new();
        for (k, v) in y.borrow().properties.iter() {
            if !cx.is_internal_prop(yc, k) {
                a.insert(rhp_values::ArrayKey::Str(k.clone()), v.clone());
            }
        }
        a
    };
    let get_y = |k: &PhpStr| yp.get(&rhp_values::ArrayKey::Str(k.clone())).cloned();
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
        .properties
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
pub fn strict_eq(a: &Value, b: &Value) -> bool {
    let a = crate::deref(a);
    let b = crate::deref(b);
    use Value::*;
    match (&a, &b) {
        (Null, Null) => true,
        (Bool(x), Bool(y)) => x == y,
        (Int(x), Int(y)) => x == y,
        (Float(x), Float(y)) => x == y, // NaN !== NaN, as PHP
        (Str(x), Str(y)) => x == y,
        (Array(x), Array(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|((xk, xv), (yk, yv))| xk == yk && strict_eq(xv, yv))
        }
        (Object(x), Object(y)) => std::rc::Rc::ptr_eq(x, y),
        (Closure(x), Closure(y)) => std::rc::Rc::ptr_eq(x, y),
        (Resource(x), Resource(y)) => x.id == y.id,
        _ => false,
    }
}

/// The `<=>` result as a PHP int.
pub fn spaceship(a: &Value, b: &Value) -> Value {
    Value::Int(match loose_compare(a, b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    })
}
