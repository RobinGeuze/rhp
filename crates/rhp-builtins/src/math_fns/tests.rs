// Expected values captured from php 8.5.8.
#![allow(clippy::approx_constant)] // deliberate literal test values, not PI

use super::*;
use crate::test_support::TestHost;
use rhp_values::fmt;

fn call(func: crate::NativeFn, args: &[Value]) -> Value {
    func(&mut TestHost::new(), args).unwrap()
}

fn fv(v: Value) -> f64 {
    match v {
        Value::Float(x) => x,
        Value::Int(n) => n as f64,
        _ => panic!("expected float"),
    }
}

/// Render a Value the way var_dump would (for float/int/string), to assert
/// exact byte output including shortest-round-trip float formatting.
fn dump(v: &Value) -> String {
    match v {
        Value::Int(n) => format!("int({n})"),
        Value::Float(x) => {
            format!(
                "float({})",
                String::from_utf8(fmt::float_serialize(*x)).unwrap()
            )
        }
        Value::Str(s) => format!(
            "string({}) \"{}\"",
            s.len(),
            String::from_utf8_lossy(s.as_bytes())
        ),
        _ => String::from_utf8(conv::to_string_bytes(v)).unwrap(),
    }
}

fn i(n: i64) -> Value {
    Value::Int(n)
}
fn fl(x: f64) -> Value {
    Value::Float(x)
}
fn st(s: &str) -> Value {
    Value::str(s)
}

#[test]
fn floor_ceil() {
    assert_eq!(dump(&call(fn_floor, &[fl(4.7)])), "float(4)");
    assert_eq!(dump(&call(fn_floor, &[fl(-4.7)])), "float(-5)");
    assert_eq!(dump(&call(fn_floor, &[i(4)])), "float(4)");
    assert_eq!(dump(&call(fn_ceil, &[fl(4.1)])), "float(5)");
    assert_eq!(dump(&call(fn_ceil, &[fl(-4.1)])), "float(-4)");
}

#[test]
fn round_basic_and_precision() {
    // half away from zero
    assert_eq!(dump(&call(fn_round, &[fl(2.5)])), "float(3)");
    assert_eq!(dump(&call(fn_round, &[fl(-2.5)])), "float(-3)");
    assert_eq!(dump(&call(fn_round, &[fl(3.5)])), "float(4)");
    assert_eq!(dump(&call(fn_round, &[fl(0.5)])), "float(1)");
    assert_eq!(dump(&call(fn_round, &[fl(1.5)])), "float(2)");
    // pre-rounding cases
    assert_eq!(dump(&call(fn_round, &[fl(1.55), i(1)])), "float(1.6)");
    assert_eq!(dump(&call(fn_round, &[fl(1.45), i(1)])), "float(1.5)");
    assert_eq!(dump(&call(fn_round, &[fl(3.14159), i(2)])), "float(3.14)");
    assert_eq!(dump(&call(fn_round, &[fl(5.055), i(2)])), "float(5.06)");
    assert_eq!(dump(&call(fn_round, &[fl(0.285), i(2)])), "float(0.29)");
    assert_eq!(dump(&call(fn_round, &[fl(2.675), i(2)])), "float(2.68)");
    assert_eq!(dump(&call(fn_round, &[fl(1.005), i(2)])), "float(1.01)");
    assert_eq!(dump(&call(fn_round, &[fl(35.855), i(2)])), "float(35.86)");
    assert_eq!(dump(&call(fn_round, &[fl(8.075), i(2)])), "float(8.08)");
    // negative precision
    assert_eq!(
        dump(&call(fn_round, &[i(1241757), i(-3)])),
        "float(1242000)"
    );
    assert_eq!(
        dump(&call(fn_round, &[fl(1234.5678), i(-2)])),
        "float(1200)"
    );
    assert_eq!(dump(&call(fn_round, &[i(250), i(-2)])), "float(300)");
    // negative zero preserved
    assert_eq!(dump(&call(fn_round, &[fl(-0.4)])), "float(-0)");
}

