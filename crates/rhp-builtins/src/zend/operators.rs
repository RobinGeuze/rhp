//! Zend/zend_operators.c conversions that raise diagnostics (php 8.5's
//! float-to-int and NAN warnings), for builtins.

use rhp_values::{Thrown, Value};

use crate::host::Host;

/// `ZEND_DOUBLE_FITS_LONG`.
pub fn fits_long(d: f64) -> bool {
    !(d >= 9_223_372_036_854_775_808.0 || d < -9_223_372_036_854_775_808.0)
}

/// `zend_oob_double_to_long_error`.
#[cold]
pub fn oob_double_to_long_error(host: &mut dyn Host, d: f64) -> Result<(), Thrown> {
    let shown = rhp_values::fmt::float_serialize(d);
    host.emit_diagnostic(
        crate::E_WARNING,
        &[
            b"The float ".as_slice(),
            &shown,
            b" is not representable as an int, cast occurred",
        ]
        .concat(),
    )
}

/// `zend_incompatible_double_to_long_error`.
#[cold]
pub fn incompatible_double_to_long_error(host: &mut dyn Host, d: f64) -> Result<(), Thrown> {
    let shown = rhp_values::fmt::float_serialize(d);
    host.emit_diagnostic(
        crate::E_DEPRECATED,
        &[
            b"Implicit conversion from float ".as_slice(),
            &shown,
            b" to int loses precision",
        ]
        .concat(),
    )
}

/// `zend_nan_coerced_to_type_warning`.
#[cold]
pub fn nan_coerced(host: &mut dyn Host, ty: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(
        crate::E_WARNING,
        format!("unexpected NAN value was coerced to {ty}").as_bytes(),
    )
}

/// `zend_dval_to_lval`: warns when `d` is out of range (then wraps modulo
/// 2^64) or not finite (then 0).
pub fn dval_to_lval(host: &mut dyn Host, d: f64) -> Result<i64, Thrown> {
    if !d.is_finite() || !fits_long(d) {
        oob_double_to_long_error(host, d)?;
    }
    Ok(rhp_values::float_to_int(d))
}

/// `zend_dval_to_lval_safe`: [`dval_to_lval`], then the deprecation when a
/// fraction is lost.
pub fn dval_to_lval_safe(host: &mut dyn Host, d: f64) -> Result<i64, Thrown> {
    let l = dval_to_lval(host, d)?;
    if l as f64 != d && fits_long(d) {
        incompatible_double_to_long_error(host, d)?;
    }
    Ok(l)
}

/// `zval_get_long` (not strict): like `conv::to_int`, with the float
/// warning. Objects are the caller's business.
pub fn zval_get_long(host: &mut dyn Host, v: &Value) -> Result<i64, Thrown> {
    match v {
        Value::Float(f) => dval_to_lval(host, *f),
        Value::Ref(r) => {
            let inner = r.borrow().clone();
            zval_get_long(host, &inner)
        }
        _ => Ok(rhp_values::conv::to_int(v)),
    }
}

/// The diagnostics `zend_dval_to_lval_safe(d)` raises, in order (for
/// callers that queue them): the out-of-range warning, then the
/// lost-fraction deprecation.
pub fn dval_to_lval_safe_msgs(d: f64) -> Vec<(i64, Vec<u8>)> {
    let mut out = Vec::new();
    let shown = rhp_values::fmt::float_serialize(d);
    if !d.is_finite() || !fits_long(d) {
        out.push((
            crate::E_WARNING,
            [
                b"The float ".as_slice(),
                &shown,
                b" is not representable as an int, cast occurred",
            ]
            .concat(),
        ));
    }
    if rhp_values::float_to_int(d) as f64 != d && fits_long(d) {
        out.push((
            crate::E_DEPRECATED,
            [
                b"Implicit conversion from float ".as_slice(),
                &shown,
                b" to int loses precision",
            ]
            .concat(),
        ));
    }
    out
}
