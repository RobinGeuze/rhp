//! The VM side of loose comparison and of the object handlers that conversions
//! and operators consult: `cast_object` (an internal class's handler, else
//! `__toString`), internal `compare` handlers, `do_operation`, enum
//! detection, the conversion notice.

use rhp_builtins::object::{CastTarget, Op};
use rhp_semantics::ops::CompareCx;
use rhp_values::{ClassId, ClassKind, ObjectHandle, PhpStr, Thrown, Value};

use crate::Vm;

impl Vm {
    /// The object handlers of `h`'s class (`None` for plain classes).
    pub(crate) fn handlers_of(
        &self,
        h: &ObjectHandle,
    ) -> Option<&'static rhp_builtins::object::ObjectHandlers> {
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
        match target {
            CastTarget::Str => {
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
            t => Ok(rhp_builtins::object::std_cast(t)),
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
        self.raise_error(8, msg);
    }
}
