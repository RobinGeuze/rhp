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
