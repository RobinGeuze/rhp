//! Object handlers for internal classes: the part of php's
//! `zend_object_handlers` rhp routes, registered by the extension that owns
//! the class and inherited by its subclasses. Native per-object state lives
//! in the object's payload ([`rhp_values::Payload`]). See
//! docs/design/object-handlers.md.

use std::rc::Rc;
use std::sync::OnceLock;

pub use rhp_values::ops::CastTarget;
use rhp_values::{conv, ObjectHandle, PhpArray, Thrown, Value};

use crate::Host;

/// What a property table is fetched for (php's `zend_prop_purpose`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PropPurpose {
    /// `var_dump`, `print_r` (php's default: the `__debugInfo()` result,
    /// else the properties).
    Debug,
    /// `(array)`.
    ArrayCast,
    /// `serialize` of an object without `__serialize` / `__sleep`.
    Serialize,
    /// `var_export`.
    VarExport,
    /// `json_encode` of an object that is not `JsonSerializable`.
    Json,
    /// `get_object_vars`.
    GetObjectVars,
}

/// The operator a `do_operation` handler is asked for (php's opcode). `-$x`
/// arrives as `Mul` by -1 and `++` / `--` as `Add` / `Sub` of 1, as php
/// compiles them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Shl,
    Shr,
    BitOr,
    BitAnd,
    BitXor,
    Concat,
    /// Unary: the second operand is `None`.
    BitNot,
}

/// `get_properties_for`: the table `purpose` sees, keyed as php's property
/// table is (mangled names, or integer keys), or `None` for the default view
/// (php's `zend_std_get_properties_for`).
pub type GetPropertiesFor =
    fn(&mut dyn Host, &ObjectHandle, PropPurpose) -> Result<Option<PhpArray>, Thrown>;

/// `clone_obj` beyond the default copy: runs after the properties were
/// copied into `dst` (which has no payload yet) and before `__clone`.
pub type CloneFn = fn(&mut dyn Host, &ObjectHandle, &ObjectHandle) -> Result<(), Thrown>;

/// `compare(op1, op2)`, called with the operands in order for the object
/// operand whose class registered it (op1 when both are objects).
/// `Ok(None)` is php's `zend_std_compare_objects` (what a handler that
/// starts with `ZEND_COMPARE_OBJECTS_FALLBACK` returns for a pair it does
/// not handle); an error is php's exception, which makes the comparison
/// uncomparable.
pub type CompareFn = fn(&mut dyn Host, &Value, &Value) -> Result<Option<i32>, Thrown>;

/// `do_operation(op, op1, op2)`. `Ok(None)` is FAILURE: the engine goes on
/// with its normal operator (casting the object with `cast_object`, or
/// raising php's operator error). An error is php's SUCCESS with an
/// exception set.
pub type DoOperationFn =
    fn(&mut dyn Host, Op, &Value, Option<&Value>) -> Result<Option<Value>, Thrown>;

/// `cast_object`: the converted value, or `Ok(None)` for FAILURE. A class
/// that registers one replaces php's default for every target, so it hands
/// the targets it does not convert to [`Host::std_cast_object`] (php's
/// `zend_std_cast_object_tostring`), as curl's and SimpleXML's do.
pub type CastFn = fn(&mut dyn Host, &ObjectHandle, CastTarget) -> Result<Option<Value>, Thrown>;

/// `dtor_obj` (once, with a host, before the object's `__destruct`) and
/// `free_obj_host` (once, with a host, after it).
pub type DtorFn = fn(&mut dyn Host, &ObjectHandle) -> Result<(), Thrown>;

/// php's `clone_obj` slot.
#[derive(Clone, Copy)]
pub enum CloneObj {
    /// `zend_objects_clone_obj`: copy the properties. The copy gets no
    /// payload.
    Std,
    /// The copy, then this.
    Custom(CloneFn),
    /// NULL: "Trying to clone an uncloneable object of class X".
    None,
}

/// One internal class's handlers. `free_obj` is the payload's `Drop`
/// ([`rhp_values::Payload`]).
#[derive(Clone, Copy)]
pub struct ObjectHandlers {
    pub get_properties_for: Option<GetPropertiesFor>,
    pub clone_obj: CloneObj,
    pub compare: Option<CompareFn>,
    pub do_operation: Option<DoOperationFn>,
    pub cast_object: Option<CastFn>,
    pub dtor_obj: Option<DtorFn>,
    /// The part of php's `free_obj` that may call PHP (ZipArchive closing
    /// its archive runs the archive's callbacks): run by the destructor
    /// pass right after `__destruct` (in its place when there is none),
    /// unless the destructor kept a reference to the object.
    pub free_obj_host: Option<DtorFn>,
}

