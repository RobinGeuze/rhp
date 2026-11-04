//! The `date` extension (php-src ext/date): a port of php_date.c over
//! timelib (`rhp_csrc::timelib`, php-src's ext/date/lib with its embedded
//! tz database).
//!
//! Here: the procedural functions that need no date object, the default
//! timezone, and php's per-request tz cache. The DateTime classes are still
//! the prelude's (`dt_oop_fns.rs`).

mod format;

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, CStr};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::rc::Rc;

use rhp_csrc::timelib::{self, sys, Errors, Time, Tz, Tzdb, TIMELIB_NO_CLONE, TIMELIB_UNSET};
use rhp_values::{ArrayKey, PhpArray, Value};

use crate::argparse::str_param;
use crate::ext::zpp;
use crate::{Builtin, Host, Thrown};

pub use format::php_format_date;

const PHP_DATE_TIMEZONE_GROUP_AFRICA: i64 = 0x0001;
const PHP_DATE_TIMEZONE_GROUP_AMERICA: i64 = 0x0002;
const PHP_DATE_TIMEZONE_GROUP_ANTARCTICA: i64 = 0x0004;
const PHP_DATE_TIMEZONE_GROUP_ARCTIC: i64 = 0x0008;
const PHP_DATE_TIMEZONE_GROUP_ASIA: i64 = 0x0010;
const PHP_DATE_TIMEZONE_GROUP_ATLANTIC: i64 = 0x0020;
const PHP_DATE_TIMEZONE_GROUP_AUSTRALIA: i64 = 0x0040;
const PHP_DATE_TIMEZONE_GROUP_EUROPE: i64 = 0x0080;
const PHP_DATE_TIMEZONE_GROUP_INDIAN: i64 = 0x0100;
const PHP_DATE_TIMEZONE_GROUP_PACIFIC: i64 = 0x0200;
const PHP_DATE_TIMEZONE_GROUP_UTC: i64 = 0x0400;
const PHP_DATE_TIMEZONE_GROUP_ALL: i64 = 0x07FF;
const PHP_DATE_TIMEZONE_GROUP_ALL_W_BC: i64 = 0x0FFF;
const PHP_DATE_TIMEZONE_PER_COUNTRY: i64 = 0x1000;

thread_local! {
    /// `DATEG(timezone)`: set by date_default_timezone_set().
    static TIMEZONE: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    /// `DATEG(tzcache)`: parsed zones by the name they were asked for.
    static TZCACHE: RefCell<HashMap<Vec<u8>, Rc<Tz>>> = RefCell::new(HashMap::new());
    /// A panic caught in a timelib callback, resumed once timelib returns.
    static CALLBACK_PANIC: RefCell<Option<Box<dyn Any + Send>>> = const { RefCell::new(None) };
}

/// php's RINIT/post-deactivate: forget the default zone and the tz cache.
/// A zone a date object still uses stays alive through its `Rc`.
pub(crate) fn reset_state() {
    TIMEZONE.with(|t| *t.borrow_mut() = None);
    TZCACHE.with(|c| c.borrow_mut().clear());
}

/// `DATE_TIMEZONEDB`: php's embedded database (rhp has no external one).
pub(crate) fn tzdb() -> Tzdb {
    Tzdb::builtin()
}

/// `timelib_timezone_id_is_valid(id, DATE_TIMEZONEDB)`: date.timezone's
/// check.
pub(crate) fn timezone_id_is_valid(id: &[u8]) -> bool {
    tzdb().is_valid_id(id)
}

