//! php's `calendar` extension (php-src ext/calendar): calendar.c, easter.c
//! and cal_unix.c ported; the date conversions themselves are php-src's C
//! (sdncal.h), compiled by rhp-csrc (`rhp_csrc::calendar`).

use rhp_csrc::calendar::{self as sdn, Cal, Names};
use rhp_values::{ArrayKey, PhpArray, Value};

use crate::zend::zpp::{bool_arg_or, int_arg, int_arg_or, opt_int_arg, value_error};
use crate::{Host, Thrown};

const CAL_NUM_CALS: i64 = 4;
const CAL_JEWISH_ADD_ALAFIM_GERESH: i64 = 0x2;
const CAL_JEWISH_ADD_ALAFIM: i64 = 0x4;
const CAL_JEWISH_ADD_GERESHAYIM: i64 = 0x8;

/// php's `cal_conversion_table` row.
struct Entry {
    name: &'static str,
    symbol: &'static str,
    cal: Cal,
    num_months: usize,
    max_days_in_month: i64,
    short: Names,
    long: Names,
}

const TABLE: [Entry; 4] = [
    Entry {
        name: "Gregorian",
        symbol: "CAL_GREGORIAN",
        cal: Cal::Gregorian,
        num_months: 12,
        max_days_in_month: 31,
        short: Names::MonthShort,
        long: Names::MonthLong,
    },
    Entry {
        name: "Julian",
        symbol: "CAL_JULIAN",
        cal: Cal::Julian,
        num_months: 12,
        max_days_in_month: 31,
        short: Names::MonthShort,
        long: Names::MonthLong,
    },
    Entry {
        name: "Jewish",
        symbol: "CAL_JEWISH",
        cal: Cal::Jewish,
        num_months: 13,
        max_days_in_month: 30,
        short: Names::JewishLeap,
        long: Names::JewishLeap,
    },
    Entry {
        name: "French",
        symbol: "CAL_FRENCH",
        cal: Cal::French,
        num_months: 13,
        max_days_in_month: 30,
        short: Names::French,
        long: Names::French,
    },
];

pub fn constants() -> Vec<(&'static str, Value)> {
    [
        ("CAL_GREGORIAN", 0),
        ("CAL_JULIAN", 1),
        ("CAL_JEWISH", 2),
        ("CAL_FRENCH", 3),
        ("CAL_NUM_CALS", CAL_NUM_CALS),
        ("CAL_DOW_DAYNO", 0),
        ("CAL_DOW_SHORT", 2),
        ("CAL_DOW_LONG", 1),
        ("CAL_MONTH_GREGORIAN_SHORT", 0),
        ("CAL_MONTH_GREGORIAN_LONG", 1),
        ("CAL_MONTH_JULIAN_SHORT", 2),
        ("CAL_MONTH_JULIAN_LONG", 3),
        ("CAL_MONTH_JEWISH", 4),
        ("CAL_MONTH_FRENCH", 5),
        ("CAL_EASTER_DEFAULT", 0),
        ("CAL_EASTER_ROMAN", 1),
        ("CAL_EASTER_ALWAYS_GREGORIAN", 2),
        ("CAL_EASTER_ALWAYS_JULIAN", 3),
        ("CAL_JEWISH_ADD_ALAFIM_GERESH", CAL_JEWISH_ADD_ALAFIM_GERESH),
        ("CAL_JEWISH_ADD_ALAFIM", CAL_JEWISH_ADD_ALAFIM),
        ("CAL_JEWISH_ADD_GERESHAYIM", CAL_JEWISH_ADD_GERESHAYIM),
    ]
    .into_iter()
    .map(|(n, v)| (n, Value::Int(v)))
    .collect()
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("cal_days_in_month", fn_cal_days_in_month),
            ("cal_from_jd", fn_cal_from_jd),
            ("cal_info", fn_cal_info),
            ("cal_to_jd", fn_cal_to_jd),
            ("easter_date", fn_easter_date),
            ("easter_days", fn_easter_days),
            ("frenchtojd", fn_frenchtojd),
            ("gregoriantojd", fn_gregoriantojd),
            ("jddayofweek", fn_jddayofweek),
            ("jdmonthname", fn_jdmonthname),
            ("jdtofrench", fn_jdtofrench),
            ("jdtogregorian", fn_jdtogregorian),
            ("jdtojewish", fn_jdtojewish),
            ("jdtojulian", fn_jdtojulian),
            ("jdtounix", fn_jdtounix),
            ("jewishtojd", fn_jewishtojd),
            ("juliantojd", fn_juliantojd),
            ("unixtojd", fn_unixtojd),
        ],
    );
}

