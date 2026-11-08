//! `IntlIterator` (common_enum.cpp) and `IntlPartsIterator`
//! (breakiterator_iterators.cpp): the iterators intl's enumerations and a
//! break iterator's parts come back as.
//!
//! php keeps one `zend_object_iterator` per IntlIterator, shared by its
//! methods and by `foreach` (`get_iterator`), index included; [`Zoi`] is
//! that iterator. An enumeration's elements are taken when the iterator is
//! made (iterating the same as php's live `StringEnumeration` while
//! nothing changes the source); a parts iterator drives its break
//! iterator live.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rhp_values::{ObjectHandle, PhpArray, Value};

use super::breakiterator::{self, BiObj};
use super::common::{self, Call};
use super::error::IntlError;
use super::sys::{self, UEnumeration};
use crate::zend::internal_iterator::{self, IteratorFuncs};
use crate::zend::object_handlers::ObjectHandlers;
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    get_iterator: Some(get_iterator),
    get_gc: Some(get_gc),
    ..ObjectHandlers::UNCLONEABLE
};

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlIterator",
    helper: "__intliter",
    props: "",
};

pub(super) const PARTS_SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlPartsIterator",
    helper: "__partsiter",
    props: "",
};

/// `parts_iter_key_type`.
pub(super) const KEY_SEQUENTIAL: i64 = 0;
pub(super) const KEY_LEFT: i64 = 1;
pub(super) const KEY_RIGHT: i64 = 2;

/// What an iterator walks.
enum Kind {
    /// A string enumeration's elements and the current one.
    Enum {
        items: Vec<Value>,
        pos: Option<usize>,
    },
    /// `zoi_break_iter_parts`.
    Parts {
        bi: Value,
        bio: Rc<BiObj>,
        key_type: i64,
        index_right: i64,
    },
}

/// php's `zoi_with_current`: the shared iterator.
struct Zoi {
    /// `wrapping_obj`: an enumeration's iterator holds its IntlIterator,
    /// a cycle only the collector frees (as in php).
    wrapping: RefCell<Option<Value>>,
    index: Cell<i64>,
    current: RefCell<Option<Value>>,
    kind: RefCell<Kind>,
}

impl Zoi {
    fn move_forward(&self) {
        *self.current.borrow_mut() = None;
        let mut kind = self.kind.borrow_mut();
        match &mut *kind {
            Kind::Enum { items, pos } => {
                let next = pos.map_or(0, |p| p + 1);
                *pos = Some(next);
                if let Some(v) = items.get(next) {
                    *self.current.borrow_mut() = Some(v.clone());
                }
            }
            Kind::Parts {
                bio,
                key_type,
                index_right,
                ..
            } => {
                let cur = bio.op(breakiterator::OP_CURRENT, 0);
                if cur == sys::UBRK_DONE {
                    return;
                }
                let next = bio.op(breakiterator::OP_NEXT, 0);
                if next == sys::UBRK_DONE {
                    return;
                }
                if *key_type == KEY_LEFT {
                    self.index.set(cur as i64);
                } else if *key_type == KEY_RIGHT {
                    self.index.set(next as i64);
                    *index_right = next as i64;
                }
                let text = bio.text_bytes();
                let (a, b) = (cur as usize, (next as usize).min(text.len()));
                *self.current.borrow_mut() = Some(Value::str(text[a.min(b)..b].to_vec()));
            }
        }
    }

    fn rewind(&self) {
        *self.current.borrow_mut() = None;
        match &mut *self.kind.borrow_mut() {
            Kind::Enum { pos, .. } => *pos = None,
            Kind::Parts { bio, .. } => {
                bio.op(breakiterator::OP_FIRST, 0);
            }
        }
        self.move_forward();
    }

    fn key(&self) -> Value {
        if let Kind::Parts {
            key_type,
            index_right,
            ..
        } = &*self.kind.borrow()
        {
            if *key_type == KEY_RIGHT && self.index.get() == 0 {
                return Value::Int(*index_right);
            }
        }
        Value::Int(self.index.get())
    }
}

/// `IntlIterator_object`.
#[derive(Default)]
pub(super) struct IterObj {
    err: RefCell<IntlError>,
    zoi: RefCell<Option<Rc<Zoi>>>,
}

fn new_zoi(kind: Kind) -> Rc<Zoi> {
    Rc::new(Zoi {
        wrapping: RefCell::new(None),
        index: Cell::new(0),
        current: RefCell::new(None),
        kind: RefCell::new(kind),
    })
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
        items.push(Value::str(s.to_vec()));
    }
    // SAFETY: the enumeration the caller handed over.
    unsafe { sys::uenum_close(en) };
    let obj = host
        .instantiate(b"IntlIterator", Vec::new())
        .ok_or_else(|| Thrown::error("Error", "Class \"IntlIterator\" not found"))?;
    let ii = common::state_of::<IterObj>(&obj).expect("an object");
    let zoi = new_zoi(Kind::Enum { items, pos: None });
    *zoi.wrapping.borrow_mut() = Some(obj.clone());
    *ii.zoi.borrow_mut() = Some(zoi);
    Ok(obj)
}

