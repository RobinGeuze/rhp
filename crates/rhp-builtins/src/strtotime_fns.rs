//! `strtotime()` — parse an English date/time description into a Unix timestamp.
//!
//! A string without its own zone is local time in the default zone. The
//! calendar/clock math (civil<->timestamp
//! conversions, month-length, broken-down time, 2-digit-year adjustment) is
//! *reused* from [`crate::datetime_fns`] rather than re-implemented here; this
//! module only adds the grammar on top.
//!
//! ## Supported input forms
//! * Absolute dates: `Y-m-d`, `Y/m/d`, `m/d/Y`, `d-m-Y`, each with an optional
//!   `H:i` / `H:i:s` time (space- or `T`-separated), and the ISO-8601 form
//!   `Y-m-dTH:i:s` with a trailing `Z`, `+HH:MM`, `+HHMM` or `+HH` zone.
//! * Textual dates: `D Month Y`, `Month D, Y`, `D Mon YY` (with an optional
//!   trailing `H:i:s`).
//! * Time-only: `H:i` / `H:i:s` → that time on the base day.
//! * Epoch: `@<seconds>` with optional fractional seconds (floored to an int).
//! * Keywords: `now`, `today`, `midnight`, `noon`, `tomorrow`, `yesterday`.
//! * Relative: one or more signed `N <unit>` groups (`year month week fortnight
//!   day hour minute second`, plus the `yr mon hr min sec` abbreviations), an
//!   optional trailing `ago`, and `next/last week`, `next/last/previous month`,
//!   `next/last year`.
//! * Weekdays: `<weekday>`, `this/next/last <weekday>`.
//! * Compound: `first day of` / `last day of` + (`this/next/previous month`,
//!   `<Month> <Year>`).
//!
//! Case-insensitive; surrounding and internal whitespace is tolerated; commas
//! are treated as separators. Anything else yields `false`.
//!
//! ## Out of scope
//! * Locale month/day names beyond English.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use rhp_values::{conv, Value};

use crate::datetime_fns::{
    adjust_year, break_down, days_from_civil, days_in_month, DAY_FULL, MONTH_FULL, SECS_PER_DAY,
};
use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

// ---------------------------------------------------------------------------
// tiny byte-slice scanners
// ---------------------------------------------------------------------------

/// Parse a leading run of ASCII digits; returns `(value, digits_consumed)`.
fn take_uint(b: &[u8]) -> (i64, usize) {
    let mut n = 0i64;
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        n = n * 10 + (b[i] - b'0') as i64;
        i += 1;
    }
    (n, i)
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

fn trim_left(b: &[u8]) -> &[u8] {
    let mut s = 0;
    while s < b.len() && b[s].is_ascii_whitespace() {
        s += 1;
    }
    &b[s..]
}

