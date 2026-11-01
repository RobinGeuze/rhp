//! Native low-level helpers backing the OOP date/time classes (`DateTime`,
//! `DateTimeImmutable`, `DateInterval`, `DateTimeZone`), whose PHP-level class
//! bodies live in the compiler `PRELUDE`.
//!
//! The classes keep the ergonomic surface (constructors, method dispatch,
//! chaining, immutability) in PHP source; this module provides only the pieces
//! that need real calendar/clock math, each verified byte-for-byte against
//! php 8.5.8 (engine timezone = UTC; fixed numeric offsets supported):
//!
//! * `__dt_now`    — current wall clock `[sec, usec]` (non-deterministic).
//! * `__dt_parse`  — parse a datetime string to `[ok, ts, us, off, zname]`.
//! * `__dt_fmt`    — render the full php date() format-char set with offset/tz.
//! * `__dt_modify` — apply a relative modifier string (`+1 day`, …).
//! * `__dt_addsub` — add/subtract a broken-down interval (calendar aware).
//! * `__dt_diff`   — calendar difference of two instants → interval fields.
//! * `__iv_parse`  — parse an ISO-8601 duration (`P1Y2M10DT2H30M15S`).
//! * `__iv_fmt`    — render `DateInterval::format()`.
//! * `__dtz_parse` — parse a timezone name/offset → `[ok, name, off]`.
//!
//! These helpers are internal plumbing: the names are prefixed with `__` so they
//! do not shadow any real php function, and the self-contained civil-date math
//! is duplicated here rather than reaching into `datetime_fns`.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use rhp_values::{conv, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// value plumbing
// ---------------------------------------------------------------------------

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn arg_int(args: &[Value], i: usize) -> i64 {
    conv::to_int(&deref(&args[i]))
}

