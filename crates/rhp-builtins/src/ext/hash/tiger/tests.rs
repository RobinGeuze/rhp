use super::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn three_pass_byte_order_matches_php() {
    // Known php `tiger192,3` values: proves S-boxes, little-endian words and
    // output byte order are all correct before relying on 4-pass.
    assert_eq!(
        hex(&digest_bytes(tiger_hash(b"", 3))),
        "3293ac630c13f0245f92bbb1766e16167a4e58492dde73f3"
    );
    assert_eq!(
        hex(&digest_bytes(tiger_hash(b"abc", 3))),
        "2aab1484e8c158f2bfb8c5ff41b57a525129131c957b5f93"
    );
}

#[test]
fn four_pass_matches_php() {
    assert_eq!(
        hex(&tiger4(b"", 24)),
        "24cc78a7f6ff3546e7984e59695ca13d804e0b686e255194"
    );
    assert_eq!(
        hex(&tiger4(b"abc", 24)),
        "538883c8fc5f28250299018e66bdf4fdb5ef7b65f2e91753"
    );
    // Truncated widths are a prefix of the 192-bit digest.
    assert_eq!(tiger4(b"abc", 16), tiger4(b"abc", 24)[..16]);
    assert_eq!(tiger4(b"abc", 20), tiger4(b"abc", 24)[..20]);
}
