//! `IntlIterator` (common_enum.cpp): the iterator intl's enumerations come
//! back as. php wraps a live `StringEnumeration` (or a break iterator's
//! parts); rhp takes the elements when the iterator is made, which
//! iterates the same as long as nothing changes the source meanwhile.
//! Before the first `rewind()` there is no current element, as in php.

use std::cell::{Cell, RefCell};

use rhp_values::Value;

use super::common::{self, Call};
use super::error::IntlError;
use super::sys::{self, UEnumeration};
use crate::zend::object_handlers::ObjectHandlers;
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers::UNCLONEABLE;

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlIterator",
    helper: "__intliter",
    props: "",
};

/// `IntlIterator_object`.
#[derive(Default)]
pub(super) struct IterObj {
    pub(super) err: RefCell<IntlError>,
    pub(super) built: Cell<bool>,
    /// The elements, with their keys when not the running index.
    pub(super) items: RefCell<Vec<(Option<Value>, Value)>>,
    /// The current element (`None`: no current, `valid()` false).
    pub(super) pos: Cell<Option<usize>>,
    pub(super) index: Cell<i64>,
    /// The break iterator of an IntlPartsIterator.
    pub(super) source: RefCell<Option<Value>>,
}

/// An IntlIterator of class `class` over `items`.
pub(super) fn make(
    host: &mut dyn Host,
    class: &[u8],
    items: Vec<(Option<Value>, Value)>,
) -> Result<Value, Thrown> {
    let obj = host
        .instantiate(class, Vec::new())
        .ok_or_else(|| Thrown::error("Error", "Class \"IntlIterator\" not found"))?;
    let ii = common::state_of::<IterObj>(&obj).expect("an object");
    ii.built.set(true);
    *ii.items.borrow_mut() = items;
    Ok(obj)
}

/// `IntlIterator_from_StringEnumeration`, from a `UEnumeration` (closed).
pub(super) fn from_uenum(host: &mut dyn Host, en: *mut UEnumeration) -> Result<Value, Thrown> {
    let mut items = Vec::new();
    loop {
        let mut status = sys::U_ZERO_ERROR;
        let mut len = 0i32;
        // SAFETY: a live enumeration.
        let p = unsafe { sys::uenum_next(en, &mut len, &mut status) };
        if p.is_null() || sys::failure(status) {
            break;
        }
        // SAFETY: `len` bytes at `p`.
        let s = unsafe { std::slice::from_raw_parts(p.cast::<u8>(), len.max(0) as usize) };
        items.push((None, Value::str(s.to_vec())));
    }
    // SAFETY: the enumeration the caller handed over.
    unsafe { sys::uenum_close(en) };
    make(host, b"IntlIterator", items)
}

/// `__intliter(method, argc, $this)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlIterator", args, &[]);
    super::error::error_reset();
    let this = call.this.clone().expect("$this");
    let ii = common::state_of::<IterObj>(&this).expect("an object");
    ii.err.borrow_mut().reset();
    if !ii.built.get() {
        return Err(Thrown::error("Error", "Found unconstructed IntlIterator"));
    }
    let _ = host;
    let items = ii.items.borrow();
    Ok(match call.name {
        "current" => match ii.pos.get() {
            Some(p) => items[p].1.clone(),
            None => Value::Null,
        },
        "key" => match ii.pos.get().and_then(|p| items[p].0.clone()) {
            Some(k) => k,
            None => Value::Int(ii.index.get()),
        },
        "next" => {
            if let Some(p) = ii.pos.get() {
                ii.pos.set((p + 1 < items.len()).then_some(p + 1));
            }
            ii.index.set(ii.index.get() + 1);
            Value::Null
        }
        "rewind" => {
            ii.index.set(0);
            ii.pos.set((!items.is_empty()).then_some(0));
            Value::Null
        }
        "valid" => Value::Bool(ii.pos.get().is_some()),
        other => unreachable!("IntlIterator::{other}"),
    })
}
