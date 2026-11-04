//! php_date.c's classes: DateTime, DateTimeImmutable, DateTimeZone,
//! DateInterval and DatePeriod (and the procedural functions that alias
//! their methods). The prelude declares the classes; every method body is
//! one `__date($op, ...)` call into [`fn_date`].

use std::ffi::{c_char, c_int, CString};
use std::rc::Rc;

use rhp_csrc::timelib::{
    self, sys, RelTime, Time, Tz, TIMELIB_NO_CLONE, TIMELIB_OVERRIDE_TIME, TIMELIB_UNSET,
    TIMELIB_ZONETYPE_ABBR, TIMELIB_ZONETYPE_ID, TIMELIB_ZONETYPE_OFFSET,
};
use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, Value};

use super::format::date_format;
use super::objects::*;
use super::{c_str, parse_tzfile, tzdb};
use crate::{Host, Thrown};

const PHP_DATE_PERIOD_EXCLUDE_START_DATE: i64 = 0x0001;
const PHP_DATE_PERIOD_INCLUDE_END_DATE: i64 = 0x0002;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(deref).unwrap_or(Value::Null)
}

fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&arg(args, i))
}

fn long(args: &[Value], i: usize) -> i64 {
    conv::to_int(&arg(args, i))
}

fn obj(args: &[Value], i: usize) -> Result<ObjectHandle, Thrown> {
    match arg(args, i) {
        Value::Object(h) => Ok(h),
        _ => Err(Thrown::error("Error", "date object expected")),
    }
}

fn opt_obj(args: &[Value], i: usize) -> Option<ObjectHandle> {
    match arg(args, i) {
        Value::Object(h) => Some(h),
        _ => None,
    }
}

fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn assoc(a: &mut PhpArray, k: &str, v: Value) {
    a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
}

fn hash_get(a: &PhpArray, k: &str) -> Option<Value> {
    a.get(&ArrayKey::from_bytes(k.as_bytes())).map(deref)
}

/// `Z_PARAM_PATH`: no NUL bytes.
fn path_check(fname: &str, n: usize, pname: &str, v: &[u8]) -> Result<(), Thrown> {
    if v.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!("{fname}(): Argument #{n} (${pname}) must not contain any null bytes"),
        ));
    }
    Ok(())
}

/// The first error of a parse, as php_date.c quotes it: `(position)
/// (character)` and the message.
fn first_error(err: &timelib::Errors) -> (i32, char, String) {
    let m = err.errors().next().expect("an error");
    let c = if m.character != 0 {
        m.character as char
    } else {
        ' '
    };
    (m.position, c, s(m.message))
}

/// `update_errors_warnings`: keep the container when it has anything in it.
fn update_errors_warnings(err: timelib::Errors) {
    let keep = err.warning_count() > 0 || err.error_count() > 0;
    super::set_last_errors(keep.then_some(err));
}

/// `php_date_get_current_time_with_fraction`.
fn current_time_with_fraction() -> (i64, i64) {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_micros() as i64),
        Err(_) => (super::php_time(), 0),
    }
}

/// php's deprecation for the `#[\Deprecated]` `__wakeup()` methods.
fn wakeup_deprecated(host: &mut dyn Host, class: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(
        crate::E_DEPRECATED,
        &format!("Method {class}::__wakeup() is deprecated since 8.5, this method is obsolete, as serialization hooks are provided by __unserialize() and __serialize()"),
    )
}

// --- DateTime / DateTimeImmutable ---------------------------------------------

const INIT_CTOR: u8 = 1;
const INIT_FORMAT: u8 = 2;

/// `php_date_initialize`: false (with the object left uninitialized) when
/// the string doesn't parse. With [`INIT_CTOR`] that also throws.
fn date_initialize(
    _host: &mut dyn Host,
    h: &ObjectHandle,
    time_str: &[u8],
    format: Option<&[u8]>,
    tzobj: Option<&ObjectHandle>,
    flags: u8,
) -> Result<bool, Thrown> {
    set_time(h, None);
    let (time_str, (mut parsed, err)) = match format {
        Some(f) => (time_str, super::parse_from_format(f, time_str)),
        None => {
            let ts: &[u8] = if time_str.is_empty() {
                b"now"
            } else {
                time_str
            };
            (ts, super::strtotime(ts))
        }
    };
    let failed = err.error_count() > 0;
    let thrown = (flags & INIT_CTOR != 0 && failed).then(|| {
        let (pos, c, msg) = first_error(&err);
        Thrown::error(
            "DateMalformedStringException",
            format!(
                "Failed to parse time string ({}) at position {pos} ({c}): {msg}",
                s(c_str(time_str))
            ),
        )
    });
    update_errors_warnings(err);
    if let Some(t) = thrown {
        return Err(t);
    }
    if failed {
        return Ok(false);
    }

    let mut now = Time::new();
    let mut tzi: *mut sys::timelib_tzinfo = std::ptr::null_mut();
    let mut pin: Option<Rc<Tz>> = None;
    match tzobj {
        Some(tzh) => match tz_of(tzh) {
            Some(TzKind::Id(tz)) => {
                tzi = tz.as_ptr();
                now.tz_info = tzi;
                pin = Some(tz);
            }
            Some(TzKind::Offset(o)) => now.z = o as i32,
            Some(TzKind::Abbr {
                utc_offset,
                dst,
                abbr,
            }) => {
                now.z = utc_offset as i32;
                now.dst = dst;
                let abbr = CString::new(c_str(&abbr)).expect("no NUL");
                // SAFETY: a fresh copy on timelib's allocator, freed with `now`.
                now.tz_abbr = unsafe { sys::timelib_strdup(abbr.as_ptr()) };
            }
            None => {
                set_time(h, Some(ZTime::new(parsed)));
                return Err(Thrown::error(
                    "Error",
                    "The DateTimeZone object has not been correctly initialized by its constructor",
                ));
            }
        },
        None if !parsed.tz_info.is_null() => {
            tzi = parsed.tz_info;
            now.tz_info = tzi;
        }
        None => {
            let tz = super::timezone_info()?;
            tzi = tz.as_ptr();
            now.tz_info = tzi;
            pin = Some(tz);
        }
    }
    now.zone_type = tzobj
        .and_then(tz_of)
        .map_or(TIMELIB_ZONETYPE_ID, |k| k.zone_type());
    let (sec, usec) = current_time_with_fraction();
    now.unixtime2local(sec);
    now.us = usec;

    if format.is_none() && time_str == b"now" {
        set_time(h, Some(ZTime::with_zone(now, pin.as_ref())));
        return Ok(true);
    }
    let mut options = TIMELIB_NO_CLONE;
    if flags & INIT_FORMAT != 0 {
        options |= TIMELIB_OVERRIDE_TIME;
    }
    parsed.fill_holes(&now, options);
    // SAFETY: the time and the zone (pinned above or by the cache) are alive.
    unsafe { sys::timelib_update_ts(parsed.as_ptr(), tzi) };
    parsed.update_from_sse();
    parsed.have_relative = 0;
    set_time(h, Some(ZTime::with_zone(parsed, pin.as_ref())));
    Ok(true)
}

/// `php_date_initialize_from_ts_long`.
fn time_from_ts_long(sec: i64, usec: i64) -> ZTime {
    let mut t = Time::new();
    t.zone_type = TIMELIB_ZONETYPE_OFFSET;
    t.unixtime2gmt(sec);
    t.update_ts(None);
    t.us = usec;
    ZTime::new(t)
}

/// php's `%g` (php_gcvt with precision 6).
fn fmt_g(f: f64) -> String {
    if f.is_nan() {
        return "NAN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "INF" } else { "-INF" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0" } else { "0" }.into();
    }
    let e = format!("{:.5e}", f.abs());
    let (mant, exp) = e.split_once('e').expect("exponent");
    let exp: i32 = exp.parse().expect("exponent");
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let sign = if f < 0.0 { "-" } else { "" };
    let decpt = exp + 1;
    if !(-3..=6).contains(&decpt) {
        let (first, rest) = digits.split_at(1);
        let rest = if rest.is_empty() { "0" } else { rest };
        let esign = if exp < 0 { '-' } else { '+' };
        format!("{sign}{first}.{rest}e{esign}{}", exp.abs())
    } else if decpt <= 0 {
        format!("{sign}0.{}{digits}", "0".repeat((-decpt) as usize))
    } else if digits.len() as i32 <= decpt {
        format!(
            "{sign}{digits}{}",
            "0".repeat((decpt - digits.len() as i32) as usize)
        )
    } else {
        let (a, b) = digits.split_at(decpt as usize);
        format!("{sign}{a}.{b}")
    }
}

