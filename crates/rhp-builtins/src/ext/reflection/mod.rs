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
/// invoke(), which ignores visibility. For a static method `$object` is the
/// reflected class's name (the called scope) or null.
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

/// `__reflect_ext($op, $name)`: ReflectionExtension's view of loaded
/// module `$name` (php's module name): `info` prints its phpinfo section
/// (php_info_print_module), `constants` and `ini` list what its MINIT
/// registers.
fn fn_ext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = str_arg(args, 0);
    let name = String::from_utf8_lossy(&str_arg(args, 1)).into_owned();
    let mut out = rhp_values::PhpArray::new();
    match op.as_slice() {
        b"info" => {
            let text = crate::ext::standard::info::module_info(&name);
            host.write_out(&text);
            return Ok(Value::Null);
        }
        b"constants" => {
            if let Some(md) = crate::registry::modules()
                .iter()
                .find(|md| md.name.eq_ignore_ascii_case(&name))
            {
                for (k, v) in (md.constants)() {
                    out.insert(rhp_values::ArrayKey::from_bytes(k.as_bytes()), v);
                }
            }
        }
        b"ini" => {
            let lc = name.to_ascii_lowercase();
            for e in crate::main::ini_table::INI_TABLE
                .iter()
                .filter(|e| e.0 == lc)
            {
                let v = crate::main::ini::ini_value(e.1.as_bytes())
                    .flatten()
                    .map_or(Value::Null, Value::str);
                out.insert(rhp_values::ArrayKey::from_bytes(e.1.as_bytes()), v);
            }
        }
        _ => {}
    }
    Ok(Value::Array(out))
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
            ("__reflect_ext", fn_ext, 2, Some(2)),
        ],
    );
}

/// The `Reflection*` classes and the engine's attribute classes
/// (`Attribute`, `Override`, …), as PHP source over `__reflect()`.
pub const PRELUDE_PHP: &str = include_str!("prelude.php");