/// The bytes C sees of a php string passed as `char *`.
fn c_str(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

/// `php_date_parse_tzfile`: the zone named `name`, parsed once per request.
pub(crate) fn parse_tzfile(name: &[u8]) -> Option<Rc<Tz>> {
    let name = c_str(name);
    if let Some(tz) = TZCACHE.with(|c| c.borrow().get(name).cloned()) {
        return Some(tz);
    }
    let tz = Rc::new(Tz::parse(name, tzdb()).ok()?);
    TZCACHE.with(|c| c.borrow_mut().insert(name.to_vec(), tz.clone()));
    Some(tz)
}

/// `php_date_parse_tzfile_wrapper`, timelib's `tz_get_wrapper`.
unsafe extern "C" fn parse_tzfile_wrapper(
    name: *const c_char,
    _db: *const sys::timelib_tzdb,
    _error_code: *mut c_int,
) -> *mut sys::timelib_tzinfo {
    guard(std::ptr::null_mut(), || {
        // SAFETY: timelib passes a NUL-terminated name.
        let name = unsafe { CStr::from_ptr(name) }.to_bytes();
        parse_tzfile(name).map_or(std::ptr::null_mut(), |tz| tz.as_ptr())
    })
}

/// Runs a callback's body; a panic must not unwind into C. It is kept and
/// resumed by [`resume_callback_panic`] once the C call has returned.
fn guard<R>(default: R, f: impl FnOnce() -> R) -> R {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => {
            CALLBACK_PANIC.with(|c| {
                c.borrow_mut().get_or_insert(p);
            });
            default
        }
    }
}

fn resume_callback_panic() {
    if let Some(p) = CALLBACK_PANIC.with(|c| c.borrow_mut().take()) {
        resume_unwind(p);
    }
}

/// `timelib_strtotime(s, len, &error, DATE_TIMEZONEDB,
/// php_date_parse_tzfile_wrapper)`.
pub(crate) fn strtotime(s: &[u8]) -> (Time, Errors) {
    let r = timelib::strtotime(s, tzdb(), Some(parse_tzfile_wrapper));
    resume_callback_panic();
    r
}

/// `timelib_parse_from_format(format, s, len, &error, DATE_TIMEZONEDB,
/// php_date_parse_tzfile_wrapper)`.
pub(crate) fn parse_from_format(format: &[u8], s: &[u8]) -> (Time, Errors) {
    let r = timelib::parse_from_format(format, s, tzdb(), Some(parse_tzfile_wrapper));
    resume_callback_panic();
    r
}

/// `guess_timezone`: date_default_timezone_set()'s zone, else date.timezone
/// (which only ever holds a valid id), else UTC.
pub(crate) fn guess_timezone() -> Vec<u8> {
    if let Some(tz) = TIMEZONE
        .with(|t| t.borrow().clone())
        .filter(|t| !t.is_empty())
    {
        return tz;
    }
    crate::runtime_fns::ini_value(b"date.timezone")
        .flatten()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| b"UTC".to_vec())
}

/// `get_timezone_info`: the default zone; `None` only if php's database
/// were corrupt.
pub(crate) fn get_timezone_info() -> Option<Rc<Tz>> {
    parse_tzfile(&guess_timezone())
}

/// [`get_timezone_info`], or php's DateError.
fn timezone_info() -> Result<Rc<Tz>, Thrown> {
    get_timezone_info().ok_or_else(|| {
        Thrown::error(
            "DateError",
            "Timezone database is corrupt. Please file a bug report as this should never happen",
        )
    })
}

/// The date constants rhp's core list (lib.rs) doesn't define yet.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("DATE_ISO8601_EXPANDED", Value::str("X-m-d\\TH:i:sP")),
        ("DATE_RFC7231", Value::str("D, d M Y H:i:s \\G\\M\\T")),
        ("DATE_RFC3339_EXTENDED", Value::str("Y-m-d\\TH:i:s.vP")),
        ("SUNFUNCS_RET_TIMESTAMP", Value::Int(0)),
        ("SUNFUNCS_RET_STRING", Value::Int(1)),
        ("SUNFUNCS_RET_DOUBLE", Value::Int(2)),
    ]
}

