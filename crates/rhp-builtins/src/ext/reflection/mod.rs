//! A pragmatic subset of the `Reflection` extension — enough for Symfony
//! Console / Composer command introspection. The `Reflection*` classes live in
//! `PRELUDE_PHP` and lean on PHP itself (`call_user_func_array`, `get_class`)
//! wherever possible; only the genuinely-native bits (class metadata, visibility-
//! bypassing property access, instantiation, parameter descriptors) delegate to
//! `__reflect_*` builtins backed by `Host`.

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

fn fn_meta(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(host.reflect_meta(&str_arg(args, 0)).unwrap_or(Value::Null))
}

/// `ReflectionClass::getConstants()`: evaluate pending constant expressions
/// first (php's `zend_update_class_constants`), then list name => value.
fn fn_constants(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_arg(args, 0);
    host.update_class_constants(&name)?;
    let consts = match host.reflect_meta(&name) {
        Some(Value::Array(a)) => a
            .get(&rhp_values::ArrayKey::from_bytes(b"constants"))
            .cloned(),
        _ => None,
    };
    Ok(consts.unwrap_or_else(|| Value::Array(rhp_values::PhpArray::new())))
}

fn fn_is_a(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_arg(args, 0);
    let other = str_arg(args, 1);
    let other = other.strip_prefix(b"\\").unwrap_or(&other);
    Ok(Value::Bool(host.reflect_is_a(&name, other)))
}

fn args_vec(v: &Value) -> Vec<Value> {
    match deref(v) {
        Value::Array(a) => a.iter().map(|(_, v)| v.clone()).collect(),
        _ => Vec::new(),
    }
}

fn fn_new_instance(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_arg(args, 0);
    let a = args.get(1).map(args_vec).unwrap_or_default();
    host.reflect_new_instance(&name, a, true)
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

fn fn_method_params(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let class = str_arg(args, 0);
    let method = str_arg(args, 1);
    Ok(host
        .reflect_method_params(&class, &method)
        .unwrap_or_else(|| Value::Array(rhp_values::PhpArray::new())))
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

fn fn_func_params(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let c = deref(&args[0]);
    Ok(host
        .reflect_callable_params(&c)
        .unwrap_or_else(|| Value::Array(rhp_values::PhpArray::new())))
}

fn fn_class_constants(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    host.update_class_constants(&str_arg(args, 0))?;
    Ok(host
        .reflect_class_constants(&str_arg(args, 0))
        .unwrap_or_else(|| Value::Array(rhp_values::PhpArray::new())))
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
    put(m, "__reflect_meta", fn_meta, 1, Some(1));
    put(m, "__reflect_constants", fn_constants, 1, Some(1));
    put(m, "__reflect_is_a", fn_is_a, 2, Some(2));
    put(m, "__reflect_new_instance", fn_new_instance, 1, Some(2));
    put(
        m,
        "__reflect_new_instance_noctor",
        fn_new_instance_noctor,
        1,
        Some(1),
    );
    put(m, "__reflect_get_prop", fn_get_prop, 3, Some(3));
    put(m, "__reflect_set_prop", fn_set_prop, 4, Some(4));
    put(m, "__reflect_method_params", fn_method_params, 2, Some(2));
    put(m, "__reflect_invoke", fn_invoke, 4, Some(4));
    put(m, "__reflect_func_params", fn_func_params, 1, Some(1));
    put(
        m,
        "__reflect_class_constants",
        fn_class_constants,
        1,
        Some(1),
    );
}

/// The `Reflection*` classes and the engine's attribute classes
/// (`Attribute`, `Override`, …), as PHP source over `__reflect()`.
pub const PRELUDE_PHP: &str = include_str!("prelude.php");
