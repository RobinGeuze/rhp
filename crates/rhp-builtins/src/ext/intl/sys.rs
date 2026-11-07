//! The ICU4C functions rhp calls, declared by hand (no bindgen), plus
//! `shim.c`'s constant table. build.rs links the ICU that php links
//! (docs/design/libraries.md) and compiles the shim against its headers.
//!
//! ICU renames every C symbol with its major version (`ubrk_open_79`): the
//! `icu!` macro appends `_<U_ICU_VERSION_MAJOR_NUM>` from the linked
//! `unicode/uvernum.h`. The enum values below are part of ICU's stable C
//! API; the shim's `_Static_assert`s check them against the headers.

#![allow(
    non_snake_case,
    non_camel_case_types,
    dead_code,
    clippy::upper_case_acronyms
)]

use std::ffi::{c_char, c_longlong};

macro_rules! opaque {
    ($($name:ident),*) => {$(
        #[repr(C)]
        pub struct $name {
            _p: [u8; 0],
            _m: std::marker::PhantomData<(*mut u8, std::marker::PhantomPinned)>,
        }
    )*};
}

opaque!(
    UBreakIterator,
    UText,
    UStringSearch,
    UCollator,
    UNormalizer2,
    UIDNA,
    UNumberFormat,
    UEnumeration,
    UFieldPosition
);

pub type UErrorCode = i32;
pub type UChar = u16;
pub type UChar32 = i32;
/// `UBool` is a one-byte boolean in C (ICU 68+).
pub type UBool = i8;

pub const U_ZERO_ERROR: UErrorCode = 0;
pub const U_ILLEGAL_ARGUMENT_ERROR: UErrorCode = 1;
pub const U_INVALID_CHAR_FOUND: UErrorCode = 10;
pub const U_BUFFER_OVERFLOW_ERROR: UErrorCode = 15;
pub const U_UNSUPPORTED_ERROR: UErrorCode = 16;
pub const U_STRING_NOT_TERMINATED_WARNING: UErrorCode = -124;

/// `U_FAILURE(code)`.
pub fn failure(code: UErrorCode) -> bool {
    code > U_ZERO_ERROR
}

/// `UBRK_CHARACTER`.
pub const UBRK_CHARACTER: i32 = 0;
/// `UBRK_DONE`.
pub const UBRK_DONE: i32 = -1;
/// `USEARCH_DONE`.
pub const USEARCH_DONE: i32 = -1;
/// `UCOL_STRENGTH`.
pub const UCOL_STRENGTH: i32 = 5;
/// `UCOL_SECONDARY`.
pub const UCOL_SECONDARY: i32 = 1;
/// `UCOL_EQUAL`.
pub const UCOL_EQUAL: i32 = 0;

/// `ULOC_FULLNAME_CAPACITY`.
pub const ULOC_FULLNAME_CAPACITY: usize = 157;
/// `ULOC_ACCEPT_FAILED`.
pub const ULOC_ACCEPT_FAILED: i32 = 0;

/// `UParseError` (`U_PARSE_CONTEXT_LEN` 16).
#[repr(C)]
#[derive(Default)]
pub struct UParseError {
    pub line: i32,
    pub offset: i32,
    pub pre_context: [UChar; 16],
    pub post_context: [UChar; 16],
}

/// `UIDNAInfo` (stable since ICU 4.6).
#[repr(C)]
pub struct UIDNAInfo {
    pub size: i16,
    pub is_transitional_different: UBool,
    pub reserved_b3: UBool,
    pub errors: u32,
    pub reserved_i2: i32,
    pub reserved_i3: i32,
}

impl UIDNAInfo {
    /// `UIDNA_INFO_INITIALIZER`.
    pub fn new() -> UIDNAInfo {
        UIDNAInfo {
            size: std::mem::size_of::<UIDNAInfo>() as i16,
            is_transitional_different: 0,
            reserved_b3: 0,
            errors: 0,
            reserved_i2: 0,
            reserved_i3: 0,
        }
    }
}

macro_rules! icu {
    ($( pub fn $name:ident ( $($a:ident : $t:ty),* $(,)? ) $(-> $r:ty)? ; )*) => {
        unsafe extern "C" { $(
            #[link_name = concat!(
                stringify!($name), "_", env!("RHP_U_ICU_VERSION_MAJOR_NUM")
            )]
            pub fn $name($($a: $t),*) $(-> $r)?;
        )* }
    };
}

