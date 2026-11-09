//! `IntlDateFormatter`, `datefmt_*` and `IntlDatePatternGenerator`, ported
//! from php-src ext/intl/dateformat/ onto ICU's `udat` / `udatpg` and, for
//! its calendar and time zone, DateFormat itself (icu.cpp).

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;

use rhp_values::{ArrayKey, ObjectHandle, PhpArray, Value};

use super::common::{self, Call};
use super::cpp;
use super::error::{self, IntlError};
use super::timezone::{self, Zone};
use super::{class_int, sys, UChar};
use crate::zend::object_handlers::{self, CloneObj, ObjectHandlers};
use crate::zend::zpp::deref;
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(clone_obj),
    ..ObjectHandlers::STD
};

pub(super) static DTPG_HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(dtpg_clone),
    ..ObjectHandlers::STD
};

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlDateFormatter",
    helper: "__datefmt",
    props: "",
};

pub(super) const DTPG_SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlDatePatternGenerator",
    helper: "__datepatgen",
    props: "",
};

pub(super) const ALIASES: &[(&str, &str)] = &[
    ("datefmt_create", "create"),
    ("datefmt_get_datetype", "getDateType"),
    ("datefmt_get_timetype", "getTimeType"),
    ("datefmt_get_calendar", "getCalendar"),
    ("datefmt_set_calendar", "setCalendar"),
    ("datefmt_get_timezone_id", "getTimeZoneId"),
    ("datefmt_get_calendar_object", "getCalendarObject"),
    ("datefmt_get_timezone", "getTimeZone"),
    ("datefmt_set_timezone", "setTimeZone"),
    ("datefmt_set_pattern", "setPattern"),
    ("datefmt_get_pattern", "getPattern"),
    ("datefmt_get_locale", "getLocale"),
    ("datefmt_set_lenient", "setLenient"),
    ("datefmt_is_lenient", "isLenient"),
    ("datefmt_format", "format"),
    ("datefmt_format_object", "formatObject"),
    ("datefmt_parse", "parse"),
    ("datefmt_localtime", "localtime"),
    ("datefmt_get_error_code", "getErrorCode"),
    ("datefmt_get_error_message", "getErrorMessage"),
];

/// `IntlDateFormatter_object`.
pub(super) struct DateFmtObj {
    err: RefCell<IntlError>,
    udatf: Cell<usize>,
    date_type: Cell<i64>,
    time_type: Cell<i64>,
    calendar: Cell<i64>,
    requested_locale: RefCell<Option<Vec<u8>>>,
}

impl Default for DateFmtObj {
    fn default() -> DateFmtObj {
        DateFmtObj {
            err: RefCell::default(),
            udatf: Cell::new(0),
            date_type: Cell::new(0),
            time_type: Cell::new(0),
            calendar: Cell::new(-1),
            requested_locale: RefCell::new(None),
        }
    }
}

impl DateFmtObj {
    fn df(&self) -> *mut c_void {
        self.udatf.get() as *mut c_void
    }
}

impl Drop for DateFmtObj {
    fn drop(&mut self) {
        if !self.df().is_null() {
            // SAFETY: the formatter this object owns.
            unsafe { sys::udat_close(self.df()) };
        }
    }
}

fn clone_obj(_host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = common::state::<DateFmtObj>(src);
    let to = Rc::new(DateFmtObj::default());
    object_handlers::set_payload(dst, to.clone());
    to.date_type.set(from.date_type.get());
    to.time_type.set(from.time_type.get());
    to.calendar.set(from.calendar.get());
    *to.requested_locale.borrow_mut() = from.requested_locale.borrow().clone();
    if from.df().is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot clone uninitialized IntlDateFormatter",
        ));
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a live formatter.
    let c = unsafe { sys::udat_clone(from.df(), &mut status) };
    to.udatf.set(c as usize);
    if sys::failure(status) {
        return Err(Thrown::error("Error", "Failed to clone IntlDateFormatter"));
    }
    Ok(())
}

/// `INTL_UDATE_FMT_OK`.
fn fmt_ok(i: i64) -> bool {
    [
        "FULL",
        "LONG",
        "MEDIUM",
        "SHORT",
        "RELATIVE_FULL",
        "RELATIVE_LONG",
        "RELATIVE_MEDIUM",
        "RELATIVE_SHORT",
        "NONE",
        "PATTERN",
    ]
    .iter()
    .any(|n| class_int("IntlDateFormatter", n) == i)
        // UDAT_RELATIVE (1 << 7) alone
        || i == 1 << 7
}

/// A calendar argument (`Z_PARAM_OBJ_OF_CLASS_OR_LONG_OR_NULL`).
enum CalArg {
    Null,
    Long(i64),
    Obj(Value),
}

fn cal_arg(host: &mut dyn Host, call: &Call, i: usize) -> Result<CalArg, Thrown> {
    Ok(match call.arg(i) {
        None | Some(Value::Null) => CalArg::Null,
        Some(Value::Int(n)) => CalArg::Long(n),
        Some(v @ Value::Object(_)) if timezone::is_a(host, &v, b"IntlCalendar") => CalArg::Obj(v),
        Some(v @ (Value::Float(_) | Value::Bool(_) | Value::Str(_)))
            if !host.caller_strict_types() =>
        {
            match crate::zend::zpp::try_get_long(host, &v)? {
                Some(n)
                    if !matches!(v, Value::Str(_))
                        || rhp_values::conv::numeric_string(
                            rhp_values::conv::to_string_bytes(&v).as_slice(),
                        )
                        .is_some() =>
                {
                    CalArg::Long(n)
                }
                _ => return Err(cal_type_error(host, call, i, &v)),
            }
        }
        Some(v) => return Err(cal_type_error(host, call, i, &v)),
    })
}

