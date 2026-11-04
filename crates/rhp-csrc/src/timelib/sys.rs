//! timelib.h (and the two structs of timelib_private.h php_date.c reads),
//! hand-written. `layout_matches_timelib_h` checks the structs against the C
//! compiler's layout. Functions are linked under their rhp_ names
//! (glue/prefix_timelib.h).

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_uchar, c_uint};

pub type timelib_sll = i64;
/// `timelib_long` on LP64, the only data model rhp builds for.
pub type timelib_long = i64;

pub const TIMELIB_UNSET: timelib_sll = -9999999;

pub const TIMELIB_NONE: c_int = 0x00;
pub const TIMELIB_OVERRIDE_TIME: c_int = 0x01;
pub const TIMELIB_NO_CLONE: c_int = 0x02;

pub const TIMELIB_ZONETYPE_NONE: c_uint = 0;
pub const TIMELIB_ZONETYPE_OFFSET: c_uint = 1;
pub const TIMELIB_ZONETYPE_ABBR: c_uint = 2;
pub const TIMELIB_ZONETYPE_ID: c_uint = 3;

/// timelib_private.h: `relative.special.type`.
pub const TIMELIB_SPECIAL_WEEKDAY: c_uint = 0x01;
pub const TIMELIB_SPECIAL_DAY_OF_WEEK_IN_MONTH: c_uint = 0x02;
pub const TIMELIB_SPECIAL_LAST_DAY_OF_WEEK_IN_MONTH: c_uint = 0x03;
/// timelib_private.h: `relative.first_last_day_of`.
pub const TIMELIB_SPECIAL_FIRST_DAY_OF_MONTH: c_int = 0x01;
pub const TIMELIB_SPECIAL_LAST_DAY_OF_MONTH: c_int = 0x02;

pub const TIMELIB_ERROR_NO_ERROR: c_int = 0x00;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct timelib_special {
    pub type_: c_uint,
    pub amount: timelib_sll,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct timelib_rel_time {
    pub y: timelib_sll,
    pub m: timelib_sll,
    pub d: timelib_sll,
    pub h: timelib_sll,
    pub i: timelib_sll,
    pub s: timelib_sll,
    pub us: timelib_sll,
    pub weekday: c_int,
    pub weekday_behavior: c_int,
    pub first_last_day_of: c_int,
    pub invert: c_int,
    pub days: timelib_sll,
    pub special: timelib_special,
    pub have_weekday_relative: c_uint,
    pub have_special_relative: c_uint,
}

#[repr(C)]
#[derive(Debug)]
pub struct timelib_time {
    pub y: timelib_sll,
    pub m: timelib_sll,
    pub d: timelib_sll,
    pub h: timelib_sll,
    pub i: timelib_sll,
    pub s: timelib_sll,
    pub us: timelib_sll,
    pub z: c_int,
    pub tz_abbr: *mut c_char,
    pub tz_info: *mut timelib_tzinfo,
    pub dst: c_int,
    pub relative: timelib_rel_time,
    pub sse: timelib_sll,
    pub have_time: c_uint,
    pub have_date: c_uint,
    pub have_zone: c_uint,
    pub have_relative: c_uint,
    pub have_weeknr_day: c_uint,
    pub sse_uptodate: c_uint,
    pub tim_uptodate: c_uint,
    pub is_localtime: c_uint,
    pub zone_type: c_uint,
}

#[repr(C)]
#[derive(Debug)]
pub struct tlocinfo {
    pub country_code: [c_char; 3],
    pub latitude: f64,
    pub longitude: f64,
    pub comments: *mut c_char,
}

/// timelib_private.h's `struct _ttinfo`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ttinfo {
    pub offset: i32,
    pub isdst: c_int,
    pub abbr_idx: c_uint,
    pub isstdcnt: c_uint,
    pub isgmtcnt: c_uint,
}

