//! Layout tests for ArrayData: packed and hash tables, holes, positions.

use super::*;

fn int(n: i64) -> ArrayKey {
    ArrayKey::Int(n)
}

fn s(x: &str) -> ArrayKey {
    ArrayKey::Str(PhpStr::from(x))
}

fn keys(a: &PhpArray) -> Vec<ArrayKey> {
    a.keys().collect()
}

#[test]
fn appends_stay_packed_and_double() {
    let mut a = PhpArray::new();
    assert!(a.is_packed());
    for i in 0..100 {
        a.push(Value::Int(i));
    }
    assert!(a.is_packed());
    assert_eq!(a.data().table_size(), 128);
    assert_eq!(a.len(), 100);
    assert_eq!(
        a.get(&int(42)).map(|v| matches!(v, Value::Int(42))),
        Some(true)
    );
    assert!(a.get(&int(100)).is_none());
    assert!(a.get(&s("x")).is_none());
}

#[test]
fn string_key_makes_a_hash() {
    let mut a = PhpArray::new();
    a.push(Value::Int(0));
    a.insert(s("k"), Value::Int(1));
    assert!(!a.is_packed());
    assert_eq!(keys(&a), vec![int(0), s("k")]);
    a.push(Value::Int(2));
    assert_eq!(keys(&a), vec![int(0), s("k"), int(1)]);
}

#[test]
fn packed_rules_follow_zend_hash() {
    // A key behind the end within the table: still packed, holes between.
    let mut a = PhpArray::new();
    a.insert(int(0), Value::Null);
    a.insert(int(5), Value::Null);
    assert!(a.is_packed());
    assert_eq!(a.data().used(), 6);
    assert_eq!(a.len(), 2);
    assert_eq!(keys(&a), vec![int(0), int(5)]);
    // Out of order: a hash.
    a.insert(int(3), Value::Null);
    assert!(!a.is_packed());
    assert_eq!(keys(&a), vec![int(0), int(5), int(3)]);
    // Too far beyond the table: a hash.
    let mut b = PhpArray::new();
    b.insert(int(100), Value::Null);
    assert!(!b.is_packed());
    // Negative keys: a hash.
    let mut c = PhpArray::new();
    c.insert(int(-1), Value::Null);
    assert!(!c.is_packed());
    assert_eq!(c.next_free(), 0);
}

#[test]
fn holes_keep_positions_until_rehash() {
    let mut a: PhpArray = (0..4)
        .map(|i| (s(&format!("k{i}")), Value::Int(i)))
        .collect();
    a.remove(&s("k1"));
    assert_eq!(a.data().used(), 4);
    assert!(a.data().at(1).is_none());
    assert_eq!(a.fetch_from(1).map(|e| e.0), Some(2));
    // Removing the tail shrinks the used part past trailing holes.
    a.remove(&s("k3"));
    assert_eq!(a.data().used(), 3);
    a.remove(&s("k2"));
    assert_eq!(a.data().used(), 1);
    assert_eq!(keys(&a), vec![s("k0")]);
}

#[test]
fn many_inserts_and_removes_find_every_key() {
    let mut a = PhpArray::new();
    let mut model = std::collections::BTreeMap::new();
    let mut order = Vec::new();
    for i in 0..3000i64 {
        let ks = format!("key{i}");
        a.insert(s(&ks), Value::Int(i));
        model.insert(ks.clone(), true);
        order.push(s(&ks));
        let ki = i * 7919 - 5000;
        a.insert(int(ki), Value::Int(i));
        order.push(int(ki));
        if i % 3 == 0 {
            let gone = format!("key{}", i / 2);
            a.remove(&s(&gone));
            model.insert(gone, false);
        }
    }
    for (k, live) in &model {
        assert_eq!(a.get(&s(k)).is_some(), *live, "{k}");
    }
    let expect: Vec<ArrayKey> = order
        .into_iter()
        .filter(|k| match k {
            ArrayKey::Str(x) => model[std::str::from_utf8(x.as_bytes()).unwrap()],
            ArrayKey::Int(_) => true,
        })
        .collect();
    assert_eq!(keys(&a), expect);
    assert_eq!(a.iter().count(), a.len());
    assert_eq!(a.iter().rev().count(), a.len());
}

#[test]
fn rehash_moves_pointer_and_iterators() {
    let mut a: PhpArray = (0..8)
        .map(|i| (s(&format!("k{i}")), Value::Int(i)))
        .collect();
    let d = a.make_mut();
    d.set_pos(6);
    d.iter_add(1, 7);
    for i in 0..4 {
        d.remove(&s(&format!("k{i}")));
    }
    // Full table with holes: the next insert compacts it.
    d.insert(s("new"), Value::Null);
    assert_eq!(d.used(), 5);
    assert_eq!(d.current().unwrap().0, s("k6"));
    assert_eq!(d.iter_pos(1), Some(3));
}

