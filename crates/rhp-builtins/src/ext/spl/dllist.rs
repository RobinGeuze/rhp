//! `SplDoublyLinkedList`'s `get_iterator` (spl_dllist_get_iterator): a
//! `foreach` walks the list with its own traverse pointer and position, so
//! nested loops over one list (and the list's own `current()`) don't
//! disturb each other. The list itself is prelude.php's linked arrays
//! (`__v` values, `__nx` / `__pv` links, `__h` / `__t` ends, by element id).

use std::cell::Cell;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, ObjectHandle, PhpStr, Value};

use crate::zend::internal_iterator::{self, IteratorFuncs};
use crate::zend::object_handlers::ObjectHandlers;
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    get_iterator: Some(get_iterator),
    ..ObjectHandlers::STD
};

const IT_LIFO: i64 = 2;
const IT_DELETE: i64 = 1;

fn key(name: &str) -> PhpStr {
    PhpStr::from(format!("\0SplDoublyLinkedList\0{name}").as_str())
}

fn slot(h: &ObjectHandle, name: &str) -> Value {
    h.borrow()
        .props()
        .get(&key(name))
        .map(deref)
        .unwrap_or(Value::Null)
}

fn id_of(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        _ => None,
    }
}

/// The element after (`next`) or before `id`.
fn link(h: &ObjectHandle, id: i64, next: bool) -> Option<i64> {
    match slot(h, if next { "__nx" } else { "__pv" }) {
        Value::Array(a) => a
            .get(&ArrayKey::Int(id))
            .map(deref)
            .as_ref()
            .and_then(id_of),
        _ => None,
    }
}

fn set(h: &ObjectHandle, name: &str, v: Value) {
    let old = h.borrow_mut().props_mut().insert(key(name), v);
    drop(old);
}

/// Remove `id` from link table `name`.
fn unlink(h: &ObjectHandle, name: &str, id: i64) {
    let mut b = h.borrow_mut();
    let old = match b.props_mut().get_mut(&key(name)) {
        Some(Value::Array(a)) => a.remove(&ArrayKey::Int(id)),
        _ => None,
    };
    drop(b);
    drop(old);
}

/// prelude.php's `__pop` (`back`) or `__shift`: unlink the tail or head
/// and release its value.
fn take_end(h: &ObjectHandle, back: bool) {
    let (end, other_end) = if back { ("__t", "__h") } else { ("__h", "__t") };
    let Some(e) = id_of(&slot(h, end)) else {
        return;
    };
    let inner = link(h, e, !back);
    match inner {
        Some(i) => unlink(h, if back { "__nx" } else { "__pv" }, i),
        None => set(h, other_end, Value::Null),
    }
    set(h, end, inner.map_or(Value::Null, Value::Int));
    let n = conv::to_int(&slot(h, "__n"));
    set(h, "__n", Value::Int(n - 1));
    unlink(h, "__nx", e);
    unlink(h, "__pv", e);
    let v = {
        let mut b = h.borrow_mut();
        match b.props_mut().get_mut(&key("__v")) {
            Some(Value::Array(a)) => a.remove(&ArrayKey::Int(e)),
            _ => None,
        }
    };
    drop(v);
}

/// `spl_dllist_it`.
struct DllIter {
    list: ObjectHandle,
    flags: i64,
    traverse: Cell<Option<i64>>,
    position: Cell<i64>,
}

impl IteratorFuncs for DllIter {
    fn valid(&self, _: &mut dyn Host) -> Result<bool, Thrown> {
        Ok(self.traverse.get().is_some())
    }

    fn current(&self, _: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        let Some(e) = self.traverse.get() else {
            return Ok(None);
        };
        Ok(match slot(&self.list, "__v") {
            Value::Array(a) => a.get(&ArrayKey::Int(e)).map(deref),
            _ => None,
        })
    }

    fn key(&self, _: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(Some(Value::Int(self.position.get())))
    }

    /// spl_dllist_it_helper_move_forward.
    fn move_forward(&self, _: &mut dyn Host) -> Result<(), Thrown> {
        let Some(old) = self.traverse.get() else {
            return Ok(());
        };
        let has = || conv::to_int(&slot(&self.list, "__n")) > 0;
        if self.flags & IT_LIFO != 0 {
            self.traverse.set(link(&self.list, old, false));
            self.position.set(self.position.get() - 1);
            if self.flags & IT_DELETE != 0 && has() {
                take_end(&self.list, true);
            }
        } else {
            self.traverse.set(link(&self.list, old, true));
            if self.flags & IT_DELETE != 0 {
                if has() {
                    take_end(&self.list, false);
                }
            } else {
                self.position.set(self.position.get() + 1);
            }
        }
        Ok(())
    }

    /// spl_dllist_it_helper_rewind.
    fn rewind(&self, _: &mut dyn Host) -> Result<(), Thrown> {
        let n = conv::to_int(&slot(&self.list, "__n"));
        if self.flags & IT_LIFO != 0 {
            self.position.set(n - 1);
            self.traverse.set(id_of(&slot(&self.list, "__t")));
        } else {
            self.position.set(0);
            self.traverse.set(id_of(&slot(&self.list, "__h")));
        }
        Ok(())
    }
}

/// spl_dllist_get_iterator.
fn get_iterator(
    host: &mut dyn Host,
    h: &ObjectHandle,
    by_ref: bool,
) -> Result<ObjectHandle, Thrown> {
    if by_ref {
        return Err(Thrown::error(
            "Error",
            "An iterator cannot be used with foreach by reference",
        ));
    }
    // `__fl` is null until first used: the class's own bits.
    let flags = match slot(h, "__fl") {
        Value::Int(f) => f,
        _ => {
            let cls = host.class_name(h.borrow().class).unwrap_or_default();
            if host.reflect_is_a(&cls, b"SplStack") {
                6
            } else if host.reflect_is_a(&cls, b"SplQueue") {
                4
            } else {
                0
            }
        }
    };
    internal_iterator::create(
        host,
        Box::new(DllIter {
            list: h.clone(),
            flags: flags & 3,
            traverse: Cell::new(id_of(&slot(h, "__tp"))),
            position: Cell::new(conv::to_int(&slot(h, "__pos"))),
        }),
    )
}
