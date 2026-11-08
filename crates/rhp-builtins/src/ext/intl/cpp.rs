//! `icu.cpp`'s functions: the ICU C++ intl needs (MessageFormat argument
//! types, TimeZone, Calendar). Objects are `*mut c_void` (`TimeZone *`,
//! `Calendar *`); strings come back malloc'd ([`take_ustr`]).

#![allow(dead_code)]

use std::ffi::{c_char, c_void};

use super::sys::{UChar, UEnumeration, UErrorCode};

#[repr(C)]
pub struct MsgType {
    pub number: i32,
    pub name: *mut UChar,
    pub name_len: i32,
    pub ty: i32,
}

#[repr(C)]
pub struct MsgArg {
    pub name: *const UChar,
    pub name_len: i32,
    pub ty: i32,
    pub d: f64,
    pub i: i64,
    pub s: *const UChar,
    pub s_len: i32,
}

#[repr(C)]
pub struct MsgParsed {
    pub ty: i32,
    pub d: f64,
    pub i: i64,
    pub s: *mut UChar,
    pub s_len: i32,
}

/// `Formattable::Type`.
pub const K_DATE: i32 = 0;
pub const K_DOUBLE: i32 = 1;
pub const K_LONG: i32 = 2;
pub const K_STRING: i32 = 3;
pub const K_ARRAY: i32 = 4;
pub const K_INT64: i32 = 5;
pub const K_OBJECT: i32 = 6;

unsafe extern "C" {
    pub fn rhp_intl_free(p: *mut c_void);

    pub fn rhp_msgfmt_uses_named(fmt: *const c_void) -> i32;
    pub fn rhp_msgfmt_arg_count(fmt: *const c_void) -> i32;
    pub fn rhp_msgfmt_arg_types(
        fmt: *mut c_void,
        out: *mut *mut MsgType,
        code: *mut i32,
        msg: *mut *const c_char,
    ) -> i32;
    pub fn rhp_msgfmt_free_types(t: *mut MsgType, n: i32);
    pub fn rhp_msgfmt_has_date_formats(fmt: *mut c_void) -> i32;
    pub fn rhp_msgfmt_set_timezone(fmt: *mut c_void, tz: *const c_void);
    pub fn rhp_msgfmt_format(
        fmt: *mut c_void,
        args: *const MsgArg,
        n: i32,
        out_len: *mut i32,
        status: *mut UErrorCode,
    ) -> *mut UChar;
    pub fn rhp_msgfmt_parse(
        fmt: *const c_void,
        src: *const UChar,
        len: i32,
        count: *mut i32,
        status: *mut UErrorCode,
    ) -> *mut MsgParsed;

    pub fn rhp_tz_create(id: *const UChar, len: i32) -> *mut c_void;
    pub fn rhp_tz_create_default() -> *mut c_void;
    pub fn rhp_tz_gmt() -> *const c_void;
    pub fn rhp_tz_unknown() -> *const c_void;
    pub fn rhp_tz_clone(tz: *const c_void) -> *mut c_void;
    pub fn rhp_tz_delete(tz: *mut c_void);
    pub fn rhp_tz_is_unknown(tz: *const c_void) -> i32;
    pub fn rhp_tz_equals(a: *const c_void, b: *const c_void) -> i32;
    pub fn rhp_tz_get_id(tz: *const c_void, len: *mut i32) -> *mut UChar;
    pub fn rhp_tz_raw_offset(tz: *const c_void) -> i32;
    pub fn rhp_tz_use_daylight(tz: *const c_void) -> i32;
    pub fn rhp_tz_dst_savings(tz: *const c_void) -> i32;
    pub fn rhp_tz_has_same_rules(a: *const c_void, b: *const c_void) -> i32;
    pub fn rhp_tz_get_offset(
        tz: *const c_void,
        date: f64,
        local: i32,
        raw: *mut i32,
        dst: *mut i32,
        status: *mut UErrorCode,
    );
    pub fn rhp_tz_display_name(
        tz: *const c_void,
        daylight: i32,
        ty: i32,
        locale: *const c_char,
        len: *mut i32,
    ) -> *mut UChar;
    pub fn rhp_cal_now() -> f64;
    pub fn rhp_tz_count_equivalent(id: *const UChar, len: i32) -> i32;
    pub fn rhp_tz_equivalent_id(
        id: *const UChar,
        len: i32,
        index: i32,
        out_len: *mut i32,
    ) -> *mut UChar;
    pub fn rhp_tz_canonical_id(
        id: *const UChar,
        len: i32,
        is_system: *mut i32,
        out_len: *mut i32,
        status: *mut UErrorCode,
    ) -> *mut UChar;
    pub fn rhp_tz_region(
        id: *const UChar,
        len: i32,
        out: *mut c_char,
        cap: i32,
        status: *mut UErrorCode,
    ) -> i32;
    pub fn rhp_tz_data_version(status: *mut UErrorCode) -> *const c_char;
    pub fn rhp_tz_iana_id(
        id: *const UChar,
        len: i32,
        out_len: *mut i32,
        status: *mut UErrorCode,
    ) -> *mut UChar;
    pub fn rhp_tz_windows_id(
        id: *const UChar,
        len: i32,
        out_len: *mut i32,
        status: *mut UErrorCode,
    ) -> *mut UChar;
    pub fn rhp_tz_id_for_windows(
        id: *const UChar,
        len: i32,
        region: *const c_char,
        out_len: *mut i32,
        status: *mut UErrorCode,
    ) -> *mut UChar;
    pub fn rhp_tz_enum(kind: i32, s: *const c_char, n: i32) -> *mut UEnumeration;
    pub fn rhp_tz_enum_ids(
        ty: i32,
        region: *const c_char,
        offset: *const i32,
        status: *mut UErrorCode,
    ) -> *mut UEnumeration;
}