fn cal_type_error(host: &dyn Host, call: &Call, i: usize, v: &Value) -> Thrown {
    call.type_error(
        i,
        &format!(
            "must be of type IntlCalendar|int|null, {} given",
            crate::zend::zpp::zval_value_name(host, v)
        ),
    )
}

/// `datefmt_process_calendar_arg`: the calendar, its php type, and
/// whether it is new (owned); `None` after the error.
fn process_calendar_arg(
    host: &mut dyn Host,
    arg: &CalArg,
    locale: &std::ffi::CStr,
    err: &RefCell<IntlError>,
    f: &str,
) -> Result<Option<(*mut c_void, i64, bool)>, Thrown> {
    let mut status = sys::U_ZERO_ERROR;
    let greg = class_int("IntlDateFormatter", "GREGORIAN");
    let trad = class_int("IntlDateFormatter", "TRADITIONAL");
    let (cal, ty, owned) = match arg {
        CalArg::Null => (
            // SAFETY: a C locale.
            unsafe { cpp::rhp_datefmt_new_calendar(0, locale.as_ptr(), &mut status) },
            greg,
            true,
        ),
        CalArg::Long(v) => {
            if *v != trad && *v != greg {
                error::errors_set(
                    host,
                    err,
                    f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "Invalid value for calendar type; it must be one of IntlDateFormatter::TRADITIONAL (locale's default calendar) or IntlDateFormatter::GREGORIAN. Alternatively, it can be an IntlCalendar object",
                )?;
                return Ok(None);
            }
            // SAFETY: a C locale.
            let c = unsafe {
                cpp::rhp_datefmt_new_calendar((*v == trad) as i32, locale.as_ptr(), &mut status)
            };
            (c, *v, true)
        }
        CalArg::Obj(o) => {
            let c = super::calendar::cal_of(o);
            if c.is_null() {
                error::errors_set(
                    host,
                    err,
                    f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "Found unconstructed IntlCalendar object",
                )?;
                return Ok(None);
            }
            (c, -1, false)
        }
    };
    if cal.is_null() && !sys::failure(status) {
        status = sys::U_MEMORY_ALLOCATION_ERROR;
    }
    if sys::failure(status) {
        if owned && !cal.is_null() {
            // SAFETY: the calendar made above.
            unsafe { cpp::rhp_cal_delete(cal) };
        }
        error::errors_set(
            host,
            err,
            f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "Failure instantiating calendar",
        )?;
        return Ok(None);
    }
    Ok(Some((cal, ty, owned)))
}