/// The stub's `#[\Deprecated]` constants: the E_DEPRECATED a fetch raises.
pub(crate) fn constant_deprecation(name: &[u8]) -> Option<String> {
    let (since, msg) = match name {
        b"DATE_RFC7231" => (
            "8.5",
            "as this format ignores the associated timezone and always uses GMT",
        ),
        b"SUNFUNCS_RET_TIMESTAMP" | b"SUNFUNCS_RET_STRING" | b"SUNFUNCS_RET_DOUBLE" => (
            "8.4",
            "as date_sunrise() and date_sunset() were deprecated in 8.1",
        ),
        _ => return None,
    };
    Some(format!(
        "Constant {} is deprecated since {since}, {msg}",
        String::from_utf8_lossy(name)
    ))
}

/// PHP_MINFO_FUNCTION(date)'s rows.
pub(crate) fn info_rows() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("date/time support", b"enabled".to_vec()),
        ("timelib version", timelib::version().as_bytes().to_vec()),
        (
            "\"Olson\" Timezone Database Version",
            tzdb().version().to_vec(),
        ),
        ("Timezone Database", b"internal".to_vec()),
        ("Default timezone", guess_timezone()),
    ]
}

/// `php_time()`.
fn php_time() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs_f64().ceil() as i64),
    }
}

fn long_or(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
    default: i64,
) -> Result<i64, Thrown> {
    if args.len() <= i {
        return Ok(default);
    }
    zpp::long_arg(host, args, i, fname, pname)
}

fn assoc(a: &mut PhpArray, k: &str, v: Value) {
    a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
}

// --- date() / gmdate() / idate() ---------------------------------------------

fn php_date(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    localtime: bool,
) -> Result<Value, Thrown> {
    let format = str_param(host, args, 0, fname, "format")?;
    let ts = zpp::opt_long_arg(host, args, 1, fname, "timestamp")?.unwrap_or_else(php_time);
    if localtime {
        timezone_info()?;
    }
    Ok(Value::str(php_format_date(&format, ts, localtime)))
}

fn fn_date(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_date(host, args, "date", true)
}

fn fn_gmdate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_date(host, args, "gmdate", false)
}

fn fn_idate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let format = str_param(host, args, 0, "idate", "format")?;
    let ts = zpp::opt_long_arg(host, args, 1, "idate", "timestamp")?;
    if format.len() != 1 {
        crate::warn(host, "idate", "idate format is one char")?;
        return Ok(Value::Bool(false));
    }
    let ts = ts.unwrap_or_else(php_time);
    timezone_info()?;
    match format::php_idate(format[0], ts, false) {
        Some(v) => Ok(Value::Int(v as i64)),
        None => {
            crate::warn(host, "idate", "Unrecognized date format token")?;
            Ok(Value::Bool(false))
        }
    }
}

// --- strtotime() ---------------------------------------------------------------

fn fn_strtotime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let times = str_param(host, args, 0, "strtotime", "datetime")?;
    let preset = zpp::opt_long_arg(host, args, 1, "strtotime", "baseTimestamp")?;
    // timelib_strtotime() expects the string to not be empty.
    if times.is_empty() {
        return Ok(Value::Bool(false));
    }
    let tzi = timezone_info()?;
    let mut now = Time::new();
    now.set_zone_id(&tzi);
    now.unixtime2local(preset.unwrap_or_else(php_time));
    let (mut t, error) = strtotime(&times);
    if error.error_count() > 0 {
        return Ok(Value::Bool(false));
    }
    t.fill_holes(&now, TIMELIB_NO_CLONE);
    t.update_ts(Some(&tzi));
    match t.date_to_int() {
        Some(ts) => Ok(Value::Int(ts)),
        None => {
            crate::warn(host, "strtotime", "Epoch doesn't fit in a PHP integer")?;
            Ok(Value::Bool(false))
        }
    }
}

// --- mktime() / gmmktime() / checkdate() ----------------------------------------