/// `php_date_initialize_from_ts_double`.
fn time_from_ts_double(fname: &str, ts: f64) -> Result<ZTime, Thrown> {
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    let range_error = || {
        Thrown::error(
            "DateRangeError",
            format!(
                "{fname}(): Argument #1 ($timestamp) must be a finite number between {} and {}.999999, {} given",
                i64::MIN,
                i64::MAX,
                fmt_g(ts)
            ),
        )
    };
    let sec_dval = ts.trunc();
    if sec_dval.is_nan() || sec_dval >= TWO_63 || sec_dval < -TWO_63 {
        return Err(range_error());
    }
    let mut sec = sec_dval as i64;
    let mut usec = ((ts % 1.0) * 1000000.0).round() as i32;
    if usec.abs() == 1000000 {
        sec += if usec > 0 { 1 } else { -1 };
        usec = 0;
    }
    if usec < 0 {
        if sec == i64::MIN {
            return Err(range_error());
        }
        sec -= 1;
        usec += 1000000;
    }
    Ok(time_from_ts_long(sec, usec as i64))
}

/// `php_date_initialize_from_hash`.
fn date_initialize_from_hash(
    host: &mut dyn Host,
    h: &ObjectHandle,
    myht: &PhpArray,
) -> Result<bool, Thrown> {
    let Some(Value::Str(date)) = hash_get(myht, "date") else {
        return Ok(false);
    };
    let Some(Value::Int(ztype)) = hash_get(myht, "timezone_type") else {
        return Ok(false);
    };
    let Some(Value::Str(zone)) = hash_get(myht, "timezone") else {
        return Ok(false);
    };
    match ztype as u32 {
        TIMELIB_ZONETYPE_OFFSET | TIMELIB_ZONETYPE_ABBR => {
            let mut tmp = date.as_bytes().to_vec();
            tmp.push(b' ');
            tmp.extend_from_slice(zone.as_bytes());
            date_initialize(host, h, &tmp, None, None, 0)
        }
        TIMELIB_ZONETYPE_ID => {
            let Some(tz) = parse_tzfile(zone.as_bytes()) else {
                return Ok(false);
            };
            // php builds a temporary DateTimeZone for the zone.
            let tmp = instantiate(host, b"DateTimeZone")?;
            set_tz(&tmp, Some(TzKind::Id(tz)));
            date_initialize(host, h, date.as_bytes(), None, Some(&tmp), 0)
        }
        _ => Ok(false),
    }
}

/// `add_common_properties`: the object's own properties not in `myht` yet.
fn add_common_properties(
    host: &dyn Host,
    myht: &mut PhpArray,
    h: &ObjectHandle,
    skip: impl Fn(&[u8]) -> bool,
) {
    for (k, v) in host.object_props_mangled(h).iter() {
        if let ArrayKey::Str(name) = k {
            if !skip(name.as_bytes()) && myht.get(k).is_none() {
                myht.insert(k.clone(), v.clone());
            }
        }
    }
}

/// `update_property`: a (possibly mangled) property name written as the
/// class that declares it would.
fn update_property(
    host: &mut dyn Host,
    h: &ObjectHandle,
    key: &[u8],
    val: Value,
) -> Result<(), Thrown> {
    let class = class_name(host, h);
    let (scope, name): (Vec<u8>, &[u8]) = if key.first() == Some(&0) {
        let rest = &key[1..];
        let Some(p) = rest.iter().position(|&b| b == 0) else {
            return Ok(());
        };
        let (c, n) = (&rest[..p], &rest[p + 1..]);
        if c == b"*" {
            (class.clone(), n)
        } else {
            if host.resolve_class(c).is_none() {
                return Ok(());
            }
            (c.to_vec(), n)
        }
    } else {
        let id = h.borrow().class;
        if !host.class_has_property(id, key) {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                &format!(
                    "Creation of dynamic property {}::${} is deprecated",
                    s(c_str(&class)),
                    s(key)
                ),
            )?;
        }
        (class.clone(), key)
    };
    host.reflect_set_prop(&Value::Object(h.clone()), &scope, name, val)
}

/// `restore_custom_*_properties`.
fn restore_custom_properties(
    host: &mut dyn Host,
    h: &ObjectHandle,
    myht: &PhpArray,
    internal: &[&str],
) -> Result<(), Thrown> {
    for (k, v) in myht.iter() {
        let ArrayKey::Str(name) = k else {
            continue;
        };
        if matches!(v, Value::Ref(_)) || internal.iter().any(|i| i.as_bytes() == name.as_bytes()) {
            continue;
        }
        update_property(host, h, name.as_bytes(), v.clone())?;
    }
    Ok(())
}

const DATE_INTERNAL: [&str; 3] = ["date", "timezone_type", "timezone"];

/// `php_date_modify`.
fn date_modify(
    host: &mut dyn Host,
    h: &ObjectHandle,
    modify: &[u8],
    fname: &str,
) -> Result<bool, Thrown> {
    let Some(d) = date_obj(h).filter(|d| d.time.borrow().is_some()) else {
        return Err(uninitialized(host, h));
    };
    let (tmp, err) = super::strtotime(modify);
    let failed = err.error_count() > 0;
    let msg = failed.then(|| {
        let (pos, c, m) = first_error(&err);
        format!(
            "Failed to parse time string ({}) at position {pos} ({c}): {m}",
            s(c_str(modify))
        )
    });
    update_errors_warnings(err);
    if let Some(msg) = msg {
        if fname == "date_modify" {
            crate::warn(host, fname, &msg)?;
            return Ok(false);
        }
        return Err(Thrown::error(
            "DateMalformedStringException",
            format!("{fname}(): {msg}"),
        ));
    }
    let mut guard = d.time.borrow_mut();
    let t = guard.as_mut().expect("initialized");
    t.relative = tmp.relative;
    t.have_relative = tmp.have_relative;
    t.sse_uptodate = 0;
    if tmp.y != TIMELIB_UNSET {
        t.y = tmp.y;
    }
    if tmp.m != TIMELIB_UNSET {
        t.m = tmp.m;
    }
    if tmp.d != TIMELIB_UNSET {
        t.d = tmp.d;
    }
    if tmp.h != TIMELIB_UNSET {
        t.h = tmp.h;
        if tmp.i != TIMELIB_UNSET {
            t.i = tmp.i;
            t.s = if tmp.s != TIMELIB_UNSET { tmp.s } else { 0 };
        } else {
            t.i = 0;
            t.s = 0;
        }
    }
    if tmp.us != TIMELIB_UNSET {
        t.us = tmp.us;
    }
    // Reset timezone to UTC if we detect a "@<ts>" modification.
    if tmp.y == 1970
        && tmp.m == 1
        && tmp.d == 1
        && tmp.h == 0
        && tmp.i == 0
        && tmp.s == 0
        && tmp.us == 0
        && tmp.have_zone != 0
        && tmp.zone_type == TIMELIB_ZONETYPE_OFFSET
        && tmp.z == 0
        && tmp.dst == 0
    {
        // SAFETY: the time is alive.
        unsafe { sys::timelib_set_timezone_from_offset(t.as_ptr(), 0) };
    }
    drop(tmp);
    t.update_ts(None);
    t.update_from_sse();
    t.have_relative = 0;
    t.relative = Default::default();
    t.repin();
    Ok(true)
}

/// `php_date_add` / `php_date_sub`.
fn date_add_sub(
    host: &mut dyn Host,
    h: &ObjectHandle,
    ih: &ObjectHandle,
    sub: Option<&str>,
) -> Result<(), Thrown> {
    let t = checked_time(host, h)?;
    if !interval_state(ih).initialized {
        return Err(uninitialized(host, ih));
    }
    interval_sync_in(host, ih)?;
    let st = interval_state(ih);
    let diff = st.diff.as_ref().expect("initialized interval");
    if let Some(fname) = sub {
        if diff.have_weekday_relative != 0 || diff.have_special_relative != 0 {
            let msg = "Only non-special relative time specifications are supported for subtraction";
            if fname == "date_sub" {
                return crate::warn(host, fname, msg);
            }
            return Err(Thrown::error(
                "DateInvalidOperationException",
                format!("{fname}(): {msg}"),
            ));
        }
    }
    let wall = st.civil_or_wall == PHP_DATE_WALL;
    // SAFETY: both are alive; timelib returns a new, owned time.
    let new = unsafe {
        let (tp, ip) = (t.as_ptr(), diff.as_ptr());
        Time::from_raw(match (sub.is_some(), wall) {
            (false, true) => sys::timelib_add_wall(tp, ip),
            (false, false) => sys::timelib_add(tp, ip),
            (true, true) => sys::timelib_sub_wall(tp, ip),
            (true, false) => sys::timelib_sub(tp, ip),
        })
    };
    set_time(h, Some(ZTime::with_zone(new, t.zone())));
    Ok(())
}

/// Runs `f` on the object's time (php's `dateobj->time`).
fn with_time<R>(
    host: &dyn Host,
    h: &ObjectHandle,
    f: impl FnOnce(&mut ZTime) -> R,
) -> Result<R, Thrown> {
    let Some(d) = date_obj(h) else {
        return Err(uninitialized(host, h));
    };
    let mut g = d.time.borrow_mut();
    match g.as_mut() {
        Some(t) => {
            let r = f(t);
            t.repin();
            Ok(r)
        }
        None => {
            drop(g);
            Err(uninitialized(host, h))
        }
    }
}