/// `datefmt_ctor`: false after a failure (the global error set).
fn ctor(host: &mut dyn Host, call: &Call, dfo: &DateFmtObj) -> Result<bool, Thrown> {
    let f = call.fname.clone();
    error::error_reset();
    let locale_arg = call.opt_bytes(0);
    let date_type = call.long(1, class_int("IntlDateFormatter", "FULL"));
    let time_type = call.long(2, class_int("IntlDateFormatter", "FULL"));
    let tz_arg = call.arg(3);
    let cal = cal_arg(host, call, 4)?;
    let pattern = call.opt_bytes(5);
    dfo.err.borrow_mut().reset();
    if !dfo.df().is_null() {
        error::errors_set(
            host,
            &dfo.err,
            &f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "cannot call constructor twice",
        )?;
        return Ok(false);
    }
    if !fmt_ok(date_type) {
        error::set(
            host,
            &f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "invalid date format style",
        )?;
        return Ok(false);
    }
    if !fmt_ok(time_type) {
        error::set(
            host,
            &f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "invalid time format style",
        )?;
        return Ok(false);
    }
    let pat = class_int("IntlDateFormatter", "PATTERN");
    if date_type == pat && time_type != pat {
        error::set(
            host,
            &f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "datefmt_create: time format must be IntlDateFormatter::PATTERN if date format is IntlDateFormatter::PATTERN",
        )?;
        return Ok(false);
    }
    let locale_len = locale_arg.as_ref().map_or(0, |l| l.len());
    if super::locale_too_long(host, &f, locale_len)? {
        return Ok(false);
    }
    let locale_str = match locale_arg {
        Some(l) if !l.is_empty() => l,
        _ => super::locale_get_default(),
    };
    let loc_c = super::c_string(&locale_str);
    let canon = super::canonicalize_locale_string(&loc_c);
    let final_locale = canon.clone().unwrap_or_else(|| loc_c.clone());
    // SAFETY: a C locale.
    let iso3 = unsafe { cpp::rhp_locale_iso3_len(final_locale.as_ptr()) };
    if iso3 < 0 || (locale_len == 1 && locale_str[0] != b'C') || (locale_len > 1 && iso3 == 0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{f}(): Argument #1 ($locale) \"{}\" is invalid",
                String::from_utf8_lossy(loc_c.as_bytes())
            ),
        ));
    }
    let Some((cal, cal_type, cal_owned)) =
        process_calendar_arg(host, &cal, &final_locale, &dfo.err, &f)?
    else {
        return Ok(false);
    };
    let delete_cal = |c: *mut c_void| {
        if cal_owned {
            // SAFETY: the calendar made for this formatter.
            unsafe { cpp::rhp_cal_delete(c) };
        }
    };
    let explicit_tz = !matches!(tz_arg, None | Some(Value::Null));
    let mut tz: Option<Zone> = None;
    if explicit_tz || cal_owned {
        match timezone::process_argument(host, tz_arg.as_ref(), Some(&dfo.err), &f) {
            Ok(Some(z)) => tz = Some(z),
            Ok(None) => {
                delete_cal(cal);
                return Ok(false);
            }
            Err(e) => {
                delete_cal(cal);
                return Err(e);
            }
        }
    }
    let mut svalue: Vec<UChar> = Vec::new();
    if let Some(p) = pattern.as_ref().filter(|p| !p.is_empty()) {
        match super::utf8_to_utf16(p) {
            Ok(u) => svalue = u,
            Err(st) => {
                dfo.err.borrow_mut().code = st;
                error::set(host, &f, st, "error converting pattern to UTF-16")?;
                delete_cal(cal);
                return Ok(false);
            }
        }
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a C locale, the pattern's units.
    let df = unsafe {
        sys::udat_open(
            time_type as i32,
            date_type as i32,
            final_locale.as_ptr(),
            std::ptr::null(),
            0,
            if svalue.is_empty() {
                std::ptr::null()
            } else {
                svalue.as_ptr()
            },
            svalue.len() as i32,
            &mut status,
        )
    };
    dfo.udatf.set(df as usize);
    dfo.err.borrow_mut().code = status;
    if !svalue.is_empty() {
        // SAFETY: a formatter (ICU checks it), the pattern's units.
        unsafe { sys::udat_applyPattern(df, 1, svalue.as_ptr(), svalue.len() as i32) };
        if sys::failure(status) {
            error::set(host, &f, status, "error applying pattern")?;
            delete_cal(cal);
            return Ok(false);
        }
    }
    if !sys::failure(status) {
        // SAFETY: a live formatter; it adopts what it is given.
        unsafe {
            if cal_owned {
                cpp::rhp_datefmt_adopt_calendar(df, cal);
            } else {
                cpp::rhp_datefmt_set_calendar(df, cal);
            }
            if let Some(z) = tz {
                cpp::rhp_datefmt_adopt_tz(df, z.into_raw());
            }
        }
    } else {
        error::set(host, &f, status, "date formatter creation failed")?;
        delete_cal(cal);
        return Ok(false);
    }
    dfo.date_type.set(date_type);
    dfo.time_type.set(time_type);
    dfo.calendar.set(cal_type);
    *dfo.requested_locale.borrow_mut() = Some(final_locale.into_bytes());
    Ok(!sys::failure(error::error_code()))
}