fn php_mktime(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    gmt: bool,
) -> Result<Value, Thrown> {
    let hou = zpp::long_arg(host, args, 0, fname, "hour")?;
    let min = zpp::opt_long_arg(host, args, 1, fname, "minute")?;
    let sec = zpp::opt_long_arg(host, args, 2, fname, "second")?;
    let mon = zpp::opt_long_arg(host, args, 3, fname, "month")?;
    let day = zpp::opt_long_arg(host, args, 4, fname, "day")?;
    let yea = zpp::opt_long_arg(host, args, 5, fname, "year")?;

    // Initialize structure with current time.
    let mut now = Time::new();
    let tzi = if gmt {
        now.unixtime2gmt(php_time());
        None
    } else {
        let tzi = timezone_info()?;
        now.set_zone_id(&tzi);
        now.unixtime2local(php_time());
        Some(tzi)
    };
    now.h = hou;
    if let Some(min) = min {
        now.i = min;
    }
    if let Some(sec) = sec {
        now.s = sec;
    }
    if let Some(mon) = mon {
        now.m = mon;
    }
    if let Some(day) = day {
        now.d = day;
    }
    if let Some(mut yea) = yea {
        if (0..70).contains(&yea) {
            yea += 2000;
        } else if (70..=100).contains(&yea) {
            yea += 1900;
        }
        now.y = yea;
    }
    now.update_ts(tzi.as_deref());
    match now.date_to_int() {
        Some(ts) => Ok(Value::Int(ts)),
        None => {
            crate::warn(host, fname, "Epoch doesn't fit in a PHP integer")?;
            Ok(Value::Bool(false))
        }
    }
}

fn fn_mktime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_mktime(host, args, "mktime", false)
}

fn fn_gmmktime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_mktime(host, args, "gmmktime", true)
}

fn fn_checkdate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let m = zpp::long_arg(host, args, 0, "checkdate", "month")?;
    let d = zpp::long_arg(host, args, 1, "checkdate", "day")?;
    let y = zpp::long_arg(host, args, 2, "checkdate", "year")?;
    Ok(Value::Bool(
        (1..=32767).contains(&y) && timelib::valid_date(y, m, d),
    ))
}

// --- localtime() / getdate() ----------------------------------------------------

/// `ts` in the default zone.
fn local_time(ts: i64) -> Result<(Time, Rc<Tz>), Thrown> {
    let tzi = timezone_info()?;
    let mut t = Time::new();
    t.set_zone_id(&tzi);
    t.unixtime2local(ts);
    Ok((t, tzi))
}

fn fn_localtime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ts = zpp::opt_long_arg(host, args, 0, "localtime", "timestamp")?.unwrap_or_else(php_time);
    let associative =
        crate::argparse::bool_param(host, args, 1, "localtime", "associative", false)?;
    let (t, _tzi) = local_time(ts)?;
    let fields = [
        ("tm_sec", t.s),
        ("tm_min", t.i),
        ("tm_hour", t.h),
        ("tm_mday", t.d),
        ("tm_mon", t.m - 1),
        ("tm_year", t.y - 1900),
        ("tm_wday", timelib::day_of_week(t.y, t.m, t.d)),
        ("tm_yday", timelib::day_of_year(t.y, t.m, t.d)),
        ("tm_isdst", t.dst as i64),
    ];
    let mut a = PhpArray::new();
    for (k, v) in fields {
        if associative {
            assoc(&mut a, k, Value::Int(v));
        } else {
            a.push(Value::Int(v));
        }
    }
    Ok(Value::Array(a))
}

fn fn_getdate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ts = zpp::opt_long_arg(host, args, 0, "getdate", "timestamp")?.unwrap_or_else(php_time);
    let (t, _tzi) = local_time(ts)?;
    let mut a = PhpArray::new();
    assoc(&mut a, "seconds", Value::Int(t.s));
    assoc(&mut a, "minutes", Value::Int(t.i));
    assoc(&mut a, "hours", Value::Int(t.h));
    assoc(&mut a, "mday", Value::Int(t.d));
    assoc(
        &mut a,
        "wday",
        Value::Int(timelib::day_of_week(t.y, t.m, t.d)),
    );
    assoc(&mut a, "mon", Value::Int(t.m));
    assoc(&mut a, "year", Value::Int(t.y));
    assoc(
        &mut a,
        "yday",
        Value::Int(timelib::day_of_year(t.y, t.m, t.d)),
    );
    assoc(
        &mut a,
        "weekday",
        Value::str(format::full_day_name(t.y, t.m, t.d)),
    );
    assoc(&mut a, "month", Value::str(format::month_name(t.m)));
    a.insert(ArrayKey::Int(0), Value::Int(ts));
    Ok(Value::Array(a))
}

