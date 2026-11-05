//! ext/standard/assert.c: `assert`.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

/// `assert($assertion, $description = null)` — PHP 8: evaluate `$assertion`; if
/// falsy, throw AssertionError (assertions are treated as enabled, the CLI
/// default). A string `$description` becomes the message; a Throwable is thrown.
fn fn_assert(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if conv::to_bool(&deref(&args[0])) {
        return Ok(Value::Bool(true));
    }
    match args.get(1).map(deref) {
        Some(Value::Str(s)) => Err(Thrown::error(
            "AssertionError",
            String::from_utf8_lossy(s.as_bytes()).into_owned(),
        )),
        _ => Err(Thrown::error("AssertionError", "assert failed")),
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("assert", fn_assert)]);
}

/// assert.c's `ASSERT_*` (deprecated with assert_options()).
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("ASSERT_ACTIVE", Value::Int(1)),
        ("ASSERT_CALLBACK", Value::Int(2)),
        ("ASSERT_BAIL", Value::Int(3)),
        ("ASSERT_WARNING", Value::Int(4)),
        ("ASSERT_EXCEPTION", Value::Int(5)),
    ]
}

/// The E_DEPRECATED for reading an `ASSERT_*` constant.
pub(crate) fn constant_deprecation(name: &[u8]) -> Option<String> {
    constants()
        .iter()
        .any(|(n, _)| n.as_bytes() == name)
        .then(|| {
            format!(
                "Constant {} is deprecated since 8.3, as assert_options() is deprecated",
                String::from_utf8_lossy(name)
            )
        })
}
