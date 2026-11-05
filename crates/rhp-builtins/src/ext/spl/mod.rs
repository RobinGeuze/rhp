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

pub(crate) static CLASSES: &[ClassHandlers] = &[
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
        if let Some(s) = b.properties.get(&key("__s")) {
            let flags = b
                .properties
                .get(&key("__f"))
                .map(|f| conv::to_int(&deref(f)));
            if flags.unwrap_or(0) & 1 != 0 {
                return None;
            }
            return Some(deref(s));
        }
    }
    None
}

/// spl_array_get_properties_for: `(array)`, `var_export` and `json_encode`
/// see an array storage. The debug view (spl_array_get_debug_info, php's
/// `ArrayObject::__debugInfo()`) is the properties plus the storage as the
/// base class's private `storage`, unless a subclass has its own
/// `__debugInfo()`.
fn spl_array_get_properties_for(
    host: &mut dyn Host,
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
        PropPurpose::Debug => {
            let class = h.borrow().class;
            if host.class_has_method(class, b"__debuginfo") {
                return Ok(None);
            }
            let b = h.borrow();
            let mut a = PhpArray::new();
            for (k, v) in b.properties.iter() {
                if !host.is_internal_slot(b.class, k) {
                    a.insert(ArrayKey::Str(host.mangled_prop_name(b.class, k)), v.clone());
                }
            }
            for base in ["ArrayObject", "ArrayIterator"] {
                let key = PhpStr::from(format!("\0{base}\0__s").as_str());
                if let Some(st) = b.properties.get(&key) {
                    let name = PhpStr::from(format!("\0{base}\0storage").as_str());
                    a.insert(ArrayKey::Str(name), deref(st));
                    break;
                }
            }
            Ok(Some(a))
        }
        PropPurpose::Serialize | PropPurpose::GetObjectVars => Ok(None),
    }
}

/// `spl_autoload_register(?callable $callback = null, bool $throw = true, bool $prepend = false): bool`
fn fn_spl_autoload_register(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(deref) {
        // The default SPL autoloader is not supported; registering it is a no-op.
        None | Some(Value::Null) => Ok(Value::Bool(true)),
        Some(cb) => {
            if !host.is_callable_value(&cb) {
                return Err(Thrown::error(
                    "TypeError",
                    "spl_autoload_register(): Argument #1 ($callback) must be a valid callback",
                ));
            }
            let prepend = args
                .get(2)
                .map(|v| conv::to_bool(&deref(v)))
                .unwrap_or(false);
            host.register_autoloader(cb, prepend);
            Ok(Value::Bool(true))
        }
    }
}

/// `spl_autoload_unregister(callable $callback): bool` — not yet supported
/// (callable identity comparison); reports success without removing.
fn fn_spl_autoload_unregister(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
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
fn fn_spl_object_id(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match rhp_values::object_id(&deref(&args[0])) {
        Some(id) => Ok(Value::Int(id as i64)),
        None => Err(Thrown::type_error(
            "spl_object_id(): Argument #1 ($object) must be of type object",
        )),
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
fn fn_spl_user_props(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => Ok(Value::Array(host.object_props_mangled(&h))),
        _ => Ok(Value::Array(rhp_values::PhpArray::new())),
    }
}

/// `spl_object_hash(object $obj): string`: the handle as 16 hex digits,
/// then 16 zeros (php_spl_object_hash).
fn fn_spl_object_hash(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match rhp_values::object_id(&deref(&args[0])) {
        Some(id) => Ok(Value::str(
            format!("{id:016x}0000000000000000").into_bytes(),
        )),
        None => Err(Thrown::type_error(
            "spl_object_hash(): Argument #1 ($object) must be of type object",
        )),
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
                for (_, v) in a.iter() {
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
