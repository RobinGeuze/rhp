//! Identification runs through rhp's PCRE, so the goldens test it
//! (tests/golden/ext/fileinfo); these need no hooks.

use super::*;

#[test]
fn reports_libmagic_5_46() {
    assert_eq!(version(), 546);
}

#[test]
fn a_missing_database_fails_to_load() {
    let mut m = Magic::open(MAGIC_NONE).unwrap();
    assert!(!m.load(Some(b"/nonexistent/magic")));
}

#[test]
fn flags_round_trip() {
    let mut m = Magic::open(MAGIC_MIME).unwrap();
    assert_eq!(m.flags(), MAGIC_MIME);
    assert!(m.set_flags(MAGIC_EXTENSION));
    assert_eq!(m.flags(), MAGIC_EXTENSION);
}
