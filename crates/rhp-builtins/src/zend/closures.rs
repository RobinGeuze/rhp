//! Zend/zend_closures.c: the engine helpers behind the prelude's
//! `Closure::fromCallable` and `Closure::bind`.

use rhp_values::ops::deref;
use rhp_values::Value;

use crate::{Host, Thrown};

/// `Closure::fromCallable($callable)`: wrap a callable in a Closure (a closure is
/// returned as-is), mirroring the first-class-callable `MakeFcc` wrapping.
fn fn_closure_from_callable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let c = deref(&args[0]);
    if matches!(c, Value::Closure(_)) {
        return Ok(c);
    }
    // zend_create_closure_from_callable's error (a TypeError).
    if let Some(err) = host.check_callable(&c)? {
        return Err(Thrown::type_error(format!(
            "Failed to create closure from callable: {err}"
        )));
    }
    Ok(host.wrap_callable(c))
}

/// `Closure::bind($closure, $newThis, $newScope)` and the `bindTo` instance form.
fn fn_closure_bind(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let closure = deref(&args[0]);
    let new_this = args.get(1).map(deref).unwrap_or(Value::Null);
    host.closure_bind(&closure, &new_this, args.get(2))
}

/// `Closure::getCurrent()`: the Closure running in the calling frame.
fn fn_closure_current(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.current_closure()
        .ok_or_else(|| Thrown::error("Error", "Current function is not a closure"))
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(
        m,
        &[
            (
                "__closure_from_callable",
                fn_closure_from_callable,
                1,
                Some(1),
            ),
            ("__closure_bind", fn_closure_bind, 2, Some(3)),
            ("__closure_current", fn_closure_current, 0, Some(0)),
        ],
    );
}
