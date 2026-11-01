//! The VM side of loose comparison: the engine services `zend_compare` needs
//! for objects (`cast_object` via `__toString`, internal `compare` handlers for
//! `GMP` and `DateTimeInterface`, enum detection, the conversion notice).

use rhp_builtins::GmpCmpOperand;
use rhp_semantics::ops::{CastTarget, CompareCx};
use rhp_values::{conv, ClassId, ClassKind, ObjectHandle, PhpStr, Thrown, Value};

use crate::Vm;

impl Vm {
    fn class_is(&self, cls: ClassId, name: &[u8]) -> bool {
        match self.resolve_class_by_name(name) {
            Ok(t) => self.class(cls).ancestors.contains(&t),
            Err(_) => false,
        }
    }

    fn is_gmp(&self, h: &ObjectHandle) -> bool {
        self.class(h.borrow().class)
            .name
            .as_bytes()
            .eq_ignore_ascii_case(b"GMP")
    }

    fn gmp_num(h: &ObjectHandle) -> Vec<u8> {
        match h
            .borrow()
            .properties
            .get(&PhpStr::from_bytes(b"num".to_vec()))
        {
            Some(v) => conv::to_string_bytes(v),
            None => b"0".to_vec(),
        }
    }

    /// `zend_zval_value_name`.
    fn value_name(&self, v: &Value) -> String {
        match v {
            Value::Null => "null".into(),
            Value::Bool(true) => "true".into(),
            Value::Bool(false) => "false".into(),
            Value::Int(_) => "int".into(),
            Value::Float(_) => "float".into(),
            Value::Str(_) => "string".into(),
            Value::Array(_) => "array".into(),
            Value::Object(h) => {
                String::from_utf8_lossy(self.class(h.borrow().class).name.as_bytes()).into_owned()
            }
            Value::Closure(_) => "Closure".into(),
            Value::Resource(_) => "resource".into(),
            Value::Ref(r) => self.value_name(&r.borrow()),
        }
    }

    fn gmp_operand(&self, v: &Value) -> GmpCmpOperand {
        match v {
            Value::Object(h) if self.is_gmp(h) => GmpCmpOperand::Gmp(Self::gmp_num(h)),
            other => GmpCmpOperand::Other(other.clone(), self.value_name(other)),
        }
    }

    fn date_parts(h: &ObjectHandle) -> Option<(i64, i64)> {
        let o = h.borrow();
        let ts = o.properties.get(&PhpStr::from_bytes(b"ts".to_vec()))?;
        let us = o.properties.get(&PhpStr::from_bytes(b"us".to_vec()))?;
        Some((conv::to_int(ts), conv::to_int(us)))
    }

    /// Loose comparison with the engine behind it (`__toString` etc.).
    pub(crate) fn compare_values(
        &mut self,
        a: &Value,
        b: &Value,
    ) -> Result<std::cmp::Ordering, Thrown> {
        let r = rhp_semantics::ops::compare(a, b, self);
        // An error handler that threw on the conversion notice.
        match self.pending_throw.take() {
            Some(t) => Err(t),
            None => r,
        }
    }
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

    fn class_name(&self, class: ClassId) -> Vec<u8> {
        self.class(class).name.as_bytes().to_vec()
    }

    fn cast_object(
        &mut self,
        obj: &ObjectHandle,
        target: CastTarget,
    ) -> Result<Option<Value>, Thrown> {
        if self.is_gmp(obj) {
            let num = Self::gmp_num(obj);
            let s = String::from_utf8_lossy(&num).into_owned();
            return Ok(match target {
                CastTarget::Str => Some(Value::str(s)),
                CastTarget::Int => Some(Value::Int(s.parse::<i64>().unwrap_or_else(|_| {
                    // mpz_get_si keeps the low bits; out-of-range values are rare.
                    s.parse::<f64>().map(|f| f as i64).unwrap_or(0)
                }))),
                CastTarget::Float => Some(Value::Float(s.parse::<f64>().unwrap_or(0.0))),
                CastTarget::Bool => Some(Value::Bool(s != "0")),
                _ => None,
            });
        }
        match target {
            CastTarget::Bool => Ok(Some(Value::Bool(true))),
            CastTarget::Str => {
                let cls = obj.borrow().class;
                match self.lookup_method(cls, b"__tostring") {
                    Some(m) => {
                        let r = self.call_sync(&m, Some(obj.clone()), cls, Vec::new())?;
                        Ok(Some(Value::Str(PhpStr::from_bytes(conv::to_string_bytes(
                            &r,
                        )))))
                    }
                    None => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    fn compare_handler(
        &mut self,
        obj: &ObjectHandle,
        a: &Value,
        b: &Value,
    ) -> Option<Result<i32, Thrown>> {
        if self.is_gmp(obj) {
            let (x, y) = (self.gmp_operand(a), self.gmp_operand(b));
            return Some(rhp_builtins::gmp_compare_handler(x, y));
        }
        let cls = obj.borrow().class;
        if self.class_is(cls, b"DateTimeInterface") {
            // ZEND_COMPARE_OBJECTS_FALLBACK: both must use the date handler.
            let (Value::Object(x), Value::Object(y)) = (a, b) else {
                return None;
            };
            let (xc, yc) = (x.borrow().class, y.borrow().class);
            if !self.class_is(xc, b"DateTimeInterface") || !self.class_is(yc, b"DateTimeInterface")
            {
                return None;
            }
            return Some(match (Self::date_parts(x), Self::date_parts(y)) {
                (Some(p), Some(q)) => Ok(match p.cmp(&q) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                }),
                _ => Err(Thrown::error(
                    "Error",
                    "Trying to compare an incomplete DateTime or DateTimeImmutable object",
                )),
            });
        }
        None
    }

    fn notice(&mut self, msg: &str) {
        self.raise_error(8, msg);
    }
}
