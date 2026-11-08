//! `IntlTimeZone` and `intltz_*`, ported from php-src ext/intl/timezone/
//! onto ICU's `TimeZone` (through icu.cpp), and the time zone conversions
//! the date classes share (timezone_class.cpp, common_date.cpp).

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;

use rhp_values::{ObjectHandle, PhpArray, Value};

use super::common::{self, Call};
use super::cpp;
use super::error::{self, IntlError};
use super::{class_int, sys};
use crate::ext::date::objects::{self as dates, TzKind};
use crate::zend::object_handlers::{self, CloneObj, ObjectHandlers, PropPurpose};
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(clone_obj),
    compare: Some(compare),
    get_properties_for: Some(properties_for),
    ..ObjectHandlers::STD
};

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlTimeZone",
    helper: "__intltz",
    props: "",
};

pub(super) const ALIASES: &[(&str, &str)] = &[
    ("intltz_count_equivalent_ids", "countEquivalentIDs"),
    ("intltz_create_default", "createDefault"),
    ("intltz_create_enumeration", "createEnumeration"),
    ("intltz_create_time_zone", "createTimeZone"),
    (
        "intltz_create_time_zone_id_enumeration",
        "createTimeZoneIDEnumeration",
    ),
    ("intltz_from_date_time_zone", "fromDateTimeZone"),
    ("intltz_get_canonical_id", "getCanonicalID"),
    ("intltz_get_display_name", "getDisplayName"),
    ("intltz_get_dst_savings", "getDSTSavings"),
    ("intltz_get_equivalent_id", "getEquivalentID"),
    ("intltz_get_error_code", "getErrorCode"),
    ("intltz_get_error_message", "getErrorMessage"),
    ("intltz_get_gmt", "getGMT"),
    ("intltz_get_iana_id", "getIanaID"),
    ("intltz_get_id", "getID"),
    ("intltz_get_offset", "getOffset"),
    ("intltz_get_raw_offset", "getRawOffset"),
    ("intltz_get_region", "getRegion"),
    ("intltz_get_tz_data_version", "getTZDataVersion"),
    ("intltz_get_unknown", "getUnknown"),
    ("intltz_get_windows_id", "getWindowsID"),
    ("intltz_get_id_for_windows_id", "getIDForWindowsID"),
    ("intltz_has_same_rules", "hasSameRules"),
    ("intltz_to_date_time_zone", "toDateTimeZone"),
    ("intltz_use_daylight_time", "useDaylightTime"),
];

/// An owned ICU `TimeZone *` (deleted on drop), or a borrowed one.
pub(super) struct Zone {
    ptr: *mut c_void,
    owned: bool,
}

impl Zone {
    pub(super) fn owned(ptr: *mut c_void) -> Zone {
        Zone { ptr, owned: true }
    }
    pub(super) fn as_ptr(&self) -> *mut c_void {
        self.ptr
    }
    /// Gives up ownership (ICU adopts it).
    pub(super) fn into_raw(mut self) -> *mut c_void {
        self.owned = false;
        self.ptr
    }
    /// An owned copy.
    pub(super) fn clone_zone(&self) -> Zone {
        // SAFETY: a live zone.
        Zone::owned(unsafe { cpp::rhp_tz_clone(self.ptr) })
    }
    /// `TimeZone::getID`, in UTF-16.
    pub(super) fn id16(&self) -> Option<Vec<u16>> {
        let mut len = 0;
        // SAFETY: a live zone.
        let p = unsafe { cpp::rhp_tz_get_id(self.ptr, &mut len) };
        cpp::take_ustr(p, len)
    }
}

impl Drop for Zone {
    fn drop(&mut self) {
        if self.owned && !self.ptr.is_null() {
            // SAFETY: the zone this owns.
            unsafe { cpp::rhp_tz_delete(self.ptr) };
        }
    }
}

/// `TimeZone_object`.
#[derive(Default)]
pub(super) struct TzObj {
    pub(super) err: RefCell<IntlError>,
    zone: RefCell<Option<Zone>>,
    _set: Cell<bool>,
}

