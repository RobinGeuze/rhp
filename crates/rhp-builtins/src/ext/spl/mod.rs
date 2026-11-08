//! A slice of the `SPL` extension: the autoload registry
//! (`spl_autoload_register` and friends). The callables live on the [`Host`]
//! (the VM), which invokes them when a class reference fails to resolve.

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, PhpStr, Value};

use crate::zend::object_handlers::{ClassHandlers, ObjectHandlers, PropPurpose};
use crate::zend::zpp::{bool_param, zval_value_name};
use crate::{Host, Thrown};

/// `ArrayObject` / `ArrayIterator`'s handlers: spl_array's
/// `get_properties_for` (the storage stands in for the properties) and its
/// debug info.
static SPL_ARRAY_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(spl_array_get_properties_for),
    ..ObjectHandlers::STD
};

/// `SplFixedArray`'s spl_fixedarray_object_get_properties_for.
static SPL_FIXEDARRAY_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(spl_fixedarray_get_properties_for),
    ..ObjectHandlers::STD
};

pub(crate) static CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "SplFixedArray",
        handlers: &SPL_FIXEDARRAY_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "ArrayObject",
        handlers: &SPL_ARRAY_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "ArrayIterator",
        handlers: &SPL_ARRAY_HANDLERS,
        create_object: true,
    },
    // spl_handlers_dual_it, spl_handlers_rec_it_it
    ClassHandlers {
        class: "IteratorIterator",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "RecursiveIteratorIterator",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    // spl_filesystem_object_check_handlers
    ClassHandlers {
        class: "SplFileObject",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
];

/// The storage of an ArrayObject / ArrayIterator (or a subclass); `None`
/// with `STD_PROP_LIST`, where the properties are the properties.
fn spl_array_storage(h: &ObjectHandle) -> Option<Value> {
    let b = h.borrow();
    for base in ["ArrayObject", "ArrayIterator"] {
        let key = |n: &str| PhpStr::from(format!("\0{base}\0{n}").as_str());
        if let Some(s) = b.props().get(&key("__s")) {
            let flags = b.props().get(&key("__f")).map(|f| conv::to_int(&deref(f)));
            if flags.unwrap_or(0) & 1 != 0 {
                return None;
            }
            return Some(deref(s));
        }
    }
    None
}

/// spl_array_get_properties_for: `(array)`, `var_export` and `json_encode`
/// see an array storage. The debug view is `ArrayObject::__debugInfo()`
/// (`__spl_array_debug_info`).
fn spl_array_get_properties_for(
    _host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    match purpose {
        PropPurpose::ArrayCast | PropPurpose::VarExport | PropPurpose::Json => {
            Ok(match spl_array_storage(h) {
                Some(Value::Array(a)) => Some(a),
                _ => None,
            })
        }
        // ArrayObject::__debugInfo() (`__spl_array_debug_info`).
        PropPurpose::Debug => Ok(None),
        PropPurpose::Serialize | PropPurpose::GetObjectVars => Ok(None),
    }
}

/// spl_fixedarray_object_get_properties_for: the elements (not for
/// get_object_vars()), then the properties.
fn spl_fixedarray_get_properties_for(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose == PropPurpose::Serialize {
        return Ok(None);
    }
    let elements = {
        let key = PhpStr::from("\0SplFixedArray\0__d");
        match h.borrow().props().get(&key).map(deref) {
            Some(Value::Array(a)) => a,
            _ => PhpArray::new(),
        }
    };
    let props = host.object_props_mangled(h);
    if elements.is_empty() && props.is_empty() {
        return Ok(None);
    }
    let mut a = PhpArray::new();
    if purpose != PropPurpose::GetObjectVars {
        for (_, v) in elements.iter() {
            a.push(v.clone());
        }
    }
    for (k, v) in props.iter() {
        a.insert(k.clone(), v.clone());
    }
    Ok(Some(a))
}

/// `__spl_array_debug_info($object)`: spl_array_get_debug_info, the body
/// of ArrayObject's and ArrayIterator's `__debugInfo()`: the properties,
/// plus the storage as the base class's private `storage` unless the
/// object is its own storage.
fn fn_spl_array_debug_info(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Object(h) = deref(&args[0]) else {
        return Ok(Value::Array(PhpArray::new()));
    };
    let b = h.borrow();
    let mut a = PhpArray::new();
    for (k, v) in b.props().iter() {
        if !host.is_internal_slot(b.class, k) {
            a.insert(ArrayKey::Str(host.mangled_prop_name(b.class, k)), v.clone());
        }
    }
    for base in ["ArrayObject", "ArrayIterator"] {
        let key = |n: &str| PhpStr::from(format!("\0{base}\0{n}").as_str());
        let Some(st) = b.props().get(&key("__s")) else {
            continue;
        };
        let st = match b.props().get(&key("__o")).map(deref) {
            Some(Value::Object(o)) if std::rc::Rc::ptr_eq(&o, &h) => break,
            Some(o @ Value::Object(_)) => o,
            _ => deref(st),
        };
        let cname = host.class_name(b.class).unwrap_or_default();
        let base = if host.reflect_is_a(&cname, b"ArrayIterator") {
            "ArrayIterator"
        } else {
            "ArrayObject"
        };
        let name = PhpStr::from(format!("\0{base}\0storage").as_str());
        a.insert(ArrayKey::Str(name), st);
        break;
    }
    Ok(Value::Array(a))
}

/// `spl_autoload_register(?callable $callback = null, bool $throw = true, bool $prepend = false): bool`
fn fn_spl_autoload_register(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if args.get(1).is_some_and(|v| !conv::to_bool(&deref(v))) {
        host.emit_diagnostic(
            crate::E_NOTICE,
            b"spl_autoload_register(): Argument #2 ($do_throw) has been ignored, spl_autoload_register() will always throw",
        )?;
    }
    let prepend = args
        .get(2)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    match args.first().map(deref) {
        // No callback: the default autoloader, spl_autoload().
        None | Some(Value::Null) => {
            host.register_autoloader(&Value::str(b"spl_autoload".to_vec()), prepend)?;
            Ok(Value::Bool(true))
        }
        Some(cb) => {
            if !host.is_callable_value(&cb) {
                return Err(Thrown::error(
                    "TypeError",
                    "spl_autoload_register(): Argument #1 ($callback) must be a valid callback",
                ));
            }
            host.register_autoloader(&args[0], prepend)?;
            Ok(Value::Bool(true))
        }
    }
}

/// `spl_autoload_unregister(callable $callback): bool`: removes the
/// registered callable equal to `$callback` (autoload_func_info_equals).
fn fn_spl_autoload_unregister(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.unregister_autoloader(&args[0])?))
}

