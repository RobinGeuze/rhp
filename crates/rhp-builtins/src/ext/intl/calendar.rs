//! `IntlCalendar`, `IntlGregorianCalendar`, `intlcal_*` and
//! `intlgregcal_*`, ported from php-src ext/intl/calendar/ onto ICU's
//! `Calendar` (through icu.cpp).

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;

use rhp_values::{ObjectHandle, PhpArray, Value};

use super::common::{self, Call};
use super::cpp;
use super::error::{self, IntlError};
use super::timezone::{self, Zone};
use super::{class_int, sys};
use crate::zend::object_handlers::{self, CloneObj, ObjectHandlers, PropPurpose};
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(clone_obj),
    get_properties_for: Some(properties_for),
    ..ObjectHandlers::STD
};

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlCalendar",
    helper: "__intlcal",
    props: "",
};

pub(super) const GREG_SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlGregorianCalendar",
    helper: "__intlgregcal",
    props: "",
};

pub(super) const ALIASES: &[(&str, &str)] = &[
    ("intlcal_create_instance", "createInstance"),
    (
        "intlcal_get_keyword_values_for_locale",
        "getKeywordValuesForLocale",
    ),
    ("intlcal_get_now", "getNow"),
    ("intlcal_get_available_locales", "getAvailableLocales"),
    ("intlcal_get", "get"),
    ("intlcal_get_time", "getTime"),
    ("intlcal_set_time", "setTime"),
    ("intlcal_add", "add"),
    ("intlcal_set_time_zone", "setTimeZone"),
    ("intlcal_after", "after"),
    ("intlcal_before", "before"),
    ("intlcal_set", "set"),
    ("intlcal_roll", "roll"),
    ("intlcal_clear", "clear"),
    ("intlcal_field_difference", "fieldDifference"),
    ("intlcal_get_actual_maximum", "getActualMaximum"),
    ("intlcal_get_actual_minimum", "getActualMinimum"),
    ("intlcal_get_day_of_week_type", "getDayOfWeekType"),
    ("intlcal_get_first_day_of_week", "getFirstDayOfWeek"),
    ("intlcal_get_greatest_minimum", "getGreatestMinimum"),
    ("intlcal_get_least_maximum", "getLeastMaximum"),
    ("intlcal_get_locale", "getLocale"),
    ("intlcal_get_maximum", "getMaximum"),
    (
        "intlcal_get_minimal_days_in_first_week",
        "getMinimalDaysInFirstWeek",
    ),
    ("intlcal_get_minimum", "getMinimum"),
    ("intlcal_get_time_zone", "getTimeZone"),
    ("intlcal_get_type", "getType"),
    ("intlcal_get_weekend_transition", "getWeekendTransition"),
    ("intlcal_in_daylight_time", "inDaylightTime"),
    ("intlcal_is_equivalent_to", "isEquivalentTo"),
    ("intlcal_is_lenient", "isLenient"),
    ("intlcal_is_set", "isSet"),
    ("intlcal_is_weekend", "isWeekend"),
    ("intlcal_set_first_day_of_week", "setFirstDayOfWeek"),
    ("intlcal_set_lenient", "setLenient"),
    (
        "intlcal_set_minimal_days_in_first_week",
        "setMinimalDaysInFirstWeek",
    ),
    ("intlcal_equals", "equals"),
    (
        "intlcal_get_repeated_wall_time_option",
        "getRepeatedWallTimeOption",
    ),
    (
        "intlcal_get_skipped_wall_time_option",
        "getSkippedWallTimeOption",
    ),
    (
        "intlcal_set_repeated_wall_time_option",
        "setRepeatedWallTimeOption",
    ),
    (
        "intlcal_set_skipped_wall_time_option",
        "setSkippedWallTimeOption",
    ),
    ("intlcal_from_date_time", "fromDateTime"),
    ("intlcal_to_date_time", "toDateTime"),
    ("intlcal_get_error_code", "getErrorCode"),
    ("intlcal_get_error_message", "getErrorMessage"),
];

pub(super) const GREG_ALIASES: &[(&str, &str)] = &[
    ("intlgregcal_create_instance", "__construct"),
    ("intlgregcal_set_gregorian_change", "setGregorianChange"),
    ("intlgregcal_get_gregorian_change", "getGregorianChange"),
    ("intlgregcal_is_leap_year", "isLeapYear"),
];

/// `Calendar_object`.
#[derive(Default)]
pub(super) struct CalObj {
    pub(super) err: RefCell<IntlError>,
    ucal: Cell<usize>,
}

impl CalObj {
    pub(super) fn ptr(&self) -> *mut c_void {
        self.ucal.get() as *mut c_void
    }
    fn set(&self, c: *mut c_void) {
        let old = self.ptr();
        self.ucal.set(c as usize);
        if !old.is_null() {
            // SAFETY: the calendar this object owned.
            unsafe { cpp::rhp_cal_delete(old) };
        }
    }
}

impl Drop for CalObj {
    fn drop(&mut self) {
        if !self.ptr().is_null() {
            // SAFETY: the calendar this object owns.
            unsafe { cpp::rhp_cal_delete(self.ptr()) };
        }
    }
}