fn name(t: Names, i: i32) -> &'static [u8] {
    usize::try_from(i)
        .ok()
        .and_then(|i| sdn::name(t, i))
        .unwrap_or(b"")
}

/// `JEWISH_MONTH_NAME(year)` / `JEWISH_HEB_MONTH_NAME(year)` (year > 0).
fn jewish_names(year: i32, heb: bool) -> Names {
    let leap = sdn::months_per_year(((year - 1) % 19) as usize) == 13;
    match (heb, leap) {
        (false, true) => Names::JewishLeap,
        (false, false) => Names::Jewish,
        (true, true) => Names::JewishHebLeap,
        (true, false) => Names::JewishHeb,
    }
}

fn key(k: &str) -> ArrayKey {
    ArrayKey::from_bytes(k.as_bytes())
}

/// The ValueError for a bad calendar ID.
fn cal_arg(fname: &str, cal: i64, n: usize, pname: &str) -> Result<&'static Entry, Thrown> {
    if !(0..CAL_NUM_CALS).contains(&cal) {
        return Err(value_error(fname, n, pname, "must be a valid calendar ID"));
    }
    Ok(&TABLE[cal as usize])
}

fn cal_info_of(e: &Entry) -> Value {
    let mut months = PhpArray::with_capacity(e.num_months);
    let mut smonths = PhpArray::with_capacity(e.num_months);
    for i in 1..=e.num_months {
        months.insert(ArrayKey::Int(i as i64), Value::str(name(e.long, i as i32)));
        smonths.insert(ArrayKey::Int(i as i64), Value::str(name(e.short, i as i32)));
    }
    let mut ret = PhpArray::new();
    ret.insert(key("months"), Value::Array(months));
    ret.insert(key("abbrevmonths"), Value::Array(smonths));
    ret.insert(key("maxdaysinmonth"), Value::Int(e.max_days_in_month));
    ret.insert(key("calname"), Value::str(e.name));
    ret.insert(key("calsymbol"), Value::str(e.symbol));
    Value::Array(ret)
}

fn fn_cal_info(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cal = int_arg_or(host, args, 0, "cal_info", "calendar", -1)?;
    if cal == -1 {
        let mut a = PhpArray::with_capacity(TABLE.len());
        for e in &TABLE {
            a.push(cal_info_of(e));
        }
        return Ok(Value::Array(a));
    }
    Ok(cal_info_of(cal_arg("cal_info", cal, 1, "calendar")?))
}

/// The month and year checks cal_days_in_month and cal_to_jd share.
fn month_year_check(fname: &str, month: i64, year: i64, year_arg: usize) -> Result<(), Thrown> {
    if month <= 0 || month > i32::MAX as i64 - 1 {
        return Err(value_error(
            fname,
            2,
            "month",
            &format!("must be between 1 and {}", i32::MAX - 1),
        ));
    }
    if year > i32::MAX as i64 - 1 {
        return Err(value_error(
            fname,
            year_arg,
            "year",
            &format!("must be less than {}", i32::MAX - 1),
        ));
    }
    Ok(())
}

fn fn_cal_days_in_month(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "cal_days_in_month";
    let cal = int_arg(host, args, 0, f, "calendar")?;
    let month = int_arg(host, args, 1, f, "month")?;
    let year = int_arg(host, args, 2, f, "year")?;
    let e = cal_arg(f, cal, 1, "calendar")?;
    month_year_check(f, month, year, 3)?;
    let to_jd = |y: i64, m: i64, d: i32| e.cal.to_sdn(y as i32, m as i32, d);
    let start = to_jd(year, month, 1);
    if start == 0 {
        return Err(Thrown::error("ValueError", "Invalid date"));
    }
    let mut next = to_jd(year, 1 + month, 1);
    if next == 0 {
        // The next year after 1 BCE is 1 AD.
        if year == -1 {
            next = to_jd(1, 1, 1);
        } else {
            next = to_jd(year + 1, 1, 1);
            if e.cal == Cal::French && next == 0 {
                // The French calendar ends on 0014-13-05.
                next = 2380953;
            }
        }
    }
    Ok(Value::Int(next - start))
}