fn is_digits(b: &[u8]) -> bool {
    !b.is_empty() && b.iter().all(|c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// naming → number
// ---------------------------------------------------------------------------

/// Month number (1..12) for an English month name (full or 3-letter prefix).
fn month_num(tok: &[u8]) -> Option<i64> {
    for (i, name) in MONTH_FULL.iter().enumerate() {
        if tok.eq_ignore_ascii_case(name)
            || (name.len() >= 3 && tok.eq_ignore_ascii_case(&name[..3]))
        {
            return Some(i as i64 + 1);
        }
    }
    None
}

/// Weekday number (0 = Sunday .. 6 = Saturday) for an English weekday name.
fn weekday_num(tok: &[u8]) -> Option<i64> {
    for (i, name) in DAY_FULL.iter().enumerate() {
        if tok.eq_ignore_ascii_case(name)
            || (name.len() >= 3 && tok.eq_ignore_ascii_case(&name[..3]))
        {
            return Some(i as i64);
        }
    }
    None
}

/// Parse a 2- or 4-digit year token, applying PHP's 2-digit-year window.
fn parse_year(tok: &[u8]) -> Option<i64> {
    if !is_digits(tok) {
        return None;
    }
    Some(adjust_year(take_uint(tok).0))
}

// ---------------------------------------------------------------------------
// civil helpers (thin wrappers over datetime_fns)
// ---------------------------------------------------------------------------

/// Timestamp at `y-m-d` plus `tod` seconds-of-day. `day`/`month` may overflow
/// their natural range; the overflow rolls forward exactly as php does.
fn civil_ts(y: i64, m: i64, day: i64, tod: i64) -> i64 {
    (days_from_civil(y, m, 1) + (day - 1)) * SECS_PER_DAY + tod
}

/// Add `delta` months to `(year, month)`, normalizing into 1..12.
fn add_months(year: i64, month: i64, delta: i64) -> (i64, i64) {
    let total = (month - 1) + delta;
    (year + total.div_euclid(12), total.rem_euclid(12) + 1)
}

/// Seconds-of-day for `ts` (0..86399).
fn tod(ts: i64) -> i64 {
    ts.rem_euclid(SECS_PER_DAY)
}

/// Midnight (00:00:00) of the day containing `ts`.
fn day_start(ts: i64) -> i64 {
    ts - tod(ts)
}

// ---------------------------------------------------------------------------
// top-level dispatch
// ---------------------------------------------------------------------------

/// Parse `s` relative to `base` (Unix seconds, UTC). `None` ⇒ php `false`.
pub(crate) fn strtotime(s: &[u8], base: i64) -> Option<i64> {
    let t = trim(s);
    if t.is_empty() {
        return None;
    }
    if t[0] == b'@' {
        return parse_epoch(t);
    }

    let lower: Vec<u8> = t.iter().map(|b| b.to_ascii_lowercase()).collect();

    // comma-free, lowercased token view for the word-based grammars.
    let spaced: Vec<u8> = lower
        .iter()
        .map(|&b| if b == b',' { b' ' } else { b })
        .collect();
    let toks: Vec<&[u8]> = spaced
        .split(|c| c.is_ascii_whitespace())
        .filter(|x| !x.is_empty())
        .collect();

    parse_keyword(&toks, base)
        .or_else(|| parse_compound(&toks, base))
        .or_else(|| parse_prefix(&toks, base))
        .or_else(|| parse_bare_weekday(&toks, base))
        .or_else(|| parse_numeric_datetime(&lower, base))
        .or_else(|| parse_textual_date(&toks, base))
        .or_else(|| parse_time_only(&lower, base))
        .or_else(|| parse_relative(&toks, base))
}

// ---------------------------------------------------------------------------
// @epoch
// ---------------------------------------------------------------------------

fn parse_epoch(t: &[u8]) -> Option<i64> {
    EXPLICIT_ZONE.with(|e| e.set(true));
    let body = &t[1..];
    let (neg, body) = match body.first() {
        Some(b'-') => (true, &body[1..]),
        Some(b'+') => (false, &body[1..]),
        _ => (false, body),
    };
    let (int, n) = take_uint(body);
    if n == 0 {
        return None;
    }
    let mut rest = &body[n..];
    let mut frac_nonzero = false;
    if rest.first() == Some(&b'.') {
        let (frac, fln) = take_uint(&rest[1..]);
        if fln == 0 {
            return None;
        }
        frac_nonzero = frac > 0;
        rest = &rest[1 + fln..];
    }
    if !rest.is_empty() {
        return None;
    }
    // php floors toward negative infinity: @-100.9 → -101, @100.9 → 100.
    Some(if neg {
        if frac_nonzero {
            -int - 1
        } else {
            -int
        }
    } else {
        int
    })
}

// ---------------------------------------------------------------------------
// keywords
// ---------------------------------------------------------------------------

fn parse_keyword(toks: &[&[u8]], base: i64) -> Option<i64> {
    if toks.len() != 1 {
        return None;
    }
    let ds = day_start(base);
    match toks[0] {
        b"now" => Some(base),
        b"today" | b"midnight" => Some(ds),
        b"noon" => Some(ds + 12 * 3600),
        b"tomorrow" => Some(ds + SECS_PER_DAY),
        b"yesterday" => Some(ds - SECS_PER_DAY),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// compound: "first day of ..." / "last day of ..."
// ---------------------------------------------------------------------------

fn parse_compound(toks: &[&[u8]], base: i64) -> Option<i64> {
    if toks.len() < 4 {
        return None;
    }
    let first = match (toks[0], toks[1], toks[2]) {
        (b"first", b"day", b"of") => true,
        (b"last", b"day", b"of") => false,
        _ => return None,
    };
    let rest = &toks[3..];
    let (y, m, keep_time) = month_ref(rest, base)?;
    let day = if first { 1 } else { days_in_month(y, m) };
    let t = if keep_time { tod(base) } else { 0 };
    Some(civil_ts(y, m, day, t))
}

/// Resolve the "month context" that follows "first/last day of".
/// Returns `(year, month, keep_base_time)`.
fn month_ref(rest: &[&[u8]], base: i64) -> Option<(i64, i64, bool)> {
    let tm = break_down(base);
    if rest.len() == 2 && rest[1] == b"month" {
        return match rest[0] {
            b"this" => Some((tm.year, tm.month, true)),
            b"next" => {
                let (y, m) = add_months(tm.year, tm.month, 1);
                Some((y, m, true))
            }
            b"last" | b"previous" => {
                let (y, m) = add_months(tm.year, tm.month, -1);
                Some((y, m, true))
            }
            _ => None,
        };
    }
    if rest.len() == 2 {
        if let (Some(mo), Some(yr)) = (month_num(rest[0]), parse_year(rest[1])) {
            return Some((yr, mo, false));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// prefix: next / last / this / previous  +  weekday | week | month | year
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Dir {
    This,
    Next,
    Last,
}

fn parse_prefix(toks: &[&[u8]], base: i64) -> Option<i64> {
    if toks.len() != 2 {
        return None;
    }
    let (dir, prev) = match toks[0] {
        b"this" => (Dir::This, false),
        b"next" => (Dir::Next, false),
        b"last" => (Dir::Last, false),
        b"previous" => (Dir::Last, true),
        _ => return None,
    };

    if let Some(twd) = weekday_num(toks[1]) {
        if prev {
            return None; // "previous <weekday>" is not a php form
        }
        return Some(weekday_target(dir, twd, base));
    }

    let tm = break_down(base);
    match toks[1] {
        b"week" => {
            // php anchors next/last week to Monday 00:00 of that week,
            // keeping the base time-of-day.
            let base_day = base.div_euclid(SECS_PER_DAY);
            let monday_this = base_day - (tm.wday + 6).rem_euclid(7);
            let target = match dir {
                Dir::This => monday_this,
                Dir::Next => monday_this + 7,
                Dir::Last => monday_this - 7,
            };
            Some(target * SECS_PER_DAY + tod(base))
        }
        b"month" => {
            let d = match dir {
                Dir::This => 0,
                Dir::Next => 1,
                Dir::Last => -1,
            };
            let (y, m) = add_months(tm.year, tm.month, d);
            Some(civil_ts(y, m, tm.day, tod(base)))
        }
        b"year" => {
            let d = match dir {
                Dir::This => 0,
                Dir::Next => 12,
                Dir::Last => -12,
            };
            let (y, m) = add_months(tm.year, tm.month, d);
            Some(civil_ts(y, m, tm.day, tod(base)))
        }
        _ => None,
    }
}

/// Resolve a weekday relative to `base`, at midnight.
/// * `This`: the next occurrence on-or-after today (today if it matches).
/// * `Next`: strictly the following occurrence (never today).
/// * `Last`: strictly the previous occurrence (never today).
fn weekday_target(dir: Dir, twd: i64, base: i64) -> i64 {
    let base_day = base.div_euclid(SECS_PER_DAY);
    let bwd = break_down(base).wday;
    let day = match dir {
        Dir::This => base_day + (twd - bwd).rem_euclid(7),
        Dir::Next => {
            let mut d = (twd - bwd).rem_euclid(7);
            if d == 0 {
                d = 7;
            }
            base_day + d
        }
        Dir::Last => {
            let mut d = (bwd - twd).rem_euclid(7);
            if d == 0 {
                d = 7;
            }
            base_day - d
        }
    };
    day * SECS_PER_DAY
}

fn parse_bare_weekday(toks: &[&[u8]], base: i64) -> Option<i64> {
    if toks.len() != 1 {
        return None;
    }
    let twd = weekday_num(toks[0])?;
    Some(weekday_target(Dir::This, twd, base))
}

// ---------------------------------------------------------------------------
// absolute numeric date (+ optional time / zone)
// ---------------------------------------------------------------------------

fn parse_numeric_datetime(lower: &[u8], base: i64) -> Option<i64> {
    let _ = base; // absolute forms don't depend on base
    let t = trim(lower);
    // split the date token from an optional space-separated time.
    let (datepart, remainder) = match t.iter().position(|c| c.is_ascii_whitespace()) {
        Some(p) => (&t[..p], &t[p + 1..]),
        None => (t, &b""[..]),
    };
    // ISO forms glue the time on with a 'T' (lowercased to 't').
    let (dstr, inline_time) = match datepart.iter().position(|&c| c == b't') {
        Some(p) => (&datepart[..p], Some(&datepart[p + 1..])),
        None => (datepart, None),
    };

    let (y, mo, d) = parse_num_ymd(dstr)?;

    let time_src: &[u8] = match inline_time {
        Some(it) => it,
        None => trim(remainder),
    };
    let (hh, mi, ss, off) = if time_src.is_empty() {
        (0, 0, 0, 0)
    } else {
        parse_time_tz(time_src)?
    };

    Some(civil_ts(y, mo, d, hh * 3600 + mi * 60 + ss) - off)
}

/// Parse `Y-m-d`, `Y/m/d`, `m/d/Y`, `d-m-Y` (consuming the whole token).
fn parse_num_ymd(dstr: &[u8]) -> Option<(i64, i64, i64)> {
    let (a, na) = take_uint(dstr);
    if na == 0 {
        return None;
    }
    let sep = *dstr.get(na)?;
    if sep != b'-' && sep != b'/' {
        return None;
    }
    let (b, nb) = take_uint(&dstr[na + 1..]);
    if nb == 0 {
        return None;
    }
    let i2 = na + 1 + nb;
    if dstr.get(i2) != Some(&sep) {
        return None;
    }
    let (c, nc) = take_uint(&dstr[i2 + 1..]);
    if nc == 0 {
        return None;
    }
    if i2 + 1 + nc != dstr.len() {
        return None;
    }
    let (y, mo, d) = if na == 4 {
        (a, b, c) // Y-M-D / Y/M/D
    } else if nc == 4 {
        if sep == b'/' {
            (c, a, b) // M/D/Y (American)
        } else {
            (c, b, a) // D-M-Y (European)
        }
    } else {
        return None;
    };
    if !(1..=12).contains(&mo) || d < 1 {
        return None;
    }
    Some((y, mo, d))
}

/// Parse `H`, `H:i`, `H:i:s`, optional `.frac`, optional trailing zone.
/// Returns `(hour, minute, second, offset_seconds)`.
fn parse_time_tz(src: &[u8]) -> Option<(i64, i64, i64, i64)> {
    let (h, n) = take_uint(src);
    if n == 0 {
        return None;
    }
    let mut j = n;
    let mut mi = 0;
    let mut ss = 0;
    if src.get(j) == Some(&b':') {
        let (m, n) = take_uint(&src[j + 1..]);
        if n == 0 {
            return None;
        }
        mi = m;
        j += 1 + n;
        if src.get(j) == Some(&b':') {
            let (s, n) = take_uint(&src[j + 1..]);
            if n == 0 {
                return None;
            }
            ss = s;
            j += 1 + n;
            if src.get(j) == Some(&b'.') {
                let (_, fln) = take_uint(&src[j + 1..]);
                if fln == 0 {
                    return None;
                }
                j += 1 + fln;
            }
        }
    }
    let tail = trim_left(&src[j..]);
    let mut off = 0;
    if !tail.is_empty() {
        let (o, consumed) = parse_tz(tail)?;
        off = o;
        if !trim(&tail[consumed..]).is_empty() {
            return None;
        }
    }
    if h > 24 || mi > 59 || ss > 61 {
        return None;
    }
    Some((h, mi, ss, off))
}

/// Parse a trailing zone: `z` (lowercased Z), `+HH:MM`, `-HHMM`, `+HH`.
fn parse_tz(b: &[u8]) -> Option<(i64, usize)> {
    EXPLICIT_ZONE.with(|e| e.set(true));
    if b[0] == b'z' {
        return Some((0, 1));
    }
    if b[0] != b'+' && b[0] != b'-' {
        return None;
    }
    let sign = if b[0] == b'-' { -1 } else { 1 };
    let (hh, n1) = take_uint(&b[1..]);
    if n1 == 0 {
        return None;
    }
    let mut consumed = 1 + n1;
    if b.get(consumed) == Some(&b':') {
        let (mm, n2) = take_uint(&b[consumed + 1..]);
        if n2 == 0 {
            return None;
        }
        consumed += 1 + n2;
        return Some((sign * ((hh * 60 + mm) * 60), consumed));
    }
    if n1 == 4 {
        let mm = hh % 100;
        return Some((sign * (((hh / 100) * 60 + mm) * 60), consumed));
    }
    Some((sign * (hh * 60 * 60), consumed))
}

// ---------------------------------------------------------------------------
// textual date: "14 November 2023", "November 14, 2023", "14 Nov 23"
// ---------------------------------------------------------------------------

fn parse_textual_date(toks: &[&[u8]], base: i64) -> Option<i64> {
    let _ = base;
    let (midx, mo) = toks
        .iter()
        .enumerate()
        .find_map(|(i, tk)| month_num(tk).map(|m| (i, m)))?;

    // day: numeric token adjacent to the month name (preferring the one before).
    let (d, day_idx) = if midx > 0 && is_digits(toks[midx - 1]) {
        (take_uint(toks[midx - 1]).0, midx - 1)
    } else if midx + 1 < toks.len() && is_digits(toks[midx + 1]) {
        (take_uint(toks[midx + 1]).0, midx + 1)
    } else {
        return None;
    };

    // year: any other pure-digit token.
    let yr = toks
        .iter()
        .enumerate()
        .find(|(i, tk)| *i != day_idx && *i != midx && is_digits(tk))
        .map(|(_, tk)| take_uint(tk).0)?;
    let y = adjust_year(yr);

    // optional time token (one containing a ':').
    let (mut hh, mut mi, mut ss) = (0, 0, 0);
    if let Some(tk) = toks.iter().find(|tk| tk.contains(&b':')) {
        let (h, m, s, _off) = parse_time_tz(tk)?;
        hh = h;
        mi = m;
        ss = s;
    }

    if d < 1 {
        return None;
    }
    Some(civil_ts(y, mo, d, hh * 3600 + mi * 60 + ss))
}

// ---------------------------------------------------------------------------
// time-only: "08:30" / "08:30:15" → that time on the base day
// ---------------------------------------------------------------------------

fn parse_time_only(lower: &[u8], base: i64) -> Option<i64> {
    let t = trim(lower);
    if !t.contains(&b':') {
        return None;
    }
    let (hh, mi, ss, off) = parse_time_tz(t)?;
    Some(day_start(base) + hh * 3600 + mi * 60 + ss - off)
}

// ---------------------------------------------------------------------------
// relative: "+1 day", "1 day ago", "+1 year 2 months 10 days", "+1 fortnight"
// ---------------------------------------------------------------------------

enum Unit {
    Year,
    Month,
    Fortnight,
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
        b"fortnight" => Some(Unit::Fortnight),
        b"week" => Some(Unit::Week),
        b"day" => Some(Unit::Day),
        b"hour" | b"hr" => Some(Unit::Hour),
        b"minute" | b"min" => Some(Unit::Minute),
        b"second" | b"sec" => Some(Unit::Second),
        _ => None,
    }
}

fn parse_relative(toks: &[&[u8]], base: i64) -> Option<i64> {
    let mut dmonths = 0i64;
    let mut dsecs = 0i64;
    let mut matched = false;
    let mut ago = false;
    let mut i = 0;
    while i < toks.len() {
        let tok = toks[i];
        if tok == b"ago" {
            // "ago" must terminate the string.
            if i + 1 != toks.len() {
                return None;
            }
            ago = true;
            break;
        }
        let (sign, digits) = match tok.first() {
            Some(b'+') => (1, &tok[1..]),
            Some(b'-') => (-1, &tok[1..]),
            _ => (1, tok),
        };
        if !is_digits(digits) {
            return None;
        }
        let val = sign * take_uint(digits).0;
        i += 1;
        let unit = toks.get(i).copied()?;
        i += 1;
        match unit_kind(unit)? {
            Unit::Year => dmonths += val * 12,
            Unit::Month => dmonths += val,
            Unit::Fortnight => dsecs += val * 14 * SECS_PER_DAY,
            Unit::Week => dsecs += val * 7 * SECS_PER_DAY,
            Unit::Day => dsecs += val * SECS_PER_DAY,
            Unit::Hour => dsecs += val * 3600,
            Unit::Minute => dsecs += val * 60,
            Unit::Second => dsecs += val,
        }
        matched = true;
    }
    if !matched {
        return None;
    }
    if ago {
        dmonths = -dmonths;
        dsecs = -dsecs;
    }

    let tm = break_down(base);
    let (y, m) = add_months(tm.year, tm.month, dmonths);
    Some(civil_ts(y, m, tm.day, tod(base)) + dsecs)
}

// ---------------------------------------------------------------------------
// builtin entry point
// ---------------------------------------------------------------------------

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

thread_local! {
    /// Set while parsing when the string pins its own zone (`@epoch`, `Z`,
    /// `+HH:MM`): the result is then absolute, not default-zone local time.
    static EXPLICIT_ZONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `strtotime` in the default zone: the grammar runs on local wall-clock
/// seconds and the result is mapped back through the zone's offsets.
fn strtotime_local(s: &[u8], base: i64) -> Option<i64> {
    let t = crate::tzdb::local_type(base);
    EXPLICIT_ZONE.with(|e| e.set(false));
    let r = strtotime(s, base + t.off)?;
    if EXPLICIT_ZONE.with(|e| e.get()) {
        Some(r)
    } else {
        Some(crate::datetime_fns::local_to_ts(r, t.dst))
    }
}

fn fn_strtotime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&deref(&args[0]));
    let base = match args.get(1).map(deref) {
        None | Some(Value::Null) => now_secs(),
        Some(v) => conv::to_int(&v),
    };
    match strtotime_local(&s, base) {
        Some(ts) => Ok(Value::Int(ts)),
        None => Ok(Value::Bool(false)),
    }
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    m.insert(
        "strtotime",
        Builtin {
            name: "strtotime",
            func: fn_strtotime,
            min_args: 1,
            max_args: Some(2),
        },
    );
}

// ---------------------------------------------------------------------------
// tests — expected integers captured from php 8.5.8 (`-d date.timezone=UTC`)
// base = 1700000000 == Tue 2023-11-14 22:13:20 UTC
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: i64 = 1700000000;

    fn st(s: &str) -> Option<i64> {
        strtotime(s.as_bytes(), BASE)
    }

    /// Assert strtotime(s) == expected php integer.
    fn eq(s: &str, want: i64) {
        assert_eq!(st(s), Some(want), "input {s:?}");
    }

    #[test]
    fn absolute_numeric() {
        eq("2023-11-14", 1699920000);
        eq("2023-11-14 08:30:00", 1699950600);
        eq("2023/11/14", 1699920000);
        eq("11/14/2023", 1699920000);
        eq("14-11-2023", 1699920000);
        eq("2023-01-05 23:59:59", 1672963199);
        // day overflow rolls forward (php behavior)
        eq("31-11-2023", 1701388800);
        // leap day across all three numeric orderings
        eq("2024-02-29", 1709164800);
        eq("29-02-2024", 1709164800);
        eq("02/29/2024", 1709164800);
    }

    #[test]
    fn iso8601() {
        eq("2023-11-14T08:30:00+02:00", 1699943400);
        eq("2023-11-14T08:30:00Z", 1699950600);
        eq("2023-11-14T08:30:00", 1699950600);
        eq("2023-11-14t08:30:00z", 1699950600);
    }

    #[test]
    fn time_only() {
        eq("08:30", 1699950600);
        eq("08:30:15", 1699950615);
        eq("10:00:00", 1699956000);
    }

    #[test]
    fn textual() {
        eq("14 November 2023", 1699920000);
        eq("November 14, 2023", 1699920000);
        eq("14 Nov 23", 1699920000);
        eq("1 January 2000", 946684800);
        eq("Jan 1 2000", 946684800);
        eq("14 Nov 2023", 1699920000);
        eq("Nov 14 2023", 1699920000);
        eq("Nov 14, 2023", 1699920000);
        eq("December 25, 2023", 1703462400);
        eq("25 Dec 2023", 1703462400);
        eq("25 December 2023 14:30:00", 1703514600);
    }

    #[test]
    fn epoch() {
        eq("@1700000000", 1700000000);
        eq("@1700000000.75", 1700000000);
        eq("@-100", -100);
        eq("@0", 0);
        eq("@-100.9", -101); // php floors toward -inf
    }

    #[test]
    fn keywords() {
        eq("now", 1700000000);
        eq("today", 1699920000);
        eq("midnight", 1699920000);
        eq("noon", 1699963200);
        eq("tomorrow", 1700006400);
        eq("yesterday", 1699833600);
    }

    #[test]
    fn relative() {
        eq("+1 day", 1700086400);
        eq("-2 weeks", 1698790400);
        eq("+3 months", 1707948800);
        eq("+1 year 2 months 10 days", 1737756800);
        eq("+1 hour", 1700003600);
        eq("-30 minutes", 1699998200);
        eq("+90 seconds", 1700000090);
        eq("1 day ago", 1699913600);
        eq("2 days ago", 1699827200);
        eq("+1 fortnight", 1701209600);
        eq("+2 fortnights", 1702419200);
        eq("+1 week", 1700604800);
        eq("1 day", 1700086400);
        eq("3 weeks", 1701814400);
        eq("+0 day", 1700000000);
        eq("+2 years", 1763158400);
        eq("-1 year", 1668464000);
    }

    #[test]
    fn relative_special() {
        eq("next week", 1700518400);
        eq("last week", 1699308800);
        eq("next month", 1702592000);
        eq("last month", 1697321600);
        eq("next year", 1731622400);
        eq("last year", 1668464000);
    }

    #[test]
    fn weekdays() {
        eq("next monday", 1700438400);
        eq("last friday", 1699574400);
        eq("monday", 1700438400);
        eq("tuesday", 1699920000);
        eq("this tuesday", 1699920000);
        eq("this wednesday", 1700006400);
        eq("next tuesday", 1700524800);
        eq("last tuesday", 1699315200);
        eq("sunday", 1700352000);
        eq("wednesday", 1700006400);
        eq("friday", 1700179200);
        eq("saturday", 1700265600);
        eq("this monday", 1700438400);
        eq("this sunday", 1700352000);
    }

    #[test]
    fn compound() {
        eq("first day of next month", 1701468800);
        eq("last day of this month", 1701382400);
        eq("first day of January 2024", 1704067200);
        eq("last day of January 2024", 1706659200);
        eq("first day of this month", 1698876800);
        eq("last day of next month", 1704060800);
        eq("first day of previous month", 1696198400);
        eq("first day of March 2024", 1709251200);
        eq("last day of February 2024", 1709164800);
    }

    #[test]
    fn case_and_whitespace() {
        eq("  +1 DAY  ", 1700086400);
        eq("NEXT MONDAY", 1700438400);
        eq("Tomorrow", 1700006400);
    }

    #[test]
    fn failures() {
        assert_eq!(st("not a date"), None);
        assert_eq!(st(""), None);
        assert_eq!(st("gibberish xyz"), None);
        assert_eq!(st("2023.11.14"), None); // dot separator unsupported
        assert_eq!(st("13/14/2023"), None); // month > 12
    }

    #[test]
    fn explicit_base_is_deterministic() {
        // Same input + base must always give the same answer.
        assert_eq!(st("+1 day"), st("+1 day"));
        assert_eq!(strtotime(b"today", 0), Some(0));
        assert_eq!(strtotime(b"tomorrow", 0), Some(86400));
    }
}