fn clone_obj(_host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = common::state::<CalObj>(src);
    let to = Rc::new(CalObj::default());
    object_handlers::set_payload(dst, to.clone());
    if from.ptr().is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot clone uninitialized IntlCalendar",
        ));
    }
    // SAFETY: a live calendar.
    let c = unsafe { cpp::rhp_cal_clone(from.ptr()) };
    if c.is_null() {
        return Err(Thrown::error("Error", "Failed to clone IntlCalendar"));
    }
    to.ucal.set(c as usize);
    Ok(())
}

/// `calendar_object_create`: an IntlGregorianCalendar for a gregorian
/// calendar, else an IntlCalendar.
pub(super) fn object_create(host: &mut dyn Host, cal: *mut c_void) -> Result<Value, Thrown> {
    // SAFETY: a live calendar, which the new object takes.
    let class: &[u8] = if unsafe { cpp::rhp_cal_is_gregorian(cal) } != 0 {
        b"IntlGregorianCalendar"
    } else {
        b"IntlCalendar"
    };
    let obj = host
        .instantiate(class, Vec::new())
        .ok_or_else(|| Thrown::error("Error", "Class \"IntlCalendar\" not found"))?;
    common::state_of::<CalObj>(&obj)
        .expect("an object")
        .set(cal);
    Ok(obj)
}

/// The calendar of an IntlCalendar value (null when unconstructed).
pub(super) fn cal_of(v: &Value) -> *mut c_void {
    object_handlers::object_of(v)
        .and_then(|h| object_handlers::payload_of::<CalObj>(&h))
        .map_or(std::ptr::null_mut(), |c| c.ptr())
}

/// An IntlCalendar's time (`Calendar::getTime`): `None` when it is not
/// constructed.
pub(super) fn time_of(v: &Value) -> Option<Result<f64, sys::UErrorCode>> {
    let c = cal_of(v);
    if c.is_null() {
        return None;
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a live calendar.
    let t = unsafe { cpp::rhp_cal_get_time(c, &mut status) };
    Some(if sys::failure(status) {
        Err(status)
    } else {
        Ok(t)
    })
}

const DEBUG_FIELDS: &[(&str, &str)] = &[
    ("FIELD_ERA", "era"),
    ("FIELD_YEAR", "year"),
    ("FIELD_MONTH", "month"),
    ("FIELD_WEEK_OF_YEAR", "week of year"),
    ("FIELD_WEEK_OF_MONTH", "week of month"),
    ("FIELD_DAY_OF_YEAR", "day of year"),
    ("FIELD_DAY_OF_MONTH", "day of month"),
    ("FIELD_DAY_OF_WEEK", "day of week"),
    ("FIELD_DAY_OF_WEEK_IN_MONTH", "day of week in month"),
    ("FIELD_AM_PM", "AM/PM"),
    ("FIELD_HOUR", "hour"),
    ("FIELD_HOUR_OF_DAY", "hour of day"),
    ("FIELD_MINUTE", "minute"),
    ("FIELD_SECOND", "second"),
    ("FIELD_MILLISECOND", "millisecond"),
    ("FIELD_ZONE_OFFSET", "zone offset"),
    ("FIELD_DST_OFFSET", "DST offset"),
    ("FIELD_YEAR_WOY", "year for week of year"),
    ("FIELD_DOW_LOCAL", "localized day of week"),
    ("FIELD_EXTENDED_YEAR", "extended year"),
    ("FIELD_JULIAN_DAY", "julian day"),
    ("FIELD_MILLISECONDS_IN_DAY", "milliseconds in day"),
    ("FIELD_IS_LEAP_MONTH", "is leap month"),
];

/// `Calendar_get_debug_info`.
fn properties_for(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose != PropPurpose::Debug {
        return Ok(None);
    }
    let co = common::state::<CalObj>(h);
    let mut a = PhpArray::new();
    let c = co.ptr();
    if c.is_null() {
        common::put(&mut a, "valid", Value::Bool(false));
        return Ok(Some(a));
    }
    common::put(&mut a, "valid", Value::Bool(true));
    // SAFETY (below): a live calendar.
    let ty = super::c_bytes(unsafe { cpp::rhp_cal_type(c) });
    common::put(&mut a, "type", Value::str(ty));
    // The zone's own debug info, through a temporary IntlTimeZone.
    let zone = Zone::owned(unsafe { cpp::rhp_tz_clone(cpp::rhp_cal_get_tz(c)) });
    let tzobj = timezone::construct(host, zone)?;
    let tzh = object_handlers::object_of(&tzobj).expect("an object");
    let tzinfo = (timezone::HANDLERS.get_properties_for.expect("a handler"))(
        host,
        &tzh,
        PropPurpose::Debug,
    )?
    .unwrap_or_default();
    common::put(&mut a, "timeZone", Value::Array(tzinfo));
    let mut buf = [0 as std::ffi::c_char; 160];
    let mut status = sys::U_ZERO_ERROR;
    unsafe {
        cpp::rhp_cal_get_locale(
            c,
            sys::ULOC_VALID_LOCALE,
            buf.as_mut_ptr(),
            160,
            &mut status,
        )
    };
    let loc = if sys::failure(status) {
        super::u_error_name(status).into_bytes()
    } else {
        super::c_bytes(buf.as_ptr())
    };
    common::put(&mut a, "locale", Value::str(loc));
    let mut fields = PhpArray::new();
    for (cname, name) in DEBUG_FIELDS {
        let mut st = sys::U_ZERO_ERROR;
        let f = class_int("IntlCalendar", cname) as i32;
        let r = unsafe { cpp::rhp_cal_field_op(c, 0, f, &mut st) };
        let v = if sys::failure(st) {
            Value::str(super::u_error_name(st))
        } else {
            Value::Int(r as i64)
        };
        common::put(&mut fields, name, v);
    }
    common::put(&mut a, "fields", Value::Array(fields));
    Ok(Some(a))
}

fn field_count() -> i64 {
    class_int("IntlCalendar", "FIELD_FIELD_COUNT")
}

fn check_field(call: &Call, i: usize, field: i64) -> Result<(), Thrown> {
    if field < 0 || field >= field_count() {
        return Err(call.value_error(i, "must be a valid field"));
    }
    Ok(())
}

fn check_i32(call: &Call, i: usize, v: i64) -> Result<(), Thrown> {
    if v < i32::MIN as i64 || v > i32::MAX as i64 {
        return Err(call.value_error(i, &format!("must be between {} and {}", i32::MIN, i32::MAX)));
    }
    Ok(())
}

fn check_dow(call: &Call, i: usize, dow: i64) -> Result<(), Thrown> {
    let (sun, sat) = (
        class_int("IntlCalendar", "DOW_SUNDAY"),
        class_int("IntlCalendar", "DOW_SATURDAY"),
    );
    if dow < sun || dow > sat {
        return Err(call.value_error(i, "must be a valid day of the week"));
    }
    Ok(())
}

/// The locale argument, else the default.
fn locale_arg(call: &Call, i: usize) -> std::ffi::CString {
    super::c_string(&call.opt_bytes(i).unwrap_or_else(super::locale_get_default))
}

/// `__intlcal(method, argc, $this, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlCalendar", args, ALIASES);
    dispatch(host, &call)
}

