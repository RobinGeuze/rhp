//! php_date.c's `date_format` (date(), gmdate(), DateTimeInterface::format)
//! and `php_idate`, over a timelib time.

use rhp_csrc::timelib::{self, Time, TimeOffset};

use super::get_timezone_info;

pub(crate) const MON_FULL_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const MON_SHORT_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const DAY_FULL_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const DAY_SHORT_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn english_suffix(number: i64) -> &'static str {
    if (10..=19).contains(&number) {
        return "th";
    }
    match number % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

/// `php_date_full_day_name`.
pub(crate) fn full_day_name(y: i64, m: i64, d: i64) -> &'static str {
    let dow = timelib::day_of_week(y, m, d);
    usize::try_from(dow)
        .ok()
        .and_then(|i| DAY_FULL_NAMES.get(i))
        .unwrap_or(&"Unknown")
}

fn short_day_name(y: i64, m: i64, d: i64) -> &'static str {
    let dow = timelib::day_of_week(y, m, d);
    usize::try_from(dow)
        .ok()
        .and_then(|i| DAY_SHORT_NAMES.get(i))
        .unwrap_or(&"Unknown")
}

/// `mon_full_names[m - 1]` (php reads out of bounds for a month outside
/// 1..12, which timelib's normalised times never have).
pub(crate) fn month_name(m: i64) -> &'static str {
    month(&MON_FULL_NAMES, m)
}

