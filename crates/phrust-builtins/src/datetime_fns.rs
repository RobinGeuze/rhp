//! Native date/time builtins — deterministic, explicit-timestamp forms only.
//!
//! The engine timezone is treated as UTC, so `date`/`gmdate` are identical and
//! `mktime`/`gmmktime` are identical. No current-time functions are provided
//! (`time`, `microtime`, etc. are intentionally absent): every function here is
//! a pure function of its arguments, verified byte-for-byte against php 8.5.8
//! with `date_default_timezone_set("UTC")`.
//!
//! Arities are tightened to the deterministic shape: `date`/`gmdate` require the
//! `$timestamp` argument, and `mktime`/`gmmktime` require all six components —
//! the forms that would otherwise fall back to the current time are refused by
//! the arity check rather than returning a non-deterministic result.
//!
//! Wire this module into `lib.rs` with:
//! ```ignore
//! mod datetime_fns;            // near the other `mod` lines
//! // ...inside registry(), after the other module registrations:
//! datetime_fns::register(&mut m);
//! ```

use std::collections::HashMap;

use phrust_values::{conv, ArrayKey, PhpArray, Value};

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

const SECS_PER_DAY: i64 = 86400;

const DAY_SHORT: [&[u8]; 7] = [b"Sun", b"Mon", b"Tue", b"Wed", b"Thu", b"Fri", b"Sat"];
const DAY_FULL: [&[u8]; 7] = [
    b"Sunday",
    b"Monday",
    b"Tuesday",
    b"Wednesday",
    b"Thursday",
    b"Friday",
    b"Saturday",
];
const MONTH_FULL: [&[u8]; 12] = [
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
fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Days in month `m` (1..12) of year `y`.
fn days_in_month(y: i64, m: i64) -> i64 {
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
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
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
struct Tm {
    year: i64,
    month: i64, // 1..12
    day: i64,   // 1..31
    hour: i64,  // 0..23
    min: i64,   // 0..59
    sec: i64,   // 0..59
    wday: i64,  // 0 = Sunday .. 6 = Saturday
    yday: i64,  // 0-based day of year
}

fn break_down(ts: i64) -> Tm {
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

fn push_int(out: &mut Vec<u8>, n: i64) {
    out.extend_from_slice(n.to_string().as_bytes());
}

fn push_2(out: &mut Vec<u8>, n: i64) {
    out.extend_from_slice(format!("{n:02}").as_bytes());
}

// ---------------------------------------------------------------------------
// date / gmdate
// ---------------------------------------------------------------------------

/// Render `format` against the broken-down time `tm` (timestamp `ts`).
/// Supports: Y y m n d j H G h g i s A a D l N w z t L U and `\`-escapes.
/// Any other character is emitted literally.
fn format_date(format: &[u8], tm: &Tm, ts: i64) -> Vec<u8> {
    let mut out = Vec::with_capacity(format.len() + 16);
    let mut i = 0;
    while i < format.len() {
        let c = format[i];
        if c == b'\\' {
            // Escape: emit the next byte literally (if any).
            if i + 1 < format.len() {
                out.push(format[i + 1]);
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        match c {
            b'Y' => push_int(&mut out, tm.year),
            b'y' => push_2(&mut out, tm.year.rem_euclid(100)),
            b'm' => push_2(&mut out, tm.month),
            b'n' => push_int(&mut out, tm.month),
            b'd' => push_2(&mut out, tm.day),
            b'j' => push_int(&mut out, tm.day),
            b'H' => push_2(&mut out, tm.hour),
            b'G' => push_int(&mut out, tm.hour),
            b'h' => push_2(&mut out, hour12(tm.hour)),
            b'g' => push_int(&mut out, hour12(tm.hour)),
            b'i' => push_2(&mut out, tm.min),
            b's' => push_2(&mut out, tm.sec),
            b'A' => out.extend_from_slice(if tm.hour < 12 { b"AM" } else { b"PM" }),
            b'a' => out.extend_from_slice(if tm.hour < 12 { b"am" } else { b"pm" }),
            b'D' => out.extend_from_slice(DAY_SHORT[tm.wday as usize]),
            b'l' => out.extend_from_slice(DAY_FULL[tm.wday as usize]),
            b'M' => out.extend_from_slice(&MONTH_FULL[(tm.month - 1) as usize][..3]),
            b'F' => out.extend_from_slice(MONTH_FULL[(tm.month - 1) as usize]),
            b'N' => push_int(&mut out, if tm.wday == 0 { 7 } else { tm.wday }),
            b'w' => push_int(&mut out, tm.wday),
            b'z' => push_int(&mut out, tm.yday),
            b't' => push_int(&mut out, days_in_month(tm.year, tm.month)),
            b'L' => out.push(if is_leap(tm.year) { b'1' } else { b'0' }),
            b'S' => out.extend_from_slice(ordinal_suffix(tm.day)),
            b'U' => push_int(&mut out, ts),
            other => out.push(other),
        }
        i += 1;
    }
    out
}

/// English ordinal suffix for a day-of-month (`date("S")`): `st`/`nd`/`rd`/`th`.
/// 11, 12, 13 are always `th`; otherwise the last digit selects 1→st, 2→nd,
/// 3→rd, else th.
fn ordinal_suffix(day: i64) -> &'static [u8] {
    match (day % 100, day % 10) {
        (11..=13, _) => b"th",
        (_, 1) => b"st",
        (_, 2) => b"nd",
        (_, 3) => b"rd",
        _ => b"th",
    }
}

/// 12-hour clock value for a 0..23 hour (0 and 12 both map to 12).
fn hour12(h: i64) -> i64 {
    let h = h % 12;
    if h == 0 {
        12
    } else {
        h
    }
}

fn fn_date(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let format = conv::to_string_bytes(&deref(&args[0]));
    let ts = conv::to_int(&deref(&args[1]));
    let tm = break_down(ts);
    Ok(Value::str(format_date(&format, &tm, ts)))
}

// ---------------------------------------------------------------------------
// mktime / gmmktime
// ---------------------------------------------------------------------------

/// PHP's 2-digit-year adjustment for mktime: 0..69 -> 2000..2069,
/// 70..100 -> 1970..2000; 4-digit years pass through unchanged.
fn adjust_year(y: i64) -> i64 {
    if (0..=69).contains(&y) {
        y + 2000
    } else if (70..=100).contains(&y) {
        y + 1900
    } else {
        y
    }
}

fn fn_mktime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let hour = conv::to_int(&deref(&args[0]));
    let minute = conv::to_int(&deref(&args[1]));
    let second = conv::to_int(&deref(&args[2]));
    let month = conv::to_int(&deref(&args[3]));
    let day = conv::to_int(&deref(&args[4]));
    let year = adjust_year(conv::to_int(&deref(&args[5])));

    // Normalize the month into 1..12 with a year carry; then every day is a
    // uniform 86400 seconds (UTC, no DST), so day/hour/min/sec overflow folds
    // in by plain arithmetic.
    let year = year + (month - 1).div_euclid(12);
    let month = (month - 1).rem_euclid(12) + 1;
    let base_days = days_from_civil(year, month, 1) + (day - 1);
    let ts = base_days * SECS_PER_DAY + hour * 3600 + minute * 60 + second;
    Ok(Value::Int(ts))
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
    let ts = conv::to_int(&deref(&args[0]));
    let tm = break_down(ts);
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
    // date/gmdate require the explicit $timestamp (no current-time fallback).
    put(m, "date", fn_date, 2, Some(2));
    put(m, "gmdate", fn_date, 2, Some(2));
    // mktime/gmmktime require all six components (deterministic form).
    put(m, "mktime", fn_mktime, 6, Some(6));
    put(m, "gmmktime", fn_mktime, 6, Some(6));
    put(m, "checkdate", fn_checkdate, 3, Some(3));
    put(m, "getdate", fn_getdate, 1, Some(1));
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
                &[s("Y|y|m|n|d|j|H|G|h|g|i|s|A|a|D|l|N|w|z|t|L|U"), Value::Int(1700000000)]
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
                &[s("Y|y|m|n|d|j|H|G|h|g|i|s|A|a|D|l|N|w|z|t|L|U"), Value::Int(0)]
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
        assert_eq!(ordinal_suffix(111), b"th");
        assert_eq!(ordinal_suffix(101), b"st");
    }

    #[test]
    fn date_twelve_hour_and_escapes() {
        assert_eq!(calls(fn_date, &[s("g h A"), Value::Int(946684800)]), "12 12 AM");
        assert_eq!(calls(fn_date, &[s("g h A"), Value::Int(946728000)]), "12 12 PM");
        assert_eq!(calls(fn_date, &[s("h:i:s A"), Value::Int(1700000000)]), "10:13:20 PM");
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