/// `__datefmt(method, argc, $this, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlDateFormatter", args, ALIASES);
    let f = call.fname.clone();
    match call.name {
        "__construct" => {
            let this = call.this.clone().expect("$this");
            let dfo = common::state_of::<DateFmtObj>(&this).expect("an object");
            error::in_ctor(|| ctor(host, &call, &dfo))?;
            return Ok(Value::Null);
        }
        "create" => {
            let Some(obj) = host.instantiate(b"IntlDateFormatter", Vec::new()) else {
                return Ok(Value::Null);
            };
            let dfo = common::state_of::<DateFmtObj>(&obj).expect("an object");
            return Ok(if ctor(host, &call, &dfo)? {
                obj
            } else {
                Value::Null
            });
        }
        "formatObject" => return format_object(host, &call),
        _ => {}
    }
    error::error_reset();
    let this = call.this.clone().expect("an object");
    let dfo = common::state_of::<DateFmtObj>(&this).expect("an object");
    match call.name {
        "getErrorCode" => return Ok(Value::Int(dfo.err.borrow().code as i64)),
        "getErrorMessage" => return Ok(Value::str(dfo.err.borrow().message())),
        _ => {}
    }
    let cal = if call.name == "setCalendar" {
        Some(cal_arg(host, &call, 0)?)
    } else {
        None
    };
    dfo.err.borrow_mut().reset();
    let df = dfo.df();
    if df.is_null() {
        return Err(Thrown::error(
            "Error",
            "Found unconstructed IntlDateFormatter",
        ));
    }
    let check = |host: &mut dyn Host, status: sys::UErrorCode, msg: &str| -> Result<bool, Thrown> {
        dfo.err.borrow_mut().code = status;
        error::check_status(host, &dfo.err, &f, msg)
    };
    let retval_utf8 = |host: &mut dyn Host, u: &[UChar]| -> Result<Value, Thrown> {
        match super::utf16_to_utf8(u) {
            Ok(s) => {
                // the conversion resets the code, then INTL_METHOD_CHECK_STATUS
                // copies it to the global one (an empty result's warning goes)
                dfo.err.borrow_mut().code = sys::U_ZERO_ERROR;
                error::error_set_code(sys::U_ZERO_ERROR);
                Ok(Value::str(s))
            }
            Err(st) => {
                check(host, st, "Error converting value to UTF-8")?;
                Ok(Value::Bool(false))
            }
        }
    };
    // SAFETY (below): `df` is the object's live formatter.
    match call.name {
        "getDateType" => Ok(Value::Int(dfo.date_type.get())),
        "getTimeType" => Ok(Value::Int(dfo.time_type.get())),
        "getPattern" => {
            let mut status = sys::U_ZERO_ERROR;
            let u = super::icu_uchars(64, &mut status, |buf, cap, st| unsafe {
                sys::udat_toPattern(df, 0, buf, cap, st)
            });
            if check(host, status, "Error getting formatter pattern")? {
                return Ok(Value::Bool(false));
            }
            retval_utf8(host, &u)
        }
        "setPattern" => {
            let v = call.bytes(0);
            let u = match super::utf8_to_utf16(&v) {
                Ok(u) => u,
                Err(st) => {
                    check(host, st, "Error converting pattern to UTF-16")?;
                    return Ok(Value::Bool(false));
                }
            };
            unsafe { sys::udat_applyPattern(df, 0, u.as_ptr(), u.len() as i32) };
            if check(host, sys::U_ZERO_ERROR, "Error setting symbol value")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(true))
        }
        "getLocale" => {
            let t = call.long(0, sys::ULOC_ACTUAL_LOCALE as i64);
            let mut status = sys::U_ZERO_ERROR;
            let p = unsafe { sys::udat_getLocaleByType(df, t as i32, &mut status) };
            if check(host, status, "Error getting locale")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::str(super::c_bytes(p)))
        }
        "isLenient" => Ok(Value::Bool(unsafe { sys::udat_isLenient(df) } != 0)),
        "setLenient" => {
            unsafe { sys::udat_setLenient(df, call.bool(0, false) as sys::UBool) };
            Ok(Value::Null)
        }
        "getTimeZoneId" => {
            let z = Zone::owned(unsafe { cpp::rhp_tz_clone(cpp::rhp_datefmt_get_tz(df)) });
            let id = z.id16().unwrap_or_default();
            match super::utf16_to_utf8(&id) {
                Ok(s) => Ok(Value::str(s)),
                Err(st) => {
                    check(host, st, "Could not convert time zone id to UTF-8")?;
                    Ok(Value::Bool(false))
                }
            }
        }
        "getTimeZone" => {
            let z = Zone::owned(unsafe { cpp::rhp_tz_clone(cpp::rhp_datefmt_get_tz(df)) });
            timezone::construct(host, z)
        }
        "setTimeZone" => {
            match timezone::process_argument(host, call.arg(0).as_ref(), Some(&dfo.err), &f)? {
                Some(z) => {
                    unsafe { cpp::rhp_datefmt_adopt_tz(df, z.into_raw()) };
                    Ok(Value::Bool(true))
                }
                None => Ok(Value::Bool(false)),
            }
        }
        "getCalendar" => Ok(match dfo.calendar.get() {
            -1 => Value::Bool(false),
            c => Value::Int(c),
        }),
        "getCalendarObject" => {
            let c = unsafe { cpp::rhp_datefmt_get_calendar(df) };
            if c.is_null() {
                return Ok(Value::Null);
            }
            let clone = unsafe { cpp::rhp_cal_clone(c) };
            super::calendar::object_create(host, clone)
        }
        "setCalendar" => {
            let loc = super::c_string(&dfo.requested_locale.borrow().clone().unwrap_or_default());
            let Some((cal, ty, owned)) =
                process_calendar_arg(host, cal.as_ref().expect("an argument"), &loc, &dfo.err, &f)?
            else {
                return Ok(Value::Bool(false));
            };
            let cal = if owned {
                let tz = unsafe { cpp::rhp_tz_clone(cpp::rhp_datefmt_get_tz(df)) };
                unsafe { cpp::rhp_cal_adopt_tz(cal, tz) };
                cal
            } else {
                unsafe { cpp::rhp_cal_clone(cal) }
            };
            unsafe { cpp::rhp_datefmt_adopt_calendar(df, cal) };
            dfo.calendar.set(ty);
            Ok(Value::Bool(true))
        }
        "format" => {
            let arg = call.arg(0).unwrap_or(Value::Null);
            let ts = if let Value::Array(a) = &arg {
                if a.is_empty() {
                    return Ok(Value::Bool(false));
                }
                let t = timestamp_of_array(host, &dfo, &f, a)?;
                let code = dfo.err.borrow().code;
                if check(host, code, "date formatting failed")? {
                    return Ok(Value::Bool(false));
                }
                t
            } else {
                let t = super::dates::zval_to_millis(host, &arg, Some(&dfo.err), &f)?;
                if sys::failure(dfo.err.borrow().code) {
                    return Ok(Value::Bool(false));
                }
                t
            };
            let mut status = sys::U_ZERO_ERROR;
            let u = super::icu_uchars(0, &mut status, |buf, cap, st| unsafe {
                sys::udat_format(df, ts, buf, cap, std::ptr::null_mut(), st)
            });
            if check(host, status, "Date formatting failed")? {
                return Ok(Value::Bool(false));
            }
            retval_utf8(host, &u)
        }
        "parse" | "parseToCalendar" | "localtime" => parse(host, &call, &dfo),
        other => unreachable!("IntlDateFormatter::{other}"),
    }
}

