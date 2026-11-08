//! `IntlCalendar`, `IntlGregorianCalendar` and `intlcal_*`.

use rhp_values::Value;

/// An IntlCalendar's time (`Calendar::getTime`): `None` when it is not
/// constructed.
pub(super) fn time_of(_v: &Value) -> Option<Result<f64, super::sys::UErrorCode>> {
    None
}
