//! The `Reflection` extension. The `Reflection*` classes are PHP source
//! (`PRELUDE_PHP`) over `__reflect($op, ...)`, a metadata query the VM
//! answers (its `reflect` module), plus a few `__reflect_*` helpers for
//! instantiation, visibility-bypassing property access and invocation.
//! Only prelude code can call these `__` builtins.

use std::collections::HashMap;

use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

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
    let argv: Vec<Value> = match deref(&args[3]) {
        Value::Array(a) => a.iter().map(|(_, v)| v.clone()).collect(),
        _ => Vec::new(),
    };
    host.reflect_invoke(&obj, &class, &method, argv)
}

/// `__reflect($op, ...$args)`: a metadata query answered by the host (see
/// `Host::reflect_op`).
fn fn_reflect(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = str_arg(args, 0);
    host.reflect_op(&op, &args[1..])
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "__reflect", fn_reflect, 1, None);
    put(
        m,
        "__reflect_new_instance_noctor",
        fn_new_instance_noctor,
        1,
        Some(1),
    );
    put(m, "__reflect_get_prop", fn_get_prop, 3, Some(3));
    put(m, "__reflect_set_prop", fn_set_prop, 4, Some(4));
    put(m, "__reflect_invoke", fn_invoke, 4, Some(4));
}

/// The `Reflection*` classes and the engine's attribute classes
/// (`Attribute`, `Override`, …), as PHP source over `__reflect()`.
pub const PRELUDE_PHP: &str = include_str!("prelude.php");