/// `internal_get_timestamp`: the formatter's calendar set from a
/// localtime()-style array.
fn timestamp_of_array(
    host: &mut dyn Host,
    dfo: &DateFmtObj,
    f: &str,
    a: &PhpArray,
) -> Result<f64, Thrown> {
    let get = |host: &mut dyn Host, key: &str| -> Result<i32, Thrown> {
        if sys::failure(dfo.err.borrow().code) {
            return Ok(0);
        }
        let Some(v) = a.get(&ArrayKey::from_bytes(key.as_bytes())).map(deref) else {
            return Ok(0);
        };
        match v {
            Value::Int(n) if n > i32::MAX as i64 || n < i32::MIN as i64 => {
                let msg = format!("value {n} is out of bounds for a 32-bit integer in key '{key}'");
                error::errors_set(host, &dfo.err, f, sys::U_ILLEGAL_ARGUMENT_ERROR, &msg)?;
                Ok(0)
            }
            Value::Int(n) => Ok(n as i32),
            _ => {
                let msg = format!("parameter array contains a non-integer element for key '{key}'");
                error::errors_set(host, &dfo.err, f, sys::U_ILLEGAL_ARGUMENT_ERROR, &msg)?;
                Ok(0)
            }
        }
    };
    let year = get(host, "tm_year")?.wrapping_add(1900);
    let month = get(host, "tm_mon")?;
    let hour = get(host, "tm_hour")?;
    let minute = get(host, "tm_min")?;
    let second = get(host, "tm_sec")?;
    let mday = get(host, "tm_mday")?;
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: the formatter's calendar, cloned and closed here.
    unsafe {
        let pcal = sys::ucal_clone(sys::udat_getCalendar(dfo.df()), &mut status);
        if status != sys::U_ZERO_ERROR {
            dfo.err.borrow_mut().code = status;
            error::errors_set(host, &dfo.err, f, status, "error cloning calendar")?;
            return Ok(0.0);
        }
        sys::ucal_setDateTime(pcal, year, month, mday, hour, minute, second, &mut status);
        let r = sys::ucal_getMillis(pcal, &mut status);
        sys::ucal_close(pcal);
        dfo.err.borrow_mut().code = status;
        Ok(r)
    }
}

/// `intl_convert_utf8_offset_to_utf16`: false for an offset inside a
/// character.
fn utf8_offset_to_utf16(s: &[u8], pos: &mut i32) -> bool {
    if *pos < 0 || *pos as usize > s.len() {
        return true;
    }
    match super::utf8_to_utf16(&s[..*pos as usize]) {
        Ok(u) => {
            *pos = u.len() as i32;
            true
        }
        Err(_) => false,
    }
}

/// `intl_convert_utf16_offset_to_utf8`.
fn utf16_offset_to_utf8(u: &[UChar], pos: i32) -> i32 {
    if pos < 0 || pos as usize > u.len() {
        return pos;
    }
    let mut p = pos as usize;
    if p > 0
        && p < u.len()
        && (0xD800..0xDC00).contains(&u[p - 1])
        && (0xDC00..0xE000).contains(&u[p])
    {
        p -= 1;
    }
    match super::utf16_to_utf8(&u[..p]) {
        Ok(s) => s.len() as i32,
        Err(_) => p as i32,
    }
}