/// `__intlgregcal(method, argc, $this, ...args)`.
pub(super) fn fn_greg_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if matches!(args.first(), Some(Value::Str(s)) if s.as_bytes() == b"fn:intlgregcal_create_instance")
    {
        let argc = match args[1] {
            Value::Int(n) => n as usize,
            _ => 0,
        };
        let call = Call {
            fname: "intlgregcal_create_instance".into(),
            name: "__construct",
            shifted: false,
            this: None,
            argc,
            args: &args[3..],
        };
        return greg_ctor(host, &call, None);
    }
    let call = Call::new("IntlGregorianCalendar", args, GREG_ALIASES);
    match call.name {
        "__construct" => {
            let this = call.this.clone();
            error::in_ctor(|| greg_ctor(host, &call, this))
        }
        "createFromDate" | "createFromDateTime" => greg_from_date(host, &call),
        _ => dispatch(host, &call),
    }
}

/// `set_gregorian_calendar_time_zone`: false after the global error.
fn set_greg_zone(
    host: &mut dyn Host,
    f: &str,
    cal: *mut c_void,
    status: sys::UErrorCode,
) -> Result<bool, Thrown> {
    if sys::failure(status) {
        error::set(
            host,
            f,
            status,
            "Error creating ICU GregorianCalendar from date",
        )?;
        return Ok(false);
    }
    let name = timezone::default_zone_name()?;
    let u: Vec<u16> = String::from_utf8_lossy(&name).encode_utf16().collect();
    // SAFETY: the id's units; the calendar adopts the zone.
    unsafe {
        let tz = cpp::rhp_tz_create(u.as_ptr(), u.len() as i32);
        cpp::rhp_cal_adopt_tz(cal, tz);
    }
    Ok(true)
}