impl ObjectHandlers {
    /// php's `std_object_handlers`.
    pub const STD: ObjectHandlers = ObjectHandlers {
        get_properties_for: None,
        clone_obj: CloneObj::Std,
        compare: None,
        do_operation: None,
        cast_object: None,
        dtor_obj: None,
        free_obj_host: None,
    };
    /// `clone_obj = NULL`.
    pub const UNCLONEABLE: ObjectHandlers = ObjectHandlers {
        clone_obj: CloneObj::None,
        ..ObjectHandlers::STD
    };
    /// `clone_obj = NULL`, `compare = zend_objects_not_comparable`: the
    /// opaque handle classes (`Socket`, `XMLParser`, the OpenSSL keys…).
    pub const OPAQUE: ObjectHandlers = ObjectHandlers {
        clone_obj: CloneObj::None,
        compare: Some(not_comparable),
        ..ObjectHandlers::STD
    };
}

/// An extension's registration of its class's handlers (php's
/// `ce->create_object` and handler table, set in MINIT).
pub struct ClassHandlers {
    pub class: &'static str,
    pub handlers: &'static ObjectHandlers,
    /// php gives the class a `create_object` (its objects carry native
    /// state); false when it only replaces `default_object_handlers`
    /// (`Directory`). `ReflectionClass::newInstanceWithoutConstructor`
    /// refuses a final internal class that has one. Inherited.
    pub create_object: bool,
}

/// `zend_objects_not_comparable`: equal only to itself (the engine checks
/// identity first), uncomparable to anything else.
pub fn not_comparable(_: &mut dyn Host, _: &Value, _: &Value) -> Result<Option<i32>, Thrown> {
    Ok(Some(1))
}

/// php's `zend_std_cast_object_tostring` for every target but strings,
/// which need the engine to call `__toString()` (see
/// [`Host::std_cast_object`]).
pub fn std_cast_non_string(target: CastTarget) -> Option<Value> {
    match target {
        CastTarget::Bool => Some(Value::Bool(true)),
        _ => None,
    }
}

/// The engine's own classes with handlers (Zend/, and Reflection, which
/// php registers like an extension but rhp keeps in its core prelude).
const CORE: &[ClassHandlers] = &[
    // default_exception_handlers
    ClassHandlers {
        class: "Exception",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "Error",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "Generator",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "Fiber",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "WeakReference",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    // dir_class_object_handlers
    ClassHandlers {
        class: "Directory",
        handlers: &ObjectHandlers::OPAQUE,
        create_object: false,
    },
    // Classes with a create_object and the default handlers.
    ClassHandlers {
        class: "Closure",
        handlers: &ObjectHandlers::STD,
        create_object: true,
    },
    ClassHandlers {
        class: "WeakMap",
        handlers: &ObjectHandlers::STD,
        create_object: true,
    },
    ClassHandlers {
        class: "__PHP_Incomplete_Class",
        handlers: &ObjectHandlers::STD,
        create_object: true,
    },
    ClassHandlers {
        class: "HashContext",
        handlers: &ObjectHandlers::STD,
        create_object: true,
    },
];

/// The classes php gives `reflection_object_handlers` (`clone_obj = NULL`);
/// their subclasses inherit them.
const REFLECTION: &[ClassHandlers] = &[
    ClassHandlers {
        class: "ReflectionFunctionAbstract",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionGenerator",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionParameter",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionType",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionClass",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionProperty",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionClassConstant",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionExtension",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionZendExtension",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionReference",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionAttribute",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionFiber",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "ReflectionConstant",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
];

/// The registration of the internal class `name` (any case), if any. The
/// engine looks this up once per internal class, when it links it; a
/// subclass takes its parent's.
pub fn registered(name: &[u8]) -> Option<&'static ClassHandlers> {
    fn cmp_ci(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
        a.iter()
            .map(u8::to_ascii_lowercase)
            .cmp(b.iter().map(u8::to_ascii_lowercase))
    }
    static TABLE: OnceLock<Vec<&'static ClassHandlers>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t: Vec<&'static ClassHandlers> = CORE
            .iter()
            .chain(REFLECTION)
            .chain(crate::ext::class_handlers())
            .collect();
        t.sort_by(|a, b| cmp_ci(a.class.as_bytes(), b.class.as_bytes()));
        for w in t.windows(2) {
            assert!(
                cmp_ci(w[0].class.as_bytes(), w[1].class.as_bytes()).is_ne(),
                "handlers for {} registered twice",
                w[0].class
            );
        }
        t
    });
    t.binary_search_by(|r| cmp_ci(r.class.as_bytes(), name))
        .ok()
        .map(|i| t[i])
}

/// The object behind `v` (through a reference).
pub fn object_of(v: &Value) -> Option<ObjectHandle> {
    match v {
        Value::Object(h) => Some(h.clone()),
        Value::Ref(r) => object_of(&r.borrow()),
        _ => None,
    }
}

