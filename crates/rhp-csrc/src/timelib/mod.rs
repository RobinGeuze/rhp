//! timelib 2022.17, php-src's date library (`ext/date/lib`), with php's
//! embedded tz database. [`sys`] declares the C; the types here own what
//! timelib allocates and free it as php_date.c does. Strings go in as php
//! passes a `zend_string`: with their length and a NUL after it (the format
//! parser stops at a NUL, not at the length). Where timelib takes a plain C
//! string, it gets the bytes up to the first NUL, as php passes `ZSTR_VAL`.
//!
//! A [`Time`] does not own the `timelib_tzinfo` its `tz_info` points to
//! (timelib never frees it either): keep the [`Tz`] alive while a time uses
//! it.

pub mod sys;

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ops::{Deref, DerefMut};
use std::ptr::{self, NonNull};

pub use sys::{
    timelib_rel_time, timelib_sll, timelib_time, timelib_tzinfo, TIMELIB_NONE, TIMELIB_NO_CLONE,
    TIMELIB_OVERRIDE_TIME, TIMELIB_SPECIAL_FIRST_DAY_OF_MONTH, TIMELIB_SPECIAL_LAST_DAY_OF_MONTH,
    TIMELIB_SPECIAL_WEEKDAY, TIMELIB_UNSET, TIMELIB_ZONETYPE_ABBR, TIMELIB_ZONETYPE_ID,
    TIMELIB_ZONETYPE_NONE, TIMELIB_ZONETYPE_OFFSET,
};

/// The bytes of a C string timelib owns.
///
/// # Safety
///
/// `p` is NULL or a NUL-terminated string that outlives `'a`.
unsafe fn bytes<'a>(p: *const c_char) -> Option<&'a [u8]> {
    (!p.is_null()).then(|| CStr::from_ptr(p).to_bytes())
}

/// What C sees of a php string passed as `char *`: up to the first NUL.
fn c_string(s: &[u8]) -> CString {
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    CString::new(&s[..end]).expect("no NUL before end")
}

/// A copy of `s` with the NUL a `zend_string` has after its bytes.
fn zstr(s: &[u8]) -> Vec<u8> {
    let mut z = Vec::with_capacity(s.len() + 1);
    z.extend_from_slice(s);
    z.push(0);
    z
}

/// `TIMELIB_ASCII_VERSION`.
pub fn version() -> &'static str {
    // SAFETY: a string literal in glue/timelib.c.
    unsafe { CStr::from_ptr(sys::rhp_timelib_ascii_version()) }
        .to_str()
        .expect("ASCII")
}

/// A timezone database (`timelib_tzdb`).
#[derive(Clone, Copy)]
pub struct Tzdb(&'static sys::timelib_tzdb);

/// One identifier of a [`Tzdb`].
pub struct TzdbEntry {
    pub id: &'static [u8],
    /// The entry's backwards-compatibility flag (`data[pos + 4]`): set for
    /// the identifiers `timezone_identifiers_list()` lists by default.
    pub bc: bool,
    /// The two-letter country code (`data[pos + 5..7]`).
    pub country_code: [u8; 2],
}

impl Tzdb {
    /// `timelib_builtin_db()`: php's embedded database.
    pub fn builtin() -> Tzdb {
        // SAFETY: a pointer to const data in the archive.
        Tzdb(unsafe { &*sys::timelib_builtin_db() })
    }

    pub fn as_ptr(self) -> *const sys::timelib_tzdb {
        self.0
    }

    /// `tzdb->version`.
    pub fn version(self) -> &'static [u8] {
        // SAFETY: a static string.
        unsafe { bytes(self.0.version) }.unwrap_or_default()
    }

    /// `timelib_timezone_id_is_valid` (case-insensitive).
    pub fn is_valid_id(self, id: &[u8]) -> bool {
        let id = c_string(id);
        // SAFETY: both pointers are valid for the call.
        unsafe { sys::timelib_timezone_id_is_valid(id.as_ptr(), self.0) != 0 }
    }

    /// `timelib_timezone_identifiers_list`, in the database's order.
    pub fn identifiers(self) -> impl Iterator<Item = TzdbEntry> {
        let mut count: c_int = 0;
        // SAFETY: the index is static and holds `count` entries; each
        // entry's position points at its record in the static data, whose
        // header holds the flag and the country code.
        let index = unsafe { sys::timelib_timezone_identifiers_list(self.0, &mut count) };
        let data = self.0.data;
        (0..count.max(0) as usize).map(move |i| unsafe {
            let e = &*index.add(i);
            let rec = data.add(e.pos as usize);
            TzdbEntry {
                id: bytes(e.id).unwrap_or_default(),
                bc: *rec.add(4) == 1,
                country_code: [*rec.add(5), *rec.add(6)],
            }
        })
    }
}

