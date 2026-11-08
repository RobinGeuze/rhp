//! common_date.cpp: a PHP value as a date in milliseconds
//! (`intl_zval_to_millis`) and a DateTime's time (`intl_datetime_decompose`).

use std::cell::RefCell;

use rhp_values::Value;

use super::error::IntlError;
use super::sys;
use super::timezone::{errors_set_opt, is_a};
use crate::zend::object_handlers;
use crate::zend::zpp::deref;
use crate::{Host, Thrown};

/// `intl_datetime_decompose`'s milliseconds: the object's
/// `getTimestamp()` (a subclass may override it) plus its microseconds.
/// NaN after an error.
pub(super) fn datetime_millis(
    host: &mut dyn Host,
    v: &Value,
    err: Option<&RefCell<IntlError>>,
    fname: &str,
) -> Result<f64, Thrown> {
    let ts = host.call_callable(
        &Value::Array({
            let mut a = rhp_values::PhpArray::new();
            a.push(v.clone());
            a.push(Value::str("getTimestamp"));
            a
        }),
        &[],
    )?;
    let Value::Int(ts) = deref(&ts) else {
        let class = host.object_class_name(v).unwrap_or_default();
        let msg = format!(
            "{}::getTimestamp() did not return an int",
            String::from_utf8_lossy(&class)
        );
        errors_set_opt(host, err, fname, sys::U_INTERNAL_PROGRAM_ERROR, &msg)?;
        return Ok(f64::NAN);
    };
    let us = object_handlers::object_of(v)
        .and_then(|h| crate::ext::date::objects::time_of(&h))
        .map_or(0, |t| t.us);
    Ok(1000.0 * ts as f64 + (us / 1000) as f64)
}

/// `intl_zval_to_millis`: NaN after setting the error.
pub(super) fn zval_to_millis(
    host: &mut dyn Host,
    z: &Value,
    err: Option<&RefCell<IntlError>>,
    fname: &str,
) -> Result<f64, Thrown> {
    if err.is_some_and(|e| sys::failure(e.borrow().code)) {
        return Ok(f64::NAN);
    }
    let z = deref(z);
    match &z {
        Value::Str(s) => match rhp_values::conv::numeric_string(s.as_bytes()) {
            Some(rhp_values::conv::Num::Int(n)) => Ok(1000.0 * n as f64),
            Some(rhp_values::conv::Num::Float(f)) => Ok(f * 1000.0),
            None => {
                let msg = format!(
                    "string '{}' is not numeric, which would be required for it to be a valid date",
                    String::from_utf8_lossy(super::c_string(s.as_bytes()).as_bytes())
                );
                errors_set_opt(host, err, fname, sys::U_ILLEGAL_ARGUMENT_ERROR, &msg)?;
                Ok(f64::NAN)
            }
        },
        Value::Int(n) => Ok(1000.0 * *n as f64),
        Value::Float(f) => Ok(1000.0 * f),
        Value::Object(_) => {
            if is_a(host, &z, b"DateTimeInterface") {
                datetime_millis(host, &z, err, fname)
            } else if is_a(host, &z, b"IntlCalendar") {
                match super::calendar::time_of(&z) {
                    None => {
                        errors_set_opt(
                            host,
                            err,
                            fname,
                            sys::U_ILLEGAL_ARGUMENT_ERROR,
                            "IntlCalendar object is not properly constructed",
                        )?;
                        Ok(f64::NAN)
                    }
                    Some(Ok(ms)) => Ok(ms),
                    Some(Err(status)) => {
                        errors_set_opt(
                            host,
                            err,
                            fname,
                            status,
                            "call to internal Calendar::getTime() has failed",
                        )?;
                        Ok(f64::NAN)
                    }
                }
            } else {
                errors_set_opt(
                    host,
                    err,
                    fname,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "invalid object type for date/time (only IntlCalendar and DateTimeInterface permitted)",
                )?;
                Ok(f64::NAN)
            }
        }
        _ => {
            errors_set_opt(
                host,
                err,
                fname,
                sys::U_ILLEGAL_ARGUMENT_ERROR,
                "invalid PHP type for date",
            )?;
            Ok(f64::NAN)
        }
    }
}
