use super::*;

#[test]
fn derived_values() {
    assert_eq!(
        PHP_VERSION_ID,
        80500 + PHP_RELEASE_VERSION,
        "PHP_VERSION_ID follows release!()"
    );
    assert_eq!(parse_int("12"), 12);
    assert_eq!(parse_int("0"), 0);
    assert_eq!(
        PHP_VERSION,
        format!(
            "{PHP_MAJOR_VERSION}.{PHP_MINOR_VERSION}.{PHP_RELEASE_VERSION}{PHP_EXTRA_VERSION}"
        )
    );
    assert_eq!(ZEND_VERSION, format!("4.5.{PHP_RELEASE_VERSION}"));
    assert!(EXTENSION_DIR.ends_with(PHP_API));
    // ZLIB_VERNUM packs the dotted version one hex digit per component.
    let z: Vec<i64> = ZLIB_VERSION
        .split('.')
        .map(|p| p.parse().unwrap())
        .collect();
    let at = |i: usize| z.get(i).copied().unwrap_or(0) & 0xF;
    assert_eq!(
        ZLIB_VERNUM,
        (at(0) << 12) | (at(1) << 8) | (at(2) << 4) | at(3)
    );
}