// --- date_parse() / date_parse_from_format() ------------------------------------

/// `zval_from_error_container`: the counts and the messages by position.
pub(crate) fn add_error_container(a: &mut PhpArray, error: &Errors) {
    let list = |msgs: &mut dyn Iterator<Item = timelib::Message>| {
        let mut l = PhpArray::new();
        for m in msgs {
            l.insert(ArrayKey::Int(m.position as i64), Value::str(m.message));
        }
        Value::Array(l)
    };
    assoc(a, "warning_count", Value::Int(error.warning_count() as i64));
    assoc(a, "warnings", list(&mut error.warnings()));
    assoc(a, "error_count", Value::Int(error.error_count() as i64));
    assoc(a, "errors", list(&mut error.errors()));
}

/// `php_date_do_return_parsed_time`.
fn parsed_time_array(t: &Time, error: &Errors) -> Value {
    let mut a = PhpArray::new();
    let element = |a: &mut PhpArray, name: &str, v: i64| {
        let v = if v == TIMELIB_UNSET {
            Value::Bool(false)
        } else {
            Value::Int(v)
        };
        assoc(a, name, v);
    };
    element(&mut a, "year", t.y);
    element(&mut a, "month", t.m);
    element(&mut a, "day", t.d);
    element(&mut a, "hour", t.h);
    element(&mut a, "minute", t.i);
    element(&mut a, "second", t.s);
    let fraction = if t.us == TIMELIB_UNSET {
        Value::Bool(false)
    } else {
        Value::Float(t.us as f64 / 1000000.0)
    };
    assoc(&mut a, "fraction", fraction);
    add_error_container(&mut a, error);
    assoc(&mut a, "is_localtime", Value::Bool(t.is_localtime != 0));
    if t.is_localtime != 0 {
        element(&mut a, "zone_type", t.zone_type as i64);
        match t.zone_type {
            timelib::TIMELIB_ZONETYPE_OFFSET => {
                element(&mut a, "zone", t.z as i64);
                assoc(&mut a, "is_dst", Value::Bool(t.dst != 0));
            }
            timelib::TIMELIB_ZONETYPE_ID => {
                if let Some(abbr) = t.tz_abbr() {
                    assoc(&mut a, "tz_abbr", Value::str(abbr));
                }
                if let Some(id) = t.tz_name() {
                    assoc(&mut a, "tz_id", Value::str(id));
                }
            }
            timelib::TIMELIB_ZONETYPE_ABBR => {
                element(&mut a, "zone", t.z as i64);
                assoc(&mut a, "is_dst", Value::Bool(t.dst != 0));
                assoc(
                    &mut a,
                    "tz_abbr",
                    Value::str(t.tz_abbr().unwrap_or_default()),
                );
            }
            _ => {}
        }
    }
    if t.have_relative != 0 {
        let r = &t.relative;
        let mut rel = PhpArray::new();
        assoc(&mut rel, "year", Value::Int(r.y));
        assoc(&mut rel, "month", Value::Int(r.m));
        assoc(&mut rel, "day", Value::Int(r.d));
        assoc(&mut rel, "hour", Value::Int(r.h));
        assoc(&mut rel, "minute", Value::Int(r.i));
        assoc(&mut rel, "second", Value::Int(r.s));
        if r.have_weekday_relative != 0 {
            assoc(&mut rel, "weekday", Value::Int(r.weekday as i64));
        }
        if r.have_special_relative != 0 && r.special.type_ == timelib::TIMELIB_SPECIAL_WEEKDAY {
            assoc(&mut rel, "weekdays", Value::Int(r.special.amount));
        }
        if r.first_last_day_of != 0 {
            let key = if r.first_last_day_of == timelib::TIMELIB_SPECIAL_FIRST_DAY_OF_MONTH {
                "first_day_of_month"
            } else {
                "last_day_of_month"
            };
            assoc(&mut rel, key, Value::Bool(true));
        }
        assoc(&mut a, "relative", Value::Array(rel));
    }
    Value::Array(a)
}