/// `php_date_timezone_set`.
fn date_timezone_set(host: &dyn Host, h: &ObjectHandle, tzh: &ObjectHandle) -> Result<(), Thrown> {
    let tz = tz_of(tzh);
    with_time(host, h, |t| {
        // SAFETY: the time and the zones are alive.
        unsafe {
            match &tz {
                Some(TzKind::Offset(o)) => sys::timelib_set_timezone_from_offset(t.as_ptr(), *o),
                Some(TzKind::Abbr {
                    utc_offset,
                    dst,
                    abbr,
                }) => {
                    let abbr = CString::new(c_str(abbr)).expect("no NUL");
                    sys::timelib_set_timezone_from_abbr(
                        t.as_ptr(),
                        sys::timelib_abbr_info {
                            utc_offset: *utc_offset,
                            abbr: abbr.as_ptr() as *mut c_char,
                            dst: *dst,
                        },
                    );
                }
                Some(TzKind::Id(tz)) => {
                    sys::timelib_set_timezone(t.as_ptr(), tz.as_ptr());
                    t.set_zone(tz);
                }
                None => {}
            }
        }
        let sse = t.sse;
        t.unixtime2local(sse);
    })
}

/// `date_offset_get`'s value.
fn date_offset(t: &Time) -> i64 {
    if t.is_localtime == 0 {
        return 0;
    }
    match t.zone_type {
        // SAFETY: an ID time's zone is alive (pinned).
        TIMELIB_ZONETYPE_ID => unsafe {
            timelib::TimeOffset::from_raw(sys::timelib_get_time_zone_info(t.sse, t.tz_info)).offset
                as i64
        },
        TIMELIB_ZONETYPE_OFFSET => t.z as i64,
        TIMELIB_ZONETYPE_ABBR => (t.z + 3600 * t.dst) as i64,
        _ => 0,
    }
}

fn date_op(host: &mut dyn Host, op: &[u8], args: &[Value]) -> Result<Option<Value>, Thrown> {
    Ok(Some(match op {
        b"construct" => {
            let h = obj(args, 0)?;
            let tz = opt_obj(args, 2);
            date_initialize(host, &h, &bytes(args, 1), None, tz.as_ref(), INIT_CTOR)?;
            Value::Null
        }
        b"date_create" => {
            let h = instantiate(host, &bytes(args, 0))?;
            let tz = opt_obj(args, 2);
            if !date_initialize(host, &h, &bytes(args, 1), None, tz.as_ref(), 0)? {
                return Ok(Some(Value::Bool(false)));
            }
            Value::Object(h)
        }
        b"createFromFormat" => {
            let class = bytes(args, 0);
            let (format, datetime) = (bytes(args, 1), bytes(args, 2));
            let fname = s(&bytes(args, 4));
            path_check(&fname, 2, "datetime", &datetime)?;
            let h = instantiate(host, &class)?;
            let tz = opt_obj(args, 3);
            if !date_initialize(host, &h, &datetime, Some(&format), tz.as_ref(), INIT_FORMAT)? {
                return Ok(Some(Value::Bool(false)));
            }
            Value::Object(h)
        }
        b"createFrom" => {
            let src = obj(args, 1)?;
            let t = checked_time(host, &src)?;
            new_date(host, &bytes(args, 0), t)?
        }
        b"createFromTimestamp" => {
            let class = bytes(args, 0);
            let h = instantiate(host, &class)?;
            let t = match arg(args, 1) {
                Value::Float(f) => {
                    let base = base_date_class(host, &class);
                    time_from_ts_double(&format!("{}::createFromTimestamp", s(&base)), f)?
                }
                v => time_from_ts_long(conv::to_int(&v), 0),
            };
            set_time(&h, Some(t));
            Value::Object(h)
        }
        b"getLastErrors" => match super::last_errors_array() {
            Some(a) => Value::Array(a),
            None => Value::Bool(false),
        },
        b"__set_state" => {
            let class = bytes(args, 0);
            let h = instantiate(host, &class)?;
            let Value::Array(a) = arg(args, 1) else {
                return Ok(Some(Value::Null));
            };
            if !date_initialize_from_hash(host, &h, &a)? {
                return Err(Thrown::error(
                    "Error",
                    format!("Invalid serialization data for {} object", s(&class)),
                ));
            }
            Value::Object(h)
        }
        b"__serialize" => {
            let h = obj(args, 0)?;
            let t = checked_time(host, &h)?;
            let mut a = PhpArray::new();
            date_object_to_hash(&t, &mut a);
            add_common_properties(host, &mut a, &h, |_| false);
            Value::Array(a)
        }
        b"__unserialize" | b"__wakeup" => {
            let h = obj(args, 0)?;
            let base = base_date_class(host, &class_name(host, &h));
            let base = s(&base);
            let unserialize = op == b"__unserialize";
            let myht = if unserialize {
                match arg(args, 1) {
                    Value::Array(a) => a,
                    _ => PhpArray::new(),
                }
            } else {
                wakeup_deprecated(host, &base)?;
                h.borrow()
                    .properties
                    .iter()
                    .map(|(k, v)| (ArrayKey::Str(k.clone()), v.clone()))
                    .collect()
            };
            if !date_initialize_from_hash(host, &h, &myht)? {
                return Err(Thrown::error(
                    "Error",
                    format!("Invalid serialization data for {base} object"),
                ));
            }
            if unserialize {
                restore_custom_properties(host, &h, &myht, &DATE_INTERNAL)?;
            }
            Value::Null
        }
        b"format" => {
            let h = obj(args, 0)?;
            let t = checked_time(host, &h)?;
            Value::str(date_format(&bytes(args, 1), &t, t.is_localtime != 0))
        }
        b"modify" => {
            let h = obj(args, 0)?;
            if !date_modify(host, &h, &bytes(args, 1), &s(&bytes(args, 2)))? {
                return Ok(Some(Value::Bool(false)));
            }
            Value::Object(h)
        }
        b"add" | b"sub" => {
            let h = obj(args, 0)?;
            let ih = obj(args, 1)?;
            let fname = s(&bytes(args, 2));
            date_add_sub(host, &h, &ih, (op == b"sub").then_some(fname.as_str()))?;
            Value::Object(h)
        }
        b"getTimezone" => {
            let h = obj(args, 0)?;
            let t = checked_time(host, &h)?;
            if t.is_localtime == 0 {
                return Ok(Some(Value::Bool(false)));
            }
            let tz = instantiate(host, b"DateTimeZone")?;
            set_tz(&tz, TzKind::from_time(&t, t.zone()));
            Value::Object(tz)
        }
        b"setTimezone" => {
            let h = obj(args, 0)?;
            date_timezone_set(host, &h, &obj(args, 1)?)?;
            Value::Object(h)
        }
        b"getOffset" => {
            let t = checked_time(host, &obj(args, 0)?)?;
            Value::Int(date_offset(&t))
        }
        b"getMicrosecond" => {
            let h = obj(args, 0)?;
            Value::Int(checked_time(host, &h)?.us)
        }
        b"setTime" => {
            let h = obj(args, 0)?;
            let (hh, i, sec, us) = (long(args, 1), long(args, 2), long(args, 3), long(args, 4));
            with_time(host, &h, |t| {
                t.h = hh;
                t.i = i;
                t.s = sec;
                t.us = us;
                t.update_ts(None);
                t.update_from_sse();
            })?;
            Value::Object(h)
        }
        b"setDate" => {
            let h = obj(args, 0)?;
            let (y, m, d) = (long(args, 1), long(args, 2), long(args, 3));
            with_time(host, &h, |t| {
                t.y = y;
                t.m = m;
                t.d = d;
                t.update_ts(None);
            })?;
            Value::Object(h)
        }
        b"setISODate" => {
            let h = obj(args, 0)?;
            let (y, w, d) = (long(args, 1), long(args, 2), long(args, 3));
            with_time(host, &h, |t| {
                t.y = y;
                t.m = 1;
                t.d = 1;
                t.relative = Default::default();
                t.relative.d = timelib::daynr_from_weeknr(y, w, d);
                t.have_relative = 1;
                t.update_ts(None);
            })?;
            Value::Object(h)
        }
        b"setTimestamp" => {
            let h = obj(args, 0)?;
            let ts = long(args, 1);
            with_time(host, &h, |t| {
                t.unixtime2local(ts);
                t.update_ts(None);
                t.us = 0;
            })?;
            Value::Object(h)
        }
        b"usCheck" | b"setMicrosecond" => {
            let h = obj(args, 0)?;
            let us = long(args, 1);
            if !(0..=999999).contains(&us) {
                let base = base_date_class(host, &class_name(host, &h));
                return Err(Thrown::error(
                    "DateRangeError",
                    format!(
                        "{}::setMicrosecond(): Argument #1 ($microsecond) must be between 0 and 999999, {us} given",
                        s(&base)
                    ),
                ));
            }
            if op == b"usCheck" {
                checked_time(host, &h)?;
                return Ok(Some(Value::Null));
            }
            with_time(host, &h, |t| t.us = us)?;
            Value::Object(h)
        }
        b"getTimestamp" => {
            let h = obj(args, 0)?;
            let ts = with_time(host, &h, |t| {
                if t.sse_uptodate == 0 {
                    t.update_ts(None);
                }
                t.date_to_int()
            })?;
            match ts {
                Some(ts) => Value::Int(ts),
                None => {
                    return Err(Thrown::error(
                        "DateRangeError",
                        "Epoch doesn't fit in a PHP integer",
                    ))
                }
            }
        }
        b"diff" => {
            let (h1, h2) = (obj(args, 0)?, obj(args, 1)?);
            let t1 = checked_time(host, &h1)?;
            let t2 = checked_time(host, &h2)?;
            // SAFETY: both times are alive; timelib returns an owned result.
            let mut diff =
                unsafe { RelTime::from_raw(sys::timelib_diff(t1.as_ptr(), t2.as_ptr())) };
            if conv::to_bool(&arg(args, 2)) {
                diff.invert = 0;
            }
            new_interval(
                host,
                IntervalState {
                    diff: Some(diff),
                    initialized: true,
                    civil_or_wall: PHP_DATE_CIVIL,
                    ..IntervalState::default()
                },
            )?
        }
        _ => return Ok(None),
    }))
}