fn fn_cal_to_jd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "cal_to_jd";
    let cal = int_arg(host, args, 0, f, "calendar")?;
    let month = int_arg(host, args, 1, f, "month")?;
    let day = int_arg(host, args, 2, f, "day")?;
    let year = int_arg(host, args, 3, f, "year")?;
    let e = cal_arg(f, cal, 1, "calendar")?;
    if month <= 0 || month > i32::MAX as i64 - 1 {
        return Err(value_error(
            f,
            2,
            "month",
            &format!("must be between 1 and {}", i32::MAX - 1),
        ));
    }
    if i32::try_from(day).is_err() {
        return Err(value_error(
            f,
            3,
            "day",
            &format!("must be between {} and {}", i32::MIN, i32::MAX),
        ));
    }
    if year > i32::MAX as i64 - 1 {
        return Err(value_error(
            f,
            4,
            "year",
            &format!("must be less than {}", i32::MAX - 1),
        ));
    }
    Ok(Value::Int(e.cal.to_sdn(
        year as i32,
        month as i32,
        day as i32,
    )))
}

fn mdy(m: i32, d: i32, y: i32) -> Value {
    Value::str(format!("{m}/{d}/{y}"))
}

fn fn_cal_from_jd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "cal_from_jd";
    let jd = int_arg(host, args, 0, f, "julian_day")?;
    let cal = int_arg(host, args, 1, f, "calendar")?;
    let e = cal_arg(f, cal, 2, "calendar")?;
    let (year, month, day) = e.cal.from_sdn(jd);
    let mut r = PhpArray::new();
    r.insert(key("date"), mdy(month, day, year));
    r.insert(key("month"), Value::Int(month as i64));
    r.insert(key("day"), Value::Int(day as i64));
    r.insert(key("year"), Value::Int(year as i64));
    if e.cal != Cal::Jewish || year > 0 {
        let dow = sdn::day_of_week(jd);
        r.insert(key("dow"), Value::Int(dow as i64));
        r.insert(key("abbrevdayname"), Value::str(name(Names::DayShort, dow)));
        r.insert(key("dayname"), Value::str(name(Names::DayLong, dow)));
    } else {
        r.insert(key("dow"), Value::Null);
        r.insert(key("abbrevdayname"), Value::str(""));
        r.insert(key("dayname"), Value::str(""));
    }
    if e.cal == Cal::Jewish {
        let n = if year > 0 {
            name(jewish_names(year, false), month)
        } else {
            b""
        };
        r.insert(key("abbrevmonth"), Value::str(n));
        r.insert(key("monthname"), Value::str(n));
    } else {
        r.insert(key("abbrevmonth"), Value::str(name(e.short, month)));
        r.insert(key("monthname"), Value::str(name(e.long, month)));
    }
    Ok(Value::Array(r))
}

/// `jdto<cal>(julian_day)`: "m/d/y".
fn jd_to(host: &mut dyn Host, args: &[Value], f: &str, cal: Cal) -> Result<Value, Thrown> {
    let jd = int_arg(host, args, 0, f, "julian_day")?;
    let (y, m, d) = cal.from_sdn(jd);
    Ok(mdy(m, d, y))
}

/// `<cal>tojd(month, day, year)`; the zend_longs are truncated to int as
/// C's implicit conversion does.
fn to_jd(host: &mut dyn Host, args: &[Value], f: &str, cal: Cal) -> Result<Value, Thrown> {
    let month = int_arg(host, args, 0, f, "month")?;
    let day = int_arg(host, args, 1, f, "day")?;
    let year = int_arg(host, args, 2, f, "year")?;
    Ok(Value::Int(cal.to_sdn(
        year as i32,
        month as i32,
        day as i32,
    )))
}

fn fn_jdtogregorian(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    jd_to(host, args, "jdtogregorian", Cal::Gregorian)
}

fn fn_gregoriantojd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    to_jd(host, args, "gregoriantojd", Cal::Gregorian)
}

fn fn_jdtojulian(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    jd_to(host, args, "jdtojulian", Cal::Julian)
}

fn fn_juliantojd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    to_jd(host, args, "juliantojd", Cal::Julian)
}

fn fn_jdtofrench(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    jd_to(host, args, "jdtofrench", Cal::French)
}

fn fn_frenchtojd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    to_jd(host, args, "frenchtojd", Cal::French)
}

fn fn_jewishtojd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "jewishtojd";
    let month = int_arg(host, args, 0, f, "month")?;
    let day = int_arg(host, args, 1, f, "day")?;
    let year = int_arg(host, args, 2, f, "year")?;
    if i32::try_from(year).is_err() {
        return Err(value_error(
            f,
            3,
            "year",
            &format!("must be between {} and {}", i32::MIN, i32::MAX),
        ));
    }
    Ok(Value::Int(Cal::Jewish.to_sdn(
        year as i32,
        month as i32,
        day as i32,
    )))
}