impl TzObj {
    pub(super) fn ptr(&self) -> *mut c_void {
        self.zone
            .borrow()
            .as_ref()
            .map_or(std::ptr::null_mut(), |z| z.ptr)
    }
}

/// `timezone_object_construct`.
pub(super) fn construct(host: &mut dyn Host, zone: Zone) -> Result<Value, Thrown> {
    let obj = host
        .instantiate(b"IntlTimeZone", Vec::new())
        .ok_or_else(|| Thrown::error("Error", "Class \"IntlTimeZone\" not found"))?;
    let to = common::state_of::<TzObj>(&obj).expect("an object");
    *to.zone.borrow_mut() = Some(zone);
    Ok(obj)
}

fn clone_obj(_host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = common::state::<TzObj>(src);
    let to = Rc::new(TzObj::default());
    object_handlers::set_payload(dst, to.clone());
    if from.ptr().is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot clone uninitialized IntlTimeZone",
        ));
    }
    let z = from.zone.borrow().as_ref().map(Zone::clone_zone);
    *to.zone.borrow_mut() = z;
    Ok(())
}

/// `TimeZone_compare_objects`: equal or uncomparable.
fn compare(_host: &mut dyn Host, a: &Value, b: &Value) -> Result<Option<i32>, Thrown> {
    let (Some(x), Some(y)) = (
        object_handlers::object_of(a).and_then(|h| object_handlers::payload_of::<TzObj>(&h)),
        object_handlers::object_of(b).and_then(|h| object_handlers::payload_of::<TzObj>(&h)),
    ) else {
        return Ok(Some(1));
    };
    if x.ptr().is_null() || y.ptr().is_null() {
        return Err(Thrown::error(
            "Exception",
            "Comparison with at least one unconstructed IntlTimeZone operand",
        ));
    }
    // SAFETY: live zones.
    Ok(Some(
        if unsafe { cpp::rhp_tz_equals(x.ptr(), y.ptr()) } != 0 {
            0
        } else {
            1
        },
    ))
}

/// `TimeZone_get_debug_info`.
fn properties_for(
    _host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose != PropPurpose::Debug {
        return Ok(None);
    }
    let to = common::state::<TzObj>(h);
    let mut a = PhpArray::new();
    let tz = to.ptr();
    if tz.is_null() {
        common::put(&mut a, "valid", Value::Bool(false));
        return Ok(Some(a));
    }
    common::put(&mut a, "valid", Value::Bool(true));
    let id = to.zone.borrow().as_ref().and_then(Zone::id16);
    let Some(id) = id.and_then(|u| super::utf16_to_utf8(&u).ok()) else {
        return Ok(Some(a));
    };
    common::put(&mut a, "id", Value::str(id));
    let (mut raw, mut dst, mut status) = (0, 0, sys::U_ZERO_ERROR);
    // SAFETY: a live zone.
    unsafe { cpp::rhp_tz_get_offset(tz, cpp::rhp_cal_now(), 0, &mut raw, &mut dst, &mut status) };
    if sys::failure(status) {
        return Ok(Some(a));
    }
    common::put(&mut a, "rawOffset", Value::Int(raw as i64));
    common::put(&mut a, "currentOffset", Value::Int((raw + dst) as i64));
    Ok(Some(a))
}

/// `timezone_throw_exception_with_call_location`.
fn throw_located(fname: &str, msg: &str, info: Option<&[u8]>) -> Thrown {
    let info = info
        .map(|i| {
            format!(
                "\"{}\"",
                String::from_utf8_lossy(super::c_string(i).as_bytes())
            )
        })
        .unwrap_or_default();
    Thrown::error("IntlException", format!("{fname}(): {msg}{info}"))
}

/// The ICU id of a php DateTimeZone (or DateTime) zone
/// (`timezone_convert_datetimezone`), or the error message.
fn zone_id_of_kind(kind: &TzKind) -> Result<Vec<u8>, String> {
    Ok(match kind {
        TzKind::Id(tz) => tz.name().to_vec(),
        TzKind::Offset(secs) => offset_id(*secs)?,
        TzKind::Abbr { abbr, .. } => abbr.clone(),
    })
}

