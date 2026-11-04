//! The `date` extension (php-src ext/date): a port of php_date.c over
//! timelib (`rhp_csrc::timelib`, php-src's ext/date/lib with its embedded
//! tz database).
//!
//! Here: the procedural functions that need no date object, the default
//! timezone, php's per-request tz cache and last-errors container. The
//! classes are in `classes.rs` (methods) and `objects.rs` (native state and
//! handlers); `prelude.php` declares them.

mod classes;
mod format;
mod objects;

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
pub(crate) use objects::CLASSES;

/// The date classes and the procedural functions over them.
pub const PRELUDE_PHP: &str = include_str!("prelude.php");

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
    /// `DATEG(last_errors)`: the last parse's warnings and errors, if any.
    static LAST_ERRORS: RefCell<Option<Errors>> = const { RefCell::new(None) };
}

/// php's RINIT/post-deactivate: forget the default zone, the tz cache and
/// the last errors. A zone a date object still uses stays alive through
/// its `Rc`.
pub(crate) fn reset_state() {
    TIMEZONE.with(|t| *t.borrow_mut() = None);
    TZCACHE.with(|c| c.borrow_mut().clear());
    set_last_errors(None);
}

pub(crate) fn set_last_errors(e: Option<Errors>) {
    let old = LAST_ERRORS.with(|l| l.replace(e));
    drop(old);
}

/// date_get_last_errors()'s array, if there are errors.
pub(crate) fn last_errors_array() -> Option<PhpArray> {
    LAST_ERRORS.with(|l| {
        l.borrow().as_ref().map(|e| {
            let mut a = PhpArray::new();
            add_error_container(&mut a, e);
            a
        })
    })
}