/// php's `heb_number_to_chars`: `n` in Hebrew letters (ISO-8859-8), or
/// `None` outside 1..=9999.
fn heb_number_to_chars(mut n: i32, fl: i64) -> Option<Vec<u8>> {
    const ALEF_BET: &[u8; 24] =
        b"0\xE0\xE1\xE2\xE3\xE4\xE5\xE6\xE7\xE8\xE9\xEB\xEC\xEE\xF0\xF1\xF2\xF4\xF6\xF7\xF8\xF9\xFA\0";
    if !(1..=9999).contains(&n) {
        return None;
    }
    let mut p = Vec::with_capacity(18);
    let mut end_of_alafim = 0;
    if n / 1000 != 0 {
        p.push(ALEF_BET[(n / 1000) as usize]);
        if fl & CAL_JEWISH_ADD_ALAFIM_GERESH != 0 {
            p.push(b'\'');
        }
        if fl & CAL_JEWISH_ADD_ALAFIM != 0 {
            p.extend_from_slice(b" \xE0\xEC\xF4\xE9\xED ");
        }
        end_of_alafim = p.len();
        n %= 1000;
    }
    while n >= 400 {
        p.push(ALEF_BET[22]);
        n -= 400;
    }
    if n >= 100 {
        p.push(ALEF_BET[(18 + n / 100) as usize]);
        n %= 100;
    }
    if n == 15 || n == 16 {
        p.push(ALEF_BET[9]);
        p.push(ALEF_BET[(n - 9) as usize]);
    } else {
        if n >= 10 {
            p.push(ALEF_BET[(9 + n / 10) as usize]);
            n %= 10;
        }
        if n > 0 {
            p.push(ALEF_BET[n as usize]);
        }
    }
    if fl & CAL_JEWISH_ADD_GERESHAYIM != 0 {
        match p.len() - end_of_alafim {
            0 => {}
            1 => p.push(b'\''),
            _ => {
                let last = p[p.len() - 1];
                let at = p.len() - 1;
                p[at] = b'"';
                p.push(last);
            }
        }
    }
    Some(p)
}

fn fn_jdtojewish(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "jdtojewish";
    let jd = int_arg(host, args, 0, f, "julian_day")?;
    let heb = bool_arg_or(host, args, 1, f, "hebrew", false)?;
    let fl = int_arg_or(host, args, 2, f, "flags", 0)?;
    let (year, month, day) = Cal::Jewish.from_sdn(jd);
    if !heb {
        return Ok(mdy(month, day, year));
    }
    if year <= 0 || year > 9999 {
        return Err(Thrown::error("ValueError", "Year out of range (0-9999)"));
    }
    // "%s %s %s"; a NULL from heb_number_to_chars prints as "(null)".
    let null = || b"(null)".to_vec();
    let mut s = heb_number_to_chars(day, fl).unwrap_or_else(null);
    s.push(b' ');
    s.extend_from_slice(name(jewish_names(year, true), month));
    s.push(b' ');
    s.extend(heb_number_to_chars(year, fl).unwrap_or_else(null));
    Ok(Value::str(s))
}

fn fn_jddayofweek(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "jddayofweek";
    let jd = int_arg(host, args, 0, f, "julian_day")?;
    let mode = int_arg_or(host, args, 1, f, "mode", 0)?;
    let day = sdn::day_of_week(jd);
    Ok(match mode {
        1 => Value::str(name(Names::DayLong, day)),
        2 => Value::str(name(Names::DayShort, day)),
        _ => Value::Int(day as i64),
    })
}

fn fn_jdmonthname(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "jdmonthname";
    let jd = int_arg(host, args, 0, f, "julian_day")?;
    let mode = int_arg(host, args, 1, f, "mode")?;
    let n = match mode {
        1 => name(Names::MonthLong, Cal::Gregorian.from_sdn(jd).1),
        2 => name(Names::MonthShort, Cal::Julian.from_sdn(jd).1),
        3 => name(Names::MonthLong, Cal::Julian.from_sdn(jd).1),
        4 => {
            let (y, m, _) = Cal::Jewish.from_sdn(jd);
            if y > 0 {
                name(jewish_names(y, false), m)
            } else {
                b""
            }
        }
        5 => name(Names::French, Cal::French.from_sdn(jd).1),
        _ => name(Names::MonthShort, Cal::Gregorian.from_sdn(jd).1),
    };
    Ok(Value::str(n))
}

