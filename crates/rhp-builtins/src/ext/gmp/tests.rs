use super::*;

#[test]
fn size_guards() {
    let two = Mpz::from_i64(2);
    // 2^(64 * INT_MAX) needs INT_MAX + 1 limbs.
    assert!(check_pow(&two, 64 * INT_MAX as u64, None).is_err());
    assert!(check_pow(&two, 64 * (INT_MAX as u64 - 2), None).is_ok());
    assert!(check_pow(&two, u64::MAX >> 1, None).is_err());
    assert!(check_pow(&Mpz::from_i64(-1), u64::MAX >> 1, None).is_ok());
    assert!(check_pow(&Mpz::from_i64(3), 1 << 40, None).is_err());
    assert!(check_shl(&Mpz::new(), u64::MAX >> 1).is_ok());
    assert!(check_shl(&two, u64::MAX >> 1).is_err());
    let at = At {
        func: "f",
        num: 1,
        name: "n",
    };
    assert!(check_estimate(fact_bits(1_000_000), at).is_ok());
    assert!(check_estimate(fact_bits(u64::MAX), at).is_err());
    assert!(check_estimate(bin_bits(&Mpz::from_i64(-5), i64::MAX as u64), at).is_ok());
    assert!(check_estimate(bin_bits(&Mpz::from_i64(3), i64::MAX as u64), at).is_ok());
    let big = Mpz::uiop(sys::mpz_pow_ui, &two, 4096);
    assert!(check_estimate(bin_bits(&big, 1 << 40), at).is_err());
}

#[test]
fn php_prefixes() {
    let mut z = Mpz::new();
    for (s, base, want) in [
        (&b"  0x1F"[..], 0, &b"31"[..]),
        (b"0o17", 0, b"15"),
        (b"0b101", 2, b"5"),
        (b"-0x10", 0, b"-16"),
        (b"0x10", 16, b"16"),
        (b"1 2 3", 10, b"123"),
    ] {
        convert_str(&mut z, s, base, None).unwrap();
        assert_eq!(z.to_str(10), want, "{s:?}");
    }
    assert!(convert_str(&mut z, b"0x", 0, None).is_err());
    assert!(convert_str(&mut z, b"0b2", 0, None).is_err());
}
