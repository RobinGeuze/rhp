//! The `Reflection` extension. The `Reflection*` classes are PHP source
//! (`PRELUDE_PHP`) over `__reflect($op, ...)`, a metadata query the VM
//! answers (its `reflect` module), plus a few `__reflect_*` helpers for
//! instantiation, visibility-bypassing property access and invocation.
//! Only prelude code can call these `__` builtins.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

fn str_arg(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

fn fn_new_instance_noctor(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_arg(args, 0);
    host.reflect_new_instance(&name, Vec::new(), false)
}

/// `__reflect_get_prop($obj, $name, $class)`: read `$name` as `$class` sees it.
fn fn_get_prop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let obj = deref(&args[0]);
    let name = str_arg(args, 1);
    let class = str_arg(args, 2);
    Ok(host
        .reflect_get_prop(&obj, &class, &name)?
        .unwrap_or(Value::Null))
}

/// `__reflect_set_prop($obj, $name, $value, $class)`.
fn fn_set_prop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let obj = deref(&args[0]);
    let name = str_arg(args, 1);
    let val = deref(&args[2]);
    let class = str_arg(args, 3);
    host.reflect_set_prop(&obj, &class, &name, val)?;
    Ok(Value::Null)
}

/// `__reflect_invoke($object, $class, $method, $args)`: ReflectionMethod::
/// invoke(), which ignores visibility.
fn fn_invoke(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let obj = deref(&args[0]);
    let class = str_arg(args, 1);
    let method = str_arg(args, 2);
    let argv = match deref(&args[3]) {
        a @ Value::Array(_) => a,
        _ => Value::Array(rhp_values::PhpArray::new()),
    };
    host.reflect_invoke(&obj, &class, &method, argv)
}

/// `__reflect($op, ...$args)`: a metadata query answered by the host (see
/// `Host::reflect_op`).
fn fn_reflect(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = str_arg(args, 0);
    host.reflect_op(&op, &args[1..])
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(
        m,
        &[
            ("__reflect", fn_reflect, 1, None),
            (
                "__reflect_new_instance_noctor",
                fn_new_instance_noctor,
                1,
                Some(1),
            ),
            ("__reflect_get_prop", fn_get_prop, 3, Some(3)),
            ("__reflect_set_prop", fn_set_prop, 4, Some(4)),
            ("__reflect_invoke", fn_invoke, 4, Some(4)),
        ],
    );
}

/// The `Reflection*` classes and the engine's attribute classes
/// (`Attribute`, `Override`, …), as PHP source over `__reflect()`.
pub const PRELUDE_PHP: &str = include_str!("prelude.php");
