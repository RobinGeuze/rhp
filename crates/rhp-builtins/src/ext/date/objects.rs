//! The date classes' native state and object handlers: php_date.c's
//! `php_date_obj`, `php_timezone_obj`, `php_interval_obj` and
//! `php_period_obj`, kept in the object's payload, and the handler tables
//! php gives them. The methods are in `classes.rs`.
//!
//! A DateInterval's fields are also real properties (php's
//! `get_properties` writes them into the property table, and its
//! read/write handlers map them to the struct): the payload is filled from
//! them before it is read ([`interval_sync_in`]) and they are rewritten
//! from it after it changes ([`interval_sync_out`]).

use std::cell::{Cell, RefCell};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use rhp_csrc::timelib::{self, sys, RelTime, Time, Tz, TIMELIB_UNSET};
use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, PhpStr, Value};

use super::format::date_format;
use crate::object::{ClassHandlers, CloneObj, ObjectHandlers, PropPurpose};
use crate::{Host, Thrown};

/// A `timelib_time` with the zone its `tz_info` points to kept alive.
pub(crate) struct ZTime {
    t: Time,
    zone: Option<Rc<Tz>>,
}

impl ZTime {
    /// Takes `t`, pinning the cached zone its `tz_info` points to.
    pub(crate) fn new(t: Time) -> ZTime {
        ZTime::with_zone(t, None)
    }

    /// Takes `t`, pinning `zone` if `tz_info` points to it, else the cached
    /// zone. A zone held elsewhere stays pinned after the per-request cache
    /// was reset.
    pub(crate) fn with_zone(t: Time, zone: Option<&Rc<Tz>>) -> ZTime {
        let mut z = ZTime {
            t,
            zone: zone.cloned(),
        };
        z.repin();
        z
    }

    /// The zone `tz_info` points to.
    pub(crate) fn zone(&self) -> Option<&Rc<Tz>> {
        self.zone.as_ref()
    }

    /// Pins `zone`, which `tz_info` now points to.
    pub(crate) fn set_zone(&mut self, zone: &Rc<Tz>) {
        self.zone = Some(zone.clone());
        self.repin();
    }

    /// Pins the zone again after `tz_info` changed.
    pub(crate) fn repin(&mut self) {
        let same = self
            .zone
            .as_ref()
            .is_some_and(|z| std::ptr::eq(z.as_ptr(), self.t.tz_info));
        if !same {
            self.zone = super::cached_zone(self.t.tz_info);
        }
        debug_assert!(
            self.zone.is_some() || self.t.tz_info.is_null(),
            "a date's tz_info points to a zone nothing keeps alive"
        );
    }
}

impl Clone for ZTime {
    /// php's struct copy: `tz_abbr` duplicated, `tz_info` shared.
    fn clone(&self) -> ZTime {
        ZTime {
            t: self.t.clone(),
            zone: self.zone.clone(),
        }
    }
}

impl Deref for ZTime {
    type Target = Time;
    fn deref(&self) -> &Time {
        &self.t
    }
}

impl DerefMut for ZTime {
    fn deref_mut(&mut self) -> &mut Time {
        &mut self.t
    }
}

/// `php_date_obj`: `time` is NULL until the object is initialized.
#[derive(Default)]
pub(crate) struct DateObj {
    pub(crate) time: RefCell<Option<ZTime>>,
}

/// `php_timezone_obj`'s `type` and `tzi` (`None`: not initialized).
#[derive(Clone)]
pub(crate) enum TzKind {
    Id(Rc<Tz>),
    Offset(i64),
    Abbr {
        utc_offset: i64,
        dst: i32,
        abbr: Vec<u8>,
    },
}

impl TzKind {
    pub(crate) fn zone_type(&self) -> u32 {
        match self {
            TzKind::Offset(_) => timelib::TIMELIB_ZONETYPE_OFFSET,
            TzKind::Abbr { .. } => timelib::TIMELIB_ZONETYPE_ABBR,
            TzKind::Id(_) => timelib::TIMELIB_ZONETYPE_ID,
        }
    }