/// `datefmt_parse`, `parseToCalendar` and `datefmt_localtime`.
fn parse(host: &mut dyn Host, call: &Call, dfo: &DateFmtObj) -> Result<Value, Thrown> {
    let f = call.fname.clone();
    let text = call.bytes(0);
    let r = call.reference(1);
    // "z!" sees the reference, so a variable holding null still counts.
    let pos_arg = r.as_ref().map(deref);
    let mut parse_pos: Option<i32> = None;
    if let Some(v) = &pos_arg {
        let n = if call.name == "parseToCalendar" {
            match crate::zend::zpp::try_get_long(host, v)? {
                Some(n) => n,
                None => {
                    return Err(Thrown::type_error(format!(
                        "{f}(): Argument #2 ($offset) must be of type int, {} given",
                        crate::zend::zpp::zval_value_name(host, v)
                    )))
                }
            }
        } else {
            crate::zend::zpp::zval_get_long(host, v)?
        };
        if n > i32::MAX as i64 || n < i32::MIN as i64 {
            error::error_set_code(sys::U_ILLEGAL_ARGUMENT_ERROR);
            error::set_custom_msg(host, &f, "String index is out of valid range.")?;
            return Ok(Value::Bool(false));
        }
        let p = n as i32;
        let skip_neg = call.name == "parseToCalendar" && p == -1;
        if !skip_neg && (p as u32 as usize) > text.len() {
            return Ok(Value::Bool(false));
        }
        parse_pos = Some(p);
    }
    let df = dfo.df();
    let u = match super::utf8_to_utf16(&text) {
        Ok(u) => u,
        Err(st) => {
            dfo.err.borrow_mut().code = st;
            error::check_status(host, &dfo.err, &f, "Error converting timezone to UTF-16")?;
            return Ok(Value::Bool(false));
        }
    };
    if let Some(p) = parse_pos.as_mut() {
        if !utf8_offset_to_utf16(&text, p) {
            dfo.err.borrow_mut().code = sys::U_INVALID_CHAR_FOUND;
            error::check_status(host, &dfo.err, &f, "Invalid UTF-8 offset")?;
            return Ok(Value::Bool(false));
        }
    }
    let pp = parse_pos
        .as_mut()
        .map_or(std::ptr::null_mut(), |p| p as *mut i32);
    let mut status = sys::U_ZERO_ERROR;
    let result = if call.name == "localtime" {
        // SAFETY: a live formatter and its calendar.
        let cal = unsafe { sys::udat_getCalendar(df) } as *mut c_void;
        unsafe { sys::udat_parseCalendar(df, cal, u.as_ptr(), u.len() as i32, pp, &mut status) };
        if let Some(p) = parse_pos.as_mut() {
            *p = utf16_offset_to_utf8(&u, *p);
        }
        dfo.err.borrow_mut().code = status;
        if error::check_status(host, &dfo.err, &f, "Date parsing failed")? {
            assign_pos(host, &r, parse_pos)?;
            return Ok(Value::Bool(false));
        }
        let mut out = PhpArray::new();
        let fields = [
            ("FIELD_SECOND", "tm_sec"),
            ("FIELD_MINUTE", "tm_min"),
            ("FIELD_HOUR_OF_DAY", "tm_hour"),
            ("FIELD_YEAR", "tm_year"),
            ("FIELD_DAY_OF_MONTH", "tm_mday"),
            ("FIELD_DAY_OF_WEEK", "tm_wday"),
            ("FIELD_DAY_OF_YEAR", "tm_yday"),
            ("FIELD_MONTH", "tm_mon"),
        ];
        for (field, key) in fields {
            let mut st = sys::U_ZERO_ERROR;
            let v = unsafe { sys::ucal_get(cal, class_int("IntlCalendar", field) as i32, &mut st) }
                as i64;
            dfo.err.borrow_mut().code = st;
            if error::check_status(
                host,
                &dfo.err,
                &f,
                "Date parsing - localtime failed : could not get a field from calendar",
            )? {
                assign_pos(host, &r, parse_pos)?;
                return Ok(Value::Bool(false));
            }
            let v = match key {
                "tm_year" => v - 1900,
                "tm_wday" => v - 1,
                _ => v,
            };
            common::put(&mut out, key, Value::Int(v));
        }
        let mut st = sys::U_ZERO_ERROR;
        let dst = unsafe { sys::ucal_inDaylightTime(cal, &mut st) };
        dfo.err.borrow_mut().code = st;
        if error::check_status(
            host,
            &dfo.err,
            &f,
            "Date parsing - localtime failed : while checking if currently in DST.",
        )? {
            assign_pos(host, &r, parse_pos)?;
            return Ok(Value::Bool(false));
        }
        common::put(&mut out, "tm_isdst", Value::Int((dst == 1) as i64));
        Value::Array(out)
    } else {
        let ts = if call.name == "parseToCalendar" {
            // SAFETY: a live formatter and its calendar.
            let cal = unsafe { sys::udat_getCalendar(df) } as *mut c_void;
            unsafe {
                sys::udat_parseCalendar(df, cal, u.as_ptr(), u.len() as i32, pp, &mut status)
            };
            if let Some(p) = parse_pos.as_mut() {
                *p = utf16_offset_to_utf8(&u, *p);
            }
            dfo.err.borrow_mut().code = status;
            if error::check_status(host, &dfo.err, &f, "Calendar parsing failed")? {
                assign_pos(host, &r, parse_pos)?;
                return Ok(Value::Bool(false));
            }
            let mut st = sys::U_ZERO_ERROR;
            let t = unsafe { sys::ucal_getMillis(cal, &mut st) };
            dfo.err.borrow_mut().code = st;
            t
        } else {
            // SAFETY: a live formatter.
            let t = unsafe { sys::udat_parse(df, u.as_ptr(), u.len() as i32, pp, &mut status) };
            if let Some(p) = parse_pos.as_mut() {
                *p = utf16_offset_to_utf8(&u, *p);
            }
            dfo.err.borrow_mut().code = status;
            t
        };
        if error::check_status(host, &dfo.err, &f, "Date parsing failed")? {
            assign_pos(host, &r, parse_pos)?;
            return Ok(Value::Bool(false));
        }
        let result = ts / 1000.0;
        if result > i64::MAX as f64 || result < i64::MIN as f64 {
            Value::Float(if result < 0.0 {
                result.ceil()
            } else {
                result.floor()
            })
        } else {
            Value::Int(result as i64)
        }
    };
    assign_pos(host, &r, parse_pos)?;
    Ok(result)
}

fn assign_pos(host: &mut dyn Host, r: &Option<Value>, pos: Option<i32>) -> Result<(), Thrown> {
    if let (Some(r), Some(p)) = (r, pos) {
        common::assign_ref(host, r, Value::Int(p as i64))?;
    }
    Ok(())
}