/// The payload of the object `v`, when it is a `T`.
pub fn payload<T: std::any::Any>(v: &Value) -> Option<Rc<T>> {
    object_of(v)?.borrow().payload::<T>()
}

/// The payload of `h`, when it is a `T`.
pub fn payload_of<T: std::any::Any>(h: &ObjectHandle) -> Option<Rc<T>> {
    h.borrow().payload::<T>()
}

/// Attach `state` to `h`, dropping any previous payload after the borrow.
pub fn set_payload<T: std::any::Any>(h: &ObjectHandle, state: Rc<T>) {
    let old = h.borrow_mut().set_payload(state);
    drop(old);
}

/// The class's own handlers for `v`'s object, through the host.
pub fn handlers_of(host: &dyn Host, v: &Value) -> Option<&'static ObjectHandlers> {
    let h = object_of(v)?;
    let class = h.borrow().class;
    host.object_handlers(class)
}

/// php's `zend_get_properties_for` from a builtin: the handler's table for
/// `purpose`, or `None` when the class has none for it (the caller then
/// builds the default view).
pub fn properties_for(
    host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    let class = h.borrow().class;
    match host
        .object_handlers(class)
        .and_then(|t| t.get_properties_for)
    {
        Some(f) => f(host, h, purpose),
        None => Ok(None),
    }
}

/// Whether `v` is an object whose class has the handler table `table`
/// (php's checks on an object's handlers, e.g.
/// `ZEND_COMPARE_OBJECTS_FALLBACK`).
pub fn has_handlers(host: &dyn Host, v: &Value, table: &'static ObjectHandlers) -> bool {
    handlers_of(host, v).is_some_and(|t| std::ptr::eq(t, table))
}

/// The `cast_object` handler of `v`'s class applied to `v`; `None` when `v`
/// is not an object with one, or the handler fails. The caller then goes
/// on as for any object.
pub fn cast(host: &mut dyn Host, v: &Value, target: CastTarget) -> Result<Option<Value>, Thrown> {
    let Some(h) = object_of(v) else {
        return Ok(None);
    };
    let class = h.borrow().class;
    match host.object_handlers(class).and_then(|t| t.cast_object) {
        Some(f) => f(host, &h, target),
        None => Ok(None),
    }
}

/// `cast_object(IS_STRING)` of an object, as `zval_get_string` and
/// `zend_parse_arg_str_weak` do it: the class's handler, else
/// `__toString()`. `None` when `v` is not an object or has no string form.
pub fn to_string(host: &mut dyn Host, v: &Value) -> Result<Option<Vec<u8>>, Thrown> {
    let Some(h) = object_of(v) else {
        return Ok(None);
    };
    let class = h.borrow().class;
    let cast = match host.object_handlers(class).and_then(|t| t.cast_object) {
        Some(f) => f(host, &h, CastTarget::Str)?,
        None => host.std_cast_object(&h, CastTarget::Str)?,
    };
    Ok(cast.map(|s| conv::to_string_bytes(&s)))
}

/// php's `zend_is_true`: an object's `cast_object(_IS_BOOL)` (a zero GMP,
/// an empty SimpleXML list are false), else the value's truthiness.
pub fn is_true(host: &mut dyn Host, v: &Value) -> Result<bool, Thrown> {
    Ok(match cast(host, v, CastTarget::Bool)? {
        Some(b) => conv::to_bool(&b),
        None => conv::to_bool(v),
    })
}

/// `zval_get_long` / `zval_get_double` of an object (`(int)`, `intval()`,
/// …): its `cast_object`, else php's "could not be converted" warning
/// (`convert_object_to_type`) and 1. `None` when `v` is not an object.
pub fn object_to_number(
    host: &mut dyn Host,
    v: &Value,
    target: CastTarget,
) -> Result<Option<Value>, Thrown> {
    let class = match v {
        Value::Object(h) => {
            let class = h.borrow().class;
            host.class_name(class).unwrap_or_default()
        }
        Value::Closure(_) => b"Closure".to_vec(),
        Value::Ref(r) => {
            let inner = r.borrow().clone();
            return object_to_number(host, &inner, target);
        }
        _ => return Ok(None),
    };
    if let Some(n) = cast(host, v, target)? {
        return Ok(Some(n));
    }
    let (ty, one) = match target {
        CastTarget::Float => ("float", Value::Float(1.0)),
        _ => ("int", Value::Int(1)),
    };
    host.emit_diagnostic(
        crate::E_WARNING,
        &format!(
            "Object of class {} could not be converted to {ty}",
            String::from_utf8_lossy(&class)
        ),
    )?;
    Ok(Some(one))
}

#[cfg(test)]
mod tests;