    /// `set_timezone_from_timelib_time`; `zone` is the zone a [`ZTime`]
    /// pins.
    pub(crate) fn from_time(t: &Time, zone: Option<&Rc<Tz>>) -> Option<TzKind> {
        Some(match t.zone_type {
            timelib::TIMELIB_ZONETYPE_ID => TzKind::Id(match zone {
                Some(z) if std::ptr::eq(z.as_ptr(), t.tz_info) => z.clone(),
                _ => super::cached_zone(t.tz_info)?,
            }),
            timelib::TIMELIB_ZONETYPE_OFFSET => TzKind::Offset(t.z as i64),
            timelib::TIMELIB_ZONETYPE_ABBR => TzKind::Abbr {
                utc_offset: t.z as i64,
                dst: t.dst,
                abbr: t.tz_abbr().unwrap_or_default().to_vec(),
            },
            _ => return None,
        })
    }

    /// `php_timezone_to_string`.
    pub(crate) fn to_value(&self) -> Value {
        match self {
            TzKind::Id(tz) => Value::str(tz.name()),
            TzKind::Offset(o) => Value::str(tz_offset_str(*o)),
            TzKind::Abbr { abbr, .. } => Value::str(abbr.clone()),
        }
    }
}

#[derive(Default)]
pub(crate) struct TzObj {
    pub(crate) tz: RefCell<Option<TzKind>>,
}

pub(crate) const PHP_DATE_CIVIL: i32 = 1;
pub(crate) const PHP_DATE_WALL: i32 = 2;

/// `php_interval_obj`.
#[derive(Clone, Default)]
pub(crate) struct IntervalState {
    pub(crate) diff: Option<RelTime>,
    pub(crate) initialized: bool,
    pub(crate) civil_or_wall: i32,
    pub(crate) from_string: bool,
    pub(crate) date_string: Option<Vec<u8>>,
}

#[derive(Default)]
pub(crate) struct IntervalObj {
    pub(crate) st: RefCell<IntervalState>,
}

/// `php_period_obj`. `start_ce` is the start date's class.
#[derive(Clone, Default)]
pub(crate) struct PeriodState {
    pub(crate) start: Option<ZTime>,
    pub(crate) start_ce: Option<Vec<u8>>,
    pub(crate) current: Option<ZTime>,
    pub(crate) end: Option<ZTime>,
    pub(crate) interval: Option<RelTime>,
    pub(crate) recurrences: i32,
    pub(crate) initialized: bool,
    pub(crate) include_start_date: bool,
    pub(crate) include_end_date: bool,
}

#[derive(Default)]
pub(crate) struct PeriodObj {
    pub(crate) st: RefCell<PeriodState>,
}

/// php's `zend_internal_iterator` over a DatePeriod's `date_period_it`.
pub(crate) struct PeriodIter {
    pub(crate) period: ObjectHandle,
    pub(crate) index: Cell<i64>,
    pub(crate) current: RefCell<Option<Value>>,
    pub(crate) rewound: Cell<bool>,
}

// --- handler tables ---------------------------------------------------------

/// DateTime and DateTimeImmutable (one table: their compare handlers are
/// the same function, which `ZEND_COMPARE_OBJECTS_FALLBACK` compares).
static DATE_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(date_object_get_properties_for),
    clone_obj: CloneObj::Custom(date_object_clone_date),
    compare: Some(date_object_compare_date),
    ..ObjectHandlers::STD
};

static TIMEZONE_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(date_object_get_properties_for_timezone),
    clone_obj: CloneObj::Custom(date_object_clone_timezone),
    compare: Some(date_object_compare_timezone),
    ..ObjectHandlers::STD
};

static INTERVAL_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(date_object_get_properties_interval),
    clone_obj: CloneObj::Custom(date_object_clone_interval),
    compare: Some(date_interval_compare_objects),
    ..ObjectHandlers::STD
};