/// An owned `timelib_tzinfo`.
pub struct Tz(NonNull<sys::timelib_tzinfo>);

impl Tz {
    /// `timelib_parse_tzfile`: the zone, or timelib's error code. The
    /// zone's name is `name` as given (the lookup ignores case).
    pub fn parse(name: &[u8], db: Tzdb) -> Result<Tz, c_int> {
        let name = c_string(name);
        let mut err: c_int = 0;
        // SAFETY: valid pointers; the result is owned by the caller.
        let p = unsafe { sys::timelib_parse_tzfile(name.as_ptr(), db.as_ptr(), &mut err) };
        NonNull::new(p).map(Tz).ok_or(err)
    }

    pub fn as_ptr(&self) -> *mut sys::timelib_tzinfo {
        self.0.as_ptr()
    }

    pub fn info(&self) -> &sys::timelib_tzinfo {
        // SAFETY: owned and alive.
        unsafe { self.0.as_ref() }
    }

    /// `tz->name`.
    pub fn name(&self) -> &[u8] {
        // SAFETY: timelib allocates the name with the zone.
        unsafe { bytes(self.info().name) }.unwrap_or_default()
    }

    /// `timelib_get_time_zone_info`: the offset in effect at `ts`.
    pub fn offset_at(&self, ts: timelib_sll) -> TimeOffset {
        // SAFETY: the zone is alive; the result is owned by the caller.
        unsafe { TimeOffset::from_raw(sys::timelib_get_time_zone_info(ts, self.as_ptr())) }
    }
}

impl Drop for Tz {
    fn drop(&mut self) {
        // SAFETY: owned; freed once.
        unsafe { sys::timelib_tzinfo_dtor(self.as_ptr()) }
    }
}

/// An owned `timelib_time`. It derefs to the struct, as php_date.c reads
/// and writes its fields.
pub struct Time(NonNull<sys::timelib_time>);

impl Time {
    /// `timelib_time_ctor`: all zero.
    pub fn new() -> Time {
        // SAFETY: never NULL (emalloc).
        unsafe { Time::from_raw(sys::timelib_time_ctor()) }
    }

    /// Owns a time timelib allocated.
    ///
    /// # Safety
    ///
    /// `p` came from timelib's allocator and nothing else frees it.
    pub unsafe fn from_raw(p: *mut sys::timelib_time) -> Time {
        Time(NonNull::new(p).expect("timelib returned a time"))
    }

    pub fn as_ptr(&self) -> *mut sys::timelib_time {
        self.0.as_ptr()
    }

    /// Gives up ownership (for `timelib_time_dtor` elsewhere).
    pub fn into_raw(self) -> *mut sys::timelib_time {
        let p = self.as_ptr();
        std::mem::forget(self);
        p
    }

    /// `tz_abbr`.
    pub fn tz_abbr(&self) -> Option<&[u8]> {
        // SAFETY: the time owns its abbreviation.
        unsafe { bytes(self.tz_abbr) }
    }

    /// The name of `tz_info`.
    pub fn tz_name(&self) -> Option<&[u8]> {
        // SAFETY: the zone outlives the time (module docs).
        unsafe { self.tz_info.as_ref().and_then(|tz| bytes(tz.name)) }
    }

