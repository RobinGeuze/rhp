//! Native date/time builtins — deterministic, explicit-timestamp forms only.
//!
//! Local-time functions (`date`, `mktime`, `getdate`) use the default zone
//! (`date_default_timezone_set()` → `date.timezone` → UTC) via [`crate::tzdb`];
//! the `gm*` forms always use UTC. An omitted / null `$timestamp`
//! (or omitted `mktime` component) falls back to the current time, as in php.
//!
//! Wire this module into `lib.rs` with:
//! ```ignore
//! mod datetime_fns;            // near the other `mod` lines
//! // ...inside registry(), after the other module registrations:
//! datetime_fns::register(&mut m);
//! ```

use std::collections::HashMap;

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Deref a (possibly `Ref`) value to an owned value.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

pub(crate) const SECS_PER_DAY: i64 = 86400;

pub(crate) const DAY_FULL: [&[u8]; 7] = [
    b"Sunday",
    b"Monday",
    b"Tuesday",
    b"Wednesday",
    b"Thursday",
    b"Friday",
    b"Saturday",
];
pub(crate) const MONTH_FULL: [&[u8]; 12] = [
    b"January",
    b"February",
    b"March",
    b"April",
    b"May",
    b"June",
    b"July",
    b"August",
    b"September",
    b"October",
    b"November",
    b"December",
];

/// Is `y` a Gregorian leap year?
pub(crate) fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Days in month `m` (1..12) of year `y`.
pub(crate) fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Days since the Unix epoch (1970-01-01) for the civil date y/m/d.
/// Howard Hinnant's algorithm. `m` in [1,12], `d` in [1,31].
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Inverse of [`days_from_civil`]: civil (year, month, day) from a day count.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Fully broken-down UTC time for a Unix timestamp.
pub(crate) struct Tm {
    pub(crate) year: i64,
    pub(crate) month: i64, // 1..12
    pub(crate) day: i64,   // 1..31
    pub(crate) hour: i64,  // 0..23
    pub(crate) min: i64,   // 0..59
    pub(crate) sec: i64,   // 0..59
    pub(crate) wday: i64,  // 0 = Sunday .. 6 = Saturday
    pub(crate) yday: i64,  // 0-based day of year
}

pub(crate) fn break_down(ts: i64) -> Tm {
    let days = ts.div_euclid(SECS_PER_DAY);
    let rem = ts.rem_euclid(SECS_PER_DAY);
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 is a Thursday (index 4 with Sunday = 0).
    let wday = (days.rem_euclid(7) + 4).rem_euclid(7);
    let yday = days - days_from_civil(year, 1, 1);
    Tm {
        year,
        month,
        day,
        hour,
        min,
        sec,
        wday,
        yday,
    }
}

/// Current unix time (the default for an omitted / null `$timestamp`).
fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Optional `?int $timestamp` argument: absent or null → now.
fn ts_arg(args: &[Value], i: usize) -> i64 {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => now_ts(),
        Some(v) => conv::to_int(&v),
    }
}

/// `date()` / `gmdate()` (php_date.c `date_format` with `localtime`).
pub(crate) fn php_date(format: &[u8], ts: i64, localtime: bool) -> Vec<u8> {
    use crate::dt_oop_fns::{format_dt_x, ZoneX};
    if !localtime {
        let x = ZoneX {
            abbr: b"GMT".to_vec(),
            dst: false,
            localtime: false,
        };
        return format_dt_x(ts, 0, 0, b"UTC", Some(&x), format);
    }
    match crate::tzdb::default_zone() {
        None => {
            let x = ZoneX {
                abbr: b"UTC".to_vec(),
                dst: false,
                localtime: true,
            };
            format_dt_x(ts, 0, 0, b"UTC", Some(&x), format)
        }
        Some(z) => {
            let t = z.at(ts);
            let x = ZoneX {
                abbr: t.abbr,
                dst: t.dst,
                localtime: true,
            };
            let name = crate::runtime_fns::default_timezone_name();
            format_dt_x(ts, 0, t.off, &name, Some(&x), format)
        }
    }
}

