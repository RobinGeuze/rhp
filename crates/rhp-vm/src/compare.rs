//! The VM side of loose comparison and of the object handlers that conversions
//! and operators consult: `cast_object` (an internal class's handler, else
//! `__toString`), internal `compare` handlers, `do_operation`, enum
//! detection, the conversion notice.

use rhp_builtins::zend::object_handlers::{CastTarget, Op};
use rhp_values::conv::Num;
use rhp_values::ops::{BitOp, CompareCx};
use rhp_values::{ClassId, ClassKind, ObjectHandle, PhpStr, Thrown, Value};

use crate::Vm;

impl Vm {
    /// The object handlers of `h`'s class (`None` for plain classes).
    pub(crate) fn handlers_of(
        &self,
        h: &ObjectHandle,
    ) -> Option<&'static rhp_builtins::zend::object_handlers::ObjectHandlers> {
        self.class(h.borrow().class).handlers
    }

    /// php's `cast_object` handler: the class's own, else
    /// `zend_std_cast_object_tostring` (`__toString` for strings, true for
    /// bool, failure otherwise). `Ok(None)` is FAILURE.
    pub(crate) fn cast_object_vm(
        &mut self,
        obj: &ObjectHandle,
        target: CastTarget,
    ) -> Result<Option<Value>, Thrown> {
        if let Some(f) = self.handlers_of(obj).and_then(|t| t.cast_object) {
            return f(self, obj, target);
        }
        self.std_cast(obj, target)
    }

    /// `zend_std_cast_object_tostring`.
    pub(crate) fn std_cast(
        &mut self,
        obj: &ObjectHandle,
        target: CastTarget,
    ) -> Result<Option<Value>, Thrown> {
        if target != CastTarget::Str {
            return Ok(rhp_builtins::zend::object_handlers::std_cast_non_string(
                target,
            ));
        }
        let cls = obj.borrow().class;
        match self.lookup_method(cls, b"__tostring") {
            Some(m) => {
                let r = self.call_sync(&m, Some(obj.clone()), cls, Vec::new())?;
                Ok(Some(Value::Str(PhpStr::from_bytes(
                    rhp_values::conv::to_string_bytes(&r),
                ))))
            }
            None => Ok(None),
        }
    }

    /// An internal class's `cast_object` for a conversion the engine does
    /// itself for other objects (`(int)`, `(float)`, `(bool)`, arithmetic
    /// operands): `None` unless `v` is an object whose class has one that
    /// succeeds.
    pub(crate) fn handler_cast(
        &mut self,
        v: &Value,
        target: CastTarget,
    ) -> Result<Option<Value>, Thrown> {
        let Value::Object(h) = v else {
            return Ok(None);
        };
        match self.handlers_of(h).and_then(|t| t.cast_object) {
            Some(f) => f(self, h, target),
            None => Ok(None),
        }
    }

    /// ZEND_TRY_BINARY_OBJECT_OPERATION: the `do_operation` of op1's class
    /// when op1 is an object with one, else op2's. `Ok(None)`: no handler,
    /// or it failed; the engine goes on with the normal operator.
    pub(crate) fn try_do_operation(
        &mut self,
        op: Op,
        a: &Value,
        b: &Value,
    ) -> Result<Option<Value>, Thrown> {
        let handler = |vm: &Vm, v: &Value| match v {
            Value::Object(h) => vm.handlers_of(h).and_then(|t| t.do_operation),
            _ => None,
        };
        match handler(self, a).or_else(|| handler(self, b)) {
            Some(f) => f(self, op, a, Some(b)),
            None => Ok(None),
        }
    }

    /// ZEND_TRY_BINARY_OP1/OP2_OBJECT_OPERATION: the `do_operation` of
    /// `side`'s class (one of the operands) on `a op b`.
    fn side_do_operation(
        &mut self,
        side: &Value,
        op: Op,
        a: &Value,
        b: &Value,
    ) -> Result<Option<Value>, Thrown> {
        let f = match side {
            Value::Object(h) => self.handlers_of(h).and_then(|t| t.do_operation),
            _ => None,
        };
        match f {
            Some(f) => f(self, op, a, Some(b)),
            None => Ok(None),
        }
    }

    /// `+ - * / **` past the number fast path (`add_function_slow` and its
    /// siblings): array + array is the union; else the `do_operation` of
    /// op1's class, else op2's; then both operands as numbers
    /// (`zendi_try_convert_scalar_to_number`), failing with the
    /// operand-type error.
    #[inline(never)]
    pub(crate) fn bin_slow(
        &mut self,
        op: Op,
        f: fn(&Value, &Value) -> Result<Value, Thrown>,
        x: &Value,
        y: &Value,
    ) -> Result<Value, Thrown> {
        if op == Op::Add {
            if let (Value::Array(_), Value::Array(_)) = (x, y) {
                return rhp_values::ops::add(x, y);
            }
        }
        if let Some(v) = self.try_do_operation(op, x, y)? {
            return Ok(v);
        }
        let Some(a) = self.try_number(x)? else {
            return Err(self.binop_error(op, x, y));
        };
        let Some(b) = self.try_number(y)? else {
            return Err(self.binop_error(op, x, y));
        };
        f(&a, &b)
    }

    /// `pow_function`: numbers directly, else the object handler, then the
    /// scalar conversion; `0 ** negative` is deprecated (`safe_pow`).
    pub(crate) fn pow_op(&mut self, x: &Value, y: &Value) -> Result<Value, Thrown> {
        let num = |v: &Value| match v {
            Value::Int(n) => Some(Num::Int(*n)),
            Value::Float(f) => Some(Num::Float(*f)),
            _ => None,
        };
        let (a, b) = match (num(x), num(y)) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                if let Some(v) = self.try_do_operation(Op::Pow, x, y)? {
                    return Ok(v);
                }
                let Some(a) = self.try_number(x)? else {
                    return Err(self.binop_error(Op::Pow, x, y));
                };
                let Some(b) = self.try_number(y)? else {
                    return Err(self.binop_error(Op::Pow, x, y));
                };
                (
                    num(&a).unwrap_or(Num::Int(0)),
                    num(&b).unwrap_or(Num::Int(0)),
                )
            }
        };
        if rhp_values::ops::pow_base0_neg_exp(a, b) {
            self.diag(
                rhp_builtins::E_DEPRECATED,
                "Power of base 0 and negative exponent is deprecated",
            )?;
        }
        Ok(rhp_values::ops::pow_num(a, b))
    }

    /// `% << >> | & ^` (`convert_op1_op2_long`): two strings combine
    /// bytewise for `| & ^`; else op1's `do_operation`, then op1 as an int,
    /// then the same for op2. A failed conversion is the operand-type
    /// error, before op2's handler is tried.
    #[inline(never)]
    pub(crate) fn long_op(&mut self, op: Op, x: &Value, y: &Value) -> Result<Value, Thrown> {
        let bop = match op {
            Op::Mod => BitOp::Mod,
            Op::Shl => BitOp::Shl,
            Op::Shr => BitOp::Shr,
            Op::BitOr => BitOp::Or,
            Op::BitAnd => BitOp::And,
            _ => BitOp::Xor,
        };
        if let (Value::Str(p), Value::Str(q)) = (x, y) {
            if let Some(r) = rhp_values::ops::string_bitwise(p.as_bytes(), q.as_bytes(), &bop) {
                return Ok(Value::str(r));
            }
        }
        let a = match x {
            Value::Int(n) => *n,
            _ => {
                if let Some(v) = self.side_do_operation(x, op, x, y)? {
                    return Ok(v);
                }
                match self.try_get_long(x)? {
                    Some(n) => n,
                    None => return Err(self.binop_error(op, x, y)),
                }
            }
        };
        let b = match y {
            Value::Int(n) => *n,
            _ => {
                if let Some(v) = self.side_do_operation(y, op, x, y)? {
                    return Ok(v);
                }
                match self.try_get_long(y)? {
                    Some(n) => n,
                    None => return Err(self.binop_error(op, x, y)),
                }
            }
        };
        rhp_values::ops::long_binop(bop, a, b)
    }

    /// `bitwise_not_function` for anything but an int.
    #[inline(never)]
    pub(crate) fn bitwise_not_slow(&mut self, v: &Value) -> Result<Value, Thrown> {
        match v {
            Value::Float(f) => Ok(Value::Int(!self.dval_to_lval_safe(*f)?)),
            Value::Str(s) => Ok(Value::str(
                s.as_bytes().iter().map(|b| !b).collect::<Vec<u8>>(),
            )),
            Value::Ref(r) => {
                let inner = r.borrow().clone();
                if let Value::Int(n) = inner {
                    return Ok(Value::Int(!n));
                }
                self.bitwise_not_slow(&inner)
            }
            _ => {
                if let Value::Object(h) = v {
                    if let Some(f) = self.handlers_of(h).and_then(|t| t.do_operation) {
                        if let Some(r) = f(self, Op::BitNot, v, None)? {
                            return Ok(r);
                        }
                    }
                }
                Err(Thrown::type_error(format!(
                    "Cannot perform bitwise not on {}",
                    rhp_builtins::zval_value_name(self, v)
                )))
            }
        }
    }

    /// `.` with an object or array operand (`concat_function`): op1's
    /// `do_operation`, then op1 as a string, then the same for op2.
    pub(crate) fn concat_object(&mut self, x: &Value, y: &Value) -> Result<Value, Thrown> {
        let mut bytes = match x {
            Value::Str(s) => s.as_bytes().to_vec(),
            _ => {
                if let Some(v) = self.side_do_operation(x, Op::Concat, x, y)? {
                    return Ok(v);
                }
                self.to_string_vm(x)?
            }
        };
        match y {
            Value::Str(s) => bytes.extend_from_slice(s.as_bytes()),
            _ => {
                if let Some(v) = self.side_do_operation(y, Op::Concat, x, y)? {
                    return Ok(v);
                }
                bytes.extend_from_slice(&self.to_string_vm(y)?);
            }
        }
        Ok(Value::str(bytes))
    }

    /// `zend_binop_error`: "Unsupported operand types: T1 op T2".
    fn binop_error(&self, op: Op, x: &Value, y: &Value) -> Thrown {
        let sigil = match op {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Mod => "%",
            Op::Pow => "**",
            Op::Shl => "<<",
            Op::Shr => ">>",
            Op::BitOr => "|",
            Op::BitAnd => "&",
            Op::BitXor => "^",
            Op::Concat => ".",
            Op::BitNot => "~",
        };
        Thrown::type_error(format!(
            "Unsupported operand types: {} {sigil} {}",
            rhp_builtins::zval_type_name(self, x),
            rhp_builtins::zval_type_name(self, y)
        ))
    }

    /// `zendi_try_get_long`: `None` when `v` has no integer value (an array,
    /// a resource, a non-numeric string, an object whose cast fails).
    fn try_get_long(&mut self, v: &Value) -> Result<Option<i64>, Thrown> {
        Ok(Some(match v {
            Value::Null | Value::Undef => 0,
            Value::Bool(b) => *b as i64,
            Value::Int(n) => *n,
            Value::Float(f) => self.dval_to_lval_safe(*f)?,
            Value::Str(s) => match self.numeric_operand(s.as_bytes())? {
                None => return Ok(None),
                Some(Num::Int(n)) => n,
                Some(Num::Float(d)) => {
                    // zend_dval_to_lval_cap
                    let l = if !d.is_finite() {
                        0
                    } else if !fits_long(d) {
                        if d > 0.0 {
                            i64::MAX
                        } else {
                            i64::MIN
                        }
                    } else {
                        d as i64
                    };
                    if l as f64 != d {
                        self.diag(
                            rhp_builtins::E_DEPRECATED,
                            &[
                                b"Implicit conversion from float-string \"".as_slice(),
                                s.as_bytes(),
                                b"\" to int loses precision",
                            ]
                            .concat(),
                        )?;
                    }
                    l
                }
            },
            Value::Object(h) => match self.cast_object_vm(h, CastTarget::Int)? {
                Some(n) => rhp_values::conv::to_int(&n),
                None => return Ok(None),
            },
            Value::Ref(r) => {
                let inner = r.borrow().clone();
                return self.try_get_long(&inner);
            }
            Value::Closure(_) | Value::Array(_) | Value::Resource(_) => return Ok(None),
        }))
    }

    /// `zendi_try_convert_scalar_to_number`: an int or a float, `None` when
    /// `v` has no numeric value.
    fn try_number(&mut self, v: &Value) -> Result<Option<Value>, Thrown> {
        Ok(Some(match v {
            Value::Null | Value::Undef | Value::Bool(false) => Value::Int(0),
            Value::Bool(true) => Value::Int(1),
            Value::Int(_) | Value::Float(_) => v.clone(),
            Value::Str(s) => match self.numeric_operand(s.as_bytes())? {
                Some(Num::Int(n)) => Value::Int(n),
                Some(Num::Float(f)) => Value::Float(f),
                None => return Ok(None),
            },
            Value::Object(h) => match self.cast_object_vm(h, CastTarget::Number)? {
                Some(n @ (Value::Int(_) | Value::Float(_))) => n,
                _ => return Ok(None),
            },
            Value::Ref(r) => {
                let inner = r.borrow().clone();
                return self.try_number(&inner);
            }
            Value::Closure(_) | Value::Array(_) | Value::Resource(_) => return Ok(None),
        }))
    }

    /// `is_numeric_string_ex` with errors allowed: the number at the start
    /// of `s`, warning when other data follows it; `None` if there is none.
    fn numeric_operand(&mut self, s: &[u8]) -> Result<Option<Num>, Thrown> {
        if s.first().is_some_and(|&c| c > b'9') {
            return Ok(None);
        }
        let Some((num, end)) = rhp_values::conv::scan_number(s) else {
            return Ok(None);
        };
        if !s[end..].iter().all(|b| b" \t\n\r\x0b\x0c".contains(b)) {
            self.diag(rhp_builtins::E_WARNING, "A non-numeric value encountered")?;
        }
        Ok(Some(num))
    }

    /// `zend_dval_to_lval_safe`: `zend_dval_to_lval` (php 8.5's
    /// out-of-range warning), then the deprecation for a lost fraction.
    pub(crate) fn dval_to_lval_safe(&mut self, d: f64) -> Result<i64, Thrown> {
        let l = self.dval_to_lval(d)?;
        if l as f64 != d && fits_long(d) {
            let shown = rhp_values::fmt::float_serialize(d);
            self.diag(
                rhp_builtins::E_DEPRECATED,
                &[
                    b"Implicit conversion from float ".as_slice(),
                    &shown,
                    b" to int loses precision",
                ]
                .concat(),
            )?;
        }
        Ok(l)
    }

    /// Loose comparison with the engine behind it (`__toString` etc.).
    pub(crate) fn compare_values(
        &mut self,
        a: &Value,
        b: &Value,
    ) -> Result<std::cmp::Ordering, Thrown> {
        let r = rhp_values::ops::compare(a, b, self);
        // An error handler that threw on the conversion notice.
        match self.pending_throw.take() {
            Some(t) => Err(t),
            None => r,
        }
    }
}

