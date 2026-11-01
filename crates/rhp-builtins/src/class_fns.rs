//! Class / function table introspection builtins: `get_declared_classes`,
//! `get_declared_interfaces`, `get_declared_traits`, `trait_exists`,
//! `get_defined_functions`, `get_class_methods`, `get_class_vars`,
//! `get_included_files` / `get_required_files`. The tables live in the host
//! and are read through `Host::reflect_op`.

use std::collections::HashMap;

use rhp_semantics::deref;
use rhp_values::{ClassKind, Value};

use crate::argparse::{bool_param, str_param, value_name};
use crate::{Builtin, Host, Thrown};

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
            value_name(host, &v)
        )));
    };
    host.reflect_op(b"methods_visible", &[Value::str(class)])
}

fn fn_get_class_vars(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "get_class_vars", "class")?;
    if host.resolve_class_autoload(&name)?.is_none() {
        return Ok(Value::Bool(false));
    }
    host.reflect_op(b"default_props", &[Value::str(name), Value::Bool(true)])
}

fn fn_get_included_files(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.reflect_op(b"included", &[])
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let mut put = |name: &'static str, func: crate::NativeFn, min: usize, max: usize| {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: Some(max),
            },
        );
    };
    put("get_declared_classes", fn_get_declared_classes, 0, 0);
    put("get_declared_interfaces", fn_get_declared_interfaces, 0, 0);
    put("get_declared_traits", fn_get_declared_traits, 0, 0);
    put("trait_exists", fn_trait_exists, 1, 2);
    put("get_defined_functions", fn_get_defined_functions, 0, 1);
    put("get_class_methods", fn_get_class_methods, 1, 1);
    put("get_class_vars", fn_get_class_vars, 1, 1);
    put("get_included_files", fn_get_included_files, 0, 0);
    put("get_required_files", fn_get_included_files, 0, 0);
}
