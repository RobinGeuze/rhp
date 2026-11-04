use super::*;

#[test]
fn hash_then_verify() {
    let h = hash(2, 1024, 1, b"password", b"somesalt", 32, Type::Id).unwrap();
    assert_eq!(
        h,
        b"$argon2id$v=19$m=1024,t=2,p=1$c29tZXNhbHQ$7FfsnA6vUe7qLpL/3Kqc3uR48ZJyFbUVt7jWZlf0Htk"
            .to_vec()
    );
    let c = std::ffi::CString::new(h).unwrap();
    assert!(verify(&c, b"password", Type::Id));
    assert!(!verify(&c, b"Password", Type::Id));
    assert!(!verify(&c, b"password", Type::I));
}

#[test]
fn errors() {
    assert_eq!(hash(1, 8, 2, b"p", b"somesalt", 32, Type::I), Err(-14));
    assert_eq!(error_message(-14), "Memory cost is too small");
    assert_eq!(memory_range(), (8, 0xFFFF_FFFF));
    assert_eq!(max_lanes(), 0xFF_FFFF);
}