/// `spl_autoload_functions(): array`
fn fn_spl_autoload_functions(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for cb in host.autoload_functions() {
        a.push(cb);
    }
    Ok(Value::Array(a))
}

/// `spl_autoload_call(string $class): void`
fn fn_spl_autoload_call(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    host.autoload_call(&name)?;
    Ok(Value::Null)
}

/// `spl_object_id(object $obj): int` — the object's per-run handle id.
fn fn_spl_object_id(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    match rhp_values::object_id(&v) {
        Some(id) => Ok(Value::Int(id as i64)),
        None => Err(Thrown::type_error(format!(
            "spl_object_id(): Argument #1 ($object) must be of type object, {} given",
            zval_value_name(host, &v)
        ))),
    }
}

/// `__spl_deprecated($message)`: an E_DEPRECATED raised on behalf of an SPL
/// prelude method (php's `#[\Deprecated]` methods / zend_error notices).
fn fn_spl_deprecated(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = conv::to_string_bytes(&deref(&args[0]));
    host.emit_diagnostic(crate::E_DEPRECATED, &msg)?;
    Ok(Value::Null)
}

/// `__spl_user_props($object)`: the object's properties by mangled name,
/// without the engine slots of SPL prelude classes (for `__debugInfo`).
/// SplFileInfo::getRealPath's VCWD_REALPATH: no open_basedir check.
fn fn_spl_realpath(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let p = rhp_values::conv::to_string_bytes(&deref(&args[0]));
    Ok(crate::zend::virtual_cwd::realpath(&p).map_or(Value::Bool(false), Value::str))
}

fn fn_spl_user_props(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => Ok(Value::Array(host.object_props_mangled(&h))),
        _ => Ok(Value::Array(rhp_values::PhpArray::new())),
    }
}