/// `_php_intlgregcal_constructor_body`.
fn greg_ctor(host: &mut dyn Host, call: &Call, this: Option<Value>) -> Result<Value, Thrown> {
    let f = call.fname.clone();
    let is_ctor = this.is_some();
    error::error_reset();
    let argc = call.argc;
    if is_ctor && argc > 2 {
        host.emit_diagnostic(
            crate::E_DEPRECATED,
            b"Calling IntlGregorianCalendar::__construct() with more than 2 arguments is deprecated, \
use either IntlGregorianCalendar::createFromDate() or IntlGregorianCalendar::createFromDateTime() instead",
        )?;
    }
    if argc > 6 {
        return Err(Thrown::error("ArgumentCountError", "Too many arguments"));
    }
    let mut variant = argc;
    while variant > 0 && matches!(call.arg(variant - 1), Some(Value::Null)) {
        variant -= 1;
    }
    if variant == 4 {
        return Err(Thrown::error(
            "ArgumentCountError",
            "No variant with 4 arguments (excluding trailing NULLs)",
        ));
    }
    let mut largs = [0i64; 6];
    if variant <= 2 {
        // Z_PARAM_OBJ_OR_STR_OR_NULL, Z_PARAM_STRING_OR_NULL
        match call.arg(0) {
            None | Some(Value::Null) | Some(Value::Object(_)) | Some(Value::Str(_)) => {}
            Some(v @ (Value::Int(_) | Value::Float(_) | Value::Bool(_)))
                if !host.caller_strict_types() =>
            {
                let _ = v;
            }
            Some(v) => return Err(Thrown::type_error(format!(
                "{f}(): Argument #1 ($timezoneOrYear) must be of type object|string|null, {} given",
                crate::zend::zpp::zval_value_name(host, &v)
            ))),
        }
        if let Some(v) = call.arg(1) {
            if !matches!(
                v,
                Value::Null | Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_)
            ) {
                return Err(Thrown::type_error(format!(
                    "{f}(): Argument #2 ($localeOrMonth) must be of type ?string, {} given",
                    crate::zend::zpp::zval_value_name(host, &v)
                )));
            }
        }
    } else {
        let names = [
            "timezoneOrYear",
            "localeOrMonth",
            "day",
            "hour",
            "minute",
            "second",
        ];
        for (i, slot) in largs.iter_mut().enumerate().take(argc) {
            let v = call.arg(i).unwrap_or(Value::Null);
            *slot = crate::zend::zpp::long_value(host, &v, i + 1, &f, names[i], "int")?;
        }
    }
    let co = this
        .as_ref()
        .map(|t| common::state_of::<CalObj>(t).expect("an object"));
    if co.as_ref().is_some_and(|c| !c.ptr().is_null()) {
        return Err(Thrown::error(
            "Error",
            "IntlGregorianCalendar object is already constructed",
        ));
    }
    let gcal;
    if variant <= 2 {
        let tzarg = call.arg(0);
        let tzarg = match &tzarg {
            Some(Value::Null) | None => None,
            Some(v @ (Value::Int(_) | Value::Float(_) | Value::Bool(_))) => {
                Some(Value::str(rhp_values::conv::to_string_bytes(v)))
            }
            Some(v) => Some(v.clone()),
        };
        let tz = match timezone::process_argument(host, tzarg.as_ref(), None, &f) {
            Ok(Some(tz)) => tz,
            Ok(None) => {
                if !host.has_pending_throw() {
                    return Err(Thrown::error("IntlException", "Constructor failed"));
                }
                return Ok(Value::Null);
            }
            Err(e) => return Err(e),
        };
        let locale = match call.arg(1) {
            Some(Value::Null) | None => super::locale_get_default(),
            Some(v) => rhp_values::conv::to_string_bytes(&v),
        };
        let loc = super::c_string(&locale);
        let mut status = sys::U_ZERO_ERROR;
        // SAFETY: the calendar adopts the zone.
        let c = unsafe { cpp::rhp_gcal_create(tz.into_raw(), loc.as_ptr(), &mut status) };
        if sys::failure(status) {
            error::set(
                host,
                &f,
                status,
                "error creating ICU GregorianCalendar from time zone and locale",
            )?;
            return Ok(Value::Null);
        }
        gcal = c;
    } else {
        for (i, v) in largs.iter().enumerate().take(variant) {
            if *v < i32::MIN as i64 || *v > i32::MAX as i64 {
                let names = [
                    "timezoneOrYear",
                    "localeOrMonth",
                    "day",
                    "hour",
                    "minute",
                    "second",
                ];
                return Err(Thrown::error(
                    "ValueError",
                    format!(
                        "{f}(): Argument #{} (${}) must be between {} and {}",
                        i + 1,
                        names[i],
                        i32::MIN,
                        i32::MAX
                    ),
                ));
            }
        }
        let a: Vec<i32> = largs.iter().map(|&v| v as i32).collect();
        let mut status = sys::U_ZERO_ERROR;
        // SAFETY: `variant` fields.
        let c = unsafe { cpp::rhp_gcal_create_fields(variant as i32, a.as_ptr(), &mut status) };
        if !set_greg_zone(host, &f, c, status)? {
            // SAFETY: the calendar made above.
            unsafe { cpp::rhp_cal_delete(c) };
            return Ok(Value::Null);
        }
        gcal = c;
    }
    match co {
        Some(co) => {
            co.set(gcal);
            Ok(Value::Null)
        }
        None => {
            let obj = host
                .instantiate(b"IntlGregorianCalendar", Vec::new())
                .ok_or_else(|| Thrown::error("Error", "no IntlGregorianCalendar"))?;
            common::state_of::<CalObj>(&obj)
                .expect("an object")
                .set(gcal);
            Ok(obj)
        }
    }
}

