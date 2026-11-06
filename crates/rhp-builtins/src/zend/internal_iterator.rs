//! php's InternalIterator (Zend/zend_interfaces.c): the `Iterator` object
//! `getIterator()` returns for an internal class whose `foreach` runs on a
//! `zend_object_iterator` (DatePeriod, WeakMap). The iterator functions
//! are the class's [`IteratorFuncs`]; the class itself is in
//! core_prelude.php and its methods call `__internal_iterator`.

use std::cell::Cell;
use std::rc::Rc;

use rhp_values::{conv, ObjectHandle, PhpRef, Thrown, Value};

use crate::zend::object_handlers;
use crate::Host;

/// php's `zend_object_iterator_funcs`.
pub trait IteratorFuncs {
    fn valid(&self, host: &mut dyn Host) -> Result<bool, Thrown>;
    /// The current element; `None` past the end.
    fn current(&self, host: &mut dyn Host) -> Result<Option<Value>, Thrown>;
    /// The current key; `Ok(None)` when the iterator has no
    /// `get_current_key` (the key is then the index).
    fn key(&self, host: &mut dyn Host) -> Result<Option<Value>, Thrown>;
    fn move_forward(&self, host: &mut dyn Host) -> Result<(), Thrown>;
    fn rewind(&self, host: &mut dyn Host) -> Result<(), Thrown>;
    /// The current element as a reference, for `foreach` by reference
    /// (`get_iterator` with `by_ref`); `None` past the end.
    fn current_ref(&self, _host: &mut dyn Host) -> Result<Option<PhpRef>, Thrown> {
        Ok(None)
    }
}

/// php's `zend_internal_iterator`: the payload of an InternalIterator.
pub struct InternalIter {
    funcs: Box<dyn IteratorFuncs>,
    /// `iter->index`.
    index: Cell<i64>,
    rewind_called: Cell<bool>,
}

/// `zend_create_internal_iterator_zval`: a new InternalIterator over
/// `funcs`.
pub fn create(host: &mut dyn Host, funcs: Box<dyn IteratorFuncs>) -> Result<ObjectHandle, Thrown> {
    let Some(Value::Object(h)) = host.instantiate(b"InternalIterator", Vec::new()) else {
        return Err(Thrown::error(
            "Error",
            "Cannot instantiate InternalIterator",
        ));
    };
    object_handlers::set_payload(
        &h,
        Rc::new(InternalIter {
            funcs,
            index: Cell::new(0),
            rewind_called: Cell::new(false),
        }),
    );
    Ok(h)
}

fn fetch(h: &ObjectHandle) -> Result<Rc<InternalIter>, Thrown> {
    object_handlers::payload_of::<InternalIter>(h).ok_or_else(|| {
        Thrown::error(
            "Error",
            "The InternalIterator object has not been properly initialized",
        )
    })
}

impl InternalIter {
    fn ensure_rewound(&self, host: &mut dyn Host) -> Result<(), Thrown> {
        if !self.rewind_called.get() {
            self.rewind_called.set(true);
            self.funcs.rewind(host)?;
        }
        Ok(())
    }
}

/// The iterator steps `foreach` takes on an InternalIterator (the
/// engine's FE_FETCH over a `zend_object_iterator`).
pub enum Step {
    Rewind,
    Valid,
    Next,
    Key,
    /// The current element by reference (`None` past the end).
    CurrentRef,
}

/// One foreach step on the InternalIterator `h`. `Valid` returns a bool,
/// `CurrentRef` a `Value::Ref` (or null), `Key` the key.
pub fn step(host: &mut dyn Host, h: &ObjectHandle, s: Step) -> Result<Value, Thrown> {
    let it = fetch(h)?;
    match s {
        Step::Rewind => {
            it.rewind_called.set(true);
            it.funcs.rewind(host)?;
            it.index.set(0);
            Ok(Value::Null)
        }
        Step::Valid => Ok(Value::Bool(it.funcs.valid(host)?)),
        Step::Next => {
            it.index.set(it.index.get() + 1);
            it.funcs.move_forward(host)?;
            Ok(Value::Null)
        }
        Step::Key => Ok(match it.funcs.key(host)? {
            Some(k) => k,
            None => Value::Int(it.index.get()),
        }),
        Step::CurrentRef => Ok(it
            .funcs
            .current_ref(host)?
            .map(Value::Ref)
            .unwrap_or(Value::Null)),
    }
}

/// `__internal_iterator($this, $method)`: the InternalIterator methods.
fn fn_internal_iterator(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(h) = args.first().and_then(object_handlers::object_of) else {
        return Ok(Value::Null);
    };
    let method = args.get(1).map(conv::to_string_bytes).unwrap_or_default();
    let it = fetch(&h)?;
    if method == b"rewind" {
        return step(host, &h, Step::Rewind);
    }
    it.ensure_rewound(host)?;
    match &method[..] {
        b"current" => Ok(it
            .funcs
            .current(host)?
            .map(|v| rhp_values::ops::deref(&v))
            .unwrap_or(Value::Null)),
        b"key" => step(host, &h, Step::Key),
        b"next" => step(host, &h, Step::Next),
        _ => step(host, &h, Step::Valid),
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(
        m,
        &[("__internal_iterator", fn_internal_iterator, 2, Some(2))],
    );
}