icu! {
    pub fn u_errorName(code: UErrorCode) -> *const c_char;
    pub fn u_strFromUTF8(
        dest: *mut UChar, cap: i32, dest_len: *mut i32,
        src: *const c_char, src_len: i32, status: *mut UErrorCode,
    ) -> *mut UChar;
    pub fn u_strToUTF8(
        dest: *mut c_char, cap: i32, dest_len: *mut i32,
        src: *const UChar, src_len: i32, status: *mut UErrorCode,
    ) -> *mut c_char;
    pub fn u_countChar32(s: *const UChar, len: i32) -> i32;

    pub fn ubrk_open(
        kind: i32, locale: *const c_char, text: *const UChar, len: i32,
        status: *mut UErrorCode,
    ) -> *mut UBreakIterator;
    pub fn ubrk_clone(bi: *const UBreakIterator, status: *mut UErrorCode) -> *mut UBreakIterator;
    pub fn ubrk_close(bi: *mut UBreakIterator);
    pub fn ubrk_setText(bi: *mut UBreakIterator, text: *const UChar, len: i32, status: *mut UErrorCode);
    pub fn ubrk_setUText(bi: *mut UBreakIterator, text: *mut UText, status: *mut UErrorCode);
    pub fn ubrk_first(bi: *mut UBreakIterator) -> i32;
    pub fn ubrk_last(bi: *mut UBreakIterator) -> i32;
    pub fn ubrk_next(bi: *mut UBreakIterator) -> i32;
    pub fn ubrk_previous(bi: *mut UBreakIterator) -> i32;
    pub fn ubrk_following(bi: *mut UBreakIterator, offset: i32) -> i32;
    pub fn ubrk_current(bi: *const UBreakIterator) -> i32;
    pub fn ubrk_isBoundary(bi: *mut UBreakIterator, offset: i32) -> UBool;

    pub fn utext_openUTF8(ut: *mut UText, s: *const c_char, len: i64, status: *mut UErrorCode) -> *mut UText;
    pub fn utext_close(ut: *mut UText) -> *mut UText;

    pub fn usearch_open(
        pattern: *const UChar, pattern_len: i32, text: *const UChar, text_len: i32,
        locale: *const c_char, bi: *mut UBreakIterator, status: *mut UErrorCode,
    ) -> *mut UStringSearch;
    pub fn usearch_close(s: *mut UStringSearch);
    pub fn usearch_getCollator(s: *const UStringSearch) -> *mut UCollator;
    pub fn usearch_reset(s: *mut UStringSearch);
    pub fn usearch_setOffset(s: *mut UStringSearch, pos: i32, status: *mut UErrorCode);
    pub fn usearch_next(s: *mut UStringSearch, status: *mut UErrorCode) -> i32;
    pub fn usearch_last(s: *mut UStringSearch, status: *mut UErrorCode) -> i32;

    pub fn ucol_open(locale: *const c_char, status: *mut UErrorCode) -> *mut UCollator;
    pub fn ucol_close(c: *mut UCollator);
    pub fn ucol_setAttribute(c: *mut UCollator, attr: i32, value: i32, status: *mut UErrorCode);
    pub fn ucol_strcoll(
        c: *const UCollator, a: *const UChar, a_len: i32, b: *const UChar, b_len: i32,
    ) -> i32;
    pub fn ucol_getAttribute(c: *const UCollator, attr: i32, status: *mut UErrorCode) -> i32;
    pub fn ucol_getStrength(c: *const UCollator) -> i32;
    pub fn ucol_setStrength(c: *mut UCollator, strength: i32);
    pub fn ucol_getLocaleByType(c: *const UCollator, kind: i32, status: *mut UErrorCode) -> *const c_char;
    pub fn ucol_getSortKey(
        c: *const UCollator, src: *const UChar, len: i32, result: *mut u8, cap: i32,
    ) -> i32;

    pub fn u_isspace(c: UChar32) -> UBool;

    pub fn uloc_getDefault() -> *const c_char;
    pub fn uloc_getLanguage(l: *const c_char, buf: *mut c_char, cap: i32, status: *mut UErrorCode) -> i32;
    pub fn uloc_getScript(l: *const c_char, buf: *mut c_char, cap: i32, status: *mut UErrorCode) -> i32;
    pub fn uloc_getCountry(l: *const c_char, buf: *mut c_char, cap: i32, status: *mut UErrorCode) -> i32;
    pub fn uloc_getVariant(l: *const c_char, buf: *mut c_char, cap: i32, status: *mut UErrorCode) -> i32;
    pub fn uloc_canonicalize(l: *const c_char, buf: *mut c_char, cap: i32, status: *mut UErrorCode) -> i32;
    pub fn uloc_addLikelySubtags(l: *const c_char, buf: *mut c_char, cap: i32, status: *mut UErrorCode) -> i32;
    pub fn uloc_minimizeSubtags(l: *const c_char, buf: *mut c_char, cap: i32, status: *mut UErrorCode) -> i32;
    pub fn uloc_getISO3Language(l: *const c_char) -> *const c_char;
    pub fn uloc_isRightToLeft(l: *const c_char) -> UBool;
    pub fn uloc_getDisplayLanguage(
        l: *const c_char, disp: *const c_char, buf: *mut UChar, cap: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn uloc_getDisplayScript(
        l: *const c_char, disp: *const c_char, buf: *mut UChar, cap: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn uloc_getDisplayCountry(
        l: *const c_char, disp: *const c_char, buf: *mut UChar, cap: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn uloc_getDisplayVariant(
        l: *const c_char, disp: *const c_char, buf: *mut UChar, cap: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn uloc_getDisplayName(
        l: *const c_char, disp: *const c_char, buf: *mut UChar, cap: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn uloc_openKeywords(l: *const c_char, status: *mut UErrorCode) -> *mut UEnumeration;
    pub fn uloc_getKeywordValue(
        l: *const c_char, key: *const c_char, buf: *mut c_char, cap: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn uloc_acceptLanguageFromHTTP(
        result: *mut c_char, cap: i32, out: *mut i32, http: *const c_char,
        available: *mut UEnumeration, status: *mut UErrorCode,
    ) -> i32;
    pub fn ures_openAvailableLocales(package: *const c_char, status: *mut UErrorCode) -> *mut UEnumeration;
    pub fn uenum_next(e: *mut UEnumeration, len: *mut i32, status: *mut UErrorCode) -> *const c_char;
    pub fn uenum_close(e: *mut UEnumeration);

    pub fn unum_open(
        style: i32, pattern: *const UChar, pattern_len: i32, locale: *const c_char,
        perr: *mut UParseError, status: *mut UErrorCode,
    ) -> *mut UNumberFormat;
    pub fn unum_close(f: *mut UNumberFormat);
    pub fn unum_clone(f: *const UNumberFormat, status: *mut UErrorCode) -> *mut UNumberFormat;
    pub fn unum_format(
        f: *const UNumberFormat, n: i32, buf: *mut UChar, cap: i32, pos: *mut UFieldPosition,
        status: *mut UErrorCode,
    ) -> i32;
    pub fn unum_formatInt64(
        f: *const UNumberFormat, n: i64, buf: *mut UChar, cap: i32, pos: *mut UFieldPosition,
        status: *mut UErrorCode,
    ) -> i32;
    pub fn unum_formatDouble(
        f: *const UNumberFormat, n: f64, buf: *mut UChar, cap: i32, pos: *mut UFieldPosition,
        status: *mut UErrorCode,
    ) -> i32;
    pub fn unum_formatDoubleCurrency(
        f: *const UNumberFormat, n: f64, currency: *mut UChar, buf: *mut UChar, cap: i32,
        pos: *mut UFieldPosition, status: *mut UErrorCode,
    ) -> i32;
    pub fn unum_parse(
        f: *const UNumberFormat, text: *const UChar, len: i32, pos: *mut i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn unum_parseInt64(
        f: *const UNumberFormat, text: *const UChar, len: i32, pos: *mut i32, status: *mut UErrorCode,
    ) -> i64;
    pub fn unum_parseDouble(
        f: *const UNumberFormat, text: *const UChar, len: i32, pos: *mut i32, status: *mut UErrorCode,
    ) -> f64;
    pub fn unum_parseDoubleCurrency(
        f: *const UNumberFormat, text: *const UChar, len: i32, pos: *mut i32, currency: *mut UChar,
        status: *mut UErrorCode,
    ) -> f64;
    pub fn unum_getAttribute(f: *const UNumberFormat, attr: i32) -> i32;
    pub fn unum_setAttribute(f: *mut UNumberFormat, attr: i32, value: i32);
    pub fn unum_getDoubleAttribute(f: *const UNumberFormat, attr: i32) -> f64;
    pub fn unum_setDoubleAttribute(f: *mut UNumberFormat, attr: i32, value: f64);
    pub fn unum_getTextAttribute(
        f: *const UNumberFormat, tag: i32, buf: *mut UChar, cap: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn unum_setTextAttribute(
        f: *mut UNumberFormat, tag: i32, value: *const UChar, len: i32, status: *mut UErrorCode,
    );
    pub fn unum_getSymbol(
        f: *const UNumberFormat, sym: i32, buf: *mut UChar, cap: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn unum_setSymbol(
        f: *mut UNumberFormat, sym: i32, value: *const UChar, len: i32, status: *mut UErrorCode,
    );
    pub fn unum_toPattern(
        f: *const UNumberFormat, localized: UBool, buf: *mut UChar, cap: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn unum_applyPattern(
        f: *mut UNumberFormat, localized: UBool, pattern: *const UChar, len: i32,
        perr: *mut UParseError, status: *mut UErrorCode,
    );
    pub fn unum_getLocaleByType(f: *const UNumberFormat, kind: i32, status: *mut UErrorCode) -> *const c_char;

    pub fn unorm2_getNFCInstance(status: *mut UErrorCode) -> *const UNormalizer2;
    pub fn unorm2_getNFDInstance(status: *mut UErrorCode) -> *const UNormalizer2;
    pub fn unorm2_getNFKCInstance(status: *mut UErrorCode) -> *const UNormalizer2;
    pub fn unorm2_getNFKDInstance(status: *mut UErrorCode) -> *const UNormalizer2;
    pub fn unorm2_getNFKCCasefoldInstance(status: *mut UErrorCode) -> *const UNormalizer2;
    pub fn unorm2_normalize(
        n: *const UNormalizer2, src: *const UChar, len: i32, dest: *mut UChar, cap: i32,
        status: *mut UErrorCode,
    ) -> i32;
    pub fn unorm2_isNormalized(
        n: *const UNormalizer2, s: *const UChar, len: i32, status: *mut UErrorCode,
    ) -> UBool;
    pub fn unorm2_getRawDecomposition(
        n: *const UNormalizer2, c: UChar32, dest: *mut UChar, cap: i32, status: *mut UErrorCode,
    ) -> i32;

    pub fn uidna_openUTS46(options: u32, status: *mut UErrorCode) -> *mut UIDNA;
    pub fn uidna_close(idna: *mut UIDNA);
    pub fn uidna_nameToASCII_UTF8(
        idna: *const UIDNA, name: *const c_char, len: i32, dest: *mut c_char, cap: i32,
        info: *mut UIDNAInfo, status: *mut UErrorCode,
    ) -> i32;
    pub fn uidna_nameToUnicodeUTF8(
        idna: *const UIDNA, name: *const c_char, len: i32, dest: *mut c_char, cap: i32,
        info: *mut UIDNAInfo, status: *mut UErrorCode,
    ) -> i32;
}

/// A row of the shim's constant table: an int, or a string when `str` is
/// non-null.
#[repr(C)]
pub struct IntlConst {
    pub name: *const c_char,
    pub value: c_longlong,
    pub str: *const c_char,
}

/// A row of the shim's class-constant table; `kind` 0 int, 1 string, 2
/// null.
#[repr(C)]
pub struct IntlClassConst {
    pub class: *const c_char,
    pub name: *const c_char,
    pub kind: i32,
    pub value: c_longlong,
    pub str: *const c_char,
    pub attr: *const c_char,
}

unsafe extern "C" {
    /// php's global intl constants (consts.h), ended by a null name.
    pub fn rhp_intl_constants() -> *const IntlConst;
    /// `sizeof(UIDNAInfo)`, for the layout test.
    pub fn rhp_intl_uidna_info_size() -> usize;
    /// The intl classes' constants (class_consts.h), ended by a null class.
    pub fn rhp_intl_class_constants() -> *const IntlClassConst;
    /// `UNUM_FORMAT_SYMBOL_COUNT` (it grows with ICU).
    pub fn rhp_intl_unum_format_symbol_count() -> i32;
}