    /// Replaces `tz_abbr` with an uppercased copy of `abbr`
    /// (`timelib_time_tz_abbr_update`).
    pub fn set_tz_abbr(&mut self, abbr: &[u8]) {
        let abbr = c_string(abbr);
        // SAFETY: the time is alive; timelib frees the old abbreviation.
        unsafe { sys::timelib_time_tz_abbr_update(self.as_ptr(), abbr.as_ptr()) }
    }

    /// Attaches `tz` as a zone identifier (php_date.c's `t->tz_info = tzi;
    /// t->zone_type = TIMELIB_ZONETYPE_ID;`).
    pub fn set_zone_id(&mut self, tz: &Tz) {
        self.tz_info = tz.as_ptr();
        self.zone_type = TIMELIB_ZONETYPE_ID;
    }

    pub fn unixtime2local(&mut self, ts: timelib_sll) {
        // SAFETY: the time and its zone are alive.
        unsafe { sys::timelib_unixtime2local(self.as_ptr(), ts) }
    }

    pub fn unixtime2gmt(&mut self, ts: timelib_sll) {
        // SAFETY: the time is alive.
        unsafe { sys::timelib_unixtime2gmt(self.as_ptr(), ts) }
    }

    pub fn update_from_sse(&mut self) {
        // SAFETY: the time and its zone are alive.
        unsafe { sys::timelib_update_from_sse(self.as_ptr()) }
    }

    /// `timelib_update_ts(t, tzi)`.
    pub fn update_ts(&mut self, tzi: Option<&Tz>) {
        let tzi = tzi.map_or(ptr::null_mut(), Tz::as_ptr);
        // SAFETY: the time and the zones are alive.
        unsafe { sys::timelib_update_ts(self.as_ptr(), tzi) }
    }

    /// `timelib_fill_holes(t, now, options)`. With [`TIMELIB_NO_CLONE`],
    /// `tz_info` is `now`'s pointer afterwards.
    pub fn fill_holes(&mut self, now: &Time, options: c_int) {
        // SAFETY: both times are alive; timelib only reads `now`.
        unsafe { sys::timelib_fill_holes(self.as_ptr(), now.as_ptr(), options) }
    }

    /// `timelib_date_to_int`: `None` when `sse` doesn't fit a `zend_long`.
    pub fn date_to_int(&self) -> Option<i64> {
        let mut err: c_int = 0;
        // SAFETY: the time is alive; timelib only reads it.
        let v = unsafe { sys::timelib_date_to_int(self.as_ptr(), &mut err) };
        (err == 0).then_some(v)
    }
}

impl Default for Time {
    fn default() -> Time {
        Time::new()
    }
}

impl Clone for Time {
    /// `timelib_time_clone`: copies `tz_abbr`, shares `tz_info`.
    fn clone(&self) -> Time {
        // SAFETY: the time is alive.
        unsafe { Time::from_raw(sys::timelib_time_clone(self.as_ptr())) }
    }
}

impl Deref for Time {
    type Target = sys::timelib_time;
    fn deref(&self) -> &sys::timelib_time {
        // SAFETY: owned and alive.
        unsafe { self.0.as_ref() }
    }
}

impl DerefMut for Time {
    fn deref_mut(&mut self) -> &mut sys::timelib_time {
        // SAFETY: owned and alive.
        unsafe { self.0.as_mut() }
    }
}

impl Drop for Time {
    fn drop(&mut self) {
        // SAFETY: owned; freed once. Leaves tz_info alone.
        unsafe { sys::timelib_time_dtor(self.as_ptr()) }
    }
}

/// An owned `timelib_rel_time`.
pub struct RelTime(NonNull<sys::timelib_rel_time>);

impl RelTime {
    /// `timelib_rel_time_ctor`: all zero.
    pub fn new() -> RelTime {
        // SAFETY: never NULL (emalloc).
        unsafe { RelTime::from_raw(sys::timelib_rel_time_ctor()) }
    }

