use super::*;

extern "C" {
    static rhp_curl_layout: [usize; 0];
    static rhp_curl_layout_count: std::ffi::c_ulong;
}

/// The sizes and offsets shim.c reports, in its order, for the layout test.
fn c_layout() -> &'static [usize] {
    // SAFETY: shim.c defines `rhp_curl_layout_count` entries.
    unsafe { std::slice::from_raw_parts(rhp_curl_layout.as_ptr(), rhp_curl_layout_count as usize) }
}

/// The same, from this file's declarations.
fn rust_layout() -> Vec<usize> {
    use std::mem::{offset_of, size_of};
    vec![
        offset_of!(curl_version_info_data, ssl_version_num),
        offset_of!(curl_version_info_data, protocols),
        offset_of!(curl_version_info_data, iconv_ver_num),
        offset_of!(curl_version_info_data, brotli_version),
        size_of::<CURLMsg>(),
        offset_of!(CURLMsg, data),
        size_of::<curl_blob>(),
        size_of::<curl_certinfo>(),
        size_of::<curl_slist>(),
    ]
}

#[test]
fn c_layouts_match() {
    assert_eq!(c_layout(), &rust_layout()[..]);
}