fn fn_date_parse(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let date = str_param(host, args, 0, "date_parse", "datetime")?;
    let (t, error) = strtotime(&date);
    Ok(parsed_time_array(&t, &error))
}

fn fn_date_parse_from_format(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let format = str_param(host, args, 0, "date_parse_from_format", "format")?;
    let date = crate::argparse::path_param(host, args, 1, "date_parse_from_format", "datetime")?;
    let (t, error) = parse_from_format(&format, &date);
    Ok(parsed_time_array(&t, &error))
}

// --- default timezone -----------------------------------------------------------

fn fn_date_default_timezone_set(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let zone = str_param(host, args, 0, "date_default_timezone_set", "timezoneId")?;
    if !timezone_id_is_valid(&zone) {
        host.emit_diagnostic(
            crate::E_NOTICE,
            &format!(
                "date_default_timezone_set(): Timezone ID '{}' is invalid",
                String::from_utf8_lossy(c_str(&zone))
            ),
        )?;
        return Ok(Value::Bool(false));
    }
    TIMEZONE.with(|t| *t.borrow_mut() = Some(c_str(&zone).to_vec()));
    Ok(Value::Bool(true))
}

fn fn_date_default_timezone_get(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(timezone_info()?.name()))
}

// --- timezone_*() -------------------------------------------------------------

fn check_id_allowed(id: &[u8], what: i64) -> bool {
    const GROUPS: [(i64, &[u8]); 11] = [
        (PHP_DATE_TIMEZONE_GROUP_AFRICA, b"Africa/"),
        (PHP_DATE_TIMEZONE_GROUP_AMERICA, b"America/"),
        (PHP_DATE_TIMEZONE_GROUP_ANTARCTICA, b"Antarctica/"),
        (PHP_DATE_TIMEZONE_GROUP_ARCTIC, b"Arctic/"),
        (PHP_DATE_TIMEZONE_GROUP_ASIA, b"Asia/"),
        (PHP_DATE_TIMEZONE_GROUP_ATLANTIC, b"Atlantic/"),
        (PHP_DATE_TIMEZONE_GROUP_AUSTRALIA, b"Australia/"),
        (PHP_DATE_TIMEZONE_GROUP_EUROPE, b"Europe/"),
        (PHP_DATE_TIMEZONE_GROUP_INDIAN, b"Indian/"),
        (PHP_DATE_TIMEZONE_GROUP_PACIFIC, b"Pacific/"),
        (PHP_DATE_TIMEZONE_GROUP_UTC, b"UTC"),
    ];
    GROUPS.iter().any(|(group, prefix)| {
        what & group != 0
            && id.len() >= prefix.len()
            && id[..prefix.len()].eq_ignore_ascii_case(prefix)
    })
}