// --- DateTimeZone ---------------------------------------------------------------

/// `timezone_initialize`: the zone, or the warning text.
fn timezone_initialize(tz: &[u8]) -> Result<TzKind, String> {
    let c = CString::new(tz).expect("checked for NUL bytes");
    let mut t = Time::new();
    let mut dst: c_int = 0;
    let mut not_found: c_int = 0;
    let mut p: *const c_char = c.as_ptr();
    // SAFETY: `p` walks the NUL-terminated copy; the time is alive.
    let z = unsafe {
        sys::timelib_parse_zone(
            &mut p,
            &mut dst,
            t.as_ptr(),
            &mut not_found,
            tzdb().as_ptr(),
            Some(super::parse_tzfile_wrapper),
        )
    };
    super::resume_callback_panic();
    t.z = z as c_int;
    if t.z >= 100 * 60 * 60 || t.z <= -100 * 60 * 60 {
        return Err(format!("Timezone offset is out of range ({})", s(tz)));
    }
    t.dst = dst;
    // SAFETY: `p` points into `c`.
    let rest = unsafe { *p };
    if not_found != 0 || rest != 0 {
        return Err(format!("Unknown or bad timezone ({})", s(tz)));
    }
    TzKind::from_time(&t, None).ok_or_else(|| format!("Unknown or bad timezone ({})", s(tz)))
}

/// `php_date_timezone_initialize_from_hash`.
fn timezone_initialize_from_hash(myht: &PhpArray) -> Option<TzKind> {
    let ztype = hash_get(myht, "timezone_type")?;
    let zone = hash_get(myht, "timezone")?;
    let Value::Int(ztype) = ztype else {
        return None;
    };
    if !(TIMELIB_ZONETYPE_OFFSET as i64..=TIMELIB_ZONETYPE_ID as i64).contains(&ztype) {
        return None;
    }
    let Value::Str(zone) = zone else {
        return None;
    };
    if zone.as_bytes().contains(&0) {
        return None;
    }
    timezone_initialize(zone.as_bytes()).ok()
}

const TZ_INTERNAL: [&str; 2] = ["timezone_type", "timezone"];

/// `timezone_transitions_get`.
fn timezone_transitions(tz: &Tz, begin_ts: i64, end_ts: i64) -> PhpArray {
    let info = tz.info();
    let timecnt = info.bit64.timecnt;
    let mut out = PhpArray::new();
    let element = |ts: i64, offset: i64, isdst: bool, abbr: Vec<u8>| {
        let mut e = PhpArray::new();
        assoc(&mut e, "ts", Value::Int(ts));
        assoc(
            &mut e,
            "time",
            Value::str(super::php_format_date(b"x-m-d\\TH:i:sP", ts, false)),
        );
        assoc(&mut e, "offset", Value::Int(offset));
        assoc(&mut e, "isdst", Value::Bool(isdst));
        assoc(&mut e, "abbr", Value::str(abbr));
        Value::Array(e)
    };
    // SAFETY: the zone's arrays hold `timecnt` transitions and `typecnt`
    // types; abbreviations are NUL-terminated inside `timezone_abbr`.
    let ty = |i: usize| unsafe { *info.type_.add(i) };
    let abbr_of = |t: sys::ttinfo| unsafe {
        std::ffi::CStr::from_ptr(info.timezone_abbr.add(t.abbr_idx as usize))
            .to_bytes()
            .to_vec()
    };
    let trans = |i: u64| unsafe { *info.trans.add(i as usize) };
    let trans_idx = |i: u64| unsafe { *info.trans_idx.add(i as usize) } as usize;
    let by_index = |i: usize, ts: i64| {
        let t = ty(i);
        element(ts, t.offset as i64, t.isdst != 0, abbr_of(t))
    };
    let add = |i: u64, ts: i64| by_index(trans_idx(i), ts);
    let nominal = |ts: i64| by_index(0, ts);

    let mut begin: u64 = 0;
    let mut found = false;
    if begin_ts == i64::MIN {
        out.push(nominal(begin_ts));
        found = true;
    } else if timecnt > 0 {
        loop {
            if trans(begin) > begin_ts {
                if begin > 0 {
                    out.push(add(begin - 1, begin_ts));
                } else {
                    out.push(nominal(begin_ts));
                }
                found = true;
                break;
            }
            begin += 1;
            if begin >= timecnt {
                break;
            }
        }
    }
    // SAFETY: posix_info is NULL or the zone's parsed POSIX string.
    let posix = unsafe { info.posix_info.as_ref() };
    let has_dst_end = posix.is_some_and(|p| !p.dst_end.is_null());
    if !found {
        if timecnt > 0 {
            if has_dst_end {
                let o = tz.offset_at(begin_ts);
                out.push(element(
                    begin_ts,
                    o.offset as i64,
                    o.is_dst != 0,
                    o.abbr().to_vec(),
                ));
            } else {
                out.push(add(timecnt - 1, begin_ts));
            }
        } else {
            out.push(nominal(begin_ts));
        }
    } else {
        for i in begin..timecnt {
            if trans(i) < end_ts {
                out.push(add(i, trans(i)));
            } else {
                return out;
            }
        }
    }
    if has_dst_end && timecnt > 0 {
        let last = trans(timecnt - 1);
        let year = |ts: i64| {
            let (mut y, mut m, mut d) = (0, 0, 0);
            // SAFETY: valid out-pointers.
            unsafe { sys::timelib_unixtime2date(ts, &mut y, &mut m, &mut d) };
            y
        };
        let (start_y, end_y) = (year(last), year(end_ts));
        for y in start_y..=end_y {
            let mut tr = sys::timelib_posix_transitions::default();
            // SAFETY: the zone is alive; `tr` is a valid out-pointer.
            unsafe { sys::timelib_get_transitions_for_year(tz.as_ptr(), y, &mut tr) };
            for j in 0..tr.count {
                let ts = tr.times[j];
                if ts <= last || ts < begin_ts {
                    continue;
                }
                if ts > end_ts {
                    return out;
                }
                out.push(by_index(tr.types[j] as usize, ts));
            }
        }
    }
    out
}

