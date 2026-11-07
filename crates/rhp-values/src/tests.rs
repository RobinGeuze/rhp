use super::*;

fn key(s: &str) -> ArrayKey {
    ArrayKey::from_bytes(s.as_bytes())
}

#[test]
fn canonical_int_string_rules() {
    assert_eq!(key("0"), ArrayKey::Int(0));
    assert_eq!(key("8"), ArrayKey::Int(8));
    assert_eq!(key("-5"), ArrayKey::Int(-5));
    assert_eq!(key("9223372036854775807"), ArrayKey::Int(i64::MAX));
    // stay strings:
    for s in [
        "08",
        "00",
        "01",
        "+1",
        " 1",
        "1.5",
        "-0",
        "9223372036854775808",
        "",
    ] {
        assert!(
            matches!(key(s), ArrayKey::Str(_)),
            "{s:?} should be a string key"
        );
    }
}

#[test]
fn key_from_scalars() {
    assert_eq!(
        ArrayKey::from_value(&Value::Bool(true)),
        Some(ArrayKey::Int(1))
    );
    assert_eq!(
        ArrayKey::from_value(&Value::Bool(false)),
        Some(ArrayKey::Int(0))
    );
    assert_eq!(
        ArrayKey::from_value(&Value::Float(2.9)),
        Some(ArrayKey::Int(2))
    );
    assert_eq!(
        ArrayKey::from_value(&Value::Float(-2.9)),
        Some(ArrayKey::Int(-2))
    );
    assert_eq!(
        ArrayKey::from_value(&Value::Null),
        Some(ArrayKey::Str(PhpStr::from_bytes(Vec::new())))
    );
    assert_eq!(ArrayKey::from_value(&Value::Array(PhpArray::new())), None);
}

#[test]
fn float_to_int_cases() {
    assert_eq!(float_to_int(2.9), 2);
    assert_eq!(float_to_int(-2.9), -2);
    assert_eq!(float_to_int(f64::NAN), 0);
    assert_eq!(float_to_int(f64::INFINITY), 0);
    assert_eq!(float_to_int(0.0), 0);
}

#[test]
fn next_int_high_water() {
    // [10=>x][] -> keys 10,11
    let mut a = PhpArray::new();
    a.insert(ArrayKey::Int(10), Value::str("x"));
    a.push(Value::str("y"));
    let keys: Vec<_> = a.iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(keys, vec![ArrayKey::Int(10), ArrayKey::Int(11)]);

    // negative first key: [-5=>a][] -> keys -5,-4
    let mut b = PhpArray::new();
    b.insert(ArrayKey::Int(-5), Value::str("a"));
    b.push(Value::str("b"));
    let keys: Vec<_> = b.iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(keys, vec![ArrayKey::Int(-5), ArrayKey::Int(-4)]);
}

#[test]
fn append_after_int_max_fails() {
    let mut a = PhpArray::new();
    a.insert(ArrayKey::Int(i64::MAX), Value::Int(1));
    assert!(!a.try_push(Value::Int(2)));
    assert_eq!(a.len(), 1);
    // Freed again: php appends at PHP_INT_MAX.
    a.remove(&ArrayKey::Int(i64::MAX));
    assert!(a.try_push(Value::Int(3)));
    assert!(a.get(&ArrayKey::Int(i64::MAX)).is_some());
}

#[test]
fn removal_keeps_internal_pointer_on_element() {
    let mut a: PhpArray = (0..4).map(|i| (ArrayKey::Int(i), Value::Int(i))).collect();
    a.make_mut().ptr_next();
    a.make_mut().ptr_next();
    a.remove(&ArrayKey::Int(0));
    assert_eq!(a.current().unwrap().0, ArrayKey::Int(2));
    // The current element removed: the pointer is on the next one.
    a.remove(&ArrayKey::Int(2));
    assert_eq!(a.current().unwrap().0, ArrayKey::Int(3));
}

struct Native(Rc<std::cell::Cell<u32>>);
impl Drop for Native {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

fn object() -> ObjectHandle {
    Rc::new(RefCell::new(ObjectData::new(
        0,
        1,
        IndexMap::with_hasher(FxBuildHasher),
    )))
}

#[test]
fn payload_is_typed_and_dropped_with_the_object() {
    let drops = Rc::new(std::cell::Cell::new(0));
    let h = object();
    assert!(!h.borrow().has_payload());
    assert!(h
        .borrow_mut()
        .set_payload(Rc::new(Native(drops.clone())))
        .is_none());
    assert!(h.borrow().payload::<String>().is_none());
    let p = h.borrow().payload::<Native>().expect("payload");
    drop(h);
    // A clone a builtin still holds keeps the state alive.
    assert_eq!(drops.get(), 0);
    drop(p);
    assert_eq!(drops.get(), 1);
}

#[test]
fn payload_replace_and_take() {
    let drops = Rc::new(std::cell::Cell::new(0));
    let h = object();
    h.borrow_mut().set_payload(Rc::new(Native(drops.clone())));
    let old = h.borrow_mut().set_payload(Rc::new(Native(drops.clone())));
    assert_eq!(drops.get(), 0, "the caller drops the old payload");
    drop(old);
    assert_eq!(drops.get(), 1);
    drop(h.borrow_mut().take_payload());
    assert_eq!(drops.get(), 2);
    assert!(!h.borrow().has_payload());
    drop(h);
    assert_eq!(drops.get(), 2);
}

/// The slot is one pointer: an object's allocation (what memory_limit
/// accounting counts) stays in the 96-byte bin.
#[test]
fn payload_slot_is_one_pointer() {
    assert_eq!(std::mem::size_of::<Option<Box<Payload>>>(), 8);
    assert!(std::mem::size_of::<RefCell<ObjectData>>() + 16 <= 96);
}

#[test]
fn cow_isolation() {
    let mut a = PhpArray::new();
    a.push(Value::Int(1));
    let b = a.clone();
    a.push(Value::Int(2));
    assert_eq!(a.len(), 2);
    assert_eq!(b.len(), 1); // b unaffected — COW
}