fn offset_id(secs: i64) -> Result<Vec<u8>, String> {
    let offset_mins = (secs / 60) as i32;
    let hours = offset_mins / 60;
    let minutes = (offset_mins - hours * 60).abs();
    if offset_mins <= -24 * 60 || offset_mins >= 24 * 60 {
        return Err("object has an time zone offset that's too large".into());
    }
    Ok(format!("GMT{hours:+03}:{minutes:02}").into_bytes())
}

/// `timezone_convert_datetimezone`: `None` after setting the error
/// (object's when given, and the global one).
pub(super) fn convert_datetimezone(
    host: &mut dyn Host,
    kind: &TzKind,
    err: Option<&RefCell<IntlError>>,
    fname: &str,
) -> Result<Option<Zone>, Thrown> {
    let id = match zone_id_of_kind(kind) {
        Ok(id) => id,
        Err(m) => {
            errors_set_opt(host, err, fname, sys::U_ILLEGAL_ARGUMENT_ERROR, &m)?;
            return Ok(None);
        }
    };
    let u: Vec<u16> = id.iter().map(|&b| b as u16).collect();
    // SAFETY: the id's units.
    let z = Zone::owned(unsafe { cpp::rhp_tz_create(u.as_ptr(), u.len() as i32) });
    // SAFETY: a live zone.
    if unsafe { cpp::rhp_tz_is_unknown(z.as_ptr()) } != 0 {
        let msg = format!(
            "time zone id '{}' extracted from ext/date DateTimeZone not recognized",
            String::from_utf8_lossy(&id)
        );
        errors_set_opt(host, err, fname, sys::U_ILLEGAL_ARGUMENT_ERROR, &msg)?;
        return Ok(None);
    }
    Ok(Some(z))
}

/// `intl_errors_set(err, …)` where `err` may be NULL (the global only).
pub(super) fn errors_set_opt(
    host: &mut dyn Host,
    err: Option<&RefCell<IntlError>>,
    fname: &str,
    code: sys::UErrorCode,
    msg: &str,
) -> Result<(), Thrown> {
    match err {
        Some(e) => error::errors_set(host, e, fname, code, msg),
        None => error::set(host, fname, code, msg),
    }
}

/// The ICU zone of a php DateTime's time (`intl_datetime_decompose`'s
/// zone part): GMT for a UTC time.
pub(super) fn zone_of_datetime(
    host: &mut dyn Host,
    h: &ObjectHandle,
    err: Option<&RefCell<IntlError>>,
    fname: &str,
) -> Result<Option<Zone>, Thrown> {
    let class = String::from_utf8_lossy(&dates::class_name(host, h)).into_owned();
    let Some(t) = dates::time_of(h) else {
        let msg = format!("the {class} object is not properly initialized");
        errors_set_opt(host, err, fname, sys::U_ILLEGAL_ARGUMENT_ERROR, &msg)?;
        return Ok(None);
    };
    if t.is_localtime == 0 {
        // SAFETY: ICU's static GMT zone.
        return Ok(Some(Zone::owned(unsafe {
            cpp::rhp_tz_clone(cpp::rhp_tz_gmt())
        })));
    }
    let kind = TzKind::from_time(&t, t.zone());
    let z = match kind {
        Some(k) => convert_datetimezone(host, &k, None, fname)?,
        None => None,
    };
    if z.is_none() {
        errors_set_opt(
            host,
            err,
            fname,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "could not convert DateTime's time zone",
        )?;
    }
    Ok(z)
}

/// The default time zone's name (php's `get_timezone_info()`).
pub(super) fn default_zone_name() -> Result<Vec<u8>, Thrown> {
    Ok(crate::ext::date::timezone_info()?.name().to_vec())
}

