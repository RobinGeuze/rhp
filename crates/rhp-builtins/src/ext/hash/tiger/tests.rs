use super::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn three_pass_byte_order_matches_php() {
    // Known php `tiger192,3` values: proves S-boxes, little-endian words and
    // output byte order are all correct before relying on 4-pass. hash()
    // runs 3-pass Tiger through the `tiger` crate, so no sweep reaches this
    // path.
    assert_eq!(
        hex(&digest_bytes(tiger_hash(b"", 3))),
        "3293ac630c13f0245f92bbb1766e16167a4e58492dde73f3"
    );
    assert_eq!(
        hex(&digest_bytes(tiger_hash(b"abc", 3))),
        "2aab1484e8c158f2bfb8c5ff41b57a525129131c957b5f93"
    );
}
