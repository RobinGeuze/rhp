//! A slice of the `SPL` extension: the autoload registry
//! (`spl_autoload_register` and friends). The callables live on the [`Host`]
//! (the VM), which invokes them when a class reference fails to resolve.

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, ObjectHandle, PhpArray, PhpStr, Value};

use crate::object::{ClassHandlers, ObjectHandlers, PropPurpose};
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
    match deref(&args[0]) {
        Value::Object(h) => Ok(Value::Int(h.borrow().id as i64)),
        _ => Err(Thrown::type_error(
            "spl_object_id(): Argument #1 ($object) must be of type object",
        )),
    }
}

/// `spl_object_hash(object $obj): string` — a 32-hex-char id unique to the
/// object for its lifetime. (Derived from the handle id; not byte-identical to
/// php's hash, but stable and collision-free as a map key.)
/// `__spl_deprecated($message)`: an E_DEPRECATED raised on behalf of an SPL
/// prelude method (php's `#[\Deprecated]` methods / zend_error notices).
fn fn_spl_deprecated(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = conv::to_string_bytes(&deref(&args[0]));
    host.emit_diagnostic(crate::E_DEPRECATED, &String::from_utf8_lossy(&msg))?;
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

fn fn_spl_object_hash(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => {
            let id = h.borrow().id;
            Ok(Value::str(format!("{id:032x}").into_bytes()))
        }
        _ => Err(Thrown::type_error(
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