fn timezone_op(host: &mut dyn Host, op: &[u8], args: &[Value]) -> Result<Option<Value>, Thrown> {
    Ok(Some(match op {
        b"tz_construct" => {
            let h = obj(args, 0)?;
            let tz = bytes(args, 1);
            path_check("DateTimeZone::__construct", 1, "timezone", &tz)?;
            match timezone_initialize(&tz) {
                Ok(k) => set_tz(&h, Some(k)),
                Err(msg) => {
                    return Err(Thrown::error(
                        "DateInvalidTimeZoneException",
                        format!("DateTimeZone::__construct(): {msg}"),
                    ))
                }
            }
            Value::Null
        }
        b"tz_open" => {
            let tz = bytes(args, 0);
            path_check("timezone_open", 1, "timezone", &tz)?;
            let h = instantiate(host, b"DateTimeZone")?;
            match timezone_initialize(&tz) {
                Ok(k) => {
                    set_tz(&h, Some(k));
                    Value::Object(h)
                }
                Err(msg) => {
                    crate::warn(host, "timezone_open", &msg)?;
                    Value::Bool(false)
                }
            }
        }
        b"tz_name" => checked_tz(host, &obj(args, 0)?)?.to_value(),
        b"tz_offset" => {
            let tz = checked_tz(host, &obj(args, 0)?)?;
            let t = checked_time(host, &obj(args, 1)?)?;
            Value::Int(match tz {
                TzKind::Id(z) => z.offset_at(t.sse).offset as i64,
                TzKind::Offset(o) => o,
                TzKind::Abbr {
                    utc_offset, dst, ..
                } => utc_offset + dst as i64 * 3600,
            })
        }
        b"tz_transitions" => match checked_tz(host, &obj(args, 0)?)? {
            TzKind::Id(z) => Value::Array(timezone_transitions(&z, long(args, 1), long(args, 2))),
            _ => Value::Bool(false),
        },
        b"tz_location" => match checked_tz(host, &obj(args, 0)?)? {
            TzKind::Id(z) => {
                let loc = &z.info().location;
                let cc: Vec<u8> = loc
                    .country_code
                    .iter()
                    .take_while(|&&c| c != 0)
                    .map(|&c| c as u8)
                    .collect();
                let mut a = PhpArray::new();
                assoc(&mut a, "country_code", Value::str(cc));
                assoc(&mut a, "latitude", Value::Float(loc.latitude));
                assoc(&mut a, "longitude", Value::Float(loc.longitude));
                // SAFETY: NULL or the zone's NUL-terminated comment.
                let comments = unsafe { loc.comments.as_ref() }
                    .map(|_| {
                        unsafe { std::ffi::CStr::from_ptr(loc.comments) }
                            .to_bytes()
                            .to_vec()
                    })
                    .unwrap_or_default();
                assoc(&mut a, "comments", Value::str(comments));
                Value::Array(a)
            }
            _ => Value::Bool(false),
        },
        b"tz_identifiers" => {
            let country = match arg(args, 1) {
                Value::Null => None,
                v => Some(conv::to_string_bytes(&v)),
            };
            super::identifiers_list(
                "DateTimeZone::listIdentifiers",
                long(args, 0),
                country.as_deref(),
            )?
        }
        b"tz_serialize" => {
            let h = obj(args, 0)?;
            let tz = checked_tz(host, &h)?;
            let mut a = PhpArray::new();
            date_timezone_object_to_hash(&tz, &mut a);
            add_common_properties(host, &mut a, &h, |_| false);
            Value::Array(a)
        }
        b"tz_unserialize" | b"tz_wakeup" => {
            let h = obj(args, 0)?;
            let unserialize = op == b"tz_unserialize";
            let myht = if unserialize {
                match arg(args, 1) {
                    Value::Array(a) => a,
                    _ => PhpArray::new(),
                }
            } else {
                wakeup_deprecated(host, "DateTimeZone")?;
                h.borrow()
                    .properties
                    .iter()
                    .map(|(k, v)| (ArrayKey::Str(k.clone()), v.clone()))
                    .collect()
            };
            match timezone_initialize_from_hash(&myht) {
                Some(k) => set_tz(&h, Some(k)),
                None => {
                    return Err(Thrown::error(
                        "Error",
                        "Invalid serialization data for DateTimeZone object",
                    ))
                }
            }
            if unserialize {
                restore_custom_properties(host, &h, &myht, &TZ_INTERNAL)?;
            }
            Value::Null
        }
        b"tz_set_state" => {
            let h = instantiate(host, b"DateTimeZone")?;
            let myht = match arg(args, 0) {
                Value::Array(a) => a,
                _ => PhpArray::new(),
            };
            match timezone_initialize_from_hash(&myht) {
                Some(k) => set_tz(&h, Some(k)),
                None => {
                    return Err(Thrown::error(
                        "Error",
                        "Invalid serialization data for DateTimeZone object",
                    ))
                }
            }
            Value::Object(h)
        }
        _ => return Ok(None),
    }))
}

// --- DateInterval ----------------------------------------------------------------

/// `date_interval_initialize`.
fn date_interval_initialize(format: &[u8]) -> Result<RelTime, Thrown> {
    let iv = timelib::strtointerval(format);
    if iv.errors.error_count() > 0 {
        return Err(Thrown::error(
            "DateMalformedIntervalStringException",
            format!("Unknown or bad format ({})", s(c_str(format))),
        ));
    }
    if let Some(p) = iv.period {
        return Ok(p);
    }
    match (iv.begin, iv.end) {
        (Some(mut b), Some(mut e)) => {
            b.update_ts(None);
            e.update_ts(None);
            // SAFETY: both alive; the result is owned.
            Ok(unsafe { RelTime::from_raw(sys::timelib_diff(b.as_ptr(), e.as_ptr())) })
        }
        _ => Err(Thrown::error(
            "DateMalformedIntervalStringException",
            format!("Failed to parse interval ({})", s(c_str(format))),
        )),
    }
}