fn month(names: &[&'static str; 12], m: i64) -> &'static str {
    usize::try_from(m - 1)
        .ok()
        .and_then(|i| names.get(i))
        .unwrap_or(&"")
}

/// The `timelib_time_offset` date_format and php_idate read the zone from.
fn local_offset(t: &Time) -> TimeOffset {
    match t.zone_type {
        timelib::TIMELIB_ZONETYPE_ABBR => {
            let mut o = TimeOffset::new();
            o.offset = t.z + t.dst * 3600;
            o.leap_secs = 0;
            o.is_dst = t.dst as u32;
            o.set_abbr(t.tz_abbr().unwrap_or_default());
            o
        }
        timelib::TIMELIB_ZONETYPE_OFFSET => {
            let mut o = TimeOffset::new();
            o.offset = t.z;
            o.leap_secs = 0;
            o.is_dst = 0;
            o.set_abbr(gmt_abbr(o.offset).as_bytes());
            o
        }
        // SAFETY: an ID time's zone is alive (timelib module docs).
        timelib::TIMELIB_ZONETYPE_ID => unsafe { offset_of_zone(t) },
        _ => TimeOffset::new(),
    }
}

/// `timelib_get_time_zone_info(t->sse, t->tz_info)`.
///
/// # Safety
///
/// `t.tz_info` points to a live zone.
unsafe fn offset_of_zone(t: &Time) -> TimeOffset {
    TimeOffset::from_raw(timelib::sys::timelib_get_time_zone_info(t.sse, t.tz_info))
}

/// `snprintf(abbr, 9, "GMT%c%02d%02d", ...)`.
fn gmt_abbr(offset: i32) -> String {
    let mut s = format!(
        "GMT{}{:02}{:02}",
        if offset < 0 { '-' } else { '+' },
        (offset / 3600).abs(),
        ((offset % 3600) / 60).abs()
    );
    s.truncate(8);
    s
}

/// `date_format(format, format_len, t, localtime)`.
pub(crate) fn date_format(format: &[u8], t: &Time, localtime: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(format.len() * 2);
    if format.is_empty() {
        return out;
    }
    let offset = localtime.then(|| local_offset(t));
    // What the format characters read of the offset when `localtime`.
    let off = offset.as_ref().map_or(0, |o| o.offset);
    let abbr = offset.as_ref().map_or(&b""[..], |o| o.abbr());
    let sign = if localtime && off < 0 { '-' } else { '+' };
    let (oh, om) = if localtime {
        ((off / 3600).abs(), ((off % 3600) / 60).abs())
    } else {
        (0, 0)
    };
    let mut iso: Option<(i64, i64)> = None;
    let iso_week = |iso: &mut Option<(i64, i64)>| {
        *iso.get_or_insert_with(|| timelib::isoweek_from_date(t.y, t.m, t.d))
    };
    // php formats each item into one buffer and appends `length` bytes of
    // it: an item that writes nothing appends the previous item again.
    let mut buffer: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < format.len() {
        let item: Option<Vec<u8>> = match format[i] {
            b'd' => s(format!("{:02}", t.d as i32)),
            b'D' => s(short_day_name(t.y, t.m, t.d)),
            b'j' => s(format!("{}", t.d as i32)),
            b'l' => s(full_day_name(t.y, t.m, t.d)),
            b'S' => s(english_suffix(t.d)),
            b'w' => s(format!("{}", timelib::day_of_week(t.y, t.m, t.d) as i32)),
            b'N' => s(format!(
                "{}",
                timelib::iso_day_of_week(t.y, t.m, t.d) as i32
            )),
            b'z' => s(format!("{}", timelib::day_of_year(t.y, t.m, t.d) as i32)),
            b'W' => s(format!("{:02}", iso_week(&mut iso).0 as i32)),
            b'o' => s(format!("{}", iso_week(&mut iso).1)),
            b'F' => s(month(&MON_FULL_NAMES, t.m)),
            b'm' => s(format!("{:02}", t.m as i32)),
            b'M' => s(month(&MON_SHORT_NAMES, t.m)),
            b'n' => s(format!("{}", t.m as i32)),
            b't' => s(format!("{}", timelib::days_in_month(t.y, t.m) as i32)),
            b'L' => s(format!("{}", timelib::is_leap(t.y as i32) as i32)),
            b'y' => s(format!("{:02}", (t.y % 100) as i32)),
            b'Y' => s(format!(
                "{}{:04}",
                if t.y < 0 { "-" } else { "" },
                t.y.unsigned_abs()
            )),
            b'x' => s(format!(
                "{}{:04}",
                if t.y < 0 {
                    "-"
                } else if t.y >= 10000 {
                    "+"
                } else {
                    ""
                },
                t.y.unsigned_abs()
            )),
            b'X' => s(format!(
                "{}{:04}",
                if t.y < 0 { "-" } else { "+" },
                t.y.unsigned_abs()
            )),
            b'a' => s(if t.h >= 12 { "pm" } else { "am" }),
            b'A' => s(if t.h >= 12 { "PM" } else { "AM" }),
            b'B' => s(format!("{:03}", swatch_beat(t.sse))),
            b'g' => s(format!("{}", hour12(t.h))),
            b'G' => s(format!("{}", t.h as i32)),
            b'h' => s(format!("{:02}", hour12(t.h))),
            b'H' => s(format!("{:02}", t.h as i32)),
            b'i' => s(format!("{:02}", t.i as i32)),
            b's' => s(format!("{:02}", t.s as i32)),
            b'u' => s(format!("{:06}", t.us as i32)),
            b'v' => s(format!("{:03}", (t.us / 1000) as i32)),
            b'I' => s(format!(
                "{}",
                offset.as_ref().map_or(0, |o| o.is_dst as i32)
            )),
            b'p' if !localtime || matches!(abbr, b"UTC" | b"Z" | b"GMT+0000") => s("Z"),
            b'p' | b'P' => s(format!("{sign}{oh:02}:{om:02}")),
            b'O' => s(format!("{sign}{oh:02}{om:02}")),
            b'T' if localtime => Some(abbr.to_vec()),
            b'T' => s("GMT"),
            b'e' if !localtime => s("UTC"),
            b'e' => match t.zone_type {
                timelib::TIMELIB_ZONETYPE_ID => Some(t.tz_name().unwrap_or_default().to_vec()),
                timelib::TIMELIB_ZONETYPE_ABBR => Some(abbr.to_vec()),
                timelib::TIMELIB_ZONETYPE_OFFSET => {
                    let mut e = format!(
                        "{}{:02}:{:02}",
                        if off < 0 { '-' } else { '+' },
                        (off / 3600).abs(),
                        ((off % 3600) / 60).abs()
                    );
                    if off % 60 != 0 {
                        e.push_str(&format!(":{:02}", (off % 60).abs()));
                    }
                    s(e)
                }
                _ => None,
            },
            b'Z' => s(format!("{off}")),
            b'c' => s(format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{oh:02}:{om:02}",
                t.y, t.m as i32, t.d as i32, t.h as i32, t.i as i32, t.s as i32
            )),
            b'r' => s(format!(
                "{:>3}, {:02} {:>3} {:04} {:02}:{:02}:{:02} {sign}{oh:02}{om:02}",
                short_day_name(t.y, t.m, t.d),
                t.d as i32,
                month(&MON_SHORT_NAMES, t.m),
                t.y,
                t.h as i32,
                t.i as i32,
                t.s as i32
            )),
            b'U' => s(format!("{}", t.sse)),
            b'\\' => {
                // The next byte, or the string's terminating NUL.
                i += 1;
                Some(vec![format.get(i).copied().unwrap_or(0)])
            }
            c => Some(vec![c]),
        };
        if let Some(item) = item {
            buffer = item;
        }
        out.extend_from_slice(&buffer);
        i += 1;
    }
    out
}

fn s(item: impl Into<String>) -> Option<Vec<u8>> {
    Some(item.into().into_bytes())
}

/// Swatch Internet time, as date_format and php_idate compute it.
fn swatch_beat(sse: i64) -> i32 {
    let mut retval = (((sse % 86400) + 3600) * 10) as i32;
    if retval < 0 {
        retval += 864000;
    }
    (retval / 864) % 1000
}

fn hour12(h: i64) -> i32 {
    if h % 12 != 0 {
        (h % 12) as i32
    } else {
        12
    }
}

/// `php_format_date(format, ts, localtime)`: `ts` in the default zone, or
/// in UTC. For callers without a host (setcookie, error_log); with no
/// usable default zone it formats in UTC.
pub fn php_format_date(format: &[u8], ts: i64, localtime: bool) -> Vec<u8> {
    let mut t = Time::new();
    let tzi = if localtime { get_timezone_info() } else { None };
    match &tzi {
        Some(tz) => {
            t.set_zone_id(tz);
            t.unixtime2local(ts);
        }
        None => t.unixtime2gmt(ts),
    }
    date_format(format, &t, tzi.is_some())
}

/// `php_idate(format, ts, localtime)`. Note php's inverted flag: idate()
/// passes `localtime = false` and gets the default zone. `None` for php's
/// -1 (also a field whose value is -1).
pub(crate) fn php_idate(format: u8, ts: i64, localtime: bool) -> Option<i32> {
    let mut t = Time::new();
    let tzi = if !localtime {
        get_timezone_info()
    } else {
        None
    };
    if let Some(tz) = &tzi {
        t.set_zone_id(tz);
        t.unixtime2local(ts);
    } else {
        t.unixtime2gmt(ts);
    }
    let offset = (!localtime).then(|| match t.zone_type {
        timelib::TIMELIB_ZONETYPE_ABBR | timelib::TIMELIB_ZONETYPE_OFFSET => {
            let mut o = TimeOffset::new();
            o.offset = t.z + t.dst * 3600;
            o.is_dst = t.dst as u32;
            o
        }
        // SAFETY: the zone is alive (`tzi`).
        timelib::TIMELIB_ZONETYPE_ID => unsafe { offset_of_zone(&t) },
        _ => TimeOffset::new(),
    });
    let (isoweek, isoyear) = timelib::isoweek_from_date(t.y, t.m, t.d);
    let retval: i32 = match format {
        b'd' | b'j' => t.d as i32,
        b'N' => timelib::iso_day_of_week(t.y, t.m, t.d) as i32,
        b'w' => timelib::day_of_week(t.y, t.m, t.d) as i32,
        b'z' => timelib::day_of_year(t.y, t.m, t.d) as i32,
        b'W' => isoweek as i32,
        b'm' | b'n' => t.m as i32,
        b't' => timelib::days_in_month(t.y, t.m) as i32,
        b'L' => timelib::is_leap(t.y as i32) as i32,
        b'y' => (t.y % 100) as i32,
        b'Y' => t.y as i32,
        b'o' => isoyear as i32,
        b'B' => swatch_beat(t.sse),
        b'g' | b'h' => hour12(t.h),
        b'H' | b'G' => t.h as i32,
        b'i' => t.i as i32,
        b's' => t.s as i32,
        b'I' => offset.as_ref().map_or(0, |o| o.is_dst as i32),
        b'Z' => offset.as_ref().map_or(0, |o| o.offset),
        b'U' => t.sse as i32,
        _ => -1,
    };
    (retval != -1).then_some(retval)
}