/// `timezone_process_timezone_argument`: `arg` is `None` for php's NULL
/// (the default zone). `Ok(None)` after an exception or error.
pub(super) fn process_argument(
    host: &mut dyn Host,
    arg: Option<&Value>,
    err: Option<&RefCell<IntlError>>,
    fname: &str,
) -> Result<Option<Zone>, Thrown> {
    let id: Vec<u8> = match arg.map(crate::zend::zpp::deref) {
        None | Some(Value::Null) => default_zone_name()?,
        Some(v @ Value::Object(_)) => {
            let h = object_handlers::object_of(&v).expect("an object");
            if let Some(to) = object_handlers::payload_of::<TzObj>(&h) {
                if to.ptr().is_null() {
                    return Err(throw_located(
                        fname,
                        "passed IntlTimeZone is not properly constructed",
                        None,
                    ));
                }
                return Ok(to.zone.borrow().as_ref().map(Zone::clone_zone));
            }
            if host.object_class_name(&v).is_some() && is_a(host, &v, b"IntlTimeZone") {
                return Err(throw_located(
                    fname,
                    "passed IntlTimeZone is not properly constructed",
                    None,
                ));
            }
            if is_a(host, &v, b"DateTimeZone") {
                return match dates::tz_of(&h) {
                    Some(k) => convert_datetimezone(host, &k, err, fname),
                    None => Ok(None),
                };
            }
            match crate::zend::object_handlers::to_string(host, &v)? {
                Some(s) => s,
                None => {
                    let class = host.object_class_name(&v).unwrap_or_default();
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Object of class {} could not be converted to string",
                            String::from_utf8_lossy(&class)
                        ),
                    ));
                }
            }
        }
        Some(v) => crate::zend::zpp::zval_get_string(host, &v)?,
    };
    let Ok(u) = super::utf8_to_utf16(&id) else {
        return Err(throw_located(
            fname,
            "Time zone identifier given is not a valid UTF-8 string",
            None,
        ));
    };
    // SAFETY: the id's units.
    let z = Zone::owned(unsafe { cpp::rhp_tz_create(u.as_ptr(), u.len() as i32) });
    // SAFETY: a live zone.
    if unsafe { cpp::rhp_tz_is_unknown(z.as_ptr()) } != 0 {
        return Err(throw_located(fname, "No such time zone: ", Some(&id)));
    }
    Ok(Some(z))
}

pub(super) fn is_a(host: &dyn Host, v: &Value, class: &[u8]) -> bool {
    host.object_class_name(v)
        .is_some_and(|c| host.reflect_is_a(&c, class))
}

/// `timezone_convert_to_datetimezone`: a DateTimeZone for `zone`.
pub(super) fn to_datetimezone(
    host: &mut dyn Host,
    zone: &Zone,
    err: &RefCell<IntlError>,
    fname: &str,
) -> Result<Option<Value>, Thrown> {
    let Some(id) = zone.id16() else {
        error::errors_set(
            host,
            err,
            fname,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "could not obtain TimeZone id",
        )?;
        return Ok(None);
    };
    let gmt: Vec<u16> = "GMT".encode_utf16().collect();
    if id.starts_with(&gmt) {
        let h = dates::instantiate(host, b"DateTimeZone")?;
        // SAFETY: a live zone.
        let raw = unsafe { cpp::rhp_tz_raw_offset(zone.as_ptr()) };
        dates::set_tz(&h, Some(TzKind::Offset((raw / 1000) as i64)));
        return Ok(Some(Value::Object(h)));
    }
    let mut status = sys::U_ZERO_ERROR;
    let Some(s) = common::to_utf8(&id, &mut status) else {
        error::errors_set(host, err, fname, status, "could not convert id to UTF-8")?;
        return Ok(None);
    };
    match host.reflect_new_instance(b"DateTimeZone", vec![Value::str(s)], true) {
        Ok(v) => Ok(Some(v)),
        Err(e) => Err(host.chain_thrown(
            Thrown::error("IntlException", "DateTimeZone constructor threw exception"),
            e,
        )),
    }
}

