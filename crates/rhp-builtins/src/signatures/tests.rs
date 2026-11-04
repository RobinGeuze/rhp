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

/// Every builtin registered under a php name has a stub row, so its arity
/// is php's ([`crate::register`] takes it from there); the `__` engine
/// helpers have none.
#[test]
fn php_builtins_have_stubs() {
    let mut bad: Vec<_> = crate::registry()
        .keys()
        .filter(|name| function(name.as_bytes()).is_some() == name.starts_with("__"))
        .copied()
        .collect();
    bad.sort();
    assert!(bad.is_empty(), "{bad:?}");
}
