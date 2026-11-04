use super::*;

#[test]
fn blacklist_entries_are_prefixes_with_globs() {
    let e = vec![
        b"/a/b".to_vec(),
        b"/t/?x.inc".to_vec(),
        b"/w/*/d".to_vec(),
        b"/z/**.php".to_vec(),
    ];
    assert!(blacklisted(&e, b"/a/b/c.php"));
    assert!(blacklisted(&e, b"/a/bc.php"));
    assert!(blacklisted(&e, b"/t/1x.inc"));
    assert!(!blacklisted(&e, b"/t/12x.inc"));
    assert!(blacklisted(&e, b"/w/q/d/e.php"));
    assert!(!blacklisted(&e, b"/w/q/r/d"));
    assert!(blacklisted(&e, b"/z/q/r.php"));
    assert!(!blacklisted(&e, b"/y"));
}

#[test]
fn expands_like_expand_filepath() {
    assert_eq!(expand_path(b"./current.php", b"/d"), b"/d/current.php");
    assert_eq!(expand_path(b"a/../b", b"/d"), b"/d/b");
    assert_eq!(expand_path(b"/tmp/path/*/x", b"/d"), b"/tmp/path/*/x");
}

#[test]
fn jit_numbers() {
    assert_eq!(parse_jit_num("1205"), Some((true, 0, 5, 6)));
    assert_eq!(parse_jit_num("1255"), Some((true, 5, 5, 6)));
    assert_eq!(parse_jit_num("0"), Some((false, 0, 0, 0)));
    assert_eq!(parse_jit_num("1240"), None);
}

#[test]
fn php_hash() {
    // zend_inline_hash_func("a") = 5381 * 33 + 97.
    assert_eq!(zend_hash(b"a"), (5381 * 33 + 97) | (1 << 63));
}
