//! ext/standard/image.c: the `IMAGETYPE_*` constants (php_image.h).

use rhp_values::Value;

pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    let i = Value::Int;
    vec![
        ("IMAGETYPE_GIF", i(1)),
        ("IMAGETYPE_JPEG", i(2)),
        ("IMAGETYPE_PNG", i(3)),
        ("IMAGETYPE_SWF", i(4)),
        ("IMAGETYPE_PSD", i(5)),
        ("IMAGETYPE_BMP", i(6)),
        ("IMAGETYPE_TIFF_II", i(7)),
        ("IMAGETYPE_TIFF_MM", i(8)),
        ("IMAGETYPE_JPC", i(9)),
        ("IMAGETYPE_JP2", i(10)),
        ("IMAGETYPE_JPX", i(11)),
        ("IMAGETYPE_JB2", i(12)),
        ("IMAGETYPE_SWC", i(13)),
        ("IMAGETYPE_IFF", i(14)),
        ("IMAGETYPE_WBMP", i(15)),
        ("IMAGETYPE_JPEG2000", i(9)),
        ("IMAGETYPE_XBM", i(16)),
        ("IMAGETYPE_ICO", i(17)),
        ("IMAGETYPE_WEBP", i(18)),
        ("IMAGETYPE_AVIF", i(19)),
        ("IMAGETYPE_HEIF", i(20)),
        ("IMAGETYPE_SVG", i(21)),
        ("IMAGETYPE_UNKNOWN", i(0)),
        ("IMAGETYPE_COUNT", i(22)),
    ]
}