/// `IntlGregorianCalendar::createFromDate` / `createFromDateTime`.
fn greg_from_date(host: &mut dyn Host, call: &Call) -> Result<Value, Thrown> {
    error::error_reset();
    let f = call.fname.clone();
    let n = if call.name == "createFromDate" { 3 } else { 5 };
    let mut a = Vec::new();
    for i in 0..n {
        let v = call.long(i, 0);
        check_i32(call, i, v)?;
        a.push(v as i32);
    }
    if n == 5 {
        if let Some(s) = call.opt_long(5) {
            check_i32(call, 5, s)?;
            a.push(s as i32);
        }
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: `a.len()` fields.
    let c = unsafe { cpp::rhp_gcal_create_fields(a.len() as i32, a.as_ptr(), &mut status) };
    if !set_greg_zone(host, &f, c, status)? {
        // SAFETY: the calendar made above.
        unsafe { cpp::rhp_cal_delete(c) };
        return Ok(Value::Null);
    }
    let obj = host
        .instantiate(b"IntlGregorianCalendar", Vec::new())
        .ok_or_else(|| Thrown::error("Error", "no IntlGregorianCalendar"))?;
    common::state_of::<CalObj>(&obj).expect("an object").set(c);
    Ok(obj)
}

fn dispatch(host: &mut dyn Host, call: &Call) -> Result<Value, Thrown> {
    let f = call.fname.clone();
    match call.name {
        "__construct" => {
            return Err(Thrown::error(
                "Exception",
                "An object of this type cannot be created with the new operator",
            ))
        }
        "createInstance" => {
            error::error_reset();
            let tz = match timezone::process_argument(host, call.arg(0).as_ref(), None, &f)? {
                Some(tz) => tz,
                None => return Ok(Value::Null),
            };
            let loc = locale_arg(call, 1);
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: the calendar adopts the zone (also on failure).
            let c = unsafe { cpp::rhp_cal_create(tz.into_raw(), loc.as_ptr(), &mut status) };
            if c.is_null() {
                error::set(host, &f, status, "Error creating ICU Calendar object")?;
                return Ok(Value::Null);
            }
            return object_create(host, c);
        }
        "getKeywordValuesForLocale" => {
            error::error_reset();
            let key = super::c_string(&call.bytes(0));
            let loc = super::c_string(&call.bytes(1));
            let common = call.bool(2, false);
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: C strings.
            let en = unsafe {
                cpp::rhp_cal_keyword_values(key.as_ptr(), loc.as_ptr(), common as i32, &mut status)
            };
            if en.is_null() {
                error::set(host, &f, status, "error calling underlying method")?;
                return Ok(Value::Bool(false));
            }
            return super::iterator::from_uenum(host, en);
        }
        "getNow" => {
            error::error_reset();
            // SAFETY: plain ICU call.
            return Ok(Value::Float(unsafe { cpp::rhp_cal_now() }));
        }
        "getAvailableLocales" => {
            error::error_reset();
            let mut out = PhpArray::new();
            // SAFETY: indexes below the count.
            unsafe {
                for i in 0..cpp::rhp_cal_available_count() {
                    out.push(Value::str(super::c_bytes(cpp::rhp_cal_available_locale(i))));
                }
            }
            return Ok(Value::Array(out));
        }
        "fromDateTime" => return from_date_time(host, call),
        _ => {}
    }
    error::error_reset();
    let this = call.this.clone().expect("an object");
    let co = common::state_of::<CalObj>(&this).expect("an object");
    match call.name {
        "getErrorCode" => return Ok(Value::Int(co.err.borrow().code as i64)),
        "getErrorMessage" => return Ok(Value::str(co.err.borrow().message().into_bytes())),
        _ => {}
    }
    // Checks php makes before fetching the object.
    match call.name {
        "get" | "getActualMaximum" | "getActualMinimum" | "getGreatestMinimum"
        | "getLeastMaximum" | "getMaximum" | "getMinimum" | "isSet" => {
            check_field(call, 0, call.long(0, 0))?
        }
        "add" => {
            check_field(call, 0, call.long(0, 0))?;
            check_i32(call, 1, call.long(1, 0))?;
        }
        "fieldDifference" => check_field(call, 1, call.long(1, 0))?,
        "getDayOfWeekType" | "getWeekendTransition" | "setFirstDayOfWeek" => {
            check_dow(call, 0, call.long(0, 0))?
        }
        "getLocale" => {
            let t = call.long(0, 0);
            if t != sys::ULOC_ACTUAL_LOCALE as i64 && t != sys::ULOC_VALID_LOCALE as i64 {
                return Err(call.value_error(
                    0,
                    "must be either Locale::ACTUAL_LOCALE or Locale::VALID_LOCALE",
                ));
            }
        }
        "setMinimalDaysInFirstWeek" => {
            let n = call.long(0, 0);
            if !(1..=7).contains(&n) {
                return Err(call.value_error(0, "must be between 1 and 7"));
            }
        }
        "setRepeatedWallTimeOption" => {
            let o = call.long(0, 0);
            if o != class_int("IntlCalendar", "WALLTIME_FIRST")
                && o != class_int("IntlCalendar", "WALLTIME_LAST")
            {
                return Err(call.value_error(
                    0,
                    "must be either IntlCalendar::WALLTIME_FIRST or IntlCalendar::WALLTIME_LAST",
                ));
            }
        }
        "setSkippedWallTimeOption" => {
            let o = call.long(0, 0);
            if !["WALLTIME_FIRST", "WALLTIME_LAST", "WALLTIME_NEXT_VALID"]
                .iter()
                .any(|n| class_int("IntlCalendar", n) == o)
            {
                return Err(call.value_error(
                    0,
                    "must be one of IntlCalendar::WALLTIME_FIRST, IntlCalendar::WALLTIME_LAST, or IntlCalendar::WALLTIME_NEXT_VALID",
                ));
            }
        }
        "isLeapYear" => check_i32(call, 0, call.long(0, 0))?,
        "set" => {
            let n = call.argc;
            if call.is_method() && n > 2 {
                host.emit_diagnostic(
                    crate::E_DEPRECATED,
                    b"Calling IntlCalendar::set() with more than 2 arguments is deprecated, use either IntlCalendar::setDate() or IntlCalendar::setDateTime() instead",
                )?;
            }
            for i in 0..n {
                check_i32(call, i, call.long(i, 0))?;
            }
        }
        "setDate" => {
            for i in 0..3 {
                check_i32(call, i, call.long(i, 0))?;
            }
        }
        "setDateTime" => {
            for i in 0..5 {
                check_i32(call, i, call.long(i, 0))?;
            }
        }
        "isEquivalentTo" => {
            let o = call.arg(0).unwrap_or(Value::Null);
            if cal_of(&o).is_null() {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "{f}(): Argument #{} ($other) is uninitialized",
                        call.argno(0)
                    ),
                ));
            }
        }
        _ => {}
    }
    co.err.borrow_mut().reset();
    let c = co.ptr();
    if c.is_null() {
        return Err(Thrown::error("Error", "Found unconstructed IntlCalendar"));
    }
    let check = |host: &mut dyn Host, status: sys::UErrorCode, msg: &str| -> Result<bool, Thrown> {
        co.err.borrow_mut().code = status;
        error::check_status(host, &co.err, &f, msg)
    };
    // SAFETY (all below): `c` is the object's live calendar.
    let field_op = |host: &mut dyn Host, op: i32, uec: bool| -> Result<Value, Thrown> {
        let mut status = sys::U_ZERO_ERROR;
        let r = unsafe { cpp::rhp_cal_field_op(c, op, call.long(0, 0) as i32, &mut status) };
        let _ = uec;
        if check(host, status, "Call to ICU method has failed")? {
            return Ok(Value::Bool(false));
        }
        Ok(Value::Int(r as i64))
    };
    match call.name {
        "get" => field_op(host, 0, true),
        "getActualMaximum" => field_op(host, 1, true),
        "getActualMinimum" => field_op(host, 2, true),
        "getGreatestMinimum" => field_op(host, 3, false),
        "getLeastMaximum" => field_op(host, 4, false),
        "getMaximum" => field_op(host, 5, false),
        "getMinimum" => field_op(host, 6, false),
        "getTime" => {
            let mut status = sys::U_ZERO_ERROR;
            let t = unsafe { cpp::rhp_cal_get_time(c, &mut status) };
            if check(host, status, "error calling ICU Calendar::getTime")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Float(t))
        }
        "setTime" => {
            let mut status = sys::U_ZERO_ERROR;
            unsafe { cpp::rhp_cal_set_time(c, call.double(0, 0.0), &mut status) };
            if check(host, status, "Call to underlying method failed")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(true))
        }
        "add" => {
            let mut status = sys::U_ZERO_ERROR;
            unsafe {
                cpp::rhp_cal_add(
                    c,
                    call.long(0, 0) as i32,
                    call.long(1, 0) as i32,
                    &mut status,
                )
            };
            if check(host, status, "Call to underlying method failed")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(true))
        }
        "setTimeZone" => {
            let arg = call.arg(0);
            if matches!(arg, None | Some(Value::Null)) {
                return Ok(Value::Bool(true));
            }
            match timezone::process_argument(host, arg.as_ref(), Some(&co.err), &f)? {
                Some(tz) => {
                    unsafe { cpp::rhp_cal_adopt_tz(c, tz.into_raw()) };
                    Ok(Value::Bool(true))
                }
                None => Ok(Value::Bool(false)),
            }
        }
        "after" | "before" | "equals" => {
            let o = call.arg(0).unwrap_or(Value::Null);
            let oc = cal_of(&o);
            if oc.is_null() {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "{f}(): Argument #{} ($other) is uninitialized",
                        call.argno(0)
                    ),
                ));
            }
            let op = match call.name {
                "after" => 0,
                "before" => 1,
                _ => 2,
            };
            let mut status = sys::U_ZERO_ERROR;
            let r = unsafe { cpp::rhp_cal_cmp(c, oc, op, &mut status) };
            let msg = if op == 2 {
                "error calling ICU Calendar::equals"
            } else {
                "Error calling ICU method"
            };
            if check(host, status, msg)? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(r != 0))
        }
        "set" => {
            let n = call.argc;
            let a: Vec<i32> = (0..6).map(|i| call.long(i, 0) as i32).collect();
            match n {
                2 => {
                    check_field(call, 0, call.long(0, 0))?;
                }
                4 => {
                    return Err(Thrown::error(
                        "ArgumentCountError",
                        "IntlCalendar::set() has no variant with exactly 4 parameters",
                    ))
                }
                _ => {}
            }
            unsafe { cpp::rhp_cal_set(c, n as i32, a.as_ptr()) };
            Ok(Value::Bool(true))
        }
        "setDate" => {
            let a: Vec<i32> = (0..3).map(|i| call.long(i, 0) as i32).collect();
            unsafe { cpp::rhp_cal_set(c, 3, a.as_ptr()) };
            Ok(Value::Null)
        }
        "setDateTime" => {
            let mut a: Vec<i32> = (0..5).map(|i| call.long(i, 0) as i32).collect();
            if let Some(s) = call.opt_long(5) {
                check_i32(call, 5, s)?;
                a.push(s as i32);
            }
            unsafe { cpp::rhp_cal_set(c, a.len() as i32, a.as_ptr()) };
            Ok(Value::Null)
        }
        "roll" => {
            let field = call.long(0, 0);
            check_field(call, 0, field)?;
            let value = match call.arg(1) {
                Some(Value::Bool(b)) => {
                    host.emit_diagnostic(
                        crate::E_DEPRECATED,
                        format!("{f}(): Passing bool is deprecated, use 1 or -1 instead")
                            .as_bytes(),
                    )?;
                    if b {
                        1
                    } else {
                        -1
                    }
                }
                Some(v) => {
                    let n = crate::zend::zpp::zval_get_long(host, &v)?;
                    check_i32(call, 1, n)?;
                    n
                }
                None => 0,
            };
            let mut status = sys::U_ZERO_ERROR;
            unsafe { cpp::rhp_cal_roll(c, field as i32, value as i32, &mut status) };
            if check(host, status, "Error calling ICU Calendar::roll")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(true))
        }
        "clear" => {
            match call.opt_long(0) {
                None => unsafe { cpp::rhp_cal_clear(c, -1) },
                Some(field) => {
                    check_field(call, 0, field)?;
                    unsafe { cpp::rhp_cal_clear(c, field as i32) }
                }
            }
            Ok(Value::Bool(true))
        }
        "fieldDifference" => {
            let mut status = sys::U_ZERO_ERROR;
            let r = unsafe {
                cpp::rhp_cal_field_difference(
                    c,
                    call.double(0, 0.0),
                    call.long(1, 0) as i32,
                    &mut status,
                )
            };
            if check(host, status, "Call to ICU method has failed")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Int(r as i64))
        }
        "getDayOfWeekType" | "getWeekendTransition" => {
            let mut status = sys::U_ZERO_ERROR;
            let dow = call.long(0, 0) as i32;
            let (r, msg) = if call.name == "getDayOfWeekType" {
                (
                    unsafe { cpp::rhp_cal_day_of_week_type(c, dow, &mut status) },
                    "Call to ICU method has failed",
                )
            } else {
                (
                    unsafe { cpp::rhp_cal_weekend_transition(c, dow, &mut status) },
                    "Error calling ICU method",
                )
            };
            if check(host, status, msg)? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Int(r as i64))
        }
        "getFirstDayOfWeek" => {
            let mut status = sys::U_ZERO_ERROR;
            let r = unsafe { cpp::rhp_cal_first_day_of_week(c, &mut status) };
            if check(host, status, "Call to ICU method has failed")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Int(r as i64))
        }
        "getLocale" => {
            let mut buf = [0 as std::ffi::c_char; 160];
            let mut status = sys::U_ZERO_ERROR;
            unsafe {
                cpp::rhp_cal_get_locale(
                    c,
                    call.long(0, 0) as i32,
                    buf.as_mut_ptr(),
                    160,
                    &mut status,
                )
            };
            if check(host, status, "Call to ICU method has failed")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::str(super::c_bytes(buf.as_ptr())))
        }
        "getMinimalDaysInFirstWeek" => {
            Ok(Value::Int(unsafe { cpp::rhp_cal_minimal_days(c) } as i64))
        }
        "getTimeZone" => {
            let tz = Zone::owned(unsafe { cpp::rhp_tz_clone(cpp::rhp_cal_get_tz(c)) });
            timezone::construct(host, tz)
        }
        "getType" => Ok(Value::str(super::c_bytes(unsafe { cpp::rhp_cal_type(c) }))),
        "inDaylightTime" => {
            let mut status = sys::U_ZERO_ERROR;
            let r = unsafe { cpp::rhp_cal_in_daylight(c, &mut status) };
            if check(host, status, "Error calling ICU method")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(r != 0))
        }
        "isEquivalentTo" => {
            let o = call.arg(0).unwrap_or(Value::Null);
            Ok(Value::Bool(
                unsafe { cpp::rhp_cal_is_equivalent(c, cal_of(&o)) } != 0,
            ))
        }
        "isLenient" => Ok(Value::Bool(unsafe { cpp::rhp_cal_is_lenient(c) } != 0)),
        "isSet" => Ok(Value::Bool(
            unsafe { cpp::rhp_cal_is_set(c, call.long(0, 0) as i32) } != 0,
        )),
        "isWeekend" => match call.arg(0) {
            None | Some(Value::Null) => Ok(Value::Bool(unsafe { cpp::rhp_cal_is_weekend(c) } != 0)),
            Some(_) => {
                let mut status = sys::U_ZERO_ERROR;
                let r = unsafe { cpp::rhp_cal_is_weekend_at(c, call.double(0, 0.0), &mut status) };
                if check(host, status, "Error calling ICU method")? {
                    return Ok(Value::Bool(false));
                }
                Ok(Value::Bool(r != 0))
            }
        },
        "setFirstDayOfWeek" => {
            unsafe { cpp::rhp_cal_set_first_day_of_week(c, call.long(0, 0) as i32) };
            Ok(Value::Bool(true))
        }
        "setLenient" => {
            unsafe { cpp::rhp_cal_set_lenient(c, call.bool(0, false) as i32) };
            Ok(Value::Bool(true))
        }
        "setMinimalDaysInFirstWeek" => {
            unsafe { cpp::rhp_cal_set_minimal_days(c, call.long(0, 0) as i32) };
            Ok(Value::Bool(true))
        }
        "getRepeatedWallTimeOption" => {
            Ok(Value::Int(unsafe { cpp::rhp_cal_wall_time(c, 0) } as i64))
        }
        "getSkippedWallTimeOption" => {
            Ok(Value::Int(unsafe { cpp::rhp_cal_wall_time(c, 1) } as i64))
        }
        "setRepeatedWallTimeOption" => {
            unsafe { cpp::rhp_cal_set_wall_time(c, 0, call.long(0, 0) as i32) };
            Ok(Value::Bool(true))
        }
        "setSkippedWallTimeOption" => {
            unsafe { cpp::rhp_cal_set_wall_time(c, 1, call.long(0, 0) as i32) };
            Ok(Value::Bool(true))
        }
        "toDateTime" => to_date_time(host, call, &co),
        "setGregorianChange" => {
            let mut status = sys::U_ZERO_ERROR;
            unsafe { cpp::rhp_gcal_set_change(c, call.double(0, 0.0), &mut status) };
            if check(host, status, "error calling ICU method")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(true))
        }
        "getGregorianChange" => Ok(Value::Float(unsafe { cpp::rhp_gcal_get_change(c) })),
        "isLeapYear" => Ok(Value::Bool(
            unsafe { cpp::rhp_gcal_is_leap(c, call.long(0, 0) as i32) } != 0,
        )),
        other => unreachable!("IntlCalendar::{other}"),
    }
}