#[test]
fn sqrt_pow_exp() {
    assert_eq!(dump(&call(fn_sqrt, &[i(16)])), "float(4)");
    assert_eq!(dump(&call(fn_sqrt, &[i(2)])), "float(1.4142135623730951)");
    assert!(fv(call(fn_sqrt, &[fl(-1.0)])).is_nan());
    // pow int/float typing
    assert_eq!(dump(&call(fn_pow, &[i(2), i(3)])), "int(8)");
    assert_eq!(dump(&call(fn_pow, &[i(2), i(0)])), "int(1)");
    assert_eq!(
        dump(&call(fn_pow, &[i(2), i(62)])),
        "int(4611686018427387904)"
    );
    assert_eq!(
        dump(&call(fn_pow, &[i(2), i(63)])),
        "float(9.223372036854776E+18)"
    );
    assert_eq!(
        dump(&call(fn_pow, &[i(10), i(18)])),
        "int(1000000000000000000)"
    );
    assert_eq!(dump(&call(fn_pow, &[i(10), i(19)])), "float(1.0E+19)");
    assert_eq!(dump(&call(fn_pow, &[i(2), i(-1)])), "float(0.5)");
    assert_eq!(dump(&call(fn_pow, &[fl(2.0), i(3)])), "float(8)");
    assert_eq!(dump(&call(fn_pow, &[i(-2), i(3)])), "int(-8)");
    assert_eq!(dump(&call(fn_pow, &[i(0), i(0)])), "int(1)");
    assert_eq!(dump(&call(fn_pow, &[st("2"), i(3)])), "int(8)");
    assert_eq!(dump(&call(fn_pow, &[i(2), st("3.0")])), "float(8)");
    assert_eq!(dump(&call(fn_pow, &[i(0), i(-1)])), "float(INF)");
    assert_eq!(dump(&call(fn_exp, &[i(1)])), "float(2.718281828459045)");
    assert_eq!(dump(&call(fn_exp, &[i(0)])), "float(1)");
}

#[test]
fn logs() {
    assert_eq!(dump(&call(fn_log, &[fl(std::f64::consts::E)])), "float(1)");
    assert_eq!(dump(&call(fn_log, &[i(8), i(2)])), "float(3)");
    assert_eq!(dump(&call(fn_log, &[i(100), i(10)])), "float(2)");
    assert_eq!(dump(&call(fn_log, &[i(27), i(3)])), "float(3)");
    assert!(fv(call(fn_log, &[i(5), i(1)])).is_nan());
    assert_eq!(dump(&call(fn_log, &[i(0)])), "float(-INF)");
    assert!(fv(call(fn_log, &[fl(-1.0)])).is_nan());
    assert_eq!(dump(&call(fn_log10, &[i(1000)])), "float(3)");
    // base <= 0 -> ValueError
    assert!(fn_log(&mut TestHost::new(), &[i(5), i(-2)]).is_err());
    assert!(fn_log(&mut TestHost::new(), &[i(5), i(0)]).is_err());
}

#[test]
fn trig_and_pi() {
    assert_eq!(dump(&call(fn_sin, &[i(0)])), "float(0)");
    assert_eq!(dump(&call(fn_cos, &[i(0)])), "float(1)");
    assert_eq!(dump(&call(fn_tan, &[i(0)])), "float(0)");
    assert_eq!(dump(&call(fn_asin, &[i(1)])), "float(1.5707963267948966)");
    assert_eq!(dump(&call(fn_acos, &[i(1)])), "float(0)");
    assert_eq!(dump(&call(fn_atan, &[i(1)])), "float(0.7853981633974483)");
    assert_eq!(
        dump(&call(fn_atan2, &[i(1), i(1)])),
        "float(0.7853981633974483)"
    );
    assert_eq!(dump(&call(fn_pi, &[])), "float(3.141592653589793)");
}

#[test]
fn fmod_fdiv_hypot_deg() {
    assert_eq!(dump(&call(fn_fmod, &[i(10), i(3)])), "float(1)");
    assert_eq!(dump(&call(fn_fmod, &[i(-10), i(3)])), "float(-1)");
    assert_eq!(dump(&call(fn_fmod, &[fl(10.5), i(3)])), "float(1.5)");
    assert_eq!(dump(&call(fn_fdiv, &[i(1), i(0)])), "float(INF)");
    assert_eq!(dump(&call(fn_fdiv, &[i(-1), i(0)])), "float(-INF)");
    assert!(fv(call(fn_fdiv, &[i(0), i(0)])).is_nan());
    assert_eq!(dump(&call(fn_fdiv, &[i(10), i(4)])), "float(2.5)");
    assert_eq!(dump(&call(fn_hypot, &[i(3), i(4)])), "float(5)");
    assert_eq!(
        dump(&call(fn_hypot, &[i(1), i(1)])),
        "float(1.4142135623730951)"
    );
    assert_eq!(
        dump(&call(fn_deg2rad, &[i(180)])),
        "float(3.141592653589793)"
    );
    assert_eq!(
        dump(&call(fn_rad2deg, &[fl(std::f64::consts::PI)])),
        "float(180)"
    );
    // exact representation ordering check vs php
    assert_eq!(
        String::from_utf8(fmt::float_serialize(fv(call(fn_deg2rad, &[i(1)])))).unwrap(),
        "0.017453292519943295"
    );
    assert_eq!(
        String::from_utf8(fmt::float_serialize(fv(call(fn_rad2deg, &[i(1)])))).unwrap(),
        "57.29577951308232"
    );
}

