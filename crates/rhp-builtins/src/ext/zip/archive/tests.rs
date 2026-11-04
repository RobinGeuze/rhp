use super::{cwd_expand, make_relative_path};

#[test]
fn extraction_paths_stay_below_the_target() {
    let rel = |p: &[u8]| make_relative_path(&cwd_expand(p)).unwrap();
    assert_eq!(rel(b"../../a/b.txt"), b"a/b.txt");
    assert_eq!(rel(b"/etc/passwd"), b"etc/passwd");
    assert_eq!(rel(b"dir/"), b"dir/");
    assert_eq!(rel(b"a/./b/../c"), b"a/c");
}