/// `intlcal_from_date_time`.
fn from_date_time(host: &mut dyn Host, call: &Call) -> Result<Value, Thrown> {
    error::error_reset();
    let f = call.fname.clone();
    let date = match call.arg(0) {
        Some(Value::Str(s)) => host.reflect_new_instance(b"DateTime", vec![Value::Str(s)], true)?,
        Some(v) => v,
        None => Value::Null,
    };
    let h = object_handlers::object_of(&date).expect("a DateTime");
    let Some(t) = crate::ext::date::objects::time_of(&h) else {
        error::set(
            host,
            &f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "DateTime object is unconstructed",
        )?;
        return Ok(Value::Null);
    };
    let ts = host.call_callable(
        &Value::Array({
            let mut a = PhpArray::new();
            a.push(date.clone());
            a.push(Value::str("getTimestamp"));
            a
        }),
        &[],
    )?;
    let Value::Int(ts) = crate::zend::zpp::deref(&ts) else {
        error::set(
            host,
            &f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "bad DateTime; call to DateTime::getTimestamp() failed",
        )?;
        return Ok(Value::Null);
    };
    let tz = if t.is_localtime == 0 {
        // SAFETY: ICU's static GMT zone.
        Zone::owned(unsafe { cpp::rhp_tz_clone(cpp::rhp_tz_gmt()) })
    } else {
        let kind = crate::ext::date::objects::TzKind::from_time(&t, t.zone());
        match kind {
            Some(k) => match timezone::convert_datetimezone(host, &k, None, &f)? {
                Some(z) => z,
                None => return Ok(Value::Null),
            },
            None => return Ok(Value::Null),
        }
    };
    let loc = locale_arg(call, 1);
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: the calendar adopts the zone.
    let c = unsafe { cpp::rhp_cal_create(tz.into_raw(), loc.as_ptr(), &mut status) };
    if c.is_null() {
        error::set(host, &f, status, "error creating ICU Calendar object")?;
        return Ok(Value::Null);
    }
    // SAFETY: the new calendar.
    unsafe { cpp::rhp_cal_set_time(c, ts as f64 * 1000.0, &mut status) };
    if sys::failure(status) {
        // SAFETY: the new calendar.
        unsafe { cpp::rhp_cal_delete(c) };
        error::set(host, &f, status, "error creating ICU Calendar::setTime()")?;
        return Ok(Value::Null);
    }
    object_create(host, c)
}