    /// # Safety
    ///
    /// `p` came from timelib's allocator and nothing else frees it.
    pub unsafe fn from_raw(p: *mut sys::timelib_rel_time) -> RelTime {
        RelTime(NonNull::new(p).expect("timelib returned a relative time"))
    }

    pub fn as_ptr(&self) -> *mut sys::timelib_rel_time {
        self.0.as_ptr()
    }
}

impl Default for RelTime {
    fn default() -> RelTime {
        RelTime::new()
    }
}

impl Clone for RelTime {
    fn clone(&self) -> RelTime {
        // SAFETY: alive.
        unsafe { RelTime::from_raw(sys::timelib_rel_time_clone(self.as_ptr())) }
    }
}

impl Deref for RelTime {
    type Target = sys::timelib_rel_time;
    fn deref(&self) -> &sys::timelib_rel_time {
        // SAFETY: owned and alive.
        unsafe { self.0.as_ref() }
    }
}

impl DerefMut for RelTime {
    fn deref_mut(&mut self) -> &mut sys::timelib_rel_time {
        // SAFETY: owned and alive.
        unsafe { self.0.as_mut() }
    }
}

impl Drop for RelTime {
    fn drop(&mut self) {
        // SAFETY: owned; freed once.
        unsafe { sys::timelib_rel_time_dtor(self.as_ptr()) }
    }
}

/// An owned `timelib_time_offset`.
pub struct TimeOffset(NonNull<sys::timelib_time_offset>);

impl TimeOffset {
    /// `timelib_time_offset_ctor`: all zero, no abbreviation.
    pub fn new() -> TimeOffset {
        // SAFETY: never NULL (emalloc).
        unsafe { TimeOffset::from_raw(sys::timelib_time_offset_ctor()) }
    }

    /// # Safety
    ///
    /// `p` came from timelib's allocator and nothing else frees it.
    pub unsafe fn from_raw(p: *mut sys::timelib_time_offset) -> TimeOffset {
        TimeOffset(NonNull::new(p).expect("timelib returned an offset"))
    }

    /// Replaces `abbr` with a copy of `abbr`.
    pub fn set_abbr(&mut self, abbr: &[u8]) {
        let abbr = c_string(abbr);
        // SAFETY: timelib's allocator for memory timelib frees.
        unsafe {
            let o = self.0.as_mut();
            if !o.abbr.is_null() {
                sys::timelib_free(o.abbr as *mut c_void);
            }
            o.abbr = sys::timelib_strdup(abbr.as_ptr());
        }
    }

    /// `abbr` (empty when unset).
    pub fn abbr(&self) -> &[u8] {
        // SAFETY: the offset owns its abbreviation.
        unsafe { bytes(self.abbr) }.unwrap_or_default()
    }
}

impl Default for TimeOffset {
    fn default() -> TimeOffset {
        TimeOffset::new()
    }
}

impl Deref for TimeOffset {
    type Target = sys::timelib_time_offset;
    fn deref(&self) -> &sys::timelib_time_offset {
        // SAFETY: owned and alive.
        unsafe { self.0.as_ref() }
    }
}

impl DerefMut for TimeOffset {
    fn deref_mut(&mut self) -> &mut sys::timelib_time_offset {
        // SAFETY: owned and alive.
        unsafe { self.0.as_mut() }
    }
}

impl Drop for TimeOffset {
    fn drop(&mut self) {
        // SAFETY: owned; freed once.
        unsafe { sys::timelib_time_offset_dtor(self.0.as_ptr()) }
    }
}

/// An owned `timelib_error_container`.
pub struct Errors(NonNull<sys::timelib_error_container>);

/// One warning or error.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    pub code: c_int,
    pub position: c_int,
    pub character: u8,
    pub message: &'a [u8],
}

impl Errors {
    /// # Safety
    ///
    /// `p` came from timelib and nothing else frees it.
    unsafe fn from_raw(p: *mut sys::timelib_error_container) -> Errors {
        Errors(NonNull::new(p).expect("timelib returned an error container"))
    }