/// `IntlIterator_object_get_gc` (through `zoi_with_current_get_gc`).
fn get_gc(h: &ObjectHandle, out: &mut Vec<Value>) {
    let Some(ii) = crate::zend::object_handlers::payload_of::<IterObj>(h) else {
        return;
    };
    let zoi = ii.zoi.borrow().clone();
    if let Some(w) = zoi.and_then(|z| z.wrapping.borrow().clone()) {
        out.push(w);
    }
}

/// `IntlIterator_from_BreakIterator_parts`.
pub(super) fn from_parts(
    host: &mut dyn Host,
    bi: Value,
    bio: Rc<BiObj>,
    key_type: i64,
) -> Result<Value, Thrown> {
    let obj = host
        .instantiate(b"IntlPartsIterator", Vec::new())
        .ok_or_else(|| Thrown::error("Error", "Class \"IntlPartsIterator\" not found"))?;
    let ii = common::state_of::<IterObj>(&obj).expect("an object");
    *ii.zoi.borrow_mut() = Some(new_zoi(Kind::Parts {
        bi,
        bio,
        key_type,
        index_right: 0,
    }));
    Ok(obj)
}

/// `IntlIterator_get_iterator`: `foreach` on the shared iterator, which
/// sets the index as the engine does (FE_RESET, FE_FETCH).
fn get_iterator(
    host: &mut dyn Host,
    h: &ObjectHandle,
    by_ref: bool,
) -> Result<ObjectHandle, Thrown> {
    if by_ref {
        return Err(Thrown::error(
            "Exception",
            "Iteration by reference is not supported",
        ));
    }
    let ii = common::state::<IterObj>(h);
    let Some(zoi) = ii.zoi.borrow().clone() else {
        return Err(Thrown::error(
            "Exception",
            "The IntlIterator is not properly constructed",
        ));
    };
    internal_iterator::create(
        host,
        Box::new(Engine {
            zoi,
            _obj: h.clone(),
        }),
    )
}

struct Engine {
    zoi: Rc<Zoi>,
    _obj: ObjectHandle,
}

impl IteratorFuncs for Engine {
    fn valid(&self, _host: &mut dyn Host) -> Result<bool, Thrown> {
        Ok(self.zoi.current.borrow().is_some())
    }
    fn current(&self, _host: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(self.zoi.current.borrow().clone())
    }
    fn key(&self, _host: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(Some(self.zoi.key()))
    }
    fn move_forward(&self, _host: &mut dyn Host) -> Result<(), Thrown> {
        self.zoi.index.set(self.zoi.index.get() + 1);
        self.zoi.move_forward();
        Ok(())
    }
    fn rewind(&self, _host: &mut dyn Host) -> Result<(), Thrown> {
        self.zoi.index.set(0);
        self.zoi.rewind();
        // FE_RESET leaves -1, the first FE_FETCH makes it 0.
        self.zoi.index.set(0);
        Ok(())
    }
}

fn fetch(call: &Call) -> Result<(Rc<IterObj>, Rc<Zoi>), Thrown> {
    let this = call.this.clone().expect("$this");
    let ii = common::state_of::<IterObj>(&this).expect("an object");
    let zoi = ii.zoi.borrow().clone();
    match zoi {
        Some(z) => Ok((ii, z)),
        None => Err(Thrown::error("Error", "Found unconstructed IntlIterator")),
    }
}

/// `__intliter(method, argc, $this)`.
pub(super) fn fn_helper(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlIterator", args, &[]);
    let (ii, zoi) = fetch(&call)?;
    ii.err.borrow_mut().reset();
    Ok(match call.name {
        "current" => zoi.current.borrow().clone().unwrap_or(Value::Null),
        "key" => zoi.key(),
        "next" => {
            zoi.move_forward();
            zoi.index.set(zoi.index.get() + 1);
            Value::Null
        }
        "rewind" => {
            zoi.rewind();
            Value::Null
        }
        "valid" => Value::Bool(zoi.current.borrow().is_some()),
        other => unreachable!("IntlIterator::{other}"),
    })
}

/// `__partsiter(method, argc, $this)`.
pub(super) fn fn_parts_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlPartsIterator", args, &[]);
    let (_, zoi) = fetch(&call)?;
    let bi = match &*zoi.kind.borrow() {
        Kind::Parts { bi, .. } => bi.clone(),
        Kind::Enum { .. } => Value::Null,
    };
    match call.name {
        "getBreakIterator" => Ok(bi),
        "getRuleStatus" => {
            let mut a = PhpArray::new();
            a.push(bi);
            a.push(Value::str(b"getrulestatus".to_vec()));
            host.call_callable(&Value::Array(a), &[])
        }
        other => unreachable!("IntlPartsIterator::{other}"),
    }
}