/// `intlcal_to_date_time`.
fn to_date_time(host: &mut dyn Host, call: &Call, co: &CalObj) -> Result<Value, Thrown> {
    let f = call.fname.clone();
    let c = co.ptr();
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a live calendar.
    let date = unsafe { cpp::rhp_cal_get_time(c, &mut status) } / 1000.0;
    co.err.borrow_mut().code = status;
    if error::check_status(host, &co.err, &f, "Call to ICU method has failed")? {
        return Ok(Value::Bool(false));
    }
    if date > i64::MAX as f64 || date < i64::MIN as f64 {
        error::errors_set(
            host,
            &co.err,
            &f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "The calendar date is out of the range for a 64-bit integer",
        )?;
        return Ok(Value::Bool(false));
    }
    let ts = format!("@{}", date as i64);
    // SAFETY: the calendar's zone, cloned.
    let zone = Zone::owned(unsafe { cpp::rhp_tz_clone(cpp::rhp_cal_get_tz(c)) });
    let Some(tzv) = timezone::to_datetimezone(host, &zone, &co.err, &f)? else {
        return Ok(Value::Bool(false));
    };
    let dt = host.reflect_new_instance(b"DateTime", vec![Value::str(ts), tzv.clone()], true)?;
    let r = host.call_callable(
        &Value::Array({
            let mut a = PhpArray::new();
            a.push(dt.clone());
            a.push(Value::str("setTimezone"));
            a
        }),
        &[tzv],
    )?;
    if matches!(r, Value::Bool(false)) {
        error::errors_set(
            host,
            &co.err,
            &f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "call to DateTime::setTimeZone has failed",
        )?;
        return Ok(Value::Bool(false));
    }
    Ok(dt)
}