/// `datefmt_format_object`.
fn format_object(host: &mut dyn Host, call: &Call) -> Result<Value, Thrown> {
    let f = call.fname.clone();
    let obj = call.arg(0).unwrap_or(Value::Null);
    if !matches!(obj, Value::Object(_)) {
        let given = crate::zend::zpp::zval_type_name(host, &obj);
        return Err(call.type_error(0, &format!("must be of type object, {given} given")));
    }
    let format = call.arg(1);
    let locale = super::c_string(&call.opt_bytes(2).unwrap_or_else(super::locale_get_default));
    // DateFormat::EStyle
    let (k_none, k_full, k_long, k_medium, k_short, k_relative) =
        (-1i64, 0i64, 1i64, 2i64, 3i64, 128i64);
    let valid = [
        k_none,
        k_full,
        k_long,
        k_medium,
        k_short,
        k_full | k_relative,
        k_long | k_relative,
        k_medium | k_relative,
        k_short | k_relative,
    ];
    let valid_format = |v: &Value| matches!(v, Value::Int(n) if valid.contains(n));
    let k_default = k_medium;
    let (mut date_style, mut time_style) = (k_default, k_default);
    let mut pattern: Option<std::ffi::CString> = None;
    match format.as_ref() {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) => {
            if a.len() != 2 {
                error::set(
                    host,
                    &f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "bad format; if array, it must have two elements",
                )?;
                return Ok(Value::Bool(false));
            }
            for (idx, v) in a.values().enumerate() {
                let v = deref(v);
                if !valid_format(&v) {
                    let msg = if idx == 0 {
                        "bad format; the date format (first element of the array) is not valid"
                    } else {
                        "bad format; the time format (second element of the array) is not valid"
                    };
                    error::set(host, &f, sys::U_ILLEGAL_ARGUMENT_ERROR, msg)?;
                    return Ok(Value::Bool(false));
                }
                let Value::Int(n) = v else { unreachable!() };
                if idx == 0 {
                    date_style = n;
                } else {
                    time_style = n;
                }
            }
        }
        Some(v @ Value::Int(_)) => {
            if !valid_format(v) {
                error::set(
                    host,
                    &f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "the date/time format type is invalid",
                )?;
                return Ok(Value::Bool(false));
            }
            let Value::Int(n) = v else { unreachable!() };
            date_style = *n;
            time_style = *n;
        }
        Some(v) => {
            let s = crate::zend::zpp::zval_get_string(host, v)?;
            if s.is_empty() {
                error::set(
                    host,
                    &f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "the format is empty",
                )?;
                return Ok(Value::Bool(false));
            }
            pattern = Some(super::c_string(&s));
        }
    }
    if time_style != k_none {
        time_style &= !k_relative;
    }
    let (cal, tz, date);
    if timezone::is_a(host, &obj, b"IntlCalendar") {
        let c = super::calendar::cal_of(&obj);
        if c.is_null() {
            error::set(
                host,
                &f,
                sys::U_ILLEGAL_ARGUMENT_ERROR,
                "bad IntlCalendar instance: not initialized properly",
            )?;
            return Ok(Value::Bool(false));
        }
        let mut status = sys::U_ZERO_ERROR;
        // SAFETY: a live calendar.
        unsafe {
            tz = cpp::rhp_tz_clone(cpp::rhp_cal_get_tz(c));
            date = cpp::rhp_cal_get_time(c, &mut status);
            if sys::failure(status) {
                cpp::rhp_tz_delete(tz);
                error::set(
                    host,
                    &f,
                    status,
                    "error obtaining instant from IntlCalendar",
                )?;
                return Ok(Value::Bool(false));
            }
            cal = cpp::rhp_cal_clone(c);
        }
    } else if timezone::is_a(host, &obj, b"DateTimeInterface") {
        let h = object_handlers::object_of(&obj).expect("an object");
        let ms = super::dates::datetime_millis(host, &obj, None, &f)?;
        if ms.is_nan() {
            return Ok(Value::Bool(false));
        }
        let Some(z) = timezone::zone_of_datetime(host, &h, None, &f)? else {
            return Ok(Value::Bool(false));
        };
        date = ms;
        let mut status = sys::U_ZERO_ERROR;
        // SAFETY: a C locale.
        let c = unsafe { cpp::rhp_datefmt_new_calendar(0, locale.as_ptr(), &mut status) };
        if sys::failure(status) {
            error::set(host, &f, status, "could not create GregorianCalendar")?;
            return Ok(Value::Bool(false));
        }
        cal = c;
        tz = z.into_raw();
    } else {
        error::set(
            host,
            &f,
            sys::U_ZERO_ERROR,
            "the passed object must be an instance of either IntlCalendar or DateTimeInterface",
        )?;
        return Ok(Value::Bool(false));
    }
    let (mut len, mut which, mut status) = (0, 0, sys::U_ZERO_ERROR);
    // SAFETY: the shim adopts `cal` and `tz`.
    let p = unsafe {
        cpp::rhp_datefmt_format_object(
            cal,
            tz,
            date,
            pattern.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
            date_style as i32,
            time_style as i32,
            locale.as_ptr(),
            &mut len,
            &mut which,
            &mut status,
        )
    };
    match which {
        1 => {
            error::set(host, &f, status, "could not create SimpleDateFormat")?;
            return Ok(Value::Bool(false));
        }
        2 => {
            error::set(host, &f, status, "could not create DateFormat")?;
            return Ok(Value::Bool(false));
        }
        _ => {}
    }
    let u = cpp::take_ustr(p, len).unwrap_or_default();
    match super::utf16_to_utf8(&u) {
        Ok(s) => Ok(Value::str(s)),
        Err(st) => {
            error::set(host, &f, st, "error converting result to UTF-8")?;
            Ok(Value::Bool(false))
        }
    }
}