    fn raw(&self) -> &sys::timelib_error_container {
        // SAFETY: owned and alive.
        unsafe { self.0.as_ref() }
    }

    pub fn error_count(&self) -> usize {
        self.raw().error_count.max(0) as usize
    }

    pub fn warning_count(&self) -> usize {
        self.raw().warning_count.max(0) as usize
    }

    pub fn errors(&self) -> impl Iterator<Item = Message<'_>> {
        messages(self.raw().error_messages, self.error_count())
    }

    pub fn warnings(&self) -> impl Iterator<Item = Message<'_>> {
        messages(self.raw().warning_messages, self.warning_count())
    }
}

fn messages<'a>(
    p: *const sys::timelib_error_message,
    n: usize,
) -> impl Iterator<Item = Message<'a>> {
    (0..n).map(move |i| {
        // SAFETY: the container holds `n` messages, alive as long as it.
        let m = unsafe { &*p.add(i) };
        Message {
            code: m.error_code,
            position: m.position,
            character: m.character as u8,
            message: unsafe { bytes(m.message) }.unwrap_or_default(),
        }
    })
}

impl Drop for Errors {
    fn drop(&mut self) {
        // SAFETY: owned; freed once.
        unsafe { sys::timelib_error_container_dtor(self.0.as_ptr()) }
    }
}

/// `timelib_strtotime(s, len, &errors, tzdb, wrapper)`. The time comes back
/// even when parsing failed, as timelib returns it.
pub fn strtotime(s: &[u8], db: Tzdb, wrapper: sys::timelib_tz_get_wrapper) -> (Time, Errors) {
    let z = zstr(s);
    let mut errors = ptr::null_mut();
    // SAFETY: `z` holds `len` bytes and a NUL; timelib returns owned
    // results.
    unsafe {
        let t = sys::timelib_strtotime(
            z.as_ptr() as *const c_char,
            s.len(),
            &mut errors,
            db.as_ptr(),
            wrapper,
        );
        (Time::from_raw(t), Errors::from_raw(errors))
    }
}

/// `timelib_parse_from_format(format, s, len, &errors, tzdb, wrapper)`.
pub fn parse_from_format(
    format: &[u8],
    s: &[u8],
    db: Tzdb,
    wrapper: sys::timelib_tz_get_wrapper,
) -> (Time, Errors) {
    let format = c_string(format);
    let z = zstr(s);
    let mut errors = ptr::null_mut();
    // SAFETY: as for `strtotime`.
    unsafe {
        let t = sys::timelib_parse_from_format(
            format.as_ptr(),
            z.as_ptr() as *const c_char,
            s.len(),
            &mut errors,
            db.as_ptr(),
            wrapper,
        );
        (Time::from_raw(t), Errors::from_raw(errors))
    }
}

/// What `timelib_strtointerval` parsed from an ISO 8601 interval
/// (`recurrences` is 0 without an `R` part).
pub struct Interval {
    pub begin: Option<Time>,
    pub end: Option<Time>,
    pub period: Option<RelTime>,
    pub recurrences: c_int,
    pub errors: Errors,
}

/// `timelib_strtointerval(s, len, ...)`.
pub fn strtointerval(s: &[u8]) -> Interval {
    let (mut begin, mut end, mut period) = (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    // Written only when the string has an R part; php starts from 0.
    let mut recurrences: c_int = 0;
    let z = zstr(s);
    let mut errors = ptr::null_mut();
    // SAFETY: `z` holds `len` bytes and a NUL; the outputs are owned
    // results.
    unsafe {
        sys::timelib_strtointerval(
            z.as_ptr() as *const c_char,
            s.len(),
            &mut begin,
            &mut end,
            &mut period,
            &mut recurrences,
            &mut errors,
        );
        Interval {
            begin: (!begin.is_null()).then(|| Time::from_raw(begin)),
            end: (!end.is_null()).then(|| Time::from_raw(end)),
            period: (!period.is_null()).then(|| RelTime::from_raw(period)),
            recurrences,
            errors: Errors::from_raw(errors),
        }
    }
}

/// `timelib_timezone_id_from_abbr(abbr, gmtoffset, isdst)`.
pub fn timezone_id_from_abbr(abbr: &[u8], gmtoffset: i64, isdst: c_int) -> Option<&'static [u8]> {
    let abbr = c_string(abbr);
    // SAFETY: the result is NULL or static.
    unsafe {
        bytes(sys::timelib_timezone_id_from_abbr(
            abbr.as_ptr(),
            gmtoffset,
            isdst,
        ))
    }
}