#[test]
fn base_conversions() {
    assert_eq!(dump(&call(fn_dechex, &[i(255)])), "string(2) \"ff\"");
    assert_eq!(dump(&call(fn_dechex, &[i(0)])), "string(1) \"0\"");
    assert_eq!(
        dump(&call(fn_dechex, &[i(4294967295)])),
        "string(8) \"ffffffff\""
    );
    assert_eq!(
        dump(&call(fn_dechex, &[i(-1)])),
        "string(16) \"ffffffffffffffff\""
    );
    assert_eq!(dump(&call(fn_dechex, &[fl(255.9)])), "string(2) \"ff\"");
    assert_eq!(dump(&call(fn_hexdec, &[st("ff")])), "int(255)");
    assert_eq!(dump(&call(fn_hexdec, &[st("1a")])), "int(26)");
    assert_eq!(dump(&call(fn_hexdec, &[st("0x1A")])), "int(26)");
    assert_eq!(
        dump(&call(fn_hexdec, &[st("FFFFFFFFFFFFFFFF")])),
        "float(1.8446744073709552E+19)"
    );
    assert_eq!(dump(&call(fn_hexdec, &[st("")])), "int(0)");
    assert_eq!(dump(&call(fn_decbin, &[i(5)])), "string(3) \"101\"");
    assert_eq!(
        dump(&call(fn_decbin, &[i(-1)])),
        format!("string(64) \"{}\"", "1".repeat(64))
    );
    assert_eq!(dump(&call(fn_bindec, &[st("101")])), "int(5)");
    assert_eq!(dump(&call(fn_bindec, &[st("11111111")])), "int(255)");
    assert_eq!(
        dump(&call(fn_bindec, &[st(&"1".repeat(64))])),
        "float(1.8446744073709552E+19)"
    );
    assert_eq!(dump(&call(fn_decoct, &[i(64)])), "string(3) \"100\"");
    assert_eq!(
        dump(&call(fn_decoct, &[i(-1)])),
        "string(22) \"1777777777777777777777\""
    );
    assert_eq!(dump(&call(fn_octdec, &[st("777")])), "int(511)");
    assert_eq!(dump(&call(fn_octdec, &[st("10")])), "int(8)");
}

#[test]
fn base_convert_fn() {
    assert_eq!(
        dump(&call(fn_base_convert, &[st("ff"), i(16), i(10)])),
        "string(3) \"255\""
    );
    assert_eq!(
        dump(&call(fn_base_convert, &[st("ff"), i(16), i(2)])),
        "string(8) \"11111111\""
    );
    assert_eq!(
        dump(&call(fn_base_convert, &[st("1010"), i(2), i(16)])),
        "string(1) \"a\""
    );
    assert_eq!(
        dump(&call(fn_base_convert, &[st("z"), i(36), i(10)])),
        "string(2) \"35\""
    );
    assert_eq!(
        dump(&call(fn_base_convert, &[st(""), i(16), i(10)])),
        "string(1) \"0\""
    );
    // invalid chars ignored
    assert_eq!(
        dump(&call(fn_base_convert, &[st("ff!!gg"), i(16), i(10)])),
        "string(3) \"255\""
    );
    // large values go through the lossy float path, matching php exactly
    assert_eq!(
        dump(&call(
            fn_base_convert,
            &[st("ffffffffffffffffffff"), i(16), i(10)]
        )),
        "string(25) \"1208925819614629446642046\""
    );
    assert_eq!(
        dump(&call(fn_base_convert, &[st("zzzzzzzzzzzzz"), i(36), i(10)])),
        "string(21) \"170581728179578208466\""
    );
    assert_eq!(
        dump(&call(
            fn_base_convert,
            &[st("zzzzzzzzzzzzzzzzzzzz"), i(36), i(16)]
        )),
        "string(26) \"a8b8b452291fe8000000000000\""
    );
    // base range errors
    assert!(fn_base_convert(&mut TestHost::new(), &[st("ff"), i(1), i(10)]).is_err());
    assert!(fn_base_convert(&mut TestHost::new(), &[st("ff"), i(16), i(37)]).is_err());
}

#[test]
fn hyperbolic_and_exp_anchors() {
    // Exact values php 8.5.8 returns; the ULP-sensitive cases are covered by
    // the CLI differential tests (routed through the C libm).
    assert_eq!(fv(call(fn_sinh, &[fl(0.0)])), 0.0);
    assert_eq!(fv(call(fn_cosh, &[fl(0.0)])), 1.0);
    assert_eq!(fv(call(fn_tanh, &[fl(0.0)])), 0.0);
    assert_eq!(fv(call(fn_asinh, &[fl(0.0)])), 0.0);
    assert_eq!(fv(call(fn_acosh, &[fl(1.0)])), 0.0);
    assert_eq!(fv(call(fn_atanh, &[fl(0.0)])), 0.0);
    assert_eq!(fv(call(fn_expm1, &[fl(0.0)])), 0.0);
    assert_eq!(fv(call(fn_log1p, &[fl(0.0)])), 0.0);
    // tanh saturates to 1 for large arguments
    assert_eq!(fv(call(fn_tanh, &[fl(40.0)])), 1.0);
}
