use super::*;

#[test]
fn tables_are_sorted_for_lookup() {
    let lc = |s: &str| s.to_ascii_lowercase();
    assert!(functions()
        .windows(2)
        .all(|w| lc(w[0].name) < lc(w[1].name)));
    assert!(classes().windows(2).all(|w| lc(w[0].name) < lc(w[1].name)));
    assert!(methods()
        .windows(2)
        .all(|w| (lc(w[0].class), lc(w[0].name)) < (lc(w[1].class), lc(w[1].name))));
}