/// One row of `timelib_timezone_abbreviations_list()`.
pub struct Abbreviation {
    pub name: &'static [u8],
    pub dst: bool,
    pub gmtoffset: f32,
    pub tz_id: Option<&'static [u8]>,
}

/// `timelib_timezone_abbreviations_list()`, up to its NULL-named end.
pub fn abbreviations() -> impl Iterator<Item = Abbreviation> {
    // SAFETY: a static table ending in a row with a NULL name.
    let mut p = unsafe { sys::timelib_timezone_abbreviations_list() };
    std::iter::from_fn(move || unsafe {
        let e = &*p;
        let name = bytes(e.name)?;
        p = p.add(1);
        Some(Abbreviation {
            name,
            dst: e.type_ != 0,
            gmtoffset: e.gmtoffset,
            tz_id: bytes(e.full_tz_name),
        })
    })
}

/// `timelib_get_error_message(code)`.
pub fn error_message(code: c_int) -> &'static [u8] {
    // SAFETY: static strings.
    unsafe { bytes(sys::timelib_get_error_message(code)) }.unwrap_or_default()
}

/// timelib_private.h's `timelib_is_leap` (on the `int` php_date.c passes).
pub fn is_leap(y: i32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

// dow.c: pure functions.

pub fn day_of_week(y: timelib_sll, m: timelib_sll, d: timelib_sll) -> timelib_sll {
    // SAFETY: no pointers.
    unsafe { sys::timelib_day_of_week(y, m, d) }
}

pub fn iso_day_of_week(y: timelib_sll, m: timelib_sll, d: timelib_sll) -> timelib_sll {
    // SAFETY: no pointers.
    unsafe { sys::timelib_iso_day_of_week(y, m, d) }
}

pub fn day_of_year(y: timelib_sll, m: timelib_sll, d: timelib_sll) -> timelib_sll {
    // SAFETY: no pointers.
    unsafe { sys::timelib_day_of_year(y, m, d) }
}

pub fn days_in_month(y: timelib_sll, m: timelib_sll) -> timelib_sll {
    // SAFETY: no pointers.
    unsafe { sys::timelib_days_in_month(y, m) }
}

pub fn daynr_from_weeknr(iy: timelib_sll, iw: timelib_sll, id: timelib_sll) -> timelib_sll {
    // SAFETY: no pointers.
    unsafe { sys::timelib_daynr_from_weeknr(iy, iw, id) }
}

/// `timelib_isoweek_from_date`: (ISO week, ISO year).
pub fn isoweek_from_date(y: timelib_sll, m: timelib_sll, d: timelib_sll) -> (i64, i64) {
    let (mut iw, mut iy) = (0, 0);
    // SAFETY: valid out-pointers.
    unsafe { sys::timelib_isoweek_from_date(y, m, d, &mut iw, &mut iy) };
    (iw, iy)
}

pub fn valid_date(y: timelib_sll, m: timelib_sll, d: timelib_sll) -> bool {
    // SAFETY: no pointers.
    unsafe { sys::timelib_valid_date(y, m, d) != 0 }
}

pub fn valid_time(h: timelib_sll, i: timelib_sll, s: timelib_sll) -> bool {
    // SAFETY: no pointers.
    unsafe { sys::timelib_valid_time(h, i, s) != 0 }
}

#[cfg(test)]
mod tests;