fn fn_date(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let format = conv::to_string_bytes(&deref(&args[0]));
    let ts = ts_arg(args, 1);
    Ok(Value::str(php_date(&format, ts, true)))
}

fn fn_gmdate(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let format = conv::to_string_bytes(&deref(&args[0]));
    let ts = ts_arg(args, 1);
    Ok(Value::str(php_date(&format, ts, false)))
}

/// Broken-down local time (default zone) for `ts`, plus its dst flag.
pub(crate) fn local_break_down(ts: i64) -> (Tm, bool) {
    let t = crate::tzdb::local_type(ts);
    (break_down(ts + t.off), t.dst)
}

/// Local wall-clock seconds (civil seconds as if UTC) → Unix timestamp in
/// the default zone (timelib `do_adjust_timezone`).
pub(crate) fn local_to_ts(local: i64, hint_dst: bool) -> i64 {
    match crate::tzdb::default_zone() {
        None => local,
        Some(z) => z.local_to_utc(local, hint_dst),
    }
}

/// `idate(format, timestamp = null)`: one local-time field as an int
/// (`php_idate`). A format that is not exactly one recognised character
/// returns false (php also warns).
fn fn_idate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let format = crate::argparse::str_param(host, args, 0, "idate", "format")?;
    let ts = crate::argparse::int_or_null_param(host, args, 1, "idate", "timestamp")?
        .unwrap_or_else(now_ts);
    if format.len() != 1 {
        return Ok(Value::Bool(false));
    }
    let c = format[0];
    let (tm, _) = local_break_down(ts);
    let v: i64 = match c {
        b'y' => tm.year % 100,
        b'U' => ts as i32 as i64,
        b'B' => {
            let mut r = ((ts % 86400) + 3600) * 10;
            if r < 0 {
                r += 864000;
            }
            (r / 864) % 1000
        }
        b'd' | b'j' | b'N' | b'w' | b'z' | b'W' | b'm' | b'n' | b't' | b'L' | b'Y' | b'o'
        | b'g' | b'h' | b'H' | b'G' | b'i' | b's' | b'I' | b'Z' => {
            conv::to_int(&Value::str(php_date(&[c], ts, true)))
        }
        _ => return Ok(Value::Bool(false)),
    };
    Ok(Value::Int(v as i32 as i64))
}

// ---------------------------------------------------------------------------
// mktime / gmmktime
// ---------------------------------------------------------------------------

/// PHP's 2-digit-year adjustment for mktime: 0..69 -> 2000..2069,
/// 70..100 -> 1970..2000; 4-digit years pass through unchanged.
pub(crate) fn adjust_year(y: i64) -> i64 {
    if (0..=69).contains(&y) {
        y + 2000
    } else if (70..=100).contains(&y) {
        y + 1900
    } else {
        y
    }
}

fn mktime_impl(args: &[Value], gmt: bool) -> i64 {
    // Omitted / null components default to the current time's (local for
    // mktime, UTC for gmmktime).
    let now_ts = now_ts();
    let (now, now_dst) = if gmt {
        (break_down(now_ts), false)
    } else {
        local_break_down(now_ts)
    };
    let part = |i: usize, dflt: i64| match args.get(i).map(deref) {
        None | Some(Value::Null) => dflt,
        Some(v) => conv::to_int(&v),
    };
    let hour = part(0, now.hour);
    let minute = part(1, now.min);
    let second = part(2, now.sec);
    let month = part(3, now.month);
    let day = part(4, now.day);
    let year = match args.get(5).map(deref) {
        None | Some(Value::Null) => now.year,
        Some(v) => adjust_year(conv::to_int(&v)),
    };

    // Normalize the month into 1..12 with a year carry; day/hour/min/sec
    // overflow folds in by plain arithmetic on the wall clock.
    let year = year + (month - 1).div_euclid(12);
    let month = (month - 1).rem_euclid(12) + 1;
    let base_days = days_from_civil(year, month, 1) + (day - 1);
    let local = base_days * SECS_PER_DAY + hour * 3600 + minute * 60 + second;
    if gmt {
        local
    } else {
        local_to_ts(local, now_dst)
    }
}

