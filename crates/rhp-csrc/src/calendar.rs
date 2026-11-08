//! php-src's calendar conversions (ext/calendar's sdncal.h): serial day
//! numbers to and from the Gregorian, Julian, Jewish and French republican
//! calendars, the day of the week, and the name tables. The dates are
//! `int`s in the C, as php passes them: callers truncate a zend_long the way
//! C's implicit conversion does.

use std::ffi::{c_char, c_int, CStr};

type ToSdn = unsafe extern "C" fn(c_int, c_int, c_int) -> i64;
type FromSdn = unsafe extern "C" fn(i64, *mut c_int, *mut c_int, *mut c_int);

// The C symbols carry glue/prefix_calendar.h's rhp_cal_ prefix.
extern "C" {
    fn rhp_cal_SdnToGregorian(sdn: i64, y: *mut c_int, m: *mut c_int, d: *mut c_int);
    fn rhp_cal_GregorianToSdn(y: c_int, m: c_int, d: c_int) -> i64;
    fn rhp_cal_SdnToJulian(sdn: i64, y: *mut c_int, m: *mut c_int, d: *mut c_int);
    fn rhp_cal_JulianToSdn(y: c_int, m: c_int, d: c_int) -> i64;
    fn rhp_cal_SdnToJewish(sdn: i64, y: *mut c_int, m: *mut c_int, d: *mut c_int);
    fn rhp_cal_JewishToSdn(y: c_int, m: c_int, d: c_int) -> i64;
    fn rhp_cal_SdnToFrench(sdn: i64, y: *mut c_int, m: *mut c_int, d: *mut c_int);
    fn rhp_cal_FrenchToSdn(y: c_int, m: c_int, d: c_int) -> i64;
    fn rhp_cal_DayOfWeek(sdn: i64) -> c_int;
    static rhp_cal_MonthNameShort: [*const c_char; 13];
    static rhp_cal_MonthNameLong: [*const c_char; 13];
    static rhp_cal_JewishMonthName: [*const c_char; 14];
    static rhp_cal_JewishMonthNameLeap: [*const c_char; 14];
    static rhp_cal_JewishMonthHebName: [*const c_char; 14];
    static rhp_cal_JewishMonthHebNameLeap: [*const c_char; 14];
    static rhp_cal_FrenchMonthName: [*const c_char; 14];
    static rhp_cal_DayNameShort: [*const c_char; 7];
    static rhp_cal_DayNameLong: [*const c_char; 7];
    static rhp_cal_monthsPerYear: [c_int; 19];
}

/// A calendar of php's conversion table (`cal_name_type_t` order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cal {
    Gregorian,
    Julian,
    Jewish,
    French,
}

/// A name table of sdncal.h.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Names {
    MonthShort,
    MonthLong,
    Jewish,
    JewishLeap,
    JewishHeb,
    JewishHebLeap,
    French,
    DayShort,
    DayLong,
}

impl Cal {
    fn fns(self) -> (ToSdn, FromSdn) {
        match self {
            Cal::Gregorian => (rhp_cal_GregorianToSdn, rhp_cal_SdnToGregorian),
            Cal::Julian => (rhp_cal_JulianToSdn, rhp_cal_SdnToJulian),
            Cal::Jewish => (rhp_cal_JewishToSdn, rhp_cal_SdnToJewish),
            Cal::French => (rhp_cal_FrenchToSdn, rhp_cal_SdnToFrench),
        }
    }

    /// `<Cal>ToSdn(year, month, day)`: 0 for an invalid date.
    pub fn to_sdn(self, year: i32, month: i32, day: i32) -> i64 {
        unsafe { (self.fns().0)(year, month, day) }
    }

    /// `SdnTo<Cal>(sdn)`: (year, month, day), all 0 out of range.
    pub fn from_sdn(self, sdn: i64) -> (i32, i32, i32) {
        let (mut y, mut m, mut d) = (0, 0, 0);
        unsafe { (self.fns().1)(sdn, &mut y, &mut m, &mut d) };
        (y, m, d)
    }
}

/// `DayOfWeek(sdn)`: 0 = Sunday.
pub fn day_of_week(sdn: i64) -> i32 {
    unsafe { rhp_cal_DayOfWeek(sdn) }
}

/// Entry `i` of a name table (`None` past its end).
pub fn name(table: Names, i: usize) -> Option<&'static [u8]> {
    let t: &'static [*const c_char] = unsafe {
        match table {
            Names::MonthShort => &*std::ptr::addr_of!(rhp_cal_MonthNameShort),
            Names::MonthLong => &*std::ptr::addr_of!(rhp_cal_MonthNameLong),
            Names::Jewish => &*std::ptr::addr_of!(rhp_cal_JewishMonthName),
            Names::JewishLeap => &*std::ptr::addr_of!(rhp_cal_JewishMonthNameLeap),
            Names::JewishHeb => &*std::ptr::addr_of!(rhp_cal_JewishMonthHebName),
            Names::JewishHebLeap => &*std::ptr::addr_of!(rhp_cal_JewishMonthHebNameLeap),
            Names::French => &*std::ptr::addr_of!(rhp_cal_FrenchMonthName),
            Names::DayShort => &*std::ptr::addr_of!(rhp_cal_DayNameShort),
            Names::DayLong => &*std::ptr::addr_of!(rhp_cal_DayNameLong),
        }
    };
    let p = *t.get(i)?;
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_bytes())
}

/// `monthsPerYear[i]` (the 19-year cycle: 12 or 13).
pub fn months_per_year(i: usize) -> i32 {
    unsafe { (*std::ptr::addr_of!(rhp_cal_monthsPerYear))[i] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts() {
        assert_eq!(Cal::Gregorian.to_sdn(2000, 1, 1), 2451545);
        assert_eq!(Cal::Gregorian.from_sdn(2451545), (2000, 1, 1));
        assert_eq!(day_of_week(2451545), 6);
        assert_eq!(name(Names::MonthLong, 1), Some(&b"January"[..]));
        assert_eq!(name(Names::DayShort, 0), Some(&b"Sun"[..]));
        assert_eq!(months_per_year(2), 13);
    }
}
