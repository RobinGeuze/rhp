//! Zend/zend_generators.c: the helper behind the prelude's Generator
//! methods. A method call on a generator runs natively in the VM; a call
//! that reaches the prelude body (a callable, ReflectionMethod::invoke)
//! goes through `__generator`.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

/// `__generator($this, $method, ...$args)`.
fn fn_generator(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Object(h) = deref(&args[0]) else {
        return Ok(Value::Null);
    };
    let name = conv::to_string_bytes(&deref(&args[1]));
    host.generator_method(&h, &name, args.get(2..).unwrap_or(&[]))
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(m, &[("__generator", fn_generator, 2, None)]);
}