fn fn_mktime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(mktime_impl(args, false)))
}

fn fn_gmmktime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(mktime_impl(args, true)))
}

// ---------------------------------------------------------------------------
// checkdate
// ---------------------------------------------------------------------------

fn fn_checkdate(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let month = conv::to_int(&deref(&args[0]));
    let day = conv::to_int(&deref(&args[1]));
    let year = conv::to_int(&deref(&args[2]));
    let ok = (1..=32767).contains(&year)
        && (1..=12).contains(&month)
        && day >= 1
        && day <= days_in_month(year, month);
    Ok(Value::Bool(ok))
}

// ---------------------------------------------------------------------------
// getdate
// ---------------------------------------------------------------------------

fn fn_getdate(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ts = ts_arg(args, 0);
    let (tm, _) = local_break_down(ts);
    let mut a = PhpArray::new();
    a.insert(ArrayKey::from_bytes(b"seconds"), Value::Int(tm.sec));
    a.insert(ArrayKey::from_bytes(b"minutes"), Value::Int(tm.min));
    a.insert(ArrayKey::from_bytes(b"hours"), Value::Int(tm.hour));
    a.insert(ArrayKey::from_bytes(b"mday"), Value::Int(tm.day));
    a.insert(ArrayKey::from_bytes(b"wday"), Value::Int(tm.wday));
    a.insert(ArrayKey::from_bytes(b"mon"), Value::Int(tm.month));
    a.insert(ArrayKey::from_bytes(b"year"), Value::Int(tm.year));
    a.insert(ArrayKey::from_bytes(b"yday"), Value::Int(tm.yday));
    a.insert(
        ArrayKey::from_bytes(b"weekday"),
        Value::str(DAY_FULL[tm.wday as usize].to_vec()),
    );
    a.insert(
        ArrayKey::from_bytes(b"month"),
        Value::str(MONTH_FULL[(tm.month - 1) as usize].to_vec()),
    );
    a.insert(ArrayKey::Int(0), Value::Int(ts));
    Ok(Value::Array(a))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register all date/time builtins into the shared table.
pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    // An omitted / null $timestamp means "now", as in php.
    put(m, "date", fn_date, 1, Some(2));
    put(m, "gmdate", fn_gmdate, 1, Some(2));
    put(m, "mktime", fn_mktime, 1, Some(6));
    put(m, "gmmktime", fn_gmmktime, 1, Some(6));
    put(m, "checkdate", fn_checkdate, 3, Some(3));
    put(m, "getdate", fn_getdate, 0, Some(1));
    put(m, "idate", fn_idate, 1, Some(2));
}

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8 (UTC)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct NullHost;
    impl Host for NullHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    fn s(b: &str) -> Value {
        Value::str(b)
    }
    fn call(f: crate::NativeFn, args: &[Value]) -> Value {
        f(&mut NullHost, args).expect("no throw")
    }
    fn calls(f: crate::NativeFn, args: &[Value]) -> String {
        match call(f, args) {
            Value::Str(s) => String::from_utf8(s.as_bytes().to_vec()).unwrap(),
            other => panic!("expected string, got {}", other.type_name()),
        }
    }
    fn call_int(f: crate::NativeFn, args: &[Value]) -> i64 {
        match call(f, args) {
            Value::Int(n) => n,
            other => panic!("expected int, got {}", other.type_name()),
        }
    }
    fn call_bool(f: crate::NativeFn, args: &[Value]) -> bool {
        match call(f, args) {
            Value::Bool(b) => b,
            other => panic!("expected bool, got {}", other.type_name()),
        }
    }

    #[test]
    fn date_full_and_chars() {
        assert_eq!(
            calls(fn_date, &[s("Y-m-d H:i:s"), Value::Int(1700000000)]),
            "2023-11-14 22:13:20"
        );
        assert_eq!(
            calls(
                fn_date,
                &[
                    s("Y|y|m|n|d|j|H|G|h|g|i|s|A|a|D|l|N|w|z|t|L|U"),
                    Value::Int(1700000000)
                ]
            ),
            "2023|23|11|11|14|14|22|22|10|10|13|20|PM|pm|Tue|Tuesday|2|2|317|30|0|1700000000"
        );
    }

    #[test]
    fn date_epoch_and_negative() {
        assert_eq!(
            calls(fn_date, &[s("Y-m-d H:i:s"), Value::Int(0)]),
            "1970-01-01 00:00:00"
        );
        assert_eq!(
            calls(
                fn_date,
                &[
                    s("Y|y|m|n|d|j|H|G|h|g|i|s|A|a|D|l|N|w|z|t|L|U"),
                    Value::Int(0)
                ]
            ),
            "1970|70|01|1|01|1|00|0|12|12|00|00|AM|am|Thu|Thursday|4|4|0|31|0|0"
        );
        assert_eq!(
            calls(fn_date, &[s("Y-m-d H:i:s"), Value::Int(-1)]),
            "1969-12-31 23:59:59"
        );
        assert_eq!(
            calls(fn_date, &[s("Y-m-d H:i:s"), Value::Int(-86400)]),
            "1969-12-31 00:00:00"
        );
    }

    #[test]
    fn date_ordinal_suffix() {
        // Expected values captured from php 8.5.8 `date("jS", mktime(...))`.
        let cases: &[(i64, &str)] = &[
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (4, "4th"),
            (11, "11th"),
            (12, "12th"),
            (13, "13th"),
            (21, "21st"),
            (22, "22nd"),
            (23, "23rd"),
            (31, "31st"),
        ];
        for &(day, want) in cases {
            // 2024-03-<day> at midnight UTC via mktime(0,0,0,3,day,2024).
            let ts = call_int(
                fn_mktime,
                &[
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(3),
                    Value::Int(day),
                    Value::Int(2024),
                ],
            );
            assert_eq!(calls(fn_date, &[s("jS"), Value::Int(ts)]), want);
        }
    }

    #[test]
    fn date_twelve_hour_and_escapes() {
        assert_eq!(
            calls(fn_date, &[s("g h A"), Value::Int(946684800)]),
            "12 12 AM"
        );
        assert_eq!(
            calls(fn_date, &[s("g h A"), Value::Int(946728000)]),
            "12 12 PM"
        );
        assert_eq!(
            calls(fn_date, &[s("h:i:s A"), Value::Int(1700000000)]),
            "10:13:20 PM"
        );
        // backslash escapes: \Y is a literal Y, \t \i \m \e literal letters
        assert_eq!(
            calls(fn_date, &[s(r"\Y=Y \t\i\m\e"), Value::Int(1700000000)]),
            "Y=2023 time"
        );
    }

    #[test]
    fn date_leap_doy_dim() {
        // 2020-02-29
        assert_eq!(calls(fn_date, &[s("L t"), Value::Int(1582934400)]), "1 29");
        assert_eq!(calls(fn_date, &[s("L"), Value::Int(946684800)]), "1"); // 2000
        assert_eq!(calls(fn_date, &[s("L"), Value::Int(978307200)]), "0"); // 2001
        assert_eq!(calls(fn_date, &[s("z"), Value::Int(1609459200)]), "0"); // 2021-01-01
        assert_eq!(calls(fn_date, &[s("z"), Value::Int(1640908800)]), "364"); // 2021-12-31
        assert_eq!(calls(fn_date, &[s("t"), Value::Int(1612137600)]), "28"); // 2021-02
        assert_eq!(calls(fn_date, &[s("t"), Value::Int(1617235200)]), "30"); // 2021-04
    }

    #[test]
    fn gmdate_matches_date() {
        assert_eq!(
            calls(fn_date, &[s("Y-m-d H:i:s"), Value::Int(1700000000)]),
            "2023-11-14 22:13:20"
        );
    }

    #[test]
    fn mktime_cases() {
        let i = |a: [i64; 6]| {
            call_int(
                fn_mktime,
                &[
                    Value::Int(a[0]),
                    Value::Int(a[1]),
                    Value::Int(a[2]),
                    Value::Int(a[3]),
                    Value::Int(a[4]),
                    Value::Int(a[5]),
                ],
            )
        };
        assert_eq!(i([22, 13, 20, 11, 14, 2023]), 1700000000);
        assert_eq!(i([0, 0, 0, 1, 1, 1970]), 0);
        assert_eq!(i([0, 0, 0, 13, 1, 2020]), 1609459200); // month overflow -> Jan 2021
        assert_eq!(i([0, 0, 0, 3, 0, 2020]), 1582934400); // day 0 -> last day of Feb
        assert_eq!(i([0, 0, 0, 1, 1, 1969]), -31536000);
        assert_eq!(i([0, 0, 90, 1, 1, 2000]), 946684890); // second overflow
        assert_eq!(i([0, 0, 0, 1, 1, 70]), 0); // 2-digit year 70 -> 1970
        assert_eq!(i([0, 0, 0, 1, 1, 38]), 2145916800); // 38 -> 2038
        assert_eq!(i([0, 0, 0, 1, 1, 69]), 3124224000); // 69 -> 2069
    }

    #[test]
    fn checkdate_cases() {
        let b = |m: i64, d: i64, y: i64| {
            call_bool(fn_checkdate, &[Value::Int(m), Value::Int(d), Value::Int(y)])
        };
        assert!(b(2, 29, 2020));
        assert!(!b(2, 29, 2021));
        assert!(!b(13, 1, 2020));
        assert!(!b(0, 1, 2020));
        assert!(b(12, 31, 2020));
        assert!(!b(4, 31, 2020));
        assert!(!b(1, 1, 0));
        assert!(!b(1, 1, 32768));
    }

    fn as_int(v: &Value) -> i64 {
        match v {
            Value::Int(n) => *n,
            other => panic!("expected int, got {}", other.type_name()),
        }
    }
    fn as_str(v: &Value) -> String {
        match v {
            Value::Str(s) => String::from_utf8(s.as_bytes().to_vec()).unwrap(),
            other => panic!("expected string, got {}", other.type_name()),
        }
    }

    #[test]
    fn getdate_cases() {
        let Value::Array(a) = call(fn_getdate, &[Value::Int(1700000000)]) else {
            panic!("expected array");
        };
        let keys: Vec<String> = a
            .iter()
            .map(|(k, _)| match k {
                ArrayKey::Int(n) => n.to_string(),
                ArrayKey::Str(s) => String::from_utf8(s.as_bytes().to_vec()).unwrap(),
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                "seconds", "minutes", "hours", "mday", "wday", "mon", "year", "yday", "weekday",
                "month", "0"
            ]
        );
        let geti = |name: &str| as_int(a.get(&ArrayKey::from_bytes(name.as_bytes())).unwrap());
        let gets = |name: &str| as_str(a.get(&ArrayKey::from_bytes(name.as_bytes())).unwrap());
        assert_eq!(geti("seconds"), 20);
        assert_eq!(geti("minutes"), 13);
        assert_eq!(geti("hours"), 22);
        assert_eq!(geti("mday"), 14);
        assert_eq!(geti("wday"), 2);
        assert_eq!(geti("mon"), 11);
        assert_eq!(geti("year"), 2023);
        assert_eq!(geti("yday"), 317);
        assert_eq!(gets("weekday"), "Tuesday");
        assert_eq!(gets("month"), "November");
        assert_eq!(as_int(a.get(&ArrayKey::Int(0)).unwrap()), 1700000000);
    }

    #[test]
    fn getdate_epoch() {
        let Value::Array(a) = call(fn_getdate, &[Value::Int(0)]) else {
            panic!("expected array");
        };
        let geti = |name: &str| as_int(a.get(&ArrayKey::from_bytes(name.as_bytes())).unwrap());
        let gets = |name: &str| as_str(a.get(&ArrayKey::from_bytes(name.as_bytes())).unwrap());
        assert_eq!(geti("wday"), 4);
        assert_eq!(gets("weekday"), "Thursday");
        assert_eq!(gets("month"), "January");
        assert_eq!(geti("year"), 1970);
        assert_eq!(geti("yday"), 0);
    }
}