/// timelib_private.h's `struct _tlinfo`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tlinfo {
    pub trans: i64,
    pub offset: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct timelib_tzinfo_counts32 {
    pub ttisgmtcnt: u32,
    pub ttisstdcnt: u32,
    pub leapcnt: u32,
    pub timecnt: u32,
    pub typecnt: u32,
    pub charcnt: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct timelib_tzinfo_counts64 {
    pub ttisgmtcnt: u64,
    pub ttisstdcnt: u64,
    pub leapcnt: u64,
    pub timecnt: u64,
    pub typecnt: u64,
    pub charcnt: u64,
}

/// A parsed POSIX TZ string (`dst_begin`/`dst_end` are opaque here).
#[repr(C)]
#[derive(Debug)]
pub struct timelib_posix_str {
    pub std: *mut c_char,
    pub std_offset: timelib_sll,
    pub dst: *mut c_char,
    pub dst_offset: timelib_sll,
    pub dst_begin: *mut std::ffi::c_void,
    pub dst_end: *mut std::ffi::c_void,
    pub type_index_std_type: c_int,
    pub type_index_dst_type: c_int,
}

#[repr(C)]
#[derive(Debug)]
pub struct timelib_tzinfo {
    pub name: *mut c_char,
    pub _bit32: timelib_tzinfo_counts32,
    pub bit64: timelib_tzinfo_counts64,
    pub trans: *mut i64,
    pub trans_idx: *mut c_uchar,
    pub type_: *mut ttinfo,
    pub timezone_abbr: *mut c_char,
    pub leap_times: *mut tlinfo,
    pub bc: c_uchar,
    pub location: tlocinfo,
    pub posix_string: *mut c_char,
    pub posix_info: *mut timelib_posix_str,
}

#[repr(C)]
#[derive(Debug)]
pub struct timelib_time_offset {
    pub offset: i32,
    pub leap_secs: c_uint,
    pub is_dst: c_uint,
    pub abbr: *mut c_char,
    pub transition_time: timelib_sll,
}

#[repr(C)]
#[derive(Debug)]
pub struct timelib_error_message {
    pub error_code: c_int,
    pub position: c_int,
    pub character: c_char,
    pub message: *mut c_char,
}

#[repr(C)]
#[derive(Debug)]
pub struct timelib_error_container {
    pub error_messages: *mut timelib_error_message,
    pub warning_messages: *mut timelib_error_message,
    pub error_count: c_int,
    pub warning_count: c_int,
}

#[repr(C)]
#[derive(Debug)]
pub struct timelib_tz_lookup_table {
    pub name: *const c_char,
    pub type_: c_int,
    pub gmtoffset: f32,
    pub full_tz_name: *const c_char,
}

#[repr(C)]
#[derive(Debug)]
pub struct timelib_tzdb_index_entry {
    pub id: *mut c_char,
    pub pos: c_uint,
}

#[repr(C)]
#[derive(Debug)]
pub struct timelib_tzdb {
    pub version: *const c_char,
    pub index_size: c_int,
    pub index: *const timelib_tzdb_index_entry,
    pub data: *const c_uchar,
}

#[repr(C)]
#[derive(Debug)]
pub struct timelib_abbr_info {
    pub utc_offset: timelib_sll,
    pub abbr: *mut c_char,
    pub dst: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct timelib_posix_transitions {
    pub count: usize,
    pub times: [timelib_sll; 6],
    pub types: [timelib_sll; 6],
}

/// `timelib_tz_get_wrapper`. timelib calls it with a NUL-terminated name.
pub type timelib_tz_get_wrapper = Option<
    unsafe extern "C" fn(
        tzname: *const c_char,
        tzdb: *const timelib_tzdb,
        error_code: *mut c_int,
    ) -> *mut timelib_tzinfo,
>;

extern "C" {
    // glue/timelib.c and timelib_config.h: timelib's allocator, for memory
    // timelib frees (php_date.c's timelib_strdup / timelib_free).
    #[link_name = "rhp_timelib_strdup"]
    pub fn timelib_strdup(s: *const c_char) -> *mut c_char;
    #[link_name = "rhp_csrc_emalloc"]
    pub fn timelib_malloc(size: usize) -> *mut std::ffi::c_void;
    #[link_name = "rhp_csrc_efree"]
    pub fn timelib_free(ptr: *mut std::ffi::c_void);
    /// TIMELIB_ASCII_VERSION, from timelib.h.
    pub fn rhp_timelib_ascii_version() -> *const c_char;
    pub fn rhp_timelib_layout(n: *mut usize) -> *const usize;

    // dow.c
    #[link_name = "rhp_timelib_day_of_week"]
    pub fn timelib_day_of_week(y: timelib_sll, m: timelib_sll, d: timelib_sll) -> timelib_sll;
    #[link_name = "rhp_timelib_iso_day_of_week"]
    pub fn timelib_iso_day_of_week(y: timelib_sll, m: timelib_sll, d: timelib_sll) -> timelib_sll;
    #[link_name = "rhp_timelib_day_of_year"]
    pub fn timelib_day_of_year(y: timelib_sll, m: timelib_sll, d: timelib_sll) -> timelib_sll;
    #[link_name = "rhp_timelib_daynr_from_weeknr"]
    pub fn timelib_daynr_from_weeknr(
        iy: timelib_sll,
        iw: timelib_sll,
        id: timelib_sll,
    ) -> timelib_sll;
    #[link_name = "rhp_timelib_days_in_month"]
    pub fn timelib_days_in_month(y: timelib_sll, m: timelib_sll) -> timelib_sll;
    #[link_name = "rhp_timelib_isoweek_from_date"]
    pub fn timelib_isoweek_from_date(
        y: timelib_sll,
        m: timelib_sll,
        d: timelib_sll,
        iw: *mut timelib_sll,
        iy: *mut timelib_sll,
    );
    #[link_name = "rhp_timelib_isodate_from_date"]
    pub fn timelib_isodate_from_date(
        y: timelib_sll,
        m: timelib_sll,
        d: timelib_sll,
        iy: *mut timelib_sll,
        iw: *mut timelib_sll,
        id: *mut timelib_sll,
    );
    #[link_name = "rhp_timelib_date_from_isodate"]
    pub fn timelib_date_from_isodate(
        iy: timelib_sll,
        iw: timelib_sll,
        id: timelib_sll,
        y: *mut timelib_sll,
        m: *mut timelib_sll,
        d: *mut timelib_sll,
    );
    #[link_name = "rhp_timelib_valid_time"]
    pub fn timelib_valid_time(h: timelib_sll, i: timelib_sll, s: timelib_sll) -> c_int;
    #[link_name = "rhp_timelib_valid_date"]
    pub fn timelib_valid_date(y: timelib_sll, m: timelib_sll, d: timelib_sll) -> c_int;

    // parse_date.c
    #[link_name = "rhp_timelib_strtotime"]
    pub fn timelib_strtotime(
        s: *const c_char,
        len: usize,
        errors: *mut *mut timelib_error_container,
        tzdb: *const timelib_tzdb,
        tz_get_wrapper: timelib_tz_get_wrapper,
    ) -> *mut timelib_time;
    #[link_name = "rhp_timelib_parse_from_format"]
    pub fn timelib_parse_from_format(
        format: *const c_char,
        s: *const c_char,
        len: usize,
        errors: *mut *mut timelib_error_container,
        tzdb: *const timelib_tzdb,
        tz_get_wrapper: timelib_tz_get_wrapper,
    ) -> *mut timelib_time;
    #[link_name = "rhp_timelib_fill_holes"]
    pub fn timelib_fill_holes(parsed: *mut timelib_time, now: *mut timelib_time, options: c_int);
    #[link_name = "rhp_timelib_timezone_id_from_abbr"]
    pub fn timelib_timezone_id_from_abbr(
        abbr: *const c_char,
        gmtoffset: timelib_long,
        isdst: c_int,
    ) -> *const c_char;
    #[link_name = "rhp_timelib_timezone_abbreviations_list"]
    pub fn timelib_timezone_abbreviations_list() -> *const timelib_tz_lookup_table;
    #[link_name = "rhp_timelib_parse_zone"]
    pub fn timelib_parse_zone(
        ptr: *mut *const c_char,
        dst: *mut c_int,
        t: *mut timelib_time,
        tz_not_found: *mut c_int,
        tzdb: *const timelib_tzdb,
        tz_wrapper: timelib_tz_get_wrapper,
    ) -> timelib_long;

    // parse_iso_intervals.c
    #[link_name = "rhp_timelib_strtointerval"]
    pub fn timelib_strtointerval(
        s: *const c_char,
        len: usize,
        begin: *mut *mut timelib_time,
        end: *mut *mut timelib_time,
        period: *mut *mut timelib_rel_time,
        recurrences: *mut c_int,
        errors: *mut *mut timelib_error_container,
    );

    // tm2unixtime.c
    #[link_name = "rhp_timelib_update_ts"]
    pub fn timelib_update_ts(time: *mut timelib_time, tzi: *mut timelib_tzinfo);
    #[link_name = "rhp_timelib_epoch_days_from_time"]
    pub fn timelib_epoch_days_from_time(time: *mut timelib_time) -> timelib_sll;
    #[link_name = "rhp_timelib_do_normalize"]
    pub fn timelib_do_normalize(base: *mut timelib_time);
    #[link_name = "rhp_timelib_do_rel_normalize"]
    pub fn timelib_do_rel_normalize(base: *mut timelib_time, rt: *mut timelib_rel_time);

    // unixtime2tm.c
    #[link_name = "rhp_timelib_unixtime2date"]
    pub fn timelib_unixtime2date(
        ts: timelib_sll,
        y: *mut timelib_sll,
        m: *mut timelib_sll,
        d: *mut timelib_sll,
    );
    #[link_name = "rhp_timelib_unixtime2gmt"]
    pub fn timelib_unixtime2gmt(tm: *mut timelib_time, ts: timelib_sll);
    #[link_name = "rhp_timelib_unixtime2local"]
    pub fn timelib_unixtime2local(tm: *mut timelib_time, ts: timelib_sll);
    #[link_name = "rhp_timelib_update_from_sse"]
    pub fn timelib_update_from_sse(tm: *mut timelib_time);
    #[link_name = "rhp_timelib_set_timezone_from_offset"]
    pub fn timelib_set_timezone_from_offset(t: *mut timelib_time, utc_offset: timelib_sll);
    #[link_name = "rhp_timelib_set_timezone_from_abbr"]
    pub fn timelib_set_timezone_from_abbr(t: *mut timelib_time, abbr_info: timelib_abbr_info);
    #[link_name = "rhp_timelib_set_timezone"]
    pub fn timelib_set_timezone(t: *mut timelib_time, tz: *mut timelib_tzinfo);

    // parse_tz.c
    #[link_name = "rhp_timelib_timezone_id_is_valid"]
    pub fn timelib_timezone_id_is_valid(
        timezone: *const c_char,
        tzdb: *const timelib_tzdb,
    ) -> c_int;
    #[link_name = "rhp_timelib_parse_tzfile"]
    pub fn timelib_parse_tzfile(
        timezone: *const c_char,
        tzdb: *const timelib_tzdb,
        error_code: *mut c_int,
    ) -> *mut timelib_tzinfo;
    #[link_name = "rhp_timelib_tzinfo_dtor"]
    pub fn timelib_tzinfo_dtor(tz: *mut timelib_tzinfo);
    #[link_name = "rhp_timelib_tzinfo_clone"]
    pub fn timelib_tzinfo_clone(tz: *mut timelib_tzinfo) -> *mut timelib_tzinfo;
    #[link_name = "rhp_timelib_timestamp_is_in_dst"]
    pub fn timelib_timestamp_is_in_dst(ts: timelib_sll, tz: *mut timelib_tzinfo) -> c_int;
    #[link_name = "rhp_timelib_get_time_zone_info"]
    pub fn timelib_get_time_zone_info(
        ts: timelib_sll,
        tz: *mut timelib_tzinfo,
    ) -> *mut timelib_time_offset;
    #[link_name = "rhp_timelib_get_time_zone_offset_info"]
    pub fn timelib_get_time_zone_offset_info(
        ts: timelib_sll,
        tz: *mut timelib_tzinfo,
        offset: *mut i32,
        transition_time: *mut timelib_sll,
        is_dst: *mut c_uint,
    ) -> c_int;
    #[link_name = "rhp_timelib_get_current_offset"]
    pub fn timelib_get_current_offset(t: *mut timelib_time) -> timelib_sll;
    #[link_name = "rhp_timelib_same_timezone"]
    pub fn timelib_same_timezone(one: *mut timelib_time, two: *mut timelib_time) -> c_int;
    #[link_name = "rhp_timelib_builtin_db"]
    pub fn timelib_builtin_db() -> *const timelib_tzdb;
    #[link_name = "rhp_timelib_timezone_identifiers_list"]
    pub fn timelib_timezone_identifiers_list(
        tzdb: *const timelib_tzdb,
        count: *mut c_int,
    ) -> *const timelib_tzdb_index_entry;
    #[link_name = "rhp_timelib_time_tz_abbr_update"]
    pub fn timelib_time_tz_abbr_update(tm: *mut timelib_time, tz_abbr: *const c_char);

    // timelib.c
    #[link_name = "rhp_timelib_get_error_message"]
    pub fn timelib_get_error_message(error_code: c_int) -> *const c_char;
    #[link_name = "rhp_timelib_rel_time_ctor"]
    pub fn timelib_rel_time_ctor() -> *mut timelib_rel_time;
    #[link_name = "rhp_timelib_rel_time_dtor"]
    pub fn timelib_rel_time_dtor(t: *mut timelib_rel_time);
    #[link_name = "rhp_timelib_rel_time_clone"]
    pub fn timelib_rel_time_clone(t: *mut timelib_rel_time) -> *mut timelib_rel_time;
    #[link_name = "rhp_timelib_time_ctor"]
    pub fn timelib_time_ctor() -> *mut timelib_time;
    #[link_name = "rhp_timelib_time_dtor"]
    pub fn timelib_time_dtor(t: *mut timelib_time);
    #[link_name = "rhp_timelib_time_clone"]
    pub fn timelib_time_clone(orig: *mut timelib_time) -> *mut timelib_time;
    #[link_name = "rhp_timelib_time_compare"]
    pub fn timelib_time_compare(t1: *mut timelib_time, t2: *mut timelib_time) -> c_int;
    #[link_name = "rhp_timelib_time_offset_ctor"]
    pub fn timelib_time_offset_ctor() -> *mut timelib_time_offset;
    #[link_name = "rhp_timelib_time_offset_dtor"]
    pub fn timelib_time_offset_dtor(t: *mut timelib_time_offset);
    #[link_name = "rhp_timelib_error_container_dtor"]
    pub fn timelib_error_container_dtor(errors: *mut timelib_error_container);
    #[link_name = "rhp_timelib_date_to_int"]
    pub fn timelib_date_to_int(d: *mut timelib_time, error: *mut c_int) -> timelib_long;
    #[link_name = "rhp_timelib_decimal_hour_to_hms"]
    pub fn timelib_decimal_hour_to_hms(h: f64, hour: *mut c_int, min: *mut c_int, sec: *mut c_int);
    #[link_name = "rhp_timelib_hms_to_seconds"]
    pub fn timelib_hms_to_seconds(h: timelib_sll, m: timelib_sll, s: timelib_sll) -> timelib_sll;

    // astro.c
    #[link_name = "rhp_timelib_ts_to_julianday"]
    pub fn timelib_ts_to_julianday(ts: timelib_sll) -> f64;
    #[link_name = "rhp_timelib_ts_to_j2000"]
    pub fn timelib_ts_to_j2000(ts: timelib_sll) -> f64;
    #[link_name = "rhp_timelib_astro_rise_set_altitude"]
    pub fn timelib_astro_rise_set_altitude(
        time: *mut timelib_time,
        lon: f64,
        lat: f64,
        altit: f64,
        upper_limb: c_int,
        h_rise: *mut f64,
        h_set: *mut f64,
        ts_rise: *mut timelib_sll,
        ts_set: *mut timelib_sll,
        ts_transit: *mut timelib_sll,
    ) -> c_int;

    // interval.c
    #[link_name = "rhp_timelib_diff"]
    pub fn timelib_diff(one: *mut timelib_time, two: *mut timelib_time) -> *mut timelib_rel_time;
    #[link_name = "rhp_timelib_diff_days"]
    pub fn timelib_diff_days(one: *mut timelib_time, two: *mut timelib_time) -> c_int;
    #[link_name = "rhp_timelib_add"]
    pub fn timelib_add(t: *mut timelib_time, interval: *mut timelib_rel_time) -> *mut timelib_time;
    #[link_name = "rhp_timelib_add_wall"]
    pub fn timelib_add_wall(
        t: *mut timelib_time,
        interval: *mut timelib_rel_time,
    ) -> *mut timelib_time;
    #[link_name = "rhp_timelib_sub"]
    pub fn timelib_sub(t: *mut timelib_time, interval: *mut timelib_rel_time) -> *mut timelib_time;
    #[link_name = "rhp_timelib_sub_wall"]
    pub fn timelib_sub_wall(
        t: *mut timelib_time,
        interval: *mut timelib_rel_time,
    ) -> *mut timelib_time;

    // parse_posix.c
    #[link_name = "rhp_timelib_get_transitions_for_year"]
    pub fn timelib_get_transitions_for_year(
        tz: *mut timelib_tzinfo,
        year: timelib_sll,
        transitions: *mut timelib_posix_transitions,
    );
}