#[test]
fn pointer_moves_like_php() {
    let mut a: PhpArray = (0..4).map(|i| (int(i), Value::Int(i))).collect();
    let d = a.make_mut();
    d.ptr_end();
    assert_eq!(d.current().unwrap().0, int(3));
    d.remove(&int(2));
    d.ptr_prev();
    assert_eq!(d.current().unwrap().0, int(1));
    d.ptr_next();
    assert_eq!(d.current().unwrap().0, int(3));
    d.ptr_next();
    assert!(d.current().is_none());
    // Past the end: an append becomes current.
    d.try_push(Value::Null);
    assert_eq!(d.current().unwrap().0, int(4));
}

#[test]
fn shift_and_pop_renumber() {
    let mut a: PhpArray = (0..4).map(|i| (int(i), Value::Int(i))).collect();
    let d = a.make_mut();
    assert!(matches!(d.shift(), Some(Value::Int(0))));
    assert_eq!(d.keys().collect::<Vec<_>>(), vec![int(0), int(1), int(2)]);
    assert_eq!(d.next_free(), 3);
    assert!(matches!(d.pop(), Some(Value::Int(3))));
    assert_eq!(d.next_free(), 2);
    let mut h = PhpArray::new();
    h.insert(int(5), Value::Int(5));
    h.insert(s("x"), Value::Int(0));
    h.insert(int(9), Value::Int(9));
    let d = h.make_mut();
    assert!(matches!(d.shift(), Some(Value::Int(5))));
    assert_eq!(d.keys().collect::<Vec<_>>(), vec![s("x"), int(0)]);
    assert_eq!(d.next_free(), 1);
}

#[test]
fn copies_keep_holes_and_order() {
    let mut a: PhpArray = (0..6).map(|i| (int(i), Value::Int(i))).collect();
    a.remove(&int(2));
    let mut b = a.clone();
    b.make_mut();
    assert!(b.is_packed());
    assert_eq!(b.data().used(), 6);
    assert!(b.get(&int(2)).is_none());
    let mut h: PhpArray = (0..6)
        .map(|i| (s(&format!("k{i}")), Value::Int(i)))
        .collect();
    h.remove(&s("k2"));
    let mut c = h.clone();
    let d = c.make_mut();
    assert_eq!(d.used(), 5);
    assert_eq!(keys(&c), keys(&h));
}

#[test]
fn sizes_track_php() {
    use std::mem::size_of;
    assert_eq!(size_of::<Bucket>(), 32);
    // php's zend_array is 56 bytes; the Rc box adds 16.
    assert_eq!(size_of::<ArrayData>(), 40);
    assert_eq!(size_of::<Value>(), 16);
}

#[test]
fn growth_and_conversion_keep_contents() {
    // A packed table at its size converting to a hash doubles first.
    let mut a: PhpArray = (0..8).map(|i| (int(i), Value::Int(i))).collect();
    assert_eq!(a.data().table_size(), 8);
    a.insert(int(100), Value::Int(100));
    assert!(!a.is_packed());
    assert_eq!(a.data().table_size(), 16);
    assert_eq!(a.len(), 9);
    for i in 0..8 {
        assert!(matches!(a.get(&int(i)), Some(Value::Int(n)) if *n == i));
    }
    // Sparse but within reach: the packed table grows.
    let mut b = PhpArray::new();
    for i in 0..8 {
        b.push(Value::Int(i));
    }
    b.insert(int(12), Value::Null);
    assert!(b.is_packed());
    assert_eq!(b.data().table_size(), 16);
    assert_eq!(keys(&b).last(), Some(&int(12)));
    // to_hash and a mixed table from the start.
    let mut c: PhpArray = (0..3).map(|i| (int(i), Value::Int(i))).collect();
    c.to_hash();
    assert!(!c.is_packed());
    assert_eq!(keys(&c), vec![int(0), int(1), int(2)]);
    let mut d = PhpArray::with_capacity_mixed(20);
    assert_eq!(d.data().table_size(), 32);
    d.push(Value::Null);
    assert!(!d.is_packed());
}

#[test]
fn take_and_replace_contents() {
    let mut a: PhpArray = (0..5)
        .map(|i| (s(&format!("k{i}")), Value::Int(i)))
        .collect();
    a.remove(&s("k2"));
    let d = a.make_mut();
    let all = d.take_entries();
    assert_eq!(all.len(), 4);
    assert!(d.is_empty());
    assert_eq!(d.used(), 0);
    let mut out = PhpArray::with_capacity(all.len() + 1);
    out.push(Value::Int(-1));
    for (k, v) in all {
        out.insert(k, v);
    }
    d.replace_contents(out.into_data());
    assert_eq!(d.len(), 5);
    assert_eq!(d.current().unwrap().0, int(0));
    assert_eq!(d.keys().nth(1), Some(s("k0")));
}