/// `localtime_r` of `t` (php_localtime_r), the process's TZ as php uses.
fn localtime(t: libc::time_t) -> Option<libc::tm> {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::localtime_r(&t, &mut tm) };
    (!r.is_null()).then_some(tm)
}

fn now() -> libc::time_t {
    unsafe { libc::time(std::ptr::null_mut()) }
}

fn fn_unixtojd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "unixtojd";
    let ts = match opt_int_arg(host, args, 0, f, "timestamp")? {
        None => now(),
        Some(t) if t >= 0 => t as libc::time_t,
        Some(_) => {
            return Err(value_error(
                f,
                1,
                "timestamp",
                "must be greater than or equal to 0",
            ))
        }
    };
    let Some(tm) = localtime(ts) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Int(Cal::Gregorian.to_sdn(
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
    )))
}

fn fn_jdtounix(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const SECS_PER_DAY: i64 = 24 * 3600;
    let uday = int_arg(host, args, 0, "jdtounix", "julian_day")?;
    if uday < 2440588 || (uday - 2440588) > i64::MAX / SECS_PER_DAY {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "jday must be between 2440588 and {}",
                i64::MAX / SECS_PER_DAY + 2440588
            ),
        ));
    }
    Ok(Value::Int((uday - 2440588) * SECS_PER_DAY))
}

fn fn_easter_date(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    cal_easter(host, args, "easter_date", true)
}

fn fn_easter_days(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    cal_easter(host, args, "easter_days", false)
}

/// php's `_cal_easter`: midnight on Easter (`gm`), or the days after
/// March 21st.
fn cal_easter(host: &mut dyn Host, args: &[Value], f: &str, gm: bool) -> Result<Value, Thrown> {
    let year = opt_int_arg(host, args, 0, f, "year")?;
    let method = int_arg_or(host, args, 1, f, "mode", 0)?;
    let max_year = (i64::MAX / 5) * 4;
    let year = match year {
        Some(y) => y,
        None => localtime(now()).map_or(1900, |tm| 1900 + tm.tm_year as i64),
    };
    if year <= 0 || year > max_year {
        return Err(value_error(
            f,
            1,
            "year",
            &format!("must be between 1 and {max_year}"),
        ));
    }
    if gm && year < 1970 {
        return Err(value_error(
            f,
            1,
            "year",
            "must be a year after 1970 (inclusive)",
        ));
    }
    if gm && year > 2_000_000_000 {
        return Err(value_error(
            f,
            1,
            "year",
            "must be a year before 2.000.000.000 (inclusive)",
        ));
    }
    let golden = (year % 19) + 1;
    let (mut pfm, dom);
    if (year <= 1582 && method != 2)
        || ((1583..=1752).contains(&year) && method != 1 && method != 2)
        || method == 3
    {
        let mut d = (year + (year / 4) + 5) % 7;
        if d < 0 {
            d += 7;
        }
        dom = d;
        pfm = (3 - (11 * golden) - 7) % 30;
        if pfm < 0 {
            pfm += 30;
        }
    } else {
        let mut d = (year + (year / 4) - (year / 100) + (year / 400)) % 7;
        if d < 0 {
            d += 7;
        }
        dom = d;
        let solar = (year - 1600) / 100 - (year - 1600) / 400;
        let lunar = (((year - 1400) / 100) * 8) / 25;
        pfm = (3 - (11 * golden) + solar - lunar) % 30;
        if pfm < 0 {
            pfm += 30;
        }
    }
    if pfm == 29 || (pfm == 28 && golden > 11) {
        pfm -= 1;
    }
    let mut tmp = (4 - pfm - dom) % 7;
    if tmp < 0 {
        tmp += 7;
    }
    let easter = pfm + tmp + 1;
    if !gm {
        return Ok(Value::Int(easter));
    }
    let mut te: libc::tm = unsafe { std::mem::zeroed() };
    te.tm_isdst = -1;
    te.tm_year = (year - 1900) as i32;
    if easter < 11 {
        te.tm_mon = 2;
        te.tm_mday = (easter + 21) as i32;
    } else {
        te.tm_mon = 3;
        te.tm_mday = (easter - 10) as i32;
    }
    Ok(Value::Int(unsafe { libc::mktime(&mut te) } as i64))
}
