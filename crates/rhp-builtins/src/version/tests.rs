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
        format!("{PHP_MAJOR_VERSION}.{PHP_MINOR_VERSION}.{PHP_RELEASE_VERSION}{PHP_EXTRA_VERSION}")
    );
    assert_eq!(ZEND_VERSION, format!("4.5.{PHP_RELEASE_VERSION}"));
    assert!(EXTENSION_DIR.ends_with(PHP_API));
    // ZLIB_VERNUM packs the dotted version one hex digit per component.
    assert_eq!(vernum("1.2.12"), 0x12c0);
    assert_eq!(vernum("1.3.1"), 0x1310);
    assert_eq!(vernum("1.3"), 0x1300);
    assert_eq!(vernum("1.2.11.1-motley"), 0x12b1);
    assert!(zlib_vernum() >= 0x1200, "{}", zlib_version());
}