/// A zone's id in UTF-8 (empty when it cannot be converted).
pub(super) fn id_utf8(zone: &Zone) -> Vec<u8> {
    zone.id16()
        .and_then(|u| super::utf16_to_utf8(&u).ok())
        .unwrap_or_default()
}

// --- the methods ----------------------------------------------------------------

/// `__intltz(method, argc, $this, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlTimeZone", args, ALIASES);
    let f = call.fname.clone();
    error::error_reset();
    // php converts a string id with intl_stringFromChar.
    let id_arg = |host: &mut dyn Host, i: usize| -> Result<Option<Vec<u16>>, Thrown> {
        match super::utf8_to_utf16(&call.bytes(i)) {
            Ok(u) => Ok(Some(u)),
            Err(st) => {
                error::set(host, &f, st, "could not convert time zone id to UTF-16")?;
                Ok(None)
            }
        }
    };
    let ustr = |host: &mut dyn Host, u: Option<Vec<u16>>, msg: &str| -> Result<Value, Thrown> {
        let Some(u) = u else {
            return Ok(Value::Bool(false));
        };
        match super::utf16_to_utf8(&u) {
            Ok(s) => Ok(Value::str(s)),
            Err(st) => {
                error::check_global(host, st, &f, msg)?;
                Ok(Value::Bool(false))
            }
        }
    };
    match call.name {
        "__construct" => {
            return Err(Thrown::error(
                "Exception",
                "An object of this type cannot be created with the new operator",
            ))
        }
        "createTimeZone" => {
            let Some(u) = id_arg(host, 0)? else {
                return Ok(Value::Null);
            };
            // SAFETY: the id's units.
            let z = Zone::owned(unsafe { cpp::rhp_tz_create(u.as_ptr(), u.len() as i32) });
            return construct(host, z);
        }
        "fromDateTimeZone" => {
            let v = call.arg(0).unwrap_or(Value::Null);
            let h = object_handlers::object_of(&v).expect("a DateTimeZone");
            let Some(kind) = dates::tz_of(&h) else {
                return Err(Thrown::error(
                    "Error",
                    "DateTimeZone object is unconstructed",
                ));
            };
            return match convert_datetimezone(host, &kind, None, &f)? {
                Some(z) => construct(host, z),
                None => Ok(Value::Null),
            };
        }
        "createDefault" => {
            // SAFETY: ICU's default zone.
            return construct(host, Zone::owned(unsafe { cpp::rhp_tz_create_default() }));
        }
        "getGMT" => {
            // SAFETY: ICU's static zone, borrowed.
            let z = Zone {
                ptr: unsafe { cpp::rhp_tz_gmt() } as *mut c_void,
                owned: false,
            };
            return construct(host, z);
        }
        "getUnknown" => {
            // SAFETY: ICU's static zone, borrowed.
            let z = Zone {
                ptr: unsafe { cpp::rhp_tz_unknown() } as *mut c_void,
                owned: false,
            };
            return construct(host, z);
        }
        "createEnumeration" => {
            let en = match call.arg(0) {
                None | Some(Value::Null) => unsafe { cpp::rhp_tz_enum(0, std::ptr::null(), 0) },
                Some(Value::Int(n)) => {
                    if n < i32::MIN as i64 || n > i32::MAX as i64 {
                        return Err(call.value_error(
                            0,
                            &format!("must be between {} and {}", i32::MIN, i32::MAX),
                        ));
                    }
                    // SAFETY: plain ICU call.
                    unsafe { cpp::rhp_tz_enum(2, std::ptr::null(), n as i32) }
                }
                Some(v) => {
                    let s = super::c_string(&crate::zend::zpp::zval_get_string(host, &v)?);
                    // SAFETY: a C string.
                    unsafe { cpp::rhp_tz_enum(1, s.as_ptr(), 0) }
                }
            };
            if en.is_null() {
                error::set(
                    host,
                    &f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "error obtaining enumeration",
                )?;
                return Ok(Value::Bool(false));
            }
            return super::iterator::from_uenum(host, en);
        }
        "countEquivalentIDs" => {
            let Some(u) = id_arg(host, 0)? else {
                return Ok(Value::Bool(false));
            };
            // SAFETY: the id's units.
            return Ok(Value::Int(
                unsafe { cpp::rhp_tz_count_equivalent(u.as_ptr(), u.len() as i32) } as i64,
            ));
        }
        "createTimeZoneIDEnumeration" => {
            let ty = call.long(0, 0);
            let ok = ["TYPE_ANY", "TYPE_CANONICAL", "TYPE_CANONICAL_LOCATION"]
                .iter()
                .any(|n| class_int("IntlTimeZone", n) == ty);
            if !ok {
                return Err(call.value_error(
                    0,
                    "must be one of IntlTimeZone::TYPE_ANY, IntlTimeZone::TYPE_CANONICAL, or IntlTimeZone::TYPE_CANONICAL_LOCATION",
                ));
            }
            let region = call.opt_bytes(1).map(|r| super::c_string(&r));
            let offset = match call.arg(2) {
                Some(Value::Int(n)) => {
                    if n < i32::MIN as i64 || n > i32::MAX as i64 {
                        return Err(call.value_error(
                            2,
                            &format!("must be between {} and {}", i32::MIN, i32::MAX),
                        ));
                    }
                    Some(n as i32)
                }
                _ => None,
            };
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: optional C string and offset.
            let en = unsafe {
                cpp::rhp_tz_enum_ids(
                    ty as i32,
                    region.as_ref().map_or(std::ptr::null(), |r| r.as_ptr()),
                    offset
                        .as_ref()
                        .map_or(std::ptr::null(), |o| o as *const i32),
                    &mut status,
                )
            };
            if error::check_global(host, status, &f, "error obtaining time zone id enumeration")? {
                return Ok(Value::Bool(false));
            }
            return super::iterator::from_uenum(host, en);
        }
        "getCanonicalID" => {
            let Some(u) = id_arg(host, 0)? else {
                return Ok(Value::Bool(false));
            };
            let (mut sys_id, mut len, mut status) = (0, 0, sys::U_ZERO_ERROR);
            // SAFETY: the id's units.
            let p = unsafe {
                cpp::rhp_tz_canonical_id(
                    u.as_ptr(),
                    u.len() as i32,
                    &mut sys_id,
                    &mut len,
                    &mut status,
                )
            };
            let r = cpp::take_ustr(p, len);
            if error::check_global(host, status, &f, "error obtaining canonical ID")? {
                return Ok(Value::Bool(false));
            }
            let v = ustr(host, r, "could not convert time zone id to UTF-16")?;
            if let Some(rf) = call.reference(1) {
                common::assign_ref(host, &rf, Value::Bool(sys_id != 0))?;
            }
            return Ok(v);
        }
        "getRegion" => {
            let Some(u) = id_arg(host, 0)? else {
                return Ok(Value::Bool(false));
            };
            let mut buf = [0 as std::ffi::c_char; 3];
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: a 3-byte buffer.
            let n = unsafe {
                cpp::rhp_tz_region(u.as_ptr(), u.len() as i32, buf.as_mut_ptr(), 3, &mut status)
            };
            if error::check_global(host, status, &f, "error obtaining region")? {
                return Ok(Value::Bool(false));
            }
            let s: Vec<u8> = buf[..n.clamp(0, 3) as usize]
                .iter()
                .map(|&c| c as u8)
                .collect();
            return Ok(Value::str(s));
        }
        "getTZDataVersion" => {
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: plain ICU call.
            let p = unsafe { cpp::rhp_tz_data_version(&mut status) };
            if error::check_global(host, status, &f, "error obtaining time zone data version")? {
                return Ok(Value::Bool(false));
            }
            return Ok(Value::str(super::c_bytes(p)));
        }
        "getEquivalentID" => {
            let index = call.long(1, 0);
            if index < i32::MIN as i64 || index > i32::MAX as i64 {
                return Err(
                    call.value_error(1, &format!("must be between {} and {}", i32::MIN, i32::MAX))
                );
            }
            let Some(u) = id_arg(host, 0)? else {
                return Ok(Value::Bool(false));
            };
            let mut len = 0;
            // SAFETY: the id's units.
            let p = unsafe {
                cpp::rhp_tz_equivalent_id(u.as_ptr(), u.len() as i32, index as i32, &mut len)
            };
            return ustr(
                host,
                cpp::take_ustr(p, len),
                "could not convert resulting time zone id to UTF-16",
            );
        }
        "getIanaID" => {
            let Some(u) = id_arg(host, 0)? else {
                return Ok(Value::Bool(false));
            };
            let (mut len, mut status) = (0, sys::U_ZERO_ERROR);
            // SAFETY: the id's units.
            let p =
                unsafe { cpp::rhp_tz_iana_id(u.as_ptr(), u.len() as i32, &mut len, &mut status) };
            let r = cpp::take_ustr(p, len);
            if error::check_global(host, status, &f, "error obtaining IANA ID")? {
                return Ok(Value::Bool(false));
            }
            return ustr(host, r, "could not convert time zone id to UTF-16");
        }
        "getWindowsID" => {
            let Some(u) = id_arg(host, 0)? else {
                return Ok(Value::Bool(false));
            };
            let (mut len, mut status) = (0, sys::U_ZERO_ERROR);
            // SAFETY: the id's units.
            let p = unsafe {
                cpp::rhp_tz_windows_id(u.as_ptr(), u.len() as i32, &mut len, &mut status)
            };
            let r = cpp::take_ustr(p, len);
            if error::check_global(host, status, &f, "Unable to get timezone from windows ID")? {
                return Ok(Value::Bool(false));
            }
            if r.as_ref().is_none_or(|r| r.is_empty()) {
                error::set(
                    host,
                    &f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "unknown system timezone",
                )?;
                return Ok(Value::Bool(false));
            }
            return ustr(host, r, "could not convert time zone id to UTF-8");
        }
        "getIDForWindowsID" => {
            let Some(u) = id_arg(host, 0)? else {
                return Ok(Value::Bool(false));
            };
            let region = call.opt_bytes(1).map(|r| super::c_string(&r));
            let (mut len, mut status) = (0, sys::U_ZERO_ERROR);
            // SAFETY: the id's units, an optional C string.
            let p = unsafe {
                cpp::rhp_tz_id_for_windows(
                    u.as_ptr(),
                    u.len() as i32,
                    region.as_ref().map_or(std::ptr::null(), |r| r.as_ptr()),
                    &mut len,
                    &mut status,
                )
            };
            let r = cpp::take_ustr(p, len);
            if error::check_global(host, status, &f, "unable to get windows ID for timezone")? {
                return Ok(Value::Bool(false));
            }
            if r.as_ref().is_none_or(|r| r.is_empty()) {
                error::set(
                    host,
                    &f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "unknown windows timezone",
                )?;
                return Ok(Value::Bool(false));
            }
            return ustr(host, r, "could not convert time zone id to UTF-8");
        }
        _ => {}
    }
    // instance methods
    let this = call.this.clone().expect("an object");
    let to = common::state_of::<TzObj>(&this).expect("an object");
    match call.name {
        "getErrorCode" => return Ok(Value::Int(to.err.borrow().code as i64)),
        "getErrorMessage" => return Ok(Value::str(to.err.borrow().message().into_bytes())),
        _ => {}
    }
    to.err.borrow_mut().reset();
    let tz = to.ptr();
    if tz.is_null() {
        return Err(Thrown::error("Error", "Found unconstructed IntlTimeZone"));
    }
    let to_utf8 = |host: &mut dyn Host, u: Option<Vec<u16>>, msg: &str| -> Result<Value, Thrown> {
        let u = u.unwrap_or_default();
        match super::utf16_to_utf8(&u) {
            Ok(s) => Ok(Value::str(s)),
            Err(st) => {
                to.err.borrow_mut().code = st;
                error::check_status(host, &to.err, &f, msg)?;
                Ok(Value::Bool(false))
            }
        }
    };
    // SAFETY (all below): `tz` is the object's live zone.
    match call.name {
        "getID" => {
            let id = to.zone.borrow().as_ref().and_then(Zone::id16);
            to_utf8(host, id, "could not convert id to UTF-8")
        }
        "useDaylightTime" => Ok(Value::Bool(unsafe { cpp::rhp_tz_use_daylight(tz) } != 0)),
        "getRawOffset" => Ok(Value::Int(unsafe { cpp::rhp_tz_raw_offset(tz) } as i64)),
        "getDSTSavings" => Ok(Value::Int(unsafe { cpp::rhp_tz_dst_savings(tz) } as i64)),
        "getOffset" => {
            let date = call.double(0, 0.0);
            let local = call.bool(1, false);
            let (mut raw, mut dst, mut status) = (0, 0, sys::U_ZERO_ERROR);
            unsafe {
                cpp::rhp_tz_get_offset(tz, date, local as i32, &mut raw, &mut dst, &mut status)
            };
            to.err.borrow_mut().code = status;
            if error::check_status(host, &to.err, &f, "error obtaining offset")? {
                return Ok(Value::Bool(false));
            }
            if let Some(r) = call.reference(2) {
                common::assign_ref(host, &r, Value::Int(raw as i64))?;
            }
            if let Some(r) = call.reference(3) {
                common::assign_ref(host, &r, Value::Int(dst as i64))?;
            }
            Ok(Value::Bool(true))
        }
        "hasSameRules" => {
            let other = call.arg(0).unwrap_or(Value::Null);
            let o = common::state_of::<TzObj>(&other).expect("an IntlTimeZone");
            if o.ptr().is_null() {
                error::errors_set(
                    host,
                    &to.err,
                    &f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "The second IntlTimeZone is unconstructed",
                )?;
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(
                unsafe { cpp::rhp_tz_has_same_rules(tz, o.ptr()) } != 0,
            ))
        }
        "getDisplayName" => {
            let daylight = call.bool(0, false);
            let ty = call.long(1, class_int("IntlTimeZone", "DISPLAY_LONG"));
            let found = [
                "DISPLAY_SHORT",
                "DISPLAY_LONG",
                "DISPLAY_SHORT_GENERIC",
                "DISPLAY_LONG_GENERIC",
                "DISPLAY_SHORT_GMT",
                "DISPLAY_LONG_GMT",
                "DISPLAY_SHORT_COMMONLY_USED",
                "DISPLAY_GENERIC_LOCATION",
            ]
            .iter()
            .any(|n| class_int("IntlTimeZone", n) == ty);
            if !found {
                error::errors_set(
                    host,
                    &to.err,
                    &f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "wrong display type",
                )?;
                return Ok(Value::Bool(false));
            }
            let locale = call.opt_bytes(2).unwrap_or_else(super::locale_get_default);
            let loc = super::c_string(&locale);
            let mut len = 0;
            let p = unsafe {
                cpp::rhp_tz_display_name(tz, daylight as i32, ty as i32, loc.as_ptr(), &mut len)
            };
            to_utf8(
                host,
                cpp::take_ustr(p, len),
                "could not convert resulting time zone id to UTF-16",
            )
        }
        "toDateTimeZone" => {
            let zone = to
                .zone
                .borrow()
                .as_ref()
                .map(Zone::clone_zone)
                .expect("a zone");
            Ok(to_datetimezone(host, &zone, &to.err, &f)?.unwrap_or(Value::Bool(false)))
        }
        other => unreachable!("IntlTimeZone::{other}"),
    }
}

/// An IntlTimeZone's zone, when `v` is a constructed one.
pub(super) fn zone_of(v: &Value) -> Option<Zone> {
    let h = object_handlers::object_of(v)?;
    let to = object_handlers::payload_of::<TzObj>(&h)?;
    let z = to.zone.borrow();
    z.as_ref().map(Zone::clone_zone)
}
