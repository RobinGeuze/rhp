use super::*;

#[test]
fn header_is_zend_string_sized() {
    assert_eq!(HDR, 24);
    assert_eq!(std::mem::size_of::<PhpStr>(), 8);
    assert_eq!(std::mem::size_of::<Option<PhpStr>>(), 8);
    assert_eq!(std::mem::size_of::<crate::Value>(), 16);
    assert_eq!(std::mem::size_of::<crate::ArrayKey>(), 16);
}

#[test]
fn construct_and_read() {
    for src in [&b""[..], b"a", b"hello world", &[0u8, 1, 2, 255][..]] {
        let a = PhpStr::from_slice(src);
        let b = PhpStr::from_vec(src.to_vec());
        let mut v = Vec::with_capacity(1000);
        v.extend_from_slice(src);
        let c = PhpStr::from_bytes(v);
        for s in [&a, &b, &c] {
            assert_eq!(s.as_bytes(), src);
            assert_eq!(s.len(), src.len());
            assert_eq!(s.ref_count(), 1);
        }
        assert_eq!(a, b);
        assert_eq!(b, c);
    }
    let big = vec![7u8; 5 << 20];
    let s = PhpStr::from_bytes(big.clone());
    assert_eq!(s.as_bytes(), &big[..]);
    assert_eq!(PhpStr::concat(b"ab", b"cd").as_bytes(), b"abcd");
    assert_eq!(PhpStr::from_bytes("x".to_string()).as_bytes(), b"x");
    assert_eq!(PhpStr::default().as_bytes(), b"");
}

#[test]
fn clone_shares_and_cow_copies() {
    let mut a = PhpStr::from("abc");
    let b = a.clone();
    assert!(a.ptr_eq(&b));
    assert_eq!(a.ref_count(), 2);
    assert!(a.get_mut().is_none());
    a.make_mut().push(b'd');
    assert!(!a.ptr_eq(&b));
    assert_eq!(a.as_bytes(), b"abcd");
    assert_eq!(b.as_bytes(), b"abc");
    assert_eq!(b.ref_count(), 1);
    drop(b);
    assert_eq!(a.ref_count(), 1);
}

#[test]
fn growth_keeps_contents() {
    let mut s = PhpStr::from("");
    let mut want = Vec::new();
    for i in 0..20_000u32 {
        let piece = format!("{i},");
        s.get_mut().unwrap().extend_from_slice(piece.as_bytes());
        want.extend_from_slice(piece.as_bytes());
    }
    assert_eq!(s.as_bytes(), &want[..]);
    for _ in 0..3 {
        s.make_mut().extend_self();
        want.extend_from_within(..);
    }
    assert_eq!(s.as_bytes(), &want[..]);
    s.make_mut().truncate(3);
    assert_eq!(s.as_bytes(), b"0,1");
    s.make_mut().resize(6, b' ');
    assert_eq!(s.as_bytes(), b"0,1   ");
    s.make_mut()[0] = b'x';
    assert_eq!(s.as_bytes(), b"x,1   ");
}

#[test]
fn block_follows_length() {
    // The block is always the length's size: a short string is php's 32.
    assert_eq!(alloc_size(0), 32);
    assert_eq!(alloc_size(7), 32);
    assert_eq!(alloc_size(8), 48);
}

#[test]
fn hash_is_cached_and_reset_by_writes() {
    let mut a = PhpStr::from("key");
    let b = PhpStr::from("key");
    let h = a.hash_value();
    assert_ne!(h, 0);
    assert_eq!(a.cached_hash(), h);
    assert_eq!(h, b.hash_value());
    a.make_mut().push(b'2');
    assert_eq!(a.cached_hash(), 0);
    assert_ne!(a.hash_value(), h);
    assert_ne!(a, b);
    let c = PhpStr::from("key2");
    c.hash_value();
    assert_eq!(a, c);
    let mut m = std::collections::HashMap::new();
    m.insert(a.clone(), 1);
    assert_eq!(m.get(&c), Some(&1));
}

#[test]
fn ordering_is_bytewise() {
    let mut v: Vec<PhpStr> = ["b", "a", "ab", ""].into_iter().map(PhpStr::from).collect();
    v.sort();
    let got: Vec<&[u8]> = v.iter().map(PhpStr::as_bytes).collect();
    assert_eq!(got, [&b""[..], b"a", b"ab", b"b"]);
    assert_eq!(format!("{:?}", PhpStr::from("a\"b")), "\"a\\\"b\"");
}

#[test]
fn builder_fills_its_block() {
    let mut b = PhpStr::builder(6);
    b.push(b"ab");
    b.push(b"cd");
    let s = b.finish();
    assert_eq!(s.as_bytes(), b"abcd");
    let mut b = PhpStr::builder(3);
    b.push(b"xyz");
    assert_eq!(b.finish().as_bytes(), b"xyz");
    drop(PhpStr::builder(100));
}

#[test]
fn int_digits_match_to_string() {
    for n in [0, 1, -1, 9, 10, -10, 1234567890, i64::MAX, i64::MIN] {
        assert_eq!(crate::conv::int_str(n).as_bytes(), n.to_string().as_bytes());
    }
}