/// `spl_object_hash(object $obj): string`: the handle as 16 hex digits,
/// then 16 zeros (php_spl_object_hash).
fn fn_spl_object_hash(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    match rhp_values::object_id(&v) {
        Some(id) => Ok(Value::str(
            format!("{id:016x}0000000000000000").into_bytes(),
        )),
        None => Err(Thrown::type_error(format!(
            "spl_object_hash(): Argument #1 ($object) must be of type object, {} given",
            zval_value_name(host, &v)
        ))),
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("spl_autoload_register", fn_spl_autoload_register),
            ("spl_autoload_unregister", fn_spl_autoload_unregister),
            ("spl_autoload_functions", fn_spl_autoload_functions),
            ("spl_autoload_call", fn_spl_autoload_call),
            ("spl_object_id", fn_spl_object_id),
            ("spl_object_hash", fn_spl_object_hash),
            ("iterator_to_array", fn_iterator_to_array),
            ("class_implements", fn_class_implements),
            ("class_parents", fn_class_parents),
            ("class_uses", fn_class_uses),
        ],
    );
    crate::register_helpers(
        m,
        &[
            ("__spl_deprecated", fn_spl_deprecated, 1, Some(1)),
            ("__spl_user_props", fn_spl_user_props, 1, Some(1)),
            (
                "__spl_array_debug_info",
                fn_spl_array_debug_info,
                1,
                Some(1),
            ),
            ("__spl_realpath", fn_spl_realpath, 1, Some(1)),
        ],
    );
}

/// SPL's classes in PHP source:
/// - prelude.php: the core data-structure classes (ArrayIterator and
///   friends), backed by a plain array.
/// - files.php: the filesystem classes (php-src `ext/spl/spl_directory.c`):
///   `SplFileInfo`, `DirectoryIterator`, `FilesystemIterator`,
///   `RecursiveDirectoryIterator`, `GlobIterator`, `SplFileObject` and
///   `SplTempFileObject`. Directory entries come in readdir order (php reads
///   the directory stream lazily, unsorted).
/// - iterators.php: the iterators and data structures (php-src
///   `ext/spl/spl_iterators.c`, `spl_heap.c`, `spl_observer.c`).
///   `RecursiveIteratorIterator` is a port of php's lazy per-level state
///   machine (RS_START / RS_TEST / RS_SELF / RS_CHILD / RS_NEXT, with the
///   overridable hooks); the heaps use php's exact sift-up / sift-down so
///   equal elements come out in php's order.
pub const PRELUDE_PHP: &str = concat!(
    include_str!("prelude.php"),
    include_str!("files.php"),
    include_str!("iterators.php")
);

/// `iterator_to_array($iterable, $preserve_keys = true)`. Only a plain array is
/// supported here: driving a user `Iterator` / `Generator` is done by the VM, so
/// a non-array iterable is deferred-to-VM and yields a TypeError from this
/// builtin. For an array it is a (re-keyed when `!preserve_keys`) passthrough.
fn fn_iterator_to_array(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let preserve = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(true);
    match deref(&args[0]) {
        Value::Array(a) => {
            if preserve {
                Ok(Value::Array(a))
            } else {
                let mut out = PhpArray::new();
                for v in a.values() {
                    out.push(v.clone());
                }
                Ok(Value::Array(out))
            }
        }
        _ => Err(Thrown::type_error(
            "iterator_to_array(): driving a non-array Traversable is handled by the VM, \
             not this builtin",
        )),
    }
}

/// (parents, interfaces, traits) as returned by `Host::class_relations`.
type Relations = (Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>);

/// Shared argument handling for `class_implements`/`class_parents`/`class_uses`.
fn relation(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    pick: fn(Relations) -> Vec<Vec<u8>>,
) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    let autoload = bool_param(host, args, 1, func, "autoload", true)?;
    let id = match &v {
        Value::Object(h) => Some(h.borrow().class),
        // A closure is an object of class Closure.
        Value::Closure(_) => host.resolve_class(b"Closure"),
        Value::Str(s) => {
            if autoload {
                host.resolve_class_autoload(s.as_bytes())?
            } else {
                host.resolve_class(s.as_bytes())
            }
        }
        other => {
            return Err(Thrown::type_error(format!(
                "{func}(): Argument #1 ($object_or_class) must be of type object|string, {} given",
                zval_value_name(host, other)
            )))
        }
    };
    // php: Warning "Class X does not exist[ and could not be loaded]"
    let Some(id) = id else {
        return Ok(Value::Bool(false));
    };
    let Some(rel) = host.class_relations(id) else {
        return Ok(Value::Array(PhpArray::new()));
    };
    let mut out = PhpArray::new();
    for n in pick(rel) {
        out.insert(ArrayKey::from_bytes(&n), Value::str(n));
    }
    Ok(Value::Array(out))
}

fn fn_class_implements(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    relation(host, args, "class_implements", |r| r.1)
}

fn fn_class_parents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    relation(host, args, "class_parents", |r| r.0)
}

fn fn_class_uses(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    relation(host, args, "class_uses", |r| r.2)
}