unsafe extern "C" {
    pub fn rhp_cal_create(
        tz: *mut c_void,
        locale: *const c_char,
        status: *mut UErrorCode,
    ) -> *mut c_void;
    pub fn rhp_gcal_create(
        tz: *mut c_void,
        locale: *const c_char,
        status: *mut UErrorCode,
    ) -> *mut c_void;
    pub fn rhp_gcal_create_fields(n: i32, a: *const i32, status: *mut UErrorCode) -> *mut c_void;
    pub fn rhp_cal_is_gregorian(c: *const c_void) -> i32;
    pub fn rhp_cal_delete(c: *mut c_void);
    pub fn rhp_cal_clone(c: *const c_void) -> *mut c_void;
    pub fn rhp_cal_adopt_tz(c: *mut c_void, tz: *mut c_void);
    pub fn rhp_cal_get_tz(c: *const c_void) -> *const c_void;
    pub fn rhp_cal_field_op(c: *mut c_void, op: i32, field: i32, status: *mut UErrorCode) -> i32;
    pub fn rhp_cal_get_time(c: *mut c_void, status: *mut UErrorCode) -> f64;
    pub fn rhp_cal_set_time(c: *mut c_void, t: f64, status: *mut UErrorCode);
    pub fn rhp_cal_add(c: *mut c_void, field: i32, amount: i32, status: *mut UErrorCode);
    pub fn rhp_cal_roll(c: *mut c_void, field: i32, amount: i32, status: *mut UErrorCode);
    pub fn rhp_cal_set(c: *mut c_void, n: i32, a: *const i32);
    pub fn rhp_cal_cmp(c: *mut c_void, o: *const c_void, op: i32, status: *mut UErrorCode) -> i32;
    pub fn rhp_cal_is_equivalent(c: *const c_void, o: *const c_void) -> i32;
    pub fn rhp_cal_clear(c: *mut c_void, field: i32);
    pub fn rhp_cal_field_difference(
        c: *mut c_void,
        when: f64,
        field: i32,
        status: *mut UErrorCode,
    ) -> i32;
    pub fn rhp_cal_day_of_week_type(c: *mut c_void, dow: i32, status: *mut UErrorCode) -> i32;
    pub fn rhp_cal_weekend_transition(c: *mut c_void, dow: i32, status: *mut UErrorCode) -> i32;
    pub fn rhp_cal_first_day_of_week(c: *mut c_void, status: *mut UErrorCode) -> i32;
    pub fn rhp_cal_set_first_day_of_week(c: *mut c_void, dow: i32);
    pub fn rhp_cal_get_locale(
        c: *mut c_void,
        ty: i32,
        buf: *mut c_char,
        cap: i32,
        status: *mut UErrorCode,
    );
    pub fn rhp_cal_minimal_days(c: *mut c_void) -> i32;
    pub fn rhp_cal_set_minimal_days(c: *mut c_void, n: i32);
    pub fn rhp_cal_type(c: *const c_void) -> *const c_char;
    pub fn rhp_cal_in_daylight(c: *mut c_void, status: *mut UErrorCode) -> i32;
    pub fn rhp_cal_is_lenient(c: *mut c_void) -> i32;
    pub fn rhp_cal_set_lenient(c: *mut c_void, l: i32);
    pub fn rhp_cal_is_set(c: *mut c_void, field: i32) -> i32;
    pub fn rhp_cal_is_weekend(c: *mut c_void) -> i32;
    pub fn rhp_cal_is_weekend_at(c: *mut c_void, date: f64, status: *mut UErrorCode) -> i32;
    pub fn rhp_cal_wall_time(c: *mut c_void, which: i32) -> i32;
    pub fn rhp_cal_set_wall_time(c: *mut c_void, which: i32, option: i32);
    pub fn rhp_cal_keyword_values(
        key: *const c_char,
        locale: *const c_char,
        common: i32,
        status: *mut UErrorCode,
    ) -> *mut UEnumeration;
    pub fn rhp_cal_available_count() -> i32;
    pub fn rhp_cal_available_locale(i: i32) -> *const c_char;
    pub fn rhp_gcal_set_change(c: *mut c_void, date: f64, status: *mut UErrorCode);
    pub fn rhp_gcal_get_change(c: *const c_void) -> f64;
    pub fn rhp_gcal_is_leap(c: *const c_void, y: i32) -> i32;
}