/// `ZEND_DOUBLE_FITS_LONG`.
fn fits_long(d: f64) -> bool {
    !(d >= 9_223_372_036_854_775_808.0 || d < -9_223_372_036_854_775_808.0)
}

impl CompareCx for Vm {
    fn is_enum(&self, class: ClassId) -> bool {
        self.class(class).kind == ClassKind::Enum
    }

    fn declared_props(&self, class: ClassId) -> Option<Vec<PhpStr>> {
        Some(self.class(class).props.keys().cloned().collect())
    }

    fn is_internal_prop(&self, class: ClassId, name: &PhpStr) -> bool {
        // Built-in (prelude) classes keep native state in `__`-prefixed
        // properties; php's objects hold it outside the property table.
        // ArrayObject/ArrayIterator's storage (`__s`) is what their compare
        // handler compares, so it stays visible.
        let n = name.as_bytes();
        let bare = match n.first() {
            Some(0) => n.iter().rposition(|&c| c == 0).map_or(n, |i| &n[i + 1..]),
            _ => n,
        };
        bare != b"__s" && self.internal_slot(class, name)
    }

    fn props_object(&mut self, obj: &ObjectHandle) -> Result<ObjectHandle, Thrown> {
        self.lazy_props_object(obj)
    }

    fn class_name(&self, class: ClassId) -> Vec<u8> {
        self.class(class).name.as_bytes().to_vec()
    }

    fn cast_object(
        &mut self,
        obj: &ObjectHandle,
        target: CastTarget,
    ) -> Result<Option<Value>, Thrown> {
        self.cast_object_vm(obj, target)
    }

    fn compare_handler(
        &mut self,
        obj: &ObjectHandle,
        a: &Value,
        b: &Value,
    ) -> Option<Result<i32, Thrown>> {
        let f = self.handlers_of(obj)?.compare?;
        f(self, a, b).transpose()
    }

    fn notice(&mut self, msg: &str) {
        self.raise_error(rhp_builtins::E_NOTICE, msg);
    }
}
