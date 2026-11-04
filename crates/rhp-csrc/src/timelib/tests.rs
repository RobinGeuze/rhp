use super::sys::*;
use super::*;
use std::mem::{offset_of, size_of};

/// The structs in sys.rs have the C compiler's layout (glue/timelib.c
/// lists the same sizes and offsets in the same order).
#[test]
fn layout_matches_timelib_h() {
    macro_rules! l {
        ($t:ty, $($f:tt)+) => {
            offset_of!($t, $($f)+)
        };
    }
    let rust = [
        size_of::<timelib_time>(),
        l!(timelib_time, y),
        l!(timelib_time, us),
        l!(timelib_time, z),
        l!(timelib_time, tz_abbr),
        l!(timelib_time, tz_info),
        l!(timelib_time, dst),
        l!(timelib_time, relative),
        l!(timelib_time, sse),
        l!(timelib_time, have_time),
        l!(timelib_time, have_weeknr_day),
        l!(timelib_time, sse_uptodate),
        l!(timelib_time, is_localtime),
        l!(timelib_time, zone_type),
        size_of::<timelib_rel_time>(),
        l!(timelib_rel_time, y),
        l!(timelib_rel_time, us),
        l!(timelib_rel_time, weekday),
        l!(timelib_rel_time, weekday_behavior),
        l!(timelib_rel_time, first_last_day_of),
        l!(timelib_rel_time, invert),
        l!(timelib_rel_time, days),
        l!(timelib_rel_time, special.type_),
        l!(timelib_rel_time, special.amount),
        l!(timelib_rel_time, have_weekday_relative),
        l!(timelib_rel_time, have_special_relative),
        size_of::<timelib_tzinfo>(),
        l!(timelib_tzinfo, name),
        l!(timelib_tzinfo, _bit32),
        l!(timelib_tzinfo, bit64),
        l!(timelib_tzinfo, bit64.timecnt),
        l!(timelib_tzinfo, bit64.typecnt),
        l!(timelib_tzinfo, trans),
        l!(timelib_tzinfo, trans_idx),
        l!(timelib_tzinfo, type_),
        l!(timelib_tzinfo, timezone_abbr),
        l!(timelib_tzinfo, leap_times),
        l!(timelib_tzinfo, bc),
        l!(timelib_tzinfo, location),
        l!(timelib_tzinfo, posix_string),
        l!(timelib_tzinfo, posix_info),
        size_of::<tlocinfo>(),
        l!(tlocinfo, country_code),
        l!(tlocinfo, latitude),
        l!(tlocinfo, longitude),
        l!(tlocinfo, comments),
        size_of::<ttinfo>(),
        l!(ttinfo, offset),
        l!(ttinfo, isdst),
        l!(ttinfo, abbr_idx),
        l!(ttinfo, isstdcnt),
        l!(ttinfo, isgmtcnt),
        size_of::<timelib_time_offset>(),
        l!(timelib_time_offset, offset),
        l!(timelib_time_offset, leap_secs),
        l!(timelib_time_offset, is_dst),
        l!(timelib_time_offset, abbr),
        l!(timelib_time_offset, transition_time),
        size_of::<timelib_error_message>(),
        l!(timelib_error_message, error_code),
        l!(timelib_error_message, position),
        l!(timelib_error_message, character),
        l!(timelib_error_message, message),
        size_of::<timelib_error_container>(),
        l!(timelib_error_container, error_messages),
        l!(timelib_error_container, warning_messages),
        l!(timelib_error_container, error_count),
        l!(timelib_error_container, warning_count),
        size_of::<timelib_tz_lookup_table>(),
        l!(timelib_tz_lookup_table, name),
        l!(timelib_tz_lookup_table, type_),
        l!(timelib_tz_lookup_table, gmtoffset),
        l!(timelib_tz_lookup_table, full_tz_name),
        size_of::<timelib_tzdb_index_entry>(),
        l!(timelib_tzdb_index_entry, id),
        l!(timelib_tzdb_index_entry, pos),
        size_of::<timelib_tzdb>(),
        l!(timelib_tzdb, version),
        l!(timelib_tzdb, index_size),
        l!(timelib_tzdb, index),
        l!(timelib_tzdb, data),
        size_of::<timelib_abbr_info>(),
        l!(timelib_abbr_info, utc_offset),
        l!(timelib_abbr_info, abbr),
        l!(timelib_abbr_info, dst),
        size_of::<timelib_posix_transitions>(),
        l!(timelib_posix_transitions, count),
        l!(timelib_posix_transitions, times),
        l!(timelib_posix_transitions, types),
        size_of::<timelib_posix_str>(),
        l!(timelib_posix_str, dst_offset),
        l!(timelib_posix_str, dst_begin),
        l!(timelib_posix_str, dst_end),
    ];
    let mut n = 0;
    // SAFETY: a static array of `n` entries.
    let c = unsafe { std::slice::from_raw_parts(rhp_timelib_layout(&mut n), n) };
    assert_eq!(&rust[..], c);
}

#[test]
fn strtotime_dst_overlap_and_errors() {
    let db = Tzdb::builtin();
    assert_eq!(db.version(), b"2026.5");
    assert_eq!(version(), "2022.17");
    let tz = Tz::parse(b"europe/amsterdam", db).unwrap();
    assert_eq!(tz.name(), b"europe/amsterdam");
    let mut now = Time::new();
    now.set_zone_id(&tz);
    now.unixtime2local(1769867110);
    // The later instant of Amsterdam's 2026 overlap, as php.
    let (mut t, e) = strtotime(b"2026-10-25 02:30:00", db, None);
    assert_eq!(e.error_count(), 0);
    t.fill_holes(&now, TIMELIB_NO_CLONE);
    t.update_ts(Some(&tz));
    assert_eq!(t.date_to_int(), Some(1792891800));
    // The length goes in: timelib's scanner skips the NUL and what
    // follows it, as php's date_parse("now\0x") shows.
    let (t, e) = strtotime(b"noon\0x", db, None);
    assert_eq!((e.error_count(), t.h), (0, 12));
    let (_, e) = strtotime(b"2026-02-30 25:61", db, None);
    let w: Vec<_> = e.warnings().map(|m| (m.position, m.message)).collect();
    assert_eq!(w, [(17, &b"The parsed date was invalid"[..])]);
    let errs: Vec<_> = e.errors().map(|m| (m.position, m.character)).collect();
    assert_eq!(errs, [(11, b'2'), (15, b'1')]);
}

#[test]
fn iso_intervals() {
    let iv = strtointerval(b"R5/2008-03-01T13:00:00Z/P1Y2M10DT2H30M");
    assert_eq!((iv.errors.error_count(), iv.recurrences), (0, 5));
    let begin = iv.begin.unwrap();
    assert_eq!((begin.y, begin.m, begin.d, begin.h), (2008, 3, 1, 13));
    let p = iv.period.unwrap();
    assert_eq!((p.y, p.m, p.d, p.h, p.i, p.s), (1, 2, 10, 2, 30, 0));
    let iv = strtointerval(b"P1D");
    assert_eq!(iv.recurrences, 0);
    assert!(iv.begin.is_none() && iv.end.is_none());
    assert_eq!(iv.period.unwrap().d, 1);
    assert!(strtointerval(b"P1X").errors.error_count() > 0);
}