unsafe extern "C" {
    pub fn rhp_locale_iso3_len(name: *const c_char) -> i32;
    pub fn rhp_datefmt_new_calendar(
        traditional: i32,
        locale: *const c_char,
        status: *mut UErrorCode,
    ) -> *mut c_void;
    pub fn rhp_datefmt_adopt_calendar(df: *mut c_void, cal: *mut c_void);
    pub fn rhp_datefmt_set_calendar(df: *mut c_void, cal: *const c_void);
    pub fn rhp_datefmt_adopt_tz(df: *mut c_void, tz: *mut c_void);
    pub fn rhp_datefmt_get_tz(df: *const c_void) -> *const c_void;
    pub fn rhp_datefmt_get_calendar(df: *const c_void) -> *const c_void;
    pub fn rhp_datefmt_format_object(
        cal: *mut c_void,
        tz: *mut c_void,
        date: f64,
        pattern: *const c_char,
        date_style: i32,
        time_style: i32,
        locale: *const c_char,
        len: *mut i32,
        which: *mut i32,
        status: *mut UErrorCode,
    ) -> *mut UChar;
}

/// The UTF-16 a shim function returned (malloc'd, `len` units), freed.
pub fn take_ustr(p: *mut UChar, len: i32) -> Option<Vec<UChar>> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the shim returned `len` units at `p`, which we now own.
    let v = unsafe { std::slice::from_raw_parts(p, len.max(0) as usize) }.to_vec();
    // SAFETY: malloc'd by the shim.
    unsafe { rhp_intl_free(p.cast()) };
    Some(v)
}