/// The cached zone a time's `tz_info` points to (every zone timelib hands
/// out comes from the cache, through [`parse_tzfile`]).
pub(crate) fn cached_zone(p: *mut sys::timelib_tzinfo) -> Option<Rc<Tz>> {
    if p.is_null() {
        return None;
    }
    TZCACHE.with(|c| {
        c.borrow()
            .values()
            .find(|tz| std::ptr::eq(tz.as_ptr(), p))
            .cloned()
    })
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
pub(crate) fn c_str(s: &[u8]) -> &[u8] {
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
pub(crate) unsafe extern "C" fn parse_tzfile_wrapper(
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

pub(crate) fn resume_callback_panic() {
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
pub(crate) fn timezone_info() -> Result<Rc<Tz>, Thrown> {
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

/// [`constant_deprecation`] for the stub's deprecated class constants.
pub(crate) fn class_constant_deprecation(class: &[u8], name: &[u8]) -> Option<String> {
    if !(class.eq_ignore_ascii_case(b"DateTimeInterface") && name == b"RFC7231") {
        return None;
    }
    Some("Constant DateTimeInterface::RFC7231 is deprecated since 8.5, as this format ignores the associated timezone and always uses GMT".into())
}

/// Whether `class` declares the virtual property `name`: DatePeriod's
/// readonly properties, which its prelude class serves through `__get`.
pub(crate) fn has_virtual_property(host: &dyn Host, class: u32, name: &[u8]) -> bool {
    objects::PERIOD_PROPS.iter().any(|p| p.as_bytes() == name)
        && host
            .class_name(class)
            .is_some_and(|c| host.reflect_is_a(&c, b"DatePeriod"))
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
pub(crate) fn php_time() -> i64 {
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

// --- strftime() / gmstrftime() ----------------------------------------------------

/// The E_DEPRECATED of a `#[\Deprecated]` function, raised when it is called.
fn deprecated_fn(host: &mut dyn Host, fname: &str, since: &str, msg: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(
        crate::E_DEPRECATED,
        &format!("Function {fname}() is deprecated since {since}, {msg}"),
    )
}

/// `php_strftime`: the C library's strftime in the process locale (rhp's
/// locale is always "C").
fn php_strftime(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    gmt: bool,
) -> Result<Value, Thrown> {
    deprecated_fn(
        host,
        fname,
        "8.1",
        "use IntlDateFormatter::format() instead",
    )?;
    let format = str_param(host, args, 0, fname, "format")?;
    let ts = zpp::opt_long_arg(host, args, 1, fname, "timestamp")?.unwrap_or_else(php_time);
    if format.is_empty() {
        return Ok(Value::Bool(false));
    }
    let mut t = Time::new();
    let offset = if gmt {
        t.unixtime2gmt(ts);
        None
    } else {
        let tzi = timezone_info()?;
        t.set_zone_id(&tzi);
        t.unixtime2local(ts);
        Some(tzi.offset_at(ts))
    };
    let zone = std::ffi::CString::new(match &offset {
        Some(o) => c_str(o.abbr()).to_vec(),
        None => b"GMT".to_vec(),
    })
    .expect("no NUL");
    // SAFETY: a plain C struct; zero is a valid value for every field.
    let mut ta: libc::tm = unsafe { std::mem::zeroed() };
    ta.tm_sec = t.s as c_int;
    ta.tm_min = t.i as c_int;
    ta.tm_hour = t.h as c_int;
    ta.tm_mday = t.d as c_int;
    ta.tm_mon = (t.m - 1) as c_int;
    ta.tm_year = (t.y - 1900) as c_int;
    ta.tm_wday = timelib::day_of_week(t.y, t.m, t.d) as c_int;
    ta.tm_yday = timelib::day_of_year(t.y, t.m, t.d) as c_int;
    ta.tm_isdst = offset.as_ref().map_or(0, |o| o.is_dst as c_int);
    ta.tm_gmtoff = offset.as_ref().map_or(0, |o| o.offset as libc::c_long);
    ta.tm_zone = zone.as_ptr() as _;
    let cformat = std::ffi::CString::new(c_str(&format)).expect("no NUL");
    let mut buf_len = 256usize;
    let mut max_reallocs = 5;
    let mut buf = vec![0u8; buf_len];
    let mut real_len;
    loop {
        // SAFETY: `buf` holds `buf_len` bytes; the format and the struct
        // (and the zone name it points to) are alive.
        real_len = unsafe {
            libc::strftime(
                buf.as_mut_ptr() as *mut c_char,
                buf_len,
                cformat.as_ptr(),
                &ta,
            )
        };
        if real_len != buf_len && real_len != 0 {
            break;
        }
        buf_len *= 2;
        buf.resize(buf_len, 0);
        max_reallocs -= 1;
        if max_reallocs == 0 {
            break;
        }
    }
    if real_len != 0 && real_len != buf_len {
        buf.truncate(real_len);
        return Ok(Value::str(buf));
    }
    Ok(Value::Bool(false))
}

fn fn_strftime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_strftime(host, args, "strftime", false)
}

fn fn_gmstrftime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_strftime(host, args, "gmstrftime", true)
}

// --- date_sunrise() / date_sunset() / date_sun_info() -------------------------------

/// `Z_PARAM_DOUBLE` (`None` for null when `nullable`).
fn double_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
    nullable: bool,
) -> Result<Option<f64>, Thrown> {
    let v = args.get(i).map(rhp_semantics::deref).unwrap_or(Value::Null);
    let ty = if nullable { "?float" } else { "float" };
    let wrong = |host: &dyn Host, v: &Value| {
        Thrown::type_error(format!(
            "{fname}(): Argument #{} (${pname}) must be of type {ty}, {} given",
            i + 1,
            crate::argparse::value_name(host, v)
        ))
    };
    Ok(Some(match &v {
        Value::Null if nullable => return Ok(None),
        Value::Float(f) => *f,
        Value::Int(n) => *n as f64,
        _ if host.caller_strict_types() => return Err(wrong(host, &v)),
        Value::Bool(b) => *b as i64 as f64,
        Value::Null => 0.0,
        Value::Str(st) => match rhp_values::conv::numeric_string(st.as_bytes()) {
            Some(rhp_values::conv::Num::Int(n)) => n as f64,
            Some(rhp_values::conv::Num::Float(f)) => f,
            None => return Err(wrong(host, &v)),
        },
        _ => return Err(wrong(host, &v)),
    }))
}

/// `INI_FLT(name)`.
fn ini_float(host: &dyn Host, name: &[u8]) -> f64 {
    let v = host.ini_get(name).unwrap_or_default();
    match rhp_values::conv::scan_number(&v) {
        Some((rhp_values::conv::Num::Int(n), _)) => n as f64,
        Some((rhp_values::conv::Num::Float(f), _)) => f,
        None => 0.0,
    }
}

const SUNFUNCS_RET_TIMESTAMP: i64 = 0;
const SUNFUNCS_RET_STRING: i64 = 1;
const SUNFUNCS_RET_DOUBLE: i64 = 2;

/// `timelib_astro_rise_set_altitude(t, ...)`: (rs, h_rise, h_set, rise,
/// set, transit).
fn rise_set(
    t: &Time,
    lon: f64,
    lat: f64,
    altit: f64,
    upper_limb: c_int,
) -> (c_int, f64, f64, i64, i64, i64) {
    let (mut h_rise, mut h_set) = (0.0, 0.0);
    let (mut rise, mut set, mut transit) = (0, 0, 0);
    // SAFETY: the time and its zone are alive; valid out-pointers.
    let rs = unsafe {
        sys::timelib_astro_rise_set_altitude(
            t.as_ptr(),
            lon,
            lat,
            altit,
            upper_limb,
            &mut h_rise,
            &mut h_set,
            &mut rise,
            &mut set,
            &mut transit,
        )
    };
    (rs, h_rise, h_set, rise, set, transit)
}

/// `php_do_date_sunrise_sunset`.
fn php_sunrise_sunset(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    calc_sunset: bool,
) -> Result<Value, Thrown> {
    deprecated_fn(host, fname, "8.1", "use date_sun_info() instead")?;
    let time = zpp::long_arg(host, args, 0, fname, "timestamp")?;
    let retformat = long_or(host, args, 1, fname, "returnFormat", SUNFUNCS_RET_STRING)?;
    let latitude = match double_arg(host, args, 2, fname, "latitude", true)? {
        Some(v) => v,
        None => ini_float(host, b"date.default_latitude"),
    };
    let longitude = match double_arg(host, args, 3, fname, "longitude", true)? {
        Some(v) => v,
        None => ini_float(host, b"date.default_longitude"),
    };
    let zenith = match double_arg(host, args, 4, fname, "zenith", true)? {
        Some(v) => v,
        None if calc_sunset => ini_float(host, b"date.sunset_zenith"),
        None => ini_float(host, b"date.sunrise_zenith"),
    };
    let gmt_offset = double_arg(host, args, 5, fname, "utcOffset", true)?;
    if ![
        SUNFUNCS_RET_TIMESTAMP,
        SUNFUNCS_RET_STRING,
        SUNFUNCS_RET_DOUBLE,
    ]
    .contains(&retformat)
    {
        return Err(zpp::value_error(
            fname,
            2,
            "returnFormat",
            "must be one of SUNFUNCS_RET_TIMESTAMP, SUNFUNCS_RET_STRING, or SUNFUNCS_RET_DOUBLE",
        ));
    }
    let altitude = 90.0 - zenith;
    if !latitude.is_finite() || !longitude.is_finite() {
        return Ok(Value::Bool(false));
    }
    let tzi = timezone_info()?;
    let mut t = Time::new();
    t.set_zone_id(&tzi);
    // php reads the offset before setting the time: at the epoch.
    // SAFETY: the time and its zone are alive.
    let gmt_offset = gmt_offset
        .unwrap_or_else(|| unsafe { sys::timelib_get_current_offset(t.as_ptr()) } as f64 / 3600.0);
    t.unixtime2local(time);
    let (rs, h_rise, h_set, rise, set, _) = rise_set(&t, longitude, latitude, altitude, 1);
    if rs != 0 {
        return Ok(Value::Bool(false));
    }
    if retformat == SUNFUNCS_RET_TIMESTAMP {
        return Ok(Value::Int(if calc_sunset { set } else { rise }));
    }
    let mut n = if calc_sunset { h_set } else { h_rise } + gmt_offset;
    if !(0.0..=24.0).contains(&n) {
        n -= (n / 24.0).floor() * 24.0;
    }
    if !(0.0..=24.0).contains(&n) {
        return Ok(Value::Bool(false));
    }
    Ok(match retformat {
        SUNFUNCS_RET_STRING => Value::str(format!(
            "{:02}:{:02}",
            n as i32,
            (60.0 * (n - (n as i32) as f64)) as i32
        )),
        _ => Value::Float(n),
    })
}

fn fn_date_sunrise(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_sunrise_sunset(host, args, "date_sunrise", false)
}

fn fn_date_sunset(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    php_sunrise_sunset(host, args, "date_sunset", true)
}

fn fn_date_sun_info(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "date_sun_info";
    let time = zpp::long_arg(host, args, 0, fname, "timestamp")?;
    let latitude = double_arg(host, args, 1, fname, "latitude", false)?.unwrap_or(0.0);
    let longitude = double_arg(host, args, 2, fname, "longitude", false)?.unwrap_or(0.0);
    if !latitude.is_finite() {
        return Err(zpp::value_error(fname, 2, "latitude", "must be finite"));
    }
    if !longitude.is_finite() {
        return Err(zpp::value_error(fname, 3, "longitude", "must be finite"));
    }
    let tzi = timezone_info()?;
    let mut t = Time::new();
    t.set_zone_id(&tzi);
    t.unixtime2local(time);
    let mut t2 = Time::new();
    let mut to_int = |sse: i64| {
        t2.sse = sse;
        // timelib_date_to_int, its overflow flag ignored.
        let mut dummy: c_int = 0;
        // SAFETY: the time is alive.
        Value::Int(unsafe { sys::timelib_date_to_int(t2.as_ptr(), &mut dummy) })
    };
    let mut a = PhpArray::new();
    let rows: [(f64, c_int, &str, &str); 4] = [
        (-35.0 / 60.0, 1, "sunrise", "sunset"),
        (-6.0, 0, "civil_twilight_begin", "civil_twilight_end"),
        (-12.0, 0, "nautical_twilight_begin", "nautical_twilight_end"),
        (
            -18.0,
            0,
            "astronomical_twilight_begin",
            "astronomical_twilight_end",
        ),
    ];
    for (n, (alt, limb, begin, end)) in rows.into_iter().enumerate() {
        let (rs, _, _, rise, set, transit) = rise_set(&t, longitude, latitude, alt, limb);
        match rs {
            -1 => {
                assoc(&mut a, begin, Value::Bool(false));
                assoc(&mut a, end, Value::Bool(false));
            }
            1 => {
                assoc(&mut a, begin, Value::Bool(true));
                assoc(&mut a, end, Value::Bool(true));
            }
            _ => {
                assoc(&mut a, begin, to_int(rise));
                assoc(&mut a, end, to_int(set));
            }
        }
        if n == 0 {
            assoc(&mut a, "transit", to_int(transit));
        }
    }
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
    // `option_len` counts every byte; the comparison then meets a NUL.
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
    put("strftime", fn_strftime, 1, 2);
    put("gmstrftime", fn_gmstrftime, 1, 2);
    put("date_sunrise", fn_date_sunrise, 1, 6);
    put("date_sunset", fn_date_sunset, 1, 6);
    put("date_sun_info", fn_date_sun_info, 3, 3);
    m.insert(
        "__date",
        Builtin {
            name: "__date",
            func: classes::fn_date,
            min_args: 1,
            max_args: None,
        },
    );
}