/// `date_interval_format`.
fn date_interval_format(format: &[u8], t: &RelTime) -> Vec<u8> {
    let mut out = Vec::new();
    let mut spec = false;
    for &c in format {
        if spec {
            let item = match c {
                b'Y' => format!("{:02}", t.y as i32),
                b'y' => format!("{}", t.y as i32),
                b'M' => format!("{:02}", t.m as i32),
                b'm' => format!("{}", t.m as i32),
                b'D' => format!("{:02}", t.d as i32),
                b'd' => format!("{}", t.d as i32),
                b'H' => format!("{:02}", t.h as i32),
                b'h' => format!("{}", t.h as i32),
                b'I' => format!("{:02}", t.i as i32),
                b'i' => format!("{}", t.i as i32),
                b'S' => format!("{:02}", t.s),
                b's' => format!("{}", t.s),
                b'F' => format!("{:06}", t.us),
                b'f' => format!("{}", t.us),
                b'a' => {
                    if t.days as i32 as i64 != TIMELIB_UNSET {
                        format!("{}", t.days as i32)
                    } else {
                        "(unknown)".into()
                    }
                }
                b'r' => (if t.invert != 0 { "-" } else { "" }).into(),
                b'R' => (if t.invert != 0 { "-" } else { "+" }).into(),
                b'%' => "%".into(),
                c => {
                    out.push(b'%');
                    out.push(c);
                    spec = false;
                    continue;
                }
            };
            out.extend_from_slice(item.as_bytes());
            spec = false;
        } else if c == b'%' {
            spec = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// `strtoll` of a value's string form (`DATE_A64I`).
fn a64i(host: &mut dyn Host, v: &Value) -> Result<i64, Thrown> {
    let st = crate::zend::zpp::zval_get_string(host, v)?;
    let st = c_str(&st);
    let mut i = 0;
    while i < st.len() && matches!(st[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let neg = match st.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut n: i64 = 0;
    let mut overflow = false;
    while let Some(d) = st.get(i).filter(|c| c.is_ascii_digit()) {
        match n
            .checked_mul(10)
            .and_then(|n| n.checked_add((d - b'0') as i64))
        {
            Some(v) => n = v,
            None => overflow = true,
        }
        i += 1;
    }
    Ok(match (overflow, neg) {
        (true, true) => i64::MIN,
        (true, false) => i64::MAX,
        (false, true) => -n,
        (false, false) => n,
    })
}

/// `php_date_interval_initialize_from_hash`.
fn interval_initialize_from_hash(
    host: &mut dyn Host,
    h: &ObjectHandle,
    myht: &PhpArray,
) -> Result<(), Thrown> {
    if let Some(Value::Str(ds)) = hash_get(myht, "date_string") {
        let (time, err) = super::strtotime(ds.as_bytes());
        if err.error_count() > 0 {
            let (pos, c, msg) = first_error(&err);
            return Err(Thrown::error(
                "Error",
                format!(
                    "Unknown or bad format ({}) at position {pos} ({c}) while unserializing: {msg}",
                    s(c_str(ds.as_bytes()))
                ),
            ));
        }
        let mut diff = RelTime::new();
        *diff = time.relative;
        set_interval(
            h,
            IntervalState {
                diff: Some(diff),
                initialized: true,
                civil_or_wall: PHP_DATE_CIVIL,
                from_string: true,
                date_string: Some(ds.as_bytes().to_vec()),
            },
        );
        return Ok(());
    }
    let mut diff = RelTime::new();
    // Scalars (IS_NULL..IS_STRING) read as longs, anything else is `def`.
    let scalar = |v: &Option<Value>| {
        v.as_ref()
            .filter(|v| {
                matches!(
                    v,
                    Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_)
                )
            })
            .cloned()
    };
    let read = |host: &mut dyn Host, k: &str, def: i64| -> Result<i64, Thrown> {
        match scalar(&hash_get(myht, k)) {
            Some(v) => crate::zend::zpp::zval_get_long(host, &v),
            None => Ok(def),
        }
    };
    diff.y = read(host, "y", -1)?;
    diff.m = read(host, "m", -1)?;
    diff.d = read(host, "d", -1)?;
    diff.h = read(host, "h", -1)?;
    diff.i = read(host, "i", -1)?;
    diff.s = read(host, "s", -1)?;
    if let Some(f) = hash_get(myht, "f") {
        let d = match &f {
            Value::Float(d) => *d,
            Value::Array(a) => (!a.is_empty()) as i64 as f64,
            other => conv::to_float(other),
        };
        diff.us = dval_to_lval(host, d * 1000000.0)?;
    }
    diff.weekday = read(host, "weekday", -1)? as i32;
    diff.weekday_behavior = read(host, "weekday_behavior", -1)? as i32;
    diff.first_last_day_of = read(host, "first_last_day_of", -1)? as i32;
    diff.invert = read(host, "invert", 0)? as i32;
    diff.days = match hash_get(myht, "days") {
        Some(Value::Bool(false)) => TIMELIB_UNSET,
        v => match scalar(&v) {
            Some(v) => a64i(host, &v)?,
            None => -1,
        },
    };
    diff.special.type_ = read(host, "special_type", 0)? as u32;
    diff.special.amount = match scalar(&hash_get(myht, "special_amount")) {
        Some(v) => a64i(host, &v)?,
        None => -1,
    };
    diff.have_weekday_relative = read(host, "have_weekday_relative", 0)? as u32;
    diff.have_special_relative = read(host, "have_special_relative", 0)? as u32;
    let civil_or_wall = match hash_get(myht, "civil_or_wall") {
        Some(v) => crate::zend::zpp::zval_get_long(host, &v)? as i32,
        None => PHP_DATE_CIVIL,
    };
    set_interval(
        h,
        IntervalState {
            diff: Some(diff),
            initialized: true,
            civil_or_wall,
            from_string: false,
            date_string: None,
        },
    );
    Ok(())
}

const INTERVAL_INTERNAL: [&str; 11] = [
    "date_string",
    "from_string",
    "y",
    "m",
    "d",
    "h",
    "i",
    "s",
    "f",
    "invert",
    "days",
];

fn interval_op(host: &mut dyn Host, op: &[u8], args: &[Value]) -> Result<Option<Value>, Thrown> {
    Ok(Some(match op {
        b"iv_construct" => {
            let h = obj(args, 0)?;
            let diff = date_interval_initialize(&bytes(args, 1))?;
            set_interval(
                &h,
                IntervalState {
                    diff: Some(diff),
                    initialized: true,
                    civil_or_wall: PHP_DATE_WALL,
                    ..IntervalState::default()
                },
            );
            interval_sync_out(&h);
            Value::Null
        }
        b"iv_from_string" => {
            let time_str = bytes(args, 0);
            let method = conv::to_bool(&arg(args, 1));
            let (time, err) = super::strtotime(&time_str);
            let shown = s(c_str(&time_str));
            let fail = |host: &mut dyn Host, msg: String| -> Result<Option<Value>, Thrown> {
                if method {
                    Err(Thrown::error("DateMalformedIntervalStringException", msg))
                } else {
                    crate::warn(host, "date_interval_create_from_date_string", &msg)?;
                    Ok(Some(Value::Bool(false)))
                }
            };
            if err.error_count() > 0 {
                let (pos, c, msg) = first_error(&err);
                return fail(
                    host,
                    format!("Unknown or bad format ({shown}) at position {pos} ({c}): {msg}"),
                );
            }
            if time.have_date != 0 || time.have_time != 0 || time.have_zone != 0 {
                return fail(
                    host,
                    format!("String '{shown}' contains non-relative elements"),
                );
            }
            let mut diff = RelTime::new();
            *diff = time.relative;
            new_interval(
                host,
                IntervalState {
                    diff: Some(diff),
                    initialized: true,
                    civil_or_wall: PHP_DATE_CIVIL,
                    from_string: true,
                    date_string: Some(time_str),
                },
            )?
        }
        b"iv_format" => {
            let h = obj(args, 0)?;
            if !interval_state(&h).initialized {
                return Err(uninitialized(host, &h));
            }
            interval_sync_in(host, &h)?;
            let st = interval_state(&h);
            Value::str(date_interval_format(
                &bytes(args, 1),
                st.diff.as_ref().expect("initialized"),
            ))
        }
        b"iv_serialize" => {
            let h = obj(args, 0)?;
            if !interval_state(&h).initialized {
                return Err(uninitialized(host, &h));
            }
            interval_sync_in(host, &h)?;
            let st = interval_state(&h);
            let mut a = PhpArray::new();
            date_interval_object_to_hash(&st, &mut a);
            add_common_properties(host, &mut a, &h, |k| interval_hidden(&st, k));
            Value::Array(a)
        }
        b"iv_unserialize" | b"iv_wakeup" => {
            let h = obj(args, 0)?;
            let unserialize = op == b"iv_unserialize";
            let myht = if unserialize {
                match arg(args, 1) {
                    Value::Array(a) => a,
                    _ => PhpArray::new(),
                }
            } else {
                wakeup_deprecated(host, "DateInterval")?;
                h.borrow()
                    .properties
                    .iter()
                    .map(|(k, v)| (ArrayKey::Str(k.clone()), v.clone()))
                    .collect()
            };
            interval_initialize_from_hash(host, &h, &myht)?;
            if unserialize {
                restore_custom_properties(host, &h, &myht, &INTERVAL_INTERNAL)?;
            }
            interval_sync_out(&h);
            Value::Null
        }
        b"iv_set_state" => {
            let h = instantiate(host, b"DateInterval")?;
            let myht = match arg(args, 0) {
                Value::Array(a) => a,
                _ => PhpArray::new(),
            };
            interval_initialize_from_hash(host, &h, &myht)?;
            interval_sync_out(&h);
            Value::Object(h)
        }
        _ => return Ok(None),
    }))
}

// --- DatePeriod ------------------------------------------------------------------

const MAX_RECURRENCES: i64 = i32::MAX as i64 - 8;

/// `date_period_init_iso8601_string` into `st`.
fn period_init_iso8601(
    st: &mut PeriodState,
    base_ce: &[u8],
    isostr: &[u8],
    fname: &str,
) -> Result<i64, Thrown> {
    let iv = timelib::strtointerval(isostr);
    let shown = s(c_str(isostr));
    if iv.errors.error_count() > 0 {
        return Err(Thrown::error(
            "DateMalformedPeriodStringException",
            format!("Unknown or bad format ({shown})"),
        ));
    }
    st.start = iv.begin.map(ZTime::new);
    st.end = iv.end.map(ZTime::new);
    st.interval = iv.period;
    let recurrences = iv.recurrences as i64;
    let missing = |what: &str| {
        Thrown::error(
            "DateMalformedPeriodStringException",
            format!("{fname}(): ISO interval must contain {what}, \"{shown}\" given"),
        )
    };
    if st.start.is_none() {
        return Err(missing("a start date"));
    }
    if st.interval.is_none() {
        return Err(missing("an interval"));
    }
    if st.end.is_none() && recurrences == 0 {
        return Err(missing("an end date or a recurrence count"));
    }
    if let Some(t) = st.start.as_mut() {
        t.update_ts(None);
    }
    if let Some(t) = st.end.as_mut() {
        t.update_ts(None);
    }
    st.start_ce = Some(base_ce.to_vec());
    Ok(recurrences)
}

/// `date_period_init_finish`.
fn period_init_finish(
    st: &mut PeriodState,
    options: i64,
    recurrences: i64,
    fname: &str,
) -> Result<(), Thrown> {
    if st.end.is_none() && !(1..=MAX_RECURRENCES).contains(&recurrences) {
        return Err(Thrown::error(
            "DateMalformedPeriodStringException",
            format!(
                "{fname}(): Recurrence count must be greater or equal to 1 and lower than {}",
                MAX_RECURRENCES + 1
            ),
        ));
    }
    st.include_start_date = options & PHP_DATE_PERIOD_EXCLUDE_START_DATE == 0;
    st.include_end_date = options & PHP_DATE_PERIOD_INCLUDE_END_DATE != 0;
    let recurrences = recurrences + st.include_start_date as i64 + st.include_end_date as i64;
    if recurrences > MAX_RECURRENCES {
        return Err(Thrown::error(
            "DateMalformedStringException",
            format!(
                "{fname}(): Recurrence count must be greater or equal to 1 and lower than {} (including options)",
                MAX_RECURRENCES + 1
            ),
        ));
    }
    st.recurrences = recurrences as i32;
    st.initialized = true;
    Ok(())
}

/// zpp `l` in a quiet parse (`zend_parse_arg_long_weak`): the long, or
/// `None` when the value doesn't convert in the caller's typing mode. A
/// fractional float or float string converts with php's deprecation.
fn quiet_long(
    host: &mut dyn Host,
    v: &Value,
    n: usize,
    pname: &str,
) -> Result<Option<i64>, Thrown> {
    // ZEND_DOUBLE_FITS_LONG
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    let fits = |f: f64| (-TWO_63..TWO_63).contains(&f);
    Ok(match v {
        Value::Int(i) => Some(*i),
        _ if host.caller_strict_types() => None,
        Value::Bool(b) => Some(*b as i64),
        Value::Float(f) if fits(*f) => {
            if f.fract() != 0.0 {
                let shown =
                    String::from_utf8_lossy(&rhp_values::fmt::float_serialize(*f)).into_owned();
                host.emit_diagnostic(
                    crate::E_DEPRECATED,
                    &format!("Implicit conversion from float {shown} to int loses precision"),
                )?;
            }
            Some(*f as i64)
        }
        Value::Str(st) => match conv::numeric_string(st.as_bytes()) {
            Some(conv::Num::Int(i)) => Some(i),
            Some(conv::Num::Float(f)) if fits(f) => {
                if f.fract() != 0.0 {
                    host.emit_diagnostic(
                        crate::E_DEPRECATED,
                        &format!(
                            "Implicit conversion from float-string \"{}\" to int loses precision",
                            String::from_utf8_lossy(st.as_bytes())
                        ),
                    )?;
                }
                Some(f as i64)
            }
            _ => None,
        },
        Value::Null => {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                &format!("DatePeriod::__construct(): Passing null to parameter #{n} (${pname}) of type int is deprecated"),
            )?;
            Some(0)
        }
        _ => None,
    })
}

fn is_a(host: &dyn Host, v: &Value, class: &[u8]) -> Option<ObjectHandle> {
    match v {
        Value::Object(h) => {
            let name = class_name(host, h);
            host.reflect_is_a(&name, class).then(|| h.clone())
        }
        _ => None,
    }
}

/// `php_date_period_initialize_from_hash` (no rollback, as php).
fn period_initialize_from_hash(
    host: &mut dyn Host,
    st: &mut PeriodState,
    myht: &PhpArray,
) -> Result<bool, Thrown> {
    let date_entry = |host: &dyn Host, k: &str| -> Result<Option<Option<ObjectHandle>>, ()> {
        match hash_get(myht, k) {
            None => Err(()),
            Some(Value::Null) => Ok(Some(None)),
            Some(v) => match is_a(host, &v, b"DateTimeInterface") {
                Some(h) => Ok(Some(Some(h))),
                None => Err(()),
            },
        }
    };
    let Ok(start) = date_entry(host, "start") else {
        return Ok(false);
    };
    if let Some(Some(h)) = start {
        let Some(t) = time_of(&h) else {
            return Ok(false);
        };
        st.start = Some(t);
        st.start_ce = Some(class_name(host, &h));
    }
    for k in ["end", "current"] {
        let Ok(e) = date_entry(host, k) else {
            return Ok(false);
        };
        if let Some(Some(h)) = e {
            let Some(t) = time_of(&h) else {
                return Ok(false);
            };
            if st.start_ce.is_none() {
                return Ok(false);
            }
            if k == "end" {
                st.end = Some(t);
            } else {
                st.current = Some(t);
            }
        }
    }
    match hash_get(myht, "interval") {
        Some(Value::Object(h)) if class_name(host, &h).eq_ignore_ascii_case(b"DateInterval") => {
            if !interval_state(&h).initialized {
                return Ok(false);
            }
            interval_sync_in(host, &h)?;
            st.interval = interval_state(&h).diff;
        }
        _ => return Ok(false),
    }
    match hash_get(myht, "recurrences") {
        Some(Value::Int(r)) if (0..=i32::MAX as i64).contains(&r) => st.recurrences = r as i32,
        _ => return Ok(false),
    }
    match hash_get(myht, "include_start_date") {
        Some(Value::Bool(b)) => st.include_start_date = b,
        _ => return Ok(false),
    }
    match hash_get(myht, "include_end_date") {
        Some(Value::Bool(b)) => st.include_end_date = b,
        _ => return Ok(false),
    }
    st.initialized = true;
    Ok(true)
}

const PERIOD_INTERNAL: [&str; 7] = PERIOD_PROPS;

fn is_period_prop(name: &[u8]) -> bool {
    PERIOD_PROPS.iter().any(|p| p.as_bytes() == name)
}

/// `date_period_advance`.
fn period_advance(t: &mut ZTime, interval: &RelTime) {
    t.have_relative = 1;
    t.relative = **interval;
    t.sse_uptodate = 0;
    t.update_ts(None);
    t.update_from_sse();
    t.repin();
}

/// `date_period_it_rewind`.
fn period_rewind(host: &dyn Host, ph: &ObjectHandle) -> Result<(), Thrown> {
    let po = match period_obj(ph) {
        Some(p) => p,
        None => {
            set_period(ph, PeriodState::default());
            period_obj(ph).expect("just set")
        }
    };
    let mut st = po.st.borrow_mut();
    st.current = None;
    let Some(start) = st.start.clone() else {
        drop(st);
        return Err(uninitialized_named(host, b"DatePeriod"));
    };
    let mut cur = start;
    if !st.include_start_date {
        if let Some(iv) = st.interval.clone() {
            period_advance(&mut cur, &iv);
        }
    }
    st.current = Some(cur);
    Ok(())
}

/// `date_period_it_has_more`.
fn period_has_more(st: &PeriodState, index: i64) -> bool {
    match (&st.end, &st.current) {
        (Some(end), Some(cur)) => {
            if cur.sse == end.sse {
                if st.include_end_date {
                    cur.us <= end.us
                } else {
                    cur.us < end.us
                }
            } else {
                cur.sse < end.sse
            }
        }
        (Some(_), None) => false,
        (None, _) => index < st.recurrences as i64,
    }
}

fn iterator_op(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ih = obj(args, 0)?;
    let Some(it) = crate::object::payload_of::<PeriodIter>(&ih) else {
        return Err(Thrown::error(
            "Error",
            "The InternalIterator object has not been properly initialized",
        ));
    };
    let op = bytes(args, 1);
    let ensure_rewound = |host: &mut dyn Host| -> Result<(), Thrown> {
        if !it.rewound.get() {
            it.rewound.set(true);
            it.index.set(0);
            period_rewind(host, &it.period)?;
            it.current.replace(None);
        }
        Ok(())
    };
    match &op[..] {
        b"current" => {
            ensure_rewound(host)?;
            let st = period_state(&it.period);
            let Some(cur) = st.current.clone() else {
                return Ok(Value::Null);
            };
            let ce = base_date_class(host, st.start_ce.as_deref().unwrap_or(b"DateTime"));
            let v = new_date(host, &ce, cur)?;
            it.current.replace(Some(v.clone()));
            Ok(v)
        }
        b"key" => {
            ensure_rewound(host)?;
            Ok(Value::Int(it.index.get()))
        }
        b"next" => {
            ensure_rewound(host)?;
            if let Some(po) = period_obj(&it.period) {
                let mut st = po.st.borrow_mut();
                if let (Some(iv), Some(cur)) = (st.interval.clone(), st.current.as_mut()) {
                    period_advance(cur, &iv);
                }
            }
            it.index.set(it.index.get() + 1);
            it.current.replace(None);
            Ok(Value::Null)
        }
        b"valid" => {
            ensure_rewound(host)?;
            Ok(Value::Bool(period_has_more(
                &period_state(&it.period),
                it.index.get(),
            )))
        }
        _ => {
            it.rewound.set(true);
            period_rewind(host, &it.period)?;
            it.current.replace(None);
            it.index.set(0);
            Ok(Value::Null)
        }
    }
}

/// The `DatePeriod::__construct` zpp alternatives.
enum PeriodArgs {
    Dates {
        start: ObjectHandle,
        interval: ObjectHandle,
        end: Option<ObjectHandle>,
        recurrences: i64,
        options: i64,
    },
    Iso {
        isostr: Vec<u8>,
        options: i64,
    },
}

fn period_args(host: &mut dyn Host, a: &[Value]) -> Result<Option<PeriodArgs>, Thrown> {
    let n = a.len();
    let opt_long = |host: &mut dyn Host, i: usize, p: &str| -> Result<Option<i64>, Thrown> {
        match a.get(i) {
            None => Ok(Some(0)),
            Some(v) => quiet_long(host, &deref(v), i + 1, p),
        }
    };
    // "OOl|l" and "OOO|l"
    if (3..=4).contains(&n) {
        if let (Some(start), Some(interval)) = (
            is_a(host, &deref(&a[0]), b"DateTimeInterface"),
            is_a(host, &deref(&a[1]), b"DateInterval"),
        ) {
            let third = deref(&a[2]);
            if !matches!(third, Value::Object(_)) {
                if let Some(rec) = quiet_long(host, &third, 3, "end")? {
                    if let Some(options) = opt_long(host, 3, "options")? {
                        return Ok(Some(PeriodArgs::Dates {
                            start,
                            interval,
                            end: None,
                            recurrences: rec,
                            options,
                        }));
                    }
                }
            } else if let Some(end) = is_a(host, &third, b"DateTimeInterface") {
                if let Some(options) = opt_long(host, 3, "options")? {
                    return Ok(Some(PeriodArgs::Dates {
                        start,
                        interval,
                        end: Some(end),
                        recurrences: 0,
                        options,
                    }));
                }
            }
        }
    }
    // "s|l"
    if (1..=2).contains(&n) {
        let isostr = match deref(&a[0]) {
            Value::Str(st) => Some(st.as_bytes().to_vec()),
            _ if host.caller_strict_types() => None,
            v @ (Value::Int(_) | Value::Float(_) | Value::Bool(_)) => {
                Some(conv::to_string_bytes(&v))
            }
            Value::Null => {
                host.emit_diagnostic(
                    crate::E_DEPRECATED,
                    "DatePeriod::__construct(): Passing null to parameter #1 ($start) of type string is deprecated",
                )?;
                Some(Vec::new())
            }
            _ => None,
        };
        if let Some(isostr) = isostr {
            if let Some(options) = opt_long(host, 1, "interval")? {
                return Ok(Some(PeriodArgs::Iso { isostr, options }));
            }
        }
    }
    Ok(None)
}

fn period_op(host: &mut dyn Host, op: &[u8], args: &[Value]) -> Result<Option<Value>, Thrown> {
    Ok(Some(match op {
        b"dp_construct" => {
            let h = obj(args, 0)?;
            let a: Vec<Value> = match arg(args, 1) {
                Value::Array(a) => a.iter().map(|(_, v)| v.clone()).collect(),
                _ => Vec::new(),
            };
            let fname = "DatePeriod::__construct";
            let Some(parsed) = period_args(host, &a)? else {
                return Err(Thrown::type_error(
                    "DatePeriod::__construct() accepts (DateTimeInterface, DateInterval, int [, int]), or (DateTimeInterface, DateInterval, DateTime [, int]), or (string [, int]) as arguments",
                ));
            };
            set_period(&h, PeriodState::default());
            let mut st = PeriodState::default();
            let (recurrences, options) = match parsed {
                PeriodArgs::Iso { isostr, options } => {
                    host.emit_diagnostic(
                        crate::E_DEPRECATED,
                        "Calling DatePeriod::__construct(string $isostr, int $options = 0) is deprecated, use DatePeriod::createFromISO8601String() instead",
                    )?;
                    let r = period_init_iso8601(&mut st, b"DateTime", &isostr, fname);
                    match r {
                        Ok(r) => (r, options),
                        Err(e) => {
                            set_period(&h, st);
                            return Err(e);
                        }
                    }
                }
                PeriodArgs::Dates {
                    start,
                    interval,
                    end,
                    recurrences,
                    options,
                } => {
                    let Some(t) = time_of(&start) else {
                        return Err(uninitialized_named(host, b"DateTimeInterface"));
                    };
                    let end_t = match &end {
                        Some(e) => match time_of(e) {
                            Some(t) => Some(t),
                            None => return Err(uninitialized_named(host, b"DateTimeInterface")),
                        },
                        None => None,
                    };
                    if !interval_state(&interval).initialized {
                        return Err(uninitialized(host, &interval));
                    }
                    interval_sync_in(host, &interval)?;
                    st.start = Some(t);
                    st.start_ce = Some(class_name(host, &start));
                    st.interval = interval_state(&interval).diff;
                    st.end = end_t;
                    (recurrences, options)
                }
            };
            let r = period_init_finish(&mut st, options, recurrences, fname);
            set_period(&h, st);
            r?;
            Value::Null
        }
        b"dp_from_iso" => {
            let h = instantiate(host, &bytes(args, 0))?;
            let fname = "DatePeriod::createFromISO8601String";
            let mut st = PeriodState::default();
            let r = period_init_iso8601(&mut st, b"DateTimeImmutable", &bytes(args, 1), fname)
                .and_then(|rec| period_init_finish(&mut st, long(args, 2), rec, fname));
            set_period(&h, st);
            r?;
            Value::Object(h)
        }
        b"dp_start" => {
            let h = obj(args, 0)?;
            let st = period_state(&h);
            let Some(start) = st.start.as_ref() else {
                return Err(uninitialized(host, &h));
            };
            period_datetime(host, Some(start), st.start_ce.as_deref())?
        }
        b"dp_end" => {
            let st = period_state(&obj(args, 0)?);
            period_datetime(host, st.end.as_ref(), st.start_ce.as_deref())?
        }
        b"dp_interval" => {
            let h = obj(args, 0)?;
            let st = period_state(&h);
            let Some(iv) = st.interval.as_ref() else {
                return Err(uninitialized(host, &h));
            };
            period_interval(host, Some(iv))?
        }
        b"dp_recurrences" => {
            let st = period_state(&obj(args, 0)?);
            let r =
                st.recurrences as i64 - st.include_start_date as i64 - st.include_end_date as i64;
            if r == 0 {
                Value::Null
            } else {
                Value::Int(r)
            }
        }
        b"dp_serialize" => {
            let h = obj(args, 0)?;
            let st = period_state(&h);
            if st.start.is_none() {
                return Err(uninitialized(host, &h));
            }
            let mut a = PhpArray::new();
            date_period_object_to_hash(host, &st, &mut a)?;
            add_common_properties(host, &mut a, &h, |_| false);
            Value::Array(a)
        }
        b"dp_unserialize" | b"dp_wakeup" => {
            let h = obj(args, 0)?;
            let unserialize = op == b"dp_unserialize";
            let myht = if unserialize {
                match arg(args, 1) {
                    Value::Array(a) => a,
                    _ => PhpArray::new(),
                }
            } else {
                wakeup_deprecated(host, "DatePeriod")?;
                h.borrow()
                    .properties
                    .iter()
                    .map(|(k, v)| (ArrayKey::Str(k.clone()), v.clone()))
                    .collect()
            };
            let mut st = period_state(&h);
            let ok = period_initialize_from_hash(host, &mut st, &myht)?;
            set_period(&h, st);
            if !ok {
                return Err(Thrown::error(
                    "Error",
                    "Invalid serialization data for DatePeriod object",
                ));
            }
            restore_custom_properties(host, &h, &myht, &PERIOD_INTERNAL)?;
            Value::Null
        }
        b"dp_set_state" => {
            let h = instantiate(host, b"DatePeriod")?;
            let myht = match arg(args, 0) {
                Value::Array(a) => a,
                _ => PhpArray::new(),
            };
            let mut st = PeriodState::default();
            let ok = period_initialize_from_hash(host, &mut st, &myht)?;
            set_period(&h, st);
            if !ok {
                return Err(Thrown::error(
                    "Error",
                    "Invalid serialization data for DatePeriod object",
                ));
            }
            Value::Object(h)
        }
        b"dp_iterator" => {
            let h = obj(args, 0)?;
            let it = instantiate(host, b"InternalIterator")?;
            crate::object::set_payload(
                &it,
                Rc::new(PeriodIter {
                    period: h,
                    index: std::cell::Cell::new(0),
                    current: std::cell::RefCell::new(None),
                    rewound: std::cell::Cell::new(false),
                }),
            );
            Value::Object(it)
        }
        b"dp_internal" => Value::Bool(is_period_prop(&bytes(args, 0))),
        b"dp_get" => {
            let st = period_state(&obj(args, 0)?);
            period_prop(host, &st, &s(&bytes(args, 1)))?
        }
        b"dp_set" => {
            let name = bytes(args, 1);
            if is_period_prop(&name) {
                return Err(Thrown::error(
                    "Error",
                    format!("Cannot modify readonly property DatePeriod::${}", s(&name)),
                ));
            }
            // Another name: the prelude's __set creates it, and the engine
            // raises the dynamic-property deprecation.
            Value::Null
        }
        b"dp_isset" => {
            let h = obj(args, 0)?;
            let name = bytes(args, 1);
            if !is_period_prop(&name) {
                return Ok(Some(Value::Null));
            }
            let st = period_state(&h);
            if !st.initialized {
                return Ok(Some(Value::Bool(false)));
            }
            let v = period_prop(host, &st, &s(&name))?;
            Value::Bool(!matches!(v, Value::Null))
        }
        b"dp_unset" => {
            let h = obj(args, 0)?;
            let name = bytes(args, 1);
            if is_period_prop(&name) {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot unset {}::${}",
                        s(c_str(&class_name(host, &h))),
                        s(&name)
                    ),
                ));
            }
            Value::Null
        }
        _ => return Ok(None),
    }))
}

/// `__date($op, ...$args)`: the body of every date class method and of the
/// procedural functions that alias them.
pub(crate) fn fn_date(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = bytes(args, 0);
    let rest = &args[1.min(args.len())..];
    if op == b"it" {
        return iterator_op(host, rest);
    }
    for f in [date_op, timezone_op, interval_op, period_op] {
        if let Some(v) = f(host, &op, rest)? {
            return Ok(v);
        }
    }
    Err(Thrown::error(
        "Error",
        format!("unknown date operation {}", s(&op)),
    ))
}

#[cfg(test)]
mod tests;
