use super::*;

#[test]
fn tag_tables_end_like_php() {
    for t in [
        tags::TAG_TABLE_IFD,
        tags::TAG_TABLE_GPS,
        tags::TAG_TABLE_IOP,
    ] {
        assert_eq!(tagname(0xFFFF, t), Some("No tag value"));
        assert_eq!(tagname(0xFFFE, t), Some("Computed value"));
    }
    assert_eq!(tagname(0x010F, tags::TAG_TABLE_IFD), Some("Make"));
    assert_eq!(
        tagname_key(0x1234, tags::TAG_TABLE_IFD),
        b"UndefinedTag:0x1234"
    );
}

#[test]
fn rational_conversions_follow_exif_c() {
    // SRATIONAL -1/2: php's numerator is unsigned in the float conversion.
    let v = [0xFF, 0xFF, 0xFF, 0xFF, 2, 0, 0, 0];
    assert_eq!(
        convert_any_format(&v, 0, u32::from(FMT_SRATIONAL), false),
        4294967295.0 / 2.0
    );
    assert_eq!(
        convert_any_to_int(&v, 0, u32::from(FMT_SRATIONAL), false),
        0
    );
    let v = [0, 0, 0, 0x80, 0xFF, 0xFF, 0xFF, 0xFF];
    assert_eq!(
        convert_any_to_int(&v, 0, u32::from(FMT_SRATIONAL), false),
        i32::MAX as usize
    );
}

#[test]
fn offset_info_bounds() {
    let mem: Rc<[u8]> = Rc::from(vec![0u8; 16]);
    let info = OffsetInfo::new(mem, -8, 0, 16);
    assert_eq!(info.try_get(8, 8), Some(0));
    assert_eq!(info.try_get(4, 4), None);
    assert_eq!(info.try_get(16, 9), None);
    assert!(info.contains(12, 4));
    assert!(!info.contains(12, 5));
}

#[test]
fn section_lists() {
    assert_eq!(section_list(FOUND_IFD0 | FOUND_EXIF), b"IFD0, EXIF");
    assert_eq!(section_list(0), b"");
}