static PERIOD_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(date_period_get_properties_for),
    clone_obj: CloneObj::Custom(date_object_clone_period),
    ..ObjectHandlers::STD
};

pub(crate) static CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "DateTime",
        handlers: &DATE_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DateTimeImmutable",
        handlers: &DATE_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DateTimeZone",
        handlers: &TIMEZONE_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DateInterval",
        handlers: &INTERVAL_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "DatePeriod",
        handlers: &PERIOD_HANDLERS,
        create_object: true,
    },
    // Zend's zend_internal_iterator_handlers.
    ClassHandlers {
        class: "InternalIterator",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
];

// --- payload access ---------------------------------------------------------

pub(crate) fn date_obj(h: &ObjectHandle) -> Option<Rc<DateObj>> {
    crate::object::payload_of::<DateObj>(h)
}

pub(crate) fn tz_obj(h: &ObjectHandle) -> Option<Rc<TzObj>> {
    crate::object::payload_of::<TzObj>(h)
}

pub(crate) fn interval_obj(h: &ObjectHandle) -> Option<Rc<IntervalObj>> {
    crate::object::payload_of::<IntervalObj>(h)
}

pub(crate) fn period_obj(h: &ObjectHandle) -> Option<Rc<PeriodObj>> {
    crate::object::payload_of::<PeriodObj>(h)
}

/// The object's time, if initialized.
pub(crate) fn time_of(h: &ObjectHandle) -> Option<ZTime> {
    date_obj(h).and_then(|d| d.time.borrow().clone())
}

/// The object's zone, if initialized.
pub(crate) fn tz_of(h: &ObjectHandle) -> Option<TzKind> {
    tz_obj(h).and_then(|t| t.tz.borrow().clone())
}

/// Sets (or clears) the object's time.
pub(crate) fn set_time(h: &ObjectHandle, t: Option<ZTime>) {
    match date_obj(h) {
        Some(d) => {
            let old = d.time.replace(t);
            drop(old);
        }
        None => {
            let d = DateObj::default();
            *d.time.borrow_mut() = t;
            crate::object::set_payload(h, Rc::new(d));
        }
    }
}

pub(crate) fn set_tz(h: &ObjectHandle, tz: Option<TzKind>) {
    match tz_obj(h) {
        Some(o) => {
            let old = o.tz.replace(tz);
            drop(old);
        }
        None => {
            let o = TzObj::default();
            *o.tz.borrow_mut() = tz;
            crate::object::set_payload(h, Rc::new(o));
        }
    }
}

pub(crate) fn interval_state(h: &ObjectHandle) -> IntervalState {
    interval_obj(h)
        .map(|o| o.st.borrow().clone())
        .unwrap_or_default()
}

pub(crate) fn set_interval(h: &ObjectHandle, st: IntervalState) {
    match interval_obj(h) {
        Some(o) => {
            let old = o.st.replace(st);
            drop(old);
        }
        None => crate::object::set_payload(
            h,
            Rc::new(IntervalObj {
                st: RefCell::new(st),
            }),
        ),
    }
}

pub(crate) fn period_state(h: &ObjectHandle) -> PeriodState {
    period_obj(h)
        .map(|o| o.st.borrow().clone())
        .unwrap_or_default()
}

pub(crate) fn set_period(h: &ObjectHandle, st: PeriodState) {
    match period_obj(h) {
        Some(o) => {
            let old = o.st.replace(st);
            drop(old);
        }
        None => crate::object::set_payload(
            h,
            Rc::new(PeriodObj {
                st: RefCell::new(st),
            }),
        ),
    }
}

/// The class of `h`, by name.
pub(crate) fn class_name(host: &dyn Host, h: &ObjectHandle) -> Vec<u8> {
    let class = h.borrow().class;
    host.class_name(class).unwrap_or_default()
}

