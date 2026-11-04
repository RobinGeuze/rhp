use super::*;

#[test]
fn php_g_format() {
    assert_eq!(fmt_g(1e20), "1.0e+20");
    assert_eq!(fmt_g(-9.223_372_036_854_776e18), "-9.22337e+18");
    assert_eq!(fmt_g(123456.0), "123456");
    assert_eq!(fmt_g(1234567.0), "1.23457e+6");
    assert_eq!(fmt_g(0.5), "0.5");
    assert_eq!(fmt_g(f64::INFINITY), "INF");
}

#[test]
fn interval_format_specifiers() {
    let mut r = RelTime::new();
    r.y = 1;
    r.d = 5;
    r.us = 42;
    r.days = TIMELIB_UNSET;
    r.invert = 1;
    assert_eq!(
        date_interval_format(b"%Y-%m-%D %F %a %R%r %% %x %", &r),
        b"01-0-05 000042 (unknown) -- % %x ".to_vec()
    );
}

#[test]
fn offset_strings() {
    assert_eq!(tz_offset_str(3600), "+01:00");
    assert_eq!(tz_offset_str(-5400), "-01:30");
    assert_eq!(tz_offset_str(3661), "+01:01:01");
}