/// timezone_identifiers_list() and DateTimeZone::listIdentifiers() (`fname`).
pub(crate) fn identifiers_list(
    fname: &str,
    what: i64,
    option: Option<&[u8]>,
) -> Result<Value, Thrown> {
    let option = option.map(c_str);
    if what == PHP_DATE_TIMEZONE_PER_COUNTRY && option.is_none_or(|o| o.len() != 2) {
        return Err(zpp::value_error(
            fname,
            2,
            "countryCode",
            "must be a two-letter ISO 3166-1 compatible country code when argument #1 ($timezoneGroup) is DateTimeZone::PER_COUNTRY",
        ));
    }
    let mut a = PhpArray::new();
    for e in tzdb().identifiers() {
        let listed = if what == PHP_DATE_TIMEZONE_PER_COUNTRY {
            option.is_some_and(|o| e.country_code == o)
        } else {
            what == PHP_DATE_TIMEZONE_GROUP_ALL_W_BC || (check_id_allowed(e.id, what) && e.bc)
        };
        if listed {
            a.push(Value::str(e.id));
        }
    }
    Ok(Value::Array(a))
}

fn fn_timezone_identifiers_list(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "timezone_identifiers_list";
    let what = long_or(
        host,
        args,
        0,
        fname,
        "timezoneGroup",
        PHP_DATE_TIMEZONE_GROUP_ALL,
    )?;
    let option = crate::argparse::str_or_null_param(host, args, 1, fname, "countryCode")?;
    identifiers_list(fname, what, option.as_deref())
}

fn fn_timezone_version_get(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(tzdb().version()))
}

/// timezone_abbreviations_list() and DateTimeZone::listAbbreviations().
pub(crate) fn abbreviations_list() -> Value {
    let mut out = PhpArray::new();
    for e in timelib::abbreviations() {
        let mut element = PhpArray::new();
        assoc(&mut element, "dst", Value::Bool(e.dst));
        // add_assoc_long of a float: C's conversion.
        assoc(&mut element, "offset", Value::Int(e.gmtoffset as i64));
        let id = e.tz_id.map_or(Value::Null, Value::str);
        assoc(&mut element, "timezone_id", id);
        let key = ArrayKey::from_bytes(e.name);
        let mut list = match out.get(&key) {
            Some(Value::Array(l)) => l.clone(),
            _ => PhpArray::new(),
        };
        list.push(Value::Array(element));
        out.insert(key, Value::Array(list));
    }
    Value::Array(out)
}

fn fn_timezone_abbreviations_list(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    Ok(abbreviations_list())
}

fn fn_timezone_name_from_abbr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "timezone_name_from_abbr";
    let abbr = str_param(host, args, 0, fname, "abbr")?;
    let gmtoffset = long_or(host, args, 1, fname, "utcOffset", -1)?;
    let isdst = long_or(host, args, 2, fname, "isDST", -1)?;
    Ok(
        match timelib::timezone_id_from_abbr(&abbr, gmtoffset, isdst as c_int) {
            Some(id) => Value::str(id),
            None => Value::Bool(false),
        },
    )
}

/// Register the date builtins.
pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let mut put = |name: &'static str, func: crate::NativeFn, min: usize, max: usize| {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: Some(max),
            },
        );
    };
    put("date", fn_date, 1, 2);
    put("gmdate", fn_gmdate, 1, 2);
    put("idate", fn_idate, 1, 2);
    put("strtotime", fn_strtotime, 1, 2);
    put("mktime", fn_mktime, 1, 6);
    put("gmmktime", fn_gmmktime, 1, 6);
    put("checkdate", fn_checkdate, 3, 3);
    put("localtime", fn_localtime, 0, 2);
    put("getdate", fn_getdate, 0, 1);
    put("date_parse", fn_date_parse, 1, 1);
    put("date_parse_from_format", fn_date_parse_from_format, 2, 2);
    put(
        "date_default_timezone_set",
        fn_date_default_timezone_set,
        1,
        1,
    );
    put(
        "date_default_timezone_get",
        fn_date_default_timezone_get,
        0,
        0,
    );
    put(
        "timezone_identifiers_list",
        fn_timezone_identifiers_list,
        0,
        2,
    );
    put(
        "timezone_abbreviations_list",
        fn_timezone_abbreviations_list,
        0,
        0,
    );
    put("timezone_name_from_abbr", fn_timezone_name_from_abbr, 1, 3);
    put("timezone_version_get", fn_timezone_version_get, 0, 0);
}
