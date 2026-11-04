//! Class / function table introspection builtins: `get_declared_classes`,
//! `get_declared_interfaces`, `get_declared_traits`, `trait_exists`,
//! `get_defined_functions`, `get_class_methods`, `get_class_vars`,
//! `get_included_files` / `get_required_files`. The tables live in the host
//! and are read through `Host::reflect_op`.

use rhp_semantics::deref;
use rhp_values::{ClassKind, Value};

use crate::argparse::{bool_param, str_param, zval_get_string, zval_value_name};
use crate::{Host, Thrown};

fn declared(host: &mut dyn Host, kind: &str) -> Result<Value, Thrown> {
    host.reflect_op(b"declared", &[Value::str(kind)])
}

fn fn_get_declared_classes(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    declared(host, "class")
}

fn fn_get_declared_interfaces(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    declared(host, "interface")
}

fn fn_get_declared_traits(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    declared(host, "trait")
}

fn fn_trait_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "trait_exists", "trait")?;
    let autoload = bool_param(host, args, 1, "trait_exists", "autoload", true)?;
    let id = if autoload {
        host.resolve_class_autoload(&name)?
    } else {
        host.resolve_class(&name)
    };
    Ok(Value::Bool(
        id.and_then(|id| host.class_kind(id)) == Some(ClassKind::Trait),
    ))
}

fn fn_get_defined_functions(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    bool_param(
        host,
        args,
        0,
        "get_defined_functions",
        "exclude_disabled",
        true,
    )?;
    if !args.is_empty() {
        crate::deprecated(
            host,
            "get_defined_functions",
            "The $exclude_disabled parameter has no effect since PHP 8.0",
        )?;
    }
    host.reflect_op(b"functions", &[])
}

fn fn_get_class_methods(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    let class: Option<Vec<u8>> = match &v {
        Value::Object(_) | Value::Closure(_) => host
            .object_class_name(&v)
            .or_else(|| matches!(v, Value::Closure(_)).then(|| b"Closure".to_vec())),
        Value::Str(s) => host
            .resolve_class_autoload(s.as_bytes())?
            .map(|_| s.as_bytes().to_vec()),
        _ => None,
    };
    let Some(class) = class else {
        return Err(Thrown::type_error(format!(
            "get_class_methods(): Argument #1 ($object_or_class) must be an object or a valid class name, {} given",
            zval_value_name(host, &v)
        )));
    };
    host.reflect_op(b"methods_visible", &[Value::str(class)])
}

fn fn_get_class_vars(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // zpp "C" (zend_parse_arg_class): try_convert_to_string in either
    // mode, then a class lookup, else a TypeError.
    let name = zval_get_string(host, &args[0])?;
    if host.resolve_class_autoload(&name)?.is_none() {
        return Err(Thrown::type_error(format!(
            "get_class_vars(): Argument #1 ($class) must be a valid class name, {} given",
            String::from_utf8_lossy(&name)
        )));
    }
    host.reflect_op(b"default_props", &[Value::str(name), Value::Bool(true)])
}

fn fn_get_included_files(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.reflect_op(b"included", &[])
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("get_declared_classes", fn_get_declared_classes),
            ("get_declared_interfaces", fn_get_declared_interfaces),
            ("get_declared_traits", fn_get_declared_traits),
            ("trait_exists", fn_trait_exists),
            ("get_defined_functions", fn_get_defined_functions),
            ("get_class_methods", fn_get_class_methods),
            ("get_class_vars", fn_get_class_vars),
            ("get_included_files", fn_get_included_files),
            ("get_required_files", fn_get_included_files),
        ],
    );
}