fn arg_str(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

/// A list-style PHP array `[v0, v1, ...]`.
fn list(vals: Vec<Value>) -> Value {
    let mut a = PhpArray::new();
    for v in vals {
        a.push(v);
    }
    Value::Array(a)
}

// ---------------------------------------------------------------------------
// civil date math (self-contained; Howard Hinnant's algorithms)
// ---------------------------------------------------------------------------

const SECS_PER_DAY: i64 = 86400;

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    // normalize m into 1..12 with year carry so callers can pass m-1 etc.
    let y = y + (m - 1).div_euclid(12);
    let m = (m - 1).rem_euclid(12) + 1;
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

struct Tm {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    min: i64,
    sec: i64,
    wday: i64, // 0 = Sunday
    yday: i64, // 0-based
}

/// Break an absolute instant down into its UTC wall-clock after applying `off`.
fn break_down(ts: i64) -> Tm {
    let days = ts.div_euclid(SECS_PER_DAY);
    let rem = ts.rem_euclid(SECS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let wday = (days.rem_euclid(7) + 4).rem_euclid(7);
    let yday = days - days_from_civil(year, 1, 1);
    Tm {
        year,
        month,
        day,
        hour: rem / 3600,
        min: (rem % 3600) / 60,
        sec: rem % 60,
        wday,
        yday,
    }
}

const DAY_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const DAY_FULL: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTH_FULL: [&str; 12] = [
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

// ---------------------------------------------------------------------------
// ISO-8601 week / week-year
// ---------------------------------------------------------------------------

fn iso_dow(wday: i64) -> i64 {
    if wday == 0 {
        7
    } else {
        wday
    }
}

/// Number of ISO weeks (52 or 53) in the ISO week-year `y`.
fn iso_weeks_in_year(y: i64) -> i64 {
    let p = |y: i64| (y + y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400)).rem_euclid(7);
    if p(y) == 4 || p(y - 1) == 3 {
        53
    } else {
        52
    }
}

fn iso_week(tm: &Tm) -> (i64, i64) {
    let ord = tm.yday + 1; // 1-based day of year
    let wd = iso_dow(tm.wday);
    let mut week = (ord - wd + 10).div_euclid(7);
    if week < 1 {
        (tm.year - 1, iso_weeks_in_year(tm.year - 1))
    } else if week > iso_weeks_in_year(tm.year) {
        week = 1;
        (tm.year + 1, week)
    } else {
        (tm.year, week)
    }
}

fn ordinal_suffix(day: i64) -> &'static str {
    match (day % 100, day % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    }
}

fn hour12(h: i64) -> i64 {
    let h = h % 12;
    if h == 0 {
        12
    } else {
        h
    }
}

// ---------------------------------------------------------------------------
// timezone naming
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum ZoneKind {
    Utc,
    Offset,
    Named,
}

fn zone_kind(zname: &[u8]) -> ZoneKind {
    if zname == b"UTC" {
        ZoneKind::Utc
    } else if is_offset_name(zname) {
        ZoneKind::Offset
    } else {
        ZoneKind::Named
    }
}

fn is_offset_name(z: &[u8]) -> bool {
    // "+HH:MM" / "-HH:MM"
    z.len() == 6
        && (z[0] == b'+' || z[0] == b'-')
        && z[1].is_ascii_digit()
        && z[2].is_ascii_digit()
        && z[3] == b':'
        && z[4].is_ascii_digit()
        && z[5].is_ascii_digit()
}

/// `+HHMM` form (O).
fn offset_hhmm(off: i64) -> String {
    let sign = if off < 0 { '-' } else { '+' };
    let a = off.abs();
    format!("{sign}{:02}{:02}", a / 3600, (a % 3600) / 60)
}

/// `+HH:MM` form (P).
fn offset_hh_mm(off: i64) -> String {
    let sign = if off < 0 { '-' } else { '+' };
    let a = off.abs();
    format!("{sign}{:02}:{:02}", a / 3600, (a % 3600) / 60)
}

// ---------------------------------------------------------------------------
// __dt_now
// ---------------------------------------------------------------------------

fn fn_dt_now(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(list(vec![
        Value::Int(d.as_secs() as i64),
        Value::Int(d.subsec_micros() as i64),
    ]))
}

// ---------------------------------------------------------------------------
// __dt_fmt
// ---------------------------------------------------------------------------

fn fn_dt_fmt(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ts = arg_int(args, 0);
    let us = arg_int(args, 1);
    let off = arg_int(args, 2);
    let zname = arg_str(args, 3);
    let fmt = arg_str(args, 4);
    Ok(Value::str(format_dt(ts, us, off, &zname, &fmt)))
}

/// Extra zone detail for `date()` / `gmdate()` on a database zone.
pub(crate) struct ZoneX {
    pub(crate) abbr: Vec<u8>,
    pub(crate) dst: bool,
    /// false for `gmdate()` (php_date.c `localtime == 0`).
    pub(crate) localtime: bool,
}

fn format_dt(ts: i64, us: i64, off: i64, zname: &[u8], fmt: &[u8]) -> Vec<u8> {
    format_dt_x(ts, us, off, zname, None, fmt)
}

/// `date()` / `gmdate()` rendering with a database zone's abbreviation/dst.
pub(crate) fn format_dt_x(
    ts: i64,
    us: i64,
    off: i64,
    zname: &[u8],
    x: Option<&ZoneX>,
    fmt: &[u8],
) -> Vec<u8> {
    let tm = break_down(ts + off);
    let kind = zone_kind(zname);
    let mut out = Vec::with_capacity(fmt.len() + 16);
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        if c == b'\\' {
            if i + 1 < fmt.len() {
                out.push(fmt[i + 1]);
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        emit_char(&mut out, c, &tm, ts, us, off, zname, kind, x);
        i += 1;
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn emit_char(
    out: &mut Vec<u8>,
    c: u8,
    tm: &Tm,
    ts: i64,
    us: i64,
    off: i64,
    zname: &[u8],
    kind: ZoneKind,
    x: Option<&ZoneX>,
) {
    let push = |out: &mut Vec<u8>, s: &str| out.extend_from_slice(s.as_bytes());
    let pint = |out: &mut Vec<u8>, n: i64| out.extend_from_slice(n.to_string().as_bytes());
    let p2 = |out: &mut Vec<u8>, n: i64| out.extend_from_slice(format!("{n:02}").as_bytes());
    match c {
        // day
        b'd' => p2(out, tm.day),
        b'j' => pint(out, tm.day),
        b'D' => push(out, DAY_SHORT[tm.wday as usize]),
        b'l' => push(out, DAY_FULL[tm.wday as usize]),
        b'N' => pint(out, iso_dow(tm.wday)),
        b'w' => pint(out, tm.wday),
        b'S' => push(out, ordinal_suffix(tm.day)),
        b'z' => pint(out, tm.yday),
        // week
        b'W' => p2(out, iso_week(tm).1),
        // month
        b'F' => push(out, MONTH_FULL[(tm.month - 1) as usize]),
        b'M' => push(out, &MONTH_FULL[(tm.month - 1) as usize][..3]),
        b'm' => p2(out, tm.month),
        b'n' => pint(out, tm.month),
        b't' => pint(out, days_in_month(tm.year, tm.month)),
        // year
        b'L' => out.push(if is_leap(tm.year) { b'1' } else { b'0' }),
        b'o' => pint(out, iso_week(tm).0),
        b'Y' => pint(out, tm.year),
        b'y' => p2(out, tm.year.rem_euclid(100)),
        b'X' => {
            let s = if tm.year < 0 { '-' } else { '+' };
            out.extend_from_slice(format!("{s}{:04}", tm.year.abs()).as_bytes());
        }
        b'x' => {
            if tm.year < 0 || tm.year > 9999 {
                let s = if tm.year < 0 { '-' } else { '+' };
                out.extend_from_slice(format!("{s}{:04}", tm.year.abs()).as_bytes());
            } else {
                out.extend_from_slice(format!("{:04}", tm.year).as_bytes());
            }
        }
        // time
        b'a' => push(out, if tm.hour < 12 { "am" } else { "pm" }),
        b'A' => push(out, if tm.hour < 12 { "AM" } else { "PM" }),
        b'g' => pint(out, hour12(tm.hour)),
        b'G' => pint(out, tm.hour),
        b'h' => p2(out, hour12(tm.hour)),
        b'H' => p2(out, tm.hour),
        b'i' => p2(out, tm.min),
        b's' => p2(out, tm.sec),
        b'u' => out.extend_from_slice(format!("{us:06}").as_bytes()),
        b'v' => out.extend_from_slice(format!("{:03}", us / 1000).as_bytes()),
        b'B' => {
            let secs_of_day = (ts + 3600).rem_euclid(SECS_PER_DAY);
            let beats = (secs_of_day as f64 / 86.4) as i64;
            out.extend_from_slice(format!("{beats:03}").as_bytes());
        }
        // timezone
        b'e' => match kind {
            ZoneKind::Utc => push(out, "UTC"),
            _ => out.extend_from_slice(zname),
        },
        b'T' if x.is_some() => {
            let x = x.unwrap();
            if x.localtime {
                out.extend_from_slice(&x.abbr.to_ascii_uppercase());
            } else {
                push(out, "GMT");
            }
        }
        b'T' => match kind {
            ZoneKind::Utc => push(out, "UTC"),
            ZoneKind::Named => out.extend_from_slice(zname),
            ZoneKind::Offset => push(out, &format!("GMT{}", offset_hhmm(off))),
        },
        b'I' => out.push(if x.is_some_and(|x| x.dst) { b'1' } else { b'0' }),
        b'O' => push(out, &offset_hhmm(off)),
        b'P' => push(out, &offset_hh_mm(off)),
        b'p' if x.is_some() => {
            let x = x.unwrap();
            if !x.localtime || matches!(&x.abbr[..], b"UTC" | b"Z" | b"GMT+0000") {
                out.push(b'Z');
            } else {
                push(out, &offset_hh_mm(off));
            }
        }
        b'p' => {
            if off == 0 && kind != ZoneKind::Named {
                out.push(b'Z');
            } else {
                push(out, &offset_hh_mm(off));
            }
        }
        b'Z' => pint(out, off),
        // composite
        b'c' => out.extend_from_slice(&format_dt_x(ts, us, off, zname, x, b"Y-m-d\\TH:i:sP")),
        b'r' => out.extend_from_slice(&format_dt_x(ts, us, off, zname, x, b"D, d M Y H:i:s O")),
        b'U' => pint(out, ts),
        other => out.push(other),
    }
}

// ---------------------------------------------------------------------------
// parsing: datetime strings
// ---------------------------------------------------------------------------

/// Parse an ASCII unsigned integer; returns (value, bytes_consumed).
fn take_uint(b: &[u8]) -> (i64, usize) {
    let mut n = 0i64;
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        n = n * 10 + (b[i] - b'0') as i64;
        i += 1;
    }
    (n, i)
}

/// Parse an explicit trailing timezone: "Z", "+HH:MM", "-HHMM", "+HH".
/// Returns (offset_secs, zone_name, consumed) or None.
fn parse_tz_suffix(b: &[u8]) -> Option<(i64, Vec<u8>, usize)> {
    if b.is_empty() {
        return None;
    }
    if b[0] == b'Z' {
        return Some((0, b"Z".to_vec(), 1));
    }
    if b[0] != b'+' && b[0] != b'-' {
        return None;
    }
    let sign = if b[0] == b'-' { -1 } else { 1 };
    let rest = &b[1..];
    // collect the digit/colon run
    let (hh, n1) = take_uint(rest);
    if n1 == 0 {
        return None;
    }
    let mut consumed = 1 + n1;
    let mut mm = 0i64;
    if consumed < b.len() && b[consumed] == b':' {
        let (m, n2) = take_uint(&b[consumed + 1..]);
        if n2 == 0 {
            return None;
        }
        mm = m;
        consumed += 1 + n2;
    } else if n1 == 4 {
        // "+HHMM"
        mm = hh % 100;
        let off = sign * (((hh / 100) * 60 + mm) * 60);
        return Some((off, offset_hh_mm(off).into_bytes(), consumed));
    }
    let off = sign * ((hh * 60 + mm) * 60);
    Some((off, offset_hh_mm(off).into_bytes(), consumed))
}

/// Result: Some((ts, us, off, zname)) where `zname` empty means "no explicit
/// zone in the string — use the default timezone".
fn parse_datetime(
    s: &[u8],
    now_sec: i64,
    now_us: i64,
    default_off: i64,
) -> Option<(i64, i64, i64, Vec<u8>)> {
    let t = trim(s);
    if t.is_empty() || t.eq_ignore_ascii_case(b"now") {
        return Some((now_sec, now_us, default_off, Vec::new()));
    }
    // "@<ts>" (optionally fractional) — always UTC/+00:00.
    if t[0] == b'@' {
        let body = &t[1..];
        let (neg, body) = if !body.is_empty() && body[0] == b'-' {
            (true, &body[1..])
        } else {
            (false, body)
        };
        let (secs, n) = take_uint(body);
        if n == 0 {
            return None;
        }
        let mut us = 0i64;
        let mut rest = &body[n..];
        if !rest.is_empty() && rest[0] == b'.' {
            let (frac, fln) = take_uint(&rest[1..]);
            if fln > 0 {
                // scale fractional to microseconds (6 digits)
                let mut f = frac;
                let mut digits = fln;
                while digits < 6 {
                    f *= 10;
                    digits += 1;
                }
                while digits > 6 {
                    f /= 10;
                    digits -= 1;
                }
                us = f;
            }
            rest = &rest[1 + fln..];
        }
        if !trim(rest).is_empty() {
            return None;
        }
        let sec = if neg { -secs } else { secs };
        return Some((sec, us, 0, b"+00:00".to_vec()));
    }
    // absolute calendar forms
    if let Some(r) = parse_absolute(t, default_off) {
        return Some(r);
    }
    // relative ("+1 day", "-2 months", ...)
    if let Some((ts, us)) = apply_modify(now_sec, now_us, default_off, t) {
        return Some((ts, us, default_off, Vec::new()));
    }
    None
}

fn trim(b: &[u8]) -> &[u8] {
    let mut s = 0;
    let mut e = b.len();
    while s < e && b[s].is_ascii_whitespace() {
        s += 1;
    }
    while e > s && b[e - 1].is_ascii_whitespace() {
        e -= 1;
    }
    &b[s..e]
}

/// Parse "Y-m-d", "Y/m/d", "Y-m-d H:i:s", "Y-m-dTH:i:s[.ffffff][tz]".
fn parse_absolute(t: &[u8], default_off: i64) -> Option<(i64, i64, i64, Vec<u8>)> {
    // year
    let (year, n) = take_uint(t);
    if n < 4 {
        return None;
    }
    let mut i = n;
    let sep = *t.get(i)?;
    if sep != b'-' && sep != b'/' {
        return None;
    }
    i += 1;
    let (month, n) = take_uint(&t[i..]);
    if n == 0 || !(1..=12).contains(&month) {
        return None;
    }
    i += n;
    if t.get(i) != Some(&sep) {
        return None;
    }
    i += 1;
    let (day, n) = take_uint(&t[i..]);
    if n == 0 || !(1..=31).contains(&day) {
        return None;
    }
    i += n;

    let (mut hh, mut mi, mut ss, mut us) = (0i64, 0i64, 0i64, 0i64);
    let mut off = default_off;
    let mut zname: Vec<u8> = Vec::new();

    let rest = &t[i..];
    let rest = trim_left(rest);
    if !rest.is_empty() {
        // optional time, preceded by space or 'T'
        let rest = if rest[0] == b'T' || rest[0] == b't' {
            &rest[1..]
        } else {
            rest
        };
        let (h, n) = take_uint(rest);
        if n == 0 {
            return None;
        }
        hh = h;
        let mut j = n;
        if rest.get(j) == Some(&b':') {
            let (m, n) = take_uint(&rest[j + 1..]);
            if n == 0 {
                return None;
            }
            mi = m;
            j += 1 + n;
            if rest.get(j) == Some(&b':') {
                let (s, n) = take_uint(&rest[j + 1..]);
                if n == 0 {
                    return None;
                }
                ss = s;
                j += 1 + n;
                if rest.get(j) == Some(&b'.') {
                    let (frac, fln) = take_uint(&rest[j + 1..]);
                    if fln > 0 {
                        let mut f = frac;
                        let mut d = fln;
                        while d < 6 {
                            f *= 10;
                            d += 1;
                        }
                        while d > 6 {
                            f /= 10;
                            d -= 1;
                        }
                        us = f;
                        j += 1 + fln;
                    }
                }
            }
        }
        let tail = trim_left(&rest[j..]);
        if !tail.is_empty() {
            let (o, zn, consumed) = parse_tz_suffix(tail)?;
            off = o;
            zname = zn;
            if !trim(&tail[consumed..]).is_empty() {
                return None;
            }
        }
    }

    if hh > 24 || mi > 59 || ss > 61 {
        return None;
    }
    let base_days = days_from_civil(year, month, 1) + (day - 1);
    let wall = base_days * SECS_PER_DAY + hh * 3600 + mi * 60 + ss;
    Some((wall - off, us, off, zname))
}

fn trim_left(b: &[u8]) -> &[u8] {
    let mut s = 0;
    while s < b.len() && b[s].is_ascii_whitespace() {
        s += 1;
    }
    &b[s..]
}

fn fn_dt_parse(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = arg_str(args, 0);
    let now_sec = arg_int(args, 1);
    let now_us = arg_int(args, 2);
    let default_off = arg_int(args, 3);
    match parse_datetime(&s, now_sec, now_us, default_off) {
        Some((ts, us, off, zname)) => Ok(list(vec![
            Value::Bool(true),
            Value::Int(ts),
            Value::Int(us),
            Value::Int(off),
            Value::str(zname),
        ])),
        None => Ok(list(vec![Value::Bool(false)])),
    }
}

// ---------------------------------------------------------------------------
// modify (relative strings)
// ---------------------------------------------------------------------------

/// Apply a relative modifier string to (ts, us). Returns None on parse failure.
fn apply_modify(ts: i64, us: i64, off: i64, s: &[u8]) -> Option<(i64, i64)> {
    let lower: Vec<u8> = s.iter().map(|b| b.to_ascii_lowercase()).collect();
    let toks: Vec<&[u8]> = lower
        .split(|b| b.is_ascii_whitespace() || *b == b',')
        .filter(|t| !t.is_empty())
        .collect();
    if toks.is_empty() {
        return None;
    }
    let (mut dy, mut dmo, mut dsec) = (0i64, 0i64, 0i64);
    let mut i = 0;
    let mut matched_any = false;
    while i < toks.len() {
        // number (with optional sign), possibly glued to unit later
        let num_tok = toks[i];
        let (sign, digits) = match num_tok.first() {
            Some(b'+') => (1, &num_tok[1..]),
            Some(b'-') => (-1, &num_tok[1..]),
            _ => (1, num_tok),
        };
        if digits.is_empty() || !digits.iter().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let (val, _) = take_uint(digits);
        let val = sign * val;
        i += 1;
        if i >= toks.len() {
            return None;
        }
        let unit = toks[i];
        i += 1;
        match unit_kind(unit)? {
            Unit::Year => dy += val,
            Unit::Month => dmo += val,
            Unit::Week => dsec += val * 7 * SECS_PER_DAY,
            Unit::Day => dsec += val * SECS_PER_DAY,
            Unit::Hour => dsec += val * 3600,
            Unit::Minute => dsec += val * 60,
            Unit::Second => dsec += val,
        }
        matched_any = true;
    }
    if !matched_any {
        return None;
    }
    let tm = break_down(ts + off);
    let year = tm.year + dy;
    let month = tm.month + dmo;
    let year2 = year + (month - 1).div_euclid(12);
    let month2 = (month - 1).rem_euclid(12) + 1;
    let base_days = days_from_civil(year2, month2, 1) + (tm.day - 1);
    let wall = base_days * SECS_PER_DAY + tm.hour * 3600 + tm.min * 60 + tm.sec + dsec;
    Some((wall - off, us))
}

enum Unit {
    Year,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
}

fn unit_kind(u: &[u8]) -> Option<Unit> {
    let u = if u.len() > 1 && u.ends_with(b"s") {
        &u[..u.len() - 1]
    } else {
        u
    };
    match u {
        b"year" | b"yr" => Some(Unit::Year),
        b"month" | b"mon" => Some(Unit::Month),
        b"week" => Some(Unit::Week),
        b"day" => Some(Unit::Day),
        b"hour" | b"hr" => Some(Unit::Hour),
        b"minute" | b"min" => Some(Unit::Minute),
        b"second" | b"sec" => Some(Unit::Second),
        _ => None,
    }
}

fn fn_dt_modify(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ts = arg_int(args, 0);
    let us = arg_int(args, 1);
    let off = arg_int(args, 2);
    let s = arg_str(args, 3);
    match apply_modify(ts, us, off, &s) {
        Some((ts, us)) => Ok(list(vec![
            Value::Bool(true),
            Value::Int(ts),
            Value::Int(us),
        ])),
        None => Ok(list(vec![Value::Bool(false)])),
    }
}

// ---------------------------------------------------------------------------
// add / sub
// ---------------------------------------------------------------------------

fn fn_dt_addsub(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ts = arg_int(args, 0);
    let us = arg_int(args, 1);
    let off = arg_int(args, 2);
    let y = arg_int(args, 3);
    let mo = arg_int(args, 4);
    let d = arg_int(args, 5);
    let h = arg_int(args, 6);
    let i = arg_int(args, 7);
    let s = arg_int(args, 8);
    let f = arg_int(args, 9);
    let mul = arg_int(args, 10);

    let tm = break_down(ts + off);
    let year = tm.year + mul * y;
    let month = tm.month + mul * mo;
    let year2 = year + (month - 1).div_euclid(12);
    let month2 = (month - 1).rem_euclid(12) + 1;
    let base_days = days_from_civil(year2, month2, 1) + (tm.day - 1);
    let wall = base_days * SECS_PER_DAY + tm.hour * 3600 + tm.min * 60 + tm.sec;
    let wall = wall + mul * (d * SECS_PER_DAY + h * 3600 + i * 60 + s);

    let total_us = us + mul * f;
    let carry = total_us.div_euclid(1_000_000);
    let us_final = total_us.rem_euclid(1_000_000);
    Ok(list(vec![
        Value::Int(wall - off + carry),
        Value::Int(us_final),
    ]))
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

fn fn_dt_diff(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ts1 = arg_int(args, 0);
    let us1 = arg_int(args, 1);
    let ts2 = arg_int(args, 2);
    let us2 = arg_int(args, 3);

    let micro1 = ts1 * 1_000_000 + us1;
    let micro2 = ts2 * 1_000_000 + us2;
    let invert = if micro1 > micro2 { 1 } else { 0 };
    let (one, two, one_us, two_us) = if invert == 1 {
        (ts2, ts1, us2, us1)
    } else {
        (ts1, ts2, us1, us2)
    };

    let a = break_down(one);
    let b = break_down(two);
    let mut f = two_us - one_us;
    let mut s = b.sec - a.sec;
    let mut i = b.min - a.min;
    let mut h = b.hour - a.hour;
    let mut d = b.day - a.day;
    let mut mo = b.month - a.month;
    let mut y = b.year - a.year;

    if f < 0 {
        f += 1_000_000;
        s -= 1;
    }
    if s < 0 {
        s += 60;
        i -= 1;
    }
    if i < 0 {
        i += 60;
        h -= 1;
    }
    if h < 0 {
        h += 24;
        d -= 1;
    }
    if d < 0 {
        // borrow months. The base month to draw day-lengths from, and the
        // direction it walks, differ by the sign of the diff — matched to php
        // 8.5.8 over a 3000-case random battery.
        if invert == 0 {
            let (mut by, mut bm) = (b.year, b.month);
            while d < 0 {
                bm -= 1;
                if bm < 1 {
                    bm += 12;
                    by -= 1;
                }
                d += days_in_month(by, bm);
                mo -= 1;
            }
        } else {
            let (mut by, mut bm) = (a.year, a.month);
            while d < 0 {
                d += days_in_month(by, bm);
                mo -= 1;
                bm -= 1;
                if bm < 1 {
                    bm += 12;
                    by -= 1;
                }
            }
        }
    }
    if mo < 0 {
        mo += 12;
        y -= 1;
    }

    let days = (micro2 - micro1).abs() / (1_000_000 * SECS_PER_DAY);
    Ok(list(vec![
        Value::Int(y),
        Value::Int(mo),
        Value::Int(d),
        Value::Int(h),
        Value::Int(i),
        Value::Int(s),
        Value::Int(f),
        Value::Int(days),
        Value::Int(invert),
    ]))
}

// ---------------------------------------------------------------------------
// interval parsing / formatting
// ---------------------------------------------------------------------------

/// Parse an ISO-8601 duration, e.g. `P1Y2M10DT2H30M15S`, `P1W`, `PT30M`.
fn parse_interval(spec: &[u8]) -> Option<[i64; 7]> {
    // [y, m, d, h, i, s, f]
    let mut out = [0i64; 7];
    if spec.is_empty() || spec[0] != b'P' {
        return None;
    }
    let mut i = 1;
    let mut in_time = false;
    let mut saw_any = false;
    while i < spec.len() {
        if spec[i] == b'T' {
            in_time = true;
            i += 1;
            continue;
        }
        let (n, consumed) = take_uint(&spec[i..]);
        if consumed == 0 {
            return None;
        }
        i += consumed;
        let desig = *spec.get(i)?;
        i += 1;
        match (in_time, desig) {
            (false, b'Y') => out[0] = n,
            (false, b'M') => out[1] = n,
            (false, b'W') => out[2] += n * 7,
            (false, b'D') => out[2] += n,
            (true, b'H') => out[3] = n,
            (true, b'M') => out[4] = n,
            (true, b'S') => out[5] = n,
            _ => return None,
        }
        saw_any = true;
    }
    if !saw_any {
        return None;
    }
    Some(out)
}

fn fn_iv_parse(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let spec = arg_str(args, 0);
    match parse_interval(&spec) {
        Some(v) => {
            let mut out = vec![Value::Bool(true)];
            out.extend(v.iter().map(|n| Value::Int(*n)));
            Ok(list(out))
        }
        None => Ok(list(vec![Value::Bool(false)])),
    }
}

fn fn_iv_fmt(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fmt = arg_str(args, 0);
    let y = arg_int(args, 1);
    let m = arg_int(args, 2);
    let d = arg_int(args, 3);
    let h = arg_int(args, 4);
    let i = arg_int(args, 5);
    let s = arg_int(args, 6);
    let invert = arg_int(args, 7);
    // days: -1 encodes the "false" (unknown) state for spec-built intervals.
    let days = arg_int(args, 8);

    let mut out = Vec::with_capacity(fmt.len());
    let mut k = 0;
    while k < fmt.len() {
        if fmt[k] != b'%' {
            out.push(fmt[k]);
            k += 1;
            continue;
        }
        k += 1;
        let Some(&spec) = fmt.get(k) else {
            out.push(b'%');
            break;
        };
        k += 1;
        let p = |out: &mut Vec<u8>, n: i64| out.extend_from_slice(n.to_string().as_bytes());
        let p2 = |out: &mut Vec<u8>, n: i64| out.extend_from_slice(format!("{n:02}").as_bytes());
        match spec {
            b'Y' => p2(&mut out, y),
            b'y' => p(&mut out, y),
            b'M' => p2(&mut out, m),
            b'm' => p(&mut out, m),
            b'D' => p2(&mut out, d),
            b'd' => p(&mut out, d),
            b'H' => p2(&mut out, h),
            b'h' => p(&mut out, h),
            b'I' => p2(&mut out, i),
            b'i' => p(&mut out, i),
            b'S' => p2(&mut out, s),
            b's' => p(&mut out, s),
            b'a' => {
                if days < 0 {
                    out.extend_from_slice(b"(unknown)");
                } else {
                    p(&mut out, days);
                }
            }
            b'R' => out.push(if invert == 1 { b'-' } else { b'+' }),
            b'r' => {
                if invert == 1 {
                    out.push(b'-');
                }
            }
            b'%' => out.push(b'%'),
            other => {
                out.push(b'%');
                out.push(other);
            }
        }
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// timezone parsing
// ---------------------------------------------------------------------------

/// Parse a timezone name into `(normalized_name, offset_secs)`.
/// Supports UTC, GMT, Z, and numeric offsets; everything else is refused.
fn parse_zone(name: &[u8]) -> Option<(Vec<u8>, i64)> {
    let t = trim(name);
    if t.eq_ignore_ascii_case(b"utc") {
        return Some((b"UTC".to_vec(), 0));
    }
    if t.eq_ignore_ascii_case(b"gmt") {
        return Some((b"GMT".to_vec(), 0));
    }
    if t == b"Z" {
        return Some((b"Z".to_vec(), 0));
    }
    if t.first() == Some(&b'+') || t.first() == Some(&b'-') {
        if let Some((off, zn, consumed)) = parse_tz_suffix(t) {
            if consumed == t.len() {
                return Some((zn, off));
            }
        }
    }
    None
}

fn fn_dtz_parse(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = arg_str(args, 0);
    match parse_zone(&name) {
        Some((n, off)) => Ok(list(vec![
            Value::Bool(true),
            Value::str(n),
            Value::Int(off),
        ])),
        None => Ok(list(vec![Value::Bool(false)])),
    }
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

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
    put(m, "__dt_now", fn_dt_now, 0, Some(0));
    put(m, "__dt_parse", fn_dt_parse, 4, Some(4));
    put(m, "__dt_fmt", fn_dt_fmt, 5, Some(5));
    put(m, "__dt_modify", fn_dt_modify, 4, Some(4));
    put(m, "__dt_addsub", fn_dt_addsub, 11, Some(11));
    put(m, "__dt_diff", fn_dt_diff, 4, Some(4));
    put(m, "__iv_parse", fn_iv_parse, 1, Some(1));
    put(m, "__iv_fmt", fn_iv_fmt, 9, Some(9));
    put(m, "__dtz_parse", fn_dtz_parse, 1, Some(1));
}