// --- IntlDatePatternGenerator ----------------------------------------------------

/// `IntlDatePatternGenerator_object`.
#[derive(Default)]
pub(super) struct DtpgObj {
    err: RefCell<IntlError>,
    dtpg: Cell<usize>,
}

impl DtpgObj {
    fn ptr(&self) -> *mut c_void {
        self.dtpg.get() as *mut c_void
    }
}

impl Drop for DtpgObj {
    fn drop(&mut self) {
        if !self.ptr().is_null() {
            // SAFETY: the generator this object owns.
            unsafe { sys::udatpg_close(self.ptr()) };
        }
    }
}

fn dtpg_clone(_host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = common::state::<DtpgObj>(src);
    let to = Rc::new(DtpgObj::default());
    object_handlers::set_payload(dst, to.clone());
    if from.ptr().is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot clone uninitialized IntlDatePatternGenerator",
        ));
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a live generator.
    let c = unsafe { sys::udatpg_clone(from.ptr(), &mut status) };
    if sys::failure(status) || c.is_null() {
        return Err(Thrown::error(
            "Error",
            "Failed to clone IntlDatePatternGenerator",
        ));
    }
    to.dtpg.set(c as usize);
    Ok(())
}

fn dtpg_ctor(host: &mut dyn Host, call: &Call, o: &DtpgObj) -> Result<bool, Thrown> {
    let f = call.fname.clone();
    error::error_reset();
    let locale = call.opt_bytes(0).unwrap_or_default();
    o.err.borrow_mut().reset();
    if !o.ptr().is_null() {
        error::errors_set(
            host,
            &o.err,
            &f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "Cannot call constructor twice",
        )?;
        return Ok(false);
    }
    if super::locale_too_long(host, &f, locale.len())? {
        return Ok(false);
    }
    let locale = if locale.is_empty() {
        super::locale_get_default()
    } else {
        locale
    };
    let loc = super::c_string(&locale);
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a C locale.
    let g = unsafe { sys::udatpg_open(loc.as_ptr(), &mut status) };
    o.dtpg.set(g as usize);
    o.err.borrow_mut().code = status;
    if sys::failure(status) {
        error::set(host, &f, status, "Error creating DateTimePatternGenerator")?;
        return Ok(false);
    }
    Ok(true)
}

/// `__datepatgen(method, argc, $this, ...args)`.
pub(super) fn fn_dtpg_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlDatePatternGenerator", args, &[]);
    let f = call.fname.clone();
    match call.name {
        "__construct" => {
            let this = call.this.clone().expect("$this");
            let o = common::state_of::<DtpgObj>(&this).expect("an object");
            error::in_ctor(|| dtpg_ctor(host, &call, &o))?;
            Ok(Value::Null)
        }
        "create" => {
            let Some(obj) = host.instantiate(b"IntlDatePatternGenerator", Vec::new()) else {
                return Ok(Value::Null);
            };
            let o = common::state_of::<DtpgObj>(&obj).expect("an object");
            Ok(if dtpg_ctor(host, &call, &o)? {
                obj
            } else {
                Value::Null
            })
        }
        "getBestPattern" => {
            error::error_reset();
            let this = call.this.clone().expect("$this");
            let o = common::state_of::<DtpgObj>(&this).expect("an object");
            o.err.borrow_mut().reset();
            if o.ptr().is_null() {
                return Err(Thrown::error(
                    "Error",
                    "Found unconstructed IntlDatePatternGenerator",
                ));
            }
            let check =
                |host: &mut dyn Host, st: sys::UErrorCode, msg: &str| -> Result<bool, Thrown> {
                    o.err.borrow_mut().code = st;
                    error::check_status(host, &o.err, &f, msg)
                };
            let skel = match super::utf8_to_utf16(&call.bytes(0)) {
                Ok(u) => u,
                Err(st) => {
                    check(host, st, "Skeleton is not a valid UTF-8 string")?;
                    return Ok(Value::Bool(false));
                }
            };
            let g = o.ptr();
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: a live generator, the skeleton's units.
            let cleaned = super::icu_uchars(skel.len() + 8, &mut status, |buf, cap, st| unsafe {
                sys::udatpg_getSkeleton(g, skel.as_ptr(), skel.len() as i32, buf, cap, st)
            });
            if check(host, status, "Error getting cleaned skeleton")? {
                return Ok(Value::Bool(false));
            }
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: as above.
            let best = super::icu_uchars(64, &mut status, |buf, cap, st| unsafe {
                sys::udatpg_getBestPattern(g, cleaned.as_ptr(), cleaned.len() as i32, buf, cap, st)
            });
            if check(host, status, "Error retrieving pattern")? {
                return Ok(Value::Bool(false));
            }
            match super::utf16_to_utf8(&best) {
                Ok(s) => Ok(Value::str(s)),
                Err(st) => {
                    check(host, st, "Error converting result to UTF-8")?;
                    Ok(Value::Bool(false))
                }
            }
        }
        other => unreachable!("IntlDatePatternGenerator::{other}"),
    }
}
