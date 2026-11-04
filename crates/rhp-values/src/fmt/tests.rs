use super::*;

fn echo(f: f64) -> String {
    String::from_utf8(float_echo(f)).unwrap()
}
fn ser(f: f64) -> String {
    String::from_utf8(float_serialize(f)).unwrap()
}

#[test]
fn echo_forms_match_php_8_5() {
    // ground truth from tests/golden/lang/echo_float_formats.out
    assert_eq!(echo(1.0), "1");
    assert_eq!(echo(0.1 + 0.2), "0.3");
    assert_eq!(echo(1.0 / 3.0), "0.33333333333333");
    assert_eq!(echo(1e20), "1.0E+20");
    assert_eq!(echo(1.5e-10), "1.5E-10");
    assert_eq!(echo(-0.0), "-0");
    assert_eq!(echo(0.00001), "1.0E-5");
}

#[test]
fn echo_specials() {
    assert_eq!(echo(f64::NAN), "NAN");
    assert_eq!(echo(f64::INFINITY), "INF");
    assert_eq!(echo(f64::NEG_INFINITY), "-INF");
    assert_eq!(echo(0.0), "0");
    assert_eq!(echo(-42.5), "-42.5");
    assert_eq!(echo(100.0), "100");
}

#[test]
fn serialize_forms_match_php_8_5() {
    // php -r 'var_dump(0.1+0.2);' => float(0.30000000000000004)
    assert_eq!(ser(0.1 + 0.2), "0.30000000000000004");
    // php -r 'var_dump(1/3);' => float(0.3333333333333333)
    assert_eq!(ser(1.0 / 3.0), "0.3333333333333333");
    assert_eq!(ser(1e20), "1.0E+20");
    // php -r 'var_dump(1e15, 1e16, 1e17);'
    assert_eq!(ser(1e15), "1000000000000000");
    assert_eq!(ser(1e16), "10000000000000000");
    assert_eq!(ser(1e17), "1.0E+17");
    assert_eq!(ser(1.0), "1");
    assert_eq!(ser(-0.0), "-0");
}

#[test]
fn shortest_ties_round_half_even() {
    // php -n -r 'var_export(...)', minus var_export's ".0" suffix. The ties
    // are built from exact parts (a literal would trip clippy).
    assert_eq!(ser(-(887777373534812.0 + 0.25)), "-887777373534812.2");
    assert_eq!(ser(123456789012345.0 + 0.625), "123456789012345.62");
    assert_eq!(ser(70368744177664.0 + 0.125), "70368744177664.12");
    assert_eq!(ser(2f64.powi(-25)), "2.9802322387695312E-8");
    assert_eq!(ser(1.1529215046068472e18), "1.1529215046068472E+18");
    assert_eq!(ser(5e-324), "5.0E-324");
    assert_eq!(ser(f64::MAX), "1.7976931348623157E+308");
    assert_eq!(ser(1e23), "1.0E+23");
    assert_eq!(ser(0.000123), "0.000123");
    assert_eq!(ser(123456.0), "123456");
}