const INTERNAL: [&[u8]; 6] = [
    b"DateTimeInterface",
    b"DateTime",
    b"DateTimeImmutable",
    b"DateTimeZone",
    b"DateInterval",
    b"DatePeriod",
];

fn is_internal(name: &[u8]) -> bool {
    INTERNAL.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// `date_throw_uninitialized_error(ce)` for the class named `name`.
pub(crate) fn uninitialized_named(host: &dyn Host, name: &[u8]) -> Thrown {
    let n = String::from_utf8_lossy(super::c_str(name));
    if is_internal(name) {
        return Thrown::error(
            "DateObjectError",
            format!("Object of type {n} has not been correctly initialized by calling parent::__construct() in its constructor"),
        );
    }
    let mut cur = name.to_vec();
    loop {
        let parent = host
            .resolve_class(&cur)
            .and_then(|id| host.class_parent_name(id));
        match parent {
            Some(p) if is_internal(&p) => {
                return Thrown::error(
                    "DateObjectError",
                    format!(
                        "Object of type {n} (inheriting {}) has not been correctly initialized by calling parent::__construct() in its constructor",
                        String::from_utf8_lossy(super::c_str(&p))
                    ),
                );
            }
            Some(p) => cur = p,
            None => {
                return Thrown::error(
                    "DateObjectError",
                    format!("Object of type {n} not been correctly initialized by calling parent::__construct() in its constructor"),
                )
            }
        }
    }
}

/// `DATE_CHECK_INITIALIZED(member, Z_OBJCE_P(obj))`'s error.
pub(crate) fn uninitialized(host: &dyn Host, h: &ObjectHandle) -> Thrown {
    uninitialized_named(host, &class_name(host, h))
}

/// The object's time, or php's uninitialized error.
pub(crate) fn checked_time(host: &dyn Host, h: &ObjectHandle) -> Result<ZTime, Thrown> {
    time_of(h).ok_or_else(|| uninitialized(host, h))
}

/// The object's zone, or php's uninitialized error.
pub(crate) fn checked_tz(host: &dyn Host, h: &ObjectHandle) -> Result<TzKind, Thrown> {
    tz_of(h).ok_or_else(|| uninitialized(host, h))
}

/// `php_date_instantiate` / `object_init_ex`: a new object of the class
/// named `class`, its defaults set and no constructor run.
pub(crate) fn instantiate(host: &mut dyn Host, class: &[u8]) -> Result<ObjectHandle, Thrown> {
    match host.reflect_new_instance(class, Vec::new(), false)? {
        Value::Object(h) => Ok(h),
        _ => Err(Thrown::error("Error", "Cannot instantiate date object")),
    }
}

/// A new object of class `class` holding `t` (`create_date_period_datetime`).
pub(crate) fn new_date(host: &mut dyn Host, class: &[u8], t: ZTime) -> Result<Value, Thrown> {
    let h = instantiate(host, class)?;
    set_time(&h, Some(t));
    Ok(Value::Object(h))
}

/// A new DateInterval holding `st`.
pub(crate) fn new_interval(host: &mut dyn Host, st: IntervalState) -> Result<Value, Thrown> {
    let h = instantiate(host, b"DateInterval")?;
    set_interval(&h, st);
    interval_sync_out(&h);
    Ok(Value::Object(h))
}

fn assoc(a: &mut PhpArray, k: &str, v: Value) {
    a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
}

/// `date_create_tz_offset_str`.
pub(crate) fn tz_offset_str(offset: i64) -> String {
    let offset = offset as i32;
    let seconds = offset % 60;
    let sign = if offset < 0 { '-' } else { '+' };
    let (h, m) = ((offset / 3600).abs(), ((offset % 3600) / 60).abs());
    if seconds == 0 {
        format!("{sign}{h:02}:{m:02}")
    } else {
        format!("{sign}{h:02}:{m:02}:{:02}", seconds.abs())
    }
}

/// `date_object_to_hash`.
pub(crate) fn date_object_to_hash(t: &Time, props: &mut PhpArray) {
    assoc(
        props,
        "date",
        Value::str(date_format(b"x-m-d H:i:s.u", t, true)),
    );
    if t.is_localtime != 0 {
        assoc(props, "timezone_type", Value::Int(t.zone_type as i64));
        let tz = match t.zone_type {
            timelib::TIMELIB_ZONETYPE_ID => Value::str(t.tz_name().unwrap_or_default()),
            timelib::TIMELIB_ZONETYPE_OFFSET => Value::str(tz_offset_str(t.z as i64)),
            timelib::TIMELIB_ZONETYPE_ABBR => Value::str(t.tz_abbr().unwrap_or_default()),
            _ => Value::Null,
        };
        assoc(props, "timezone", tz);
    }
}

/// `date_timezone_object_to_hash`.
pub(crate) fn date_timezone_object_to_hash(tz: &TzKind, props: &mut PhpArray) {
    assoc(props, "timezone_type", Value::Int(tz.zone_type() as i64));
    assoc(props, "timezone", tz.to_value());
}

/// `zend_std_get_properties` as a property-table view.
fn std_props(host: &dyn Host, h: &ObjectHandle) -> PhpArray {
    host.object_props_mangled(h)
}

/// The shared prologue of php's `get_properties_for` handlers: the
/// `__debugInfo()` of a subclass wins for `var_dump`.
fn debug_info_wins(host: &dyn Host, h: &ObjectHandle, purpose: PropPurpose) -> bool {
    let class = h.borrow().class;
    purpose == PropPurpose::Debug && host.class_has_method(class, b"__debuginfo")
}

fn date_object_get_properties_for(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose == PropPurpose::GetObjectVars || debug_info_wins(host, h, purpose) {
        return Ok(None);
    }
    let mut props = std_props(host, h);
    if let Some(t) = time_of(h) {
        date_object_to_hash(&t, &mut props);
    }
    Ok(Some(props))
}

fn date_object_get_properties_for_timezone(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose == PropPurpose::GetObjectVars || debug_info_wins(host, h, purpose) {
        return Ok(None);
    }
    let mut props = std_props(host, h);
    if let Some(tz) = tz_of(h) {
        date_timezone_object_to_hash(&tz, &mut props);
    }
    Ok(Some(props))
}

fn date_object_clone_date(
    _: &mut dyn Host,
    src: &ObjectHandle,
    dst: &ObjectHandle,
) -> Result<(), Thrown> {
    if let Some(t) = time_of(src) {
        set_time(dst, Some(t));
    }
    Ok(())
}

fn date_object_clone_timezone(
    _: &mut dyn Host,
    src: &ObjectHandle,
    dst: &ObjectHandle,
) -> Result<(), Thrown> {
    if let Some(tz) = tz_of(src) {
        set_tz(dst, Some(tz));
    }
    Ok(())
}

fn date_object_clone_interval(
    _: &mut dyn Host,
    src: &ObjectHandle,
    dst: &ObjectHandle,
) -> Result<(), Thrown> {
    if interval_obj(src).is_some() {
        set_interval(dst, interval_state(src));
    }
    Ok(())
}

fn date_object_clone_period(
    _: &mut dyn Host,
    src: &ObjectHandle,
    dst: &ObjectHandle,
) -> Result<(), Thrown> {
    if period_obj(src).is_some() {
        set_period(dst, period_state(src));
    }
    Ok(())
}

fn date_object_compare_date(
    host: &mut dyn Host,
    a: &Value,
    b: &Value,
) -> Result<Option<i32>, Thrown> {
    // ZEND_COMPARE_OBJECTS_FALLBACK
    if !crate::object::has_handlers(host, a, &DATE_HANDLERS)
        || !crate::object::has_handlers(host, b, &DATE_HANDLERS)
    {
        return Ok(None);
    }
    let (Some(ha), Some(hb)) = (crate::object::object_of(a), crate::object::object_of(b)) else {
        return Ok(None);
    };
    let (Some(da), Some(db)) = (date_obj(&ha), date_obj(&hb)) else {
        return Err(incomplete());
    };
    for d in [&da, &db] {
        let mut t = d.time.borrow_mut();
        let Some(t) = t.as_mut() else {
            return Err(incomplete());
        };
        if t.sse_uptodate == 0 {
            let tzi = t.tz_info;
            // SAFETY: the time and its zone are alive.
            unsafe { sys::timelib_update_ts(t.as_ptr(), tzi) };
        }
    }
    let (ta, tb) = (da.time.borrow(), db.time.borrow());
    let (Some(ta), Some(tb)) = (ta.as_ref(), tb.as_ref()) else {
        return Err(incomplete());
    };
    // SAFETY: both times are alive; timelib only reads them.
    Ok(Some(unsafe {
        sys::timelib_time_compare(ta.as_ptr(), tb.as_ptr())
    }))
}

fn incomplete() -> Thrown {
    Thrown::error(
        "DateObjectError",
        "Trying to compare an incomplete DateTime or DateTimeImmutable object",
    )
}

fn date_object_compare_timezone(
    host: &mut dyn Host,
    a: &Value,
    b: &Value,
) -> Result<Option<i32>, Thrown> {
    if !crate::object::has_handlers(host, a, &TIMEZONE_HANDLERS)
        || !crate::object::has_handlers(host, b, &TIMEZONE_HANDLERS)
    {
        return Ok(None);
    }
    let (Some(ha), Some(hb)) = (crate::object::object_of(a), crate::object::object_of(b)) else {
        return Ok(None);
    };
    let (Some(o1), Some(o2)) = (tz_of(&ha), tz_of(&hb)) else {
        return Err(Thrown::error(
            "DateObjectError",
            "Trying to compare uninitialized DateTimeZone objects",
        ));
    };
    if o1.zone_type() != o2.zone_type() {
        return Err(Thrown::error(
            "DateException",
            "Cannot compare two different kinds of DateTimeZone objects",
        ));
    }
    let differ = match (&o1, &o2) {
        (TzKind::Offset(x), TzKind::Offset(y)) => x != y,
        (TzKind::Abbr { abbr: x, .. }, TzKind::Abbr { abbr: y, .. }) => x != y,
        (TzKind::Id(x), TzKind::Id(y)) => x.name() != y.name(),
        _ => true,
    };
    Ok(Some(differ as i32))
}

fn date_interval_compare_objects(
    host: &mut dyn Host,
    a: &Value,
    b: &Value,
) -> Result<Option<i32>, Thrown> {
    if !crate::object::has_handlers(host, a, &INTERVAL_HANDLERS)
        || !crate::object::has_handlers(host, b, &INTERVAL_HANDLERS)
    {
        return Ok(None);
    }
    host.emit_diagnostic(crate::E_WARNING, "Cannot compare DateInterval objects")?;
    Ok(Some(1))
}

// --- DateInterval's properties ------------------------------------------------

const INTERVAL_FIELDS: [&str; 9] = ["y", "m", "d", "h", "i", "s", "f", "invert", "days"];

/// The value php's `date_interval_read_property` gives field `name`.
pub(crate) fn interval_field(diff: &RelTime, name: &str) -> Value {
    let v = match name {
        "y" => diff.y,
        "m" => diff.m,
        "d" => diff.d,
        "h" => diff.h,
        "i" => diff.i,
        "s" => diff.s,
        "f" => return Value::Float(diff.us as f64 / 1000000.0),
        "invert" => diff.invert as i64,
        "days" => diff.days,
        _ => return Value::Null,
    };
    if v == TIMELIB_UNSET {
        Value::Bool(false)
    } else {
        Value::Int(v)
    }
}

/// Writes the payload's fields into the property table, as php's
/// `date_object_get_properties_interval` does.
pub(crate) fn interval_sync_out(h: &ObjectHandle) {
    let st = interval_state(h);
    if !st.initialized {
        return;
    }
    let Some(diff) = st.diff.as_ref() else {
        return;
    };
    let mut o = h.borrow_mut();
    for f in INTERVAL_FIELDS {
        o.properties
            .insert(PhpStr::from(f), interval_field(diff, f));
    }
    o.properties
        .insert(PhpStr::from("from_string"), Value::Bool(st.from_string));
    if st.from_string {
        let ds = st.date_string.clone().unwrap_or_default();
        o.properties
            .insert(PhpStr::from("date_string"), Value::str(ds));
    }
}

/// Reads the fields a script may have written back into the payload, with
/// `date_interval_write_property`'s conversions (`days` is not writable).
pub(crate) fn interval_sync_in(host: &mut dyn Host, h: &ObjectHandle) -> Result<(), Thrown> {
    let Some(obj) = interval_obj(h) else {
        return Ok(());
    };
    if !obj.st.borrow().initialized {
        return Ok(());
    }
    let mut vals = Vec::new();
    {
        let o = h.borrow();
        for f in ["y", "m", "d", "h", "i", "s", "f", "invert"] {
            if let Some(v) = o.properties.get(&PhpStr::from(f)) {
                vals.push((f, rhp_semantics::deref(v)));
            }
        }
    }
    let mut longs = Vec::new();
    for (f, v) in vals {
        let n = if f == "f" {
            let d = match &v {
                Value::Float(d) => *d,
                other => conv::to_float(other),
            };
            dval_to_lval(host, d * 1000000.0)?
        } else {
            crate::argparse::zval_get_long(host, &v)?
        };
        longs.push((f, n));
    }
    let mut st = obj.st.borrow_mut();
    if let Some(diff) = st.diff.as_mut() {
        for (f, n) in longs {
            match f {
                "y" => diff.y = n,
                "m" => diff.m = n,
                "d" => diff.d = n,
                "h" => diff.h = n,
                "i" => diff.i = n,
                "s" => diff.s = n,
                "f" => diff.us = n,
                _ => diff.invert = n as i32,
            }
        }
    }
    Ok(())
}

/// `zend_dval_to_lval`: out of range (or not finite) warns.
pub(crate) fn dval_to_lval(host: &mut dyn Host, d: f64) -> Result<i64, Thrown> {
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    if !d.is_finite() || !(-TWO_63..TWO_63).contains(&d) {
        let shown = String::from_utf8_lossy(&rhp_values::fmt::float_serialize(d)).into_owned();
        host.emit_diagnostic(
            crate::E_WARNING,
            &format!("The float {shown} is not representable as an int, cast occurred"),
        )?;
    }
    Ok(rhp_values::float_to_int(d))
}

/// `date_interval_object_to_hash`.
pub(crate) fn date_interval_object_to_hash(st: &IntervalState, props: &mut PhpArray) {
    if st.from_string {
        assoc(props, "from_string", Value::Bool(true));
        let ds = st.date_string.clone().unwrap_or_default();
        assoc(props, "date_string", Value::str(ds));
        return;
    }
    let Some(diff) = st.diff.as_ref() else {
        return;
    };
    for f in INTERVAL_FIELDS {
        assoc(props, f, interval_field(diff, f));
    }
    assoc(props, "from_string", Value::Bool(false));
}

/// Whether `name` is one of the fields of a from-string interval that php
/// keeps out of its property table.
pub(crate) fn interval_hidden(st: &IntervalState, name: &[u8]) -> bool {
    st.from_string && INTERVAL_FIELDS.iter().any(|f| f.as_bytes() == name)
}

fn date_object_get_properties_interval(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    interval_sync_in(host, h)?;
    interval_sync_out(h);
    let st = interval_state(h);
    let hidden = |k: &ArrayKey| match k {
        ArrayKey::Str(s) => st.initialized && interval_hidden(&st, s.as_bytes()),
        ArrayKey::Int(_) => false,
    };
    let table = if purpose == PropPurpose::GetObjectVars {
        // get_object_vars filters php's get_properties table by the
        // caller's scope.
        match host.object_vars(&Value::Object(h.clone())) {
            Some(Value::Array(vars)) => vars,
            _ => return Ok(None),
        }
    } else if debug_info_wins(host, h, purpose) {
        return Ok(None);
    } else {
        std_props(host, h)
    };
    let mut props = PhpArray::new();
    for (k, v) in table.iter() {
        if !hidden(k) {
            props.insert(k.clone(), v.clone());
        }
    }
    Ok(Some(props))
}

// --- DatePeriod's properties --------------------------------------------------

/// The base date class of `ce` (`get_base_date_class`): DateTime or
/// DateTimeImmutable, or the class itself if neither is an ancestor.
pub(crate) fn base_date_class(host: &dyn Host, ce: &[u8]) -> Vec<u8> {
    let mut cur = ce.to_vec();
    loop {
        if cur.eq_ignore_ascii_case(b"DateTime") || cur.eq_ignore_ascii_case(b"DateTimeImmutable") {
            return cur;
        }
        match host
            .resolve_class(&cur)
            .and_then(|id| host.class_parent_name(id))
        {
            Some(p) => cur = p,
            None => return cur,
        }
    }
}

/// `create_date_period_datetime`.
pub(crate) fn period_datetime(
    host: &mut dyn Host,
    t: Option<&ZTime>,
    ce: Option<&[u8]>,
) -> Result<Value, Thrown> {
    match (t, ce) {
        (Some(t), Some(ce)) => new_date(host, ce, t.clone()),
        _ => Ok(Value::Null),
    }
}

/// `create_date_period_interval`.
pub(crate) fn period_interval(host: &mut dyn Host, iv: Option<&RelTime>) -> Result<Value, Thrown> {
    match iv {
        Some(iv) => new_interval(
            host,
            IntervalState {
                diff: Some(iv.clone()),
                initialized: true,
                ..IntervalState::default()
            },
        ),
        None => Ok(Value::Null),
    }
}

pub(crate) const PERIOD_PROPS: [&str; 7] = [
    "start",
    "current",
    "end",
    "interval",
    "recurrences",
    "include_start_date",
    "include_end_date",
];

/// `date_period_read_property` of one of the virtual properties.
pub(crate) fn period_prop(
    host: &mut dyn Host,
    st: &PeriodState,
    name: &str,
) -> Result<Value, Thrown> {
    let ce = st.start_ce.as_deref();
    Ok(match name {
        "start" => period_datetime(host, st.start.as_ref(), ce)?,
        "current" => period_datetime(host, st.current.as_ref(), ce)?,
        "end" => period_datetime(host, st.end.as_ref(), ce)?,
        "interval" => period_interval(host, st.interval.as_ref())?,
        "recurrences" => Value::Int(st.recurrences as i64),
        "include_start_date" => Value::Bool(st.include_start_date),
        "include_end_date" => Value::Bool(st.include_end_date),
        _ => Value::Null,
    })
}

/// `date_period_object_to_hash`.
pub(crate) fn date_period_object_to_hash(
    host: &mut dyn Host,
    st: &PeriodState,
    props: &mut PhpArray,
) -> Result<(), Thrown> {
    for name in PERIOD_PROPS {
        let v = period_prop(host, st, name)?;
        assoc(props, name, v);
    }
    Ok(())
}

fn date_period_get_properties_for(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if debug_info_wins(host, h, purpose) {
        return Ok(None);
    }
    let mut props = std_props(host, h);
    let st = period_state(h);
    if st.initialized {
        date_period_object_to_hash(host, &st, &mut props)?;
    }
    Ok(Some(props))
}
