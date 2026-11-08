//! Zend/zend_builtin_functions.c: the engine's own functions: `strlen` and
//! the `strcmp` family, `define`/`defined`, the class, function and constant
//! table queries (the tables live in the host, read through
//! `Host::reflect_op`), error handlers, backtraces and `gc_*`.

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, ClassKind, PhpArray, Value};

use crate::const_modules::CONST_MODULES;
use crate::ext::standard::string::find_subslice;
use crate::func_modules::FUNC_MODULES;
use crate::zend::exceptions::trace_to_string;
use crate::zend::zpp::{arg_bytes, bool_param, str_param, zval_get_string, zval_value_name};
use crate::{ext, zend, Host, Thrown, E_DEPRECATED, E_WARNING};

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

/// copy_constant_array: an array without references, or None when it
/// contains itself (validate_constant_array_argument).
fn constant_array(a: &PhpArray, seen: &mut Vec<*const ()>) -> Option<PhpArray> {
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let v = match v {
            Value::Ref(r) => {
                let p = std::rc::Rc::as_ptr(r) as *const ();
                if seen.contains(&p) {
                    return None;
                }
                seen.push(p);
                let inner = r.borrow().clone();
                let v = match &inner {
                    Value::Array(a) => Value::Array(constant_array(a, seen)?),
                    other => other.clone(),
                };
                seen.pop();
                v
            }
            Value::Array(a) => Value::Array(constant_array(a, seen)?),
            other => other.clone(),
        };
        out.insert(k.clone(), v);
    }
    Some(out)
}

fn fn_define(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    if find_subslice(&name, b"::").is_some() {
        return Err(Thrown::error(
            "ValueError",
            "define(): Argument #1 ($constant_name) cannot be a class constant",
        ));
    }
    let mut value = deref(&args[1]).clone();
    if let Value::Array(a) = &value {
        match constant_array(a, &mut Vec::new()) {
            Some(a) => value = Value::Array(a),
            None => {
                return Err(Thrown::error(
                    "ValueError",
                    "define(): Argument #2 ($value) cannot be a recursive array",
                ))
            }
        }
    }
    if args.get(2).is_some_and(|v| conv::to_bool(&deref(v))) {
        host.emit_diagnostic(
            E_WARNING,
            b"define(): Argument #3 ($case_insensitive) is ignored since declaration of case-insensitive constants is no longer supported",
        )?;
    }
    if host.define_const(&name, value) {
        return Ok(Value::Bool(true));
    }
    host.emit_diagnostic(E_WARNING, &const_redefined_message(&name))?;
    Ok(Value::Bool(false))
}

/// A global constant's key in the constant table: php lowercases the
/// namespace part (`Foo\Bar\X` → `foo\bar\X`) and keeps the short name.
/// Like php's `strrchr`, it only looks before the first NUL byte.
pub fn const_key(name: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let c_str = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
    match c_str.iter().rposition(|&b| b == b'\\') {
        Some(ns) if name[..ns].iter().any(u8::is_ascii_uppercase) => {
            let mut key = name.to_vec();
            key[..ns].make_ascii_lowercase();
            key.into()
        }
        _ => name.into(),
    }
}

/// zend_register_constant's warning for a name that is taken: the table
/// key, printed as a C string.
pub fn const_redefined_message(name: &[u8]) -> Vec<u8> {
    let key = const_key(name);
    let shown = &key[..key.iter().position(|&b| b == 0).unwrap_or(key.len())];
    [
        b"Constant ".as_slice(),
        shown,
        b" already defined, this will be an error in PHP 9",
    ]
    .concat()
}

/// zend_get_constant_ex: a class constant `Class::CONST` (incl. enum
/// cases; `self`/`parent`/`static` are the caller's) or a global constant.
/// A leading `\` is skipped. With `silent` (defined()), a missing or
/// inaccessible constant is `None`; otherwise php's Error.
pub(crate) fn resolve_const(
    host: &mut dyn Host,
    name: &[u8],
    silent: bool,
) -> Result<Option<Value>, Thrown> {
    let name = name.strip_prefix(b"\\").unwrap_or(name);
    // The last "::" (php's memrchr for ':' preceded by ':').
    let colon = name
        .iter()
        .rposition(|&b| b == b':')
        .filter(|&i| i > 0 && name[i - 1] == b':');
    if let Some(i) = colon {
        return host.class_constant_eval(&name[..i - 1], &name[i + 1..], silent);
    }
    match host.get_const(name) {
        Some(v) => {
            if !silent {
                host.const_fetch_deprecation(name)?;
            }
            Ok(Some(v))
        }
        None if silent => Ok(None),
        None => Err(Thrown::error(
            "Error",
            format!("Undefined constant \"{}\"", String::from_utf8_lossy(name)),
        )),
    }
}

fn fn_defined(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    Ok(Value::Bool(resolve_const(host, &name, true)?.is_some()))
}

fn fn_strlen(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // Without copying the string (it may be most of memory_limit).
    let n = match &args[0] {
        Value::Str(s) => s.as_bytes().len(),
        v => conv::to_string_bytes(v).len(),
    };
    Ok(Value::Int(n as i64))
}

fn fn_get_class(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(obj) = args.first() else {
        let Some(name) = host.caller_scope().and_then(|c| host.class_name(c)) else {
            return Err(Thrown::error(
                "Error",
                "get_class() without arguments must be called from within a class",
            ));
        };
        host.emit_diagnostic(
            E_DEPRECATED,
            b"Calling get_class() without arguments is deprecated",
        )?;
        return Ok(Value::str(name));
    };
    match deref(obj) {
        Value::Object(h) => {
            let cid = h.borrow().class;
            Ok(host
                .class_name(cid)
                .map(Value::str)
                .unwrap_or(Value::Bool(false)))
        }
        Value::Closure(_) => Ok(Value::str("Closure")),
        other => Err(Thrown::type_error(format!(
            "get_class(): Argument #1 ($object) must be of type object, {} given",
            zend::zpp::zval_value_name(host, &other)
        ))),
    }
}

/// Class name of an `is_a`/`is_subclass_of` subject: an object's class, or the
/// string itself when `allow_string`.
/// A string subject is looked up with autoloading (zend_lookup_class).
fn subject_class(
    host: &mut dyn Host,
    subject: &Value,
    allow_string: bool,
) -> Result<Option<Vec<u8>>, Thrown> {
    Ok(match subject {
        Value::Object(h) => host.class_name(h.borrow().class),
        Value::Closure(_) => Some(b"Closure".to_vec()),
        Value::Str(s) if allow_string => {
            host.resolve_class_autoload(s.as_bytes())?;
            Some(s.as_bytes().to_vec())
        }
        _ => None,
    })
}

fn norm_class(n: &[u8]) -> Vec<u8> {
    n.strip_prefix(b"\\").unwrap_or(n).to_ascii_lowercase()
}

fn fn_is_a(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let subject = deref(&args[0]);
    let target = conv::to_string_bytes(&deref(&args[1]));
    let allow_string = args
        .get(2)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    match subject_class(host, &subject, allow_string)? {
        Some(n) => Ok(Value::Bool(host.reflect_is_a(&n, &target))),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_is_subclass_of(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let subject = deref(&args[0]);
    let target = conv::to_string_bytes(&deref(&args[1]));
    let allow_string = args
        .get(2)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(true);
    match subject_class(host, &subject, allow_string)? {
        Some(n) => {
            // Subtype but NOT the same class (is_subclass_of is exclusive;
            // an alias names the same class).
            let same = norm_class(&n) == norm_class(&target)
                || host
                    .resolve_class(&n)
                    .is_some_and(|a| host.resolve_class(&target) == Some(a));
            Ok(Value::Bool(!same && host.reflect_is_a(&n, &target)))
        }
        None => Ok(Value::Bool(false)),
    }
}

fn fn_get_parent_class(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(deref) {
        Some(Value::Object(h)) => {
            let cid = h.borrow().class;
            Ok(host
                .class_parent_name(cid)
                .map(Value::str)
                .unwrap_or(Value::Bool(false)))
        }
        Some(Value::Closure(_)) => Ok(Value::Bool(false)),
        Some(v @ Value::Str(_)) => {
            let name = conv::to_string_bytes(&v);
            match host.resolve_class_autoload(&name)? {
                Some(cid) => Ok(host
                    .class_parent_name(cid)
                    .map(Value::str)
                    .unwrap_or(Value::Bool(false))),
                None => Err(Thrown::type_error(
                    "get_parent_class(): Argument #1 ($object_or_class) must be an object or a valid class name, string given",
                )),
            }
        }
        // The calling scope's parent.
        None => {
            host.emit_diagnostic(
                E_DEPRECATED,
                b"Calling get_parent_class() without arguments is deprecated",
            )?;
            Ok(host
                .caller_scope()
                .and_then(|c| host.class_parent_name(c))
                .map(Value::str)
                .unwrap_or(Value::Bool(false)))
        }
        Some(other) => Err(Thrown::type_error(format!(
            "get_parent_class(): Argument #1 ($object_or_class) must be an object or a valid class name, {} given",
            zend::zpp::zval_value_name(host, &other)
        ))),
    }
}

/// Resolve the class id named by an argument: an object yields its own class,
/// a string resolves via the host's class table.
fn class_id_of(host: &mut dyn Host, v: &Value) -> Result<Option<u32>, Thrown> {
    Ok(match deref(v) {
        Value::Object(h) => Some(h.borrow().class),
        Value::Closure(_) => host.resolve_class(b"Closure"),
        Value::Str(s) => host.resolve_class_autoload(s.as_bytes())?,
        _ => None,
    })
}

/// The TypeError of method_exists() / property_exists() for an argument
/// that is neither an object nor a string.
fn object_or_class_error(host: &dyn Host, func: &str, v: &Value) -> Option<Thrown> {
    match deref(v) {
        Value::Object(_) | Value::Closure(_) | Value::Str(_) => None,
        other => Some(Thrown::type_error(format!(
            "{func}(): Argument #1 ($object_or_class) must be of type object|string, {} given",
            crate::zend::zpp::zval_value_name(host, &other)
        ))),
    }
}

fn fn_method_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(e) = object_or_class_error(host, "method_exists", &args[0]) {
        return Err(e);
    }
    let method = conv::to_string_bytes(&deref(&args[1]));
    // On a class name, a parent's private method does not count.
    let by_name = !matches!(deref(&args[0]), Value::Object(_));
    let found = class_id_of(host, &args[0])?
        .map(|cid| {
            (host.class_has_method(cid, &method)
                && !host.class_method_hidden(cid, &method)
                && !(by_name && host.class_method_private_inherited(cid, &method)))
                // zend_closure_get_method: Closure's __invoke; the
                // instance methods the engine implements.
                || ([&b"__invoke"[..], b"bindto", b"call"]
                    .iter()
                    .any(|m| method.eq_ignore_ascii_case(m))
                    && host
                        .class_name(cid)
                        .is_some_and(|n| n.eq_ignore_ascii_case(b"Closure")))
                || enum_has_builtin_method(host, cid, &method)
        })
        .unwrap_or(false);
    let found = found
        || {
            let obj = deref(&args[0]);
            matches!(obj, Value::Object(_))
                && (spl_inner_has_method(host, &obj, &method, 0)
                // An object's get_method handler (PDO's driver methods),
                // unless a __call trampoline answers first.
                || (crate::zend::object_handlers::handlers_of(host, &obj)
                    .is_some_and(|t| crate::zend::object_handlers::driver_method(t, &method).is_some())
                    && !class_id_of(host, &obj)
                        .ok()
                        .flatten()
                        .is_some_and(|cid| host.class_has_method(cid, b"__call"))))
        };
    Ok(Value::Bool(found))
}

/// zend_enum_register_funcs: an enum's `cases()`, and a backed enum's
/// `from()` and `tryFrom()`.
fn enum_has_builtin_method(host: &dyn Host, cid: u32, method: &[u8]) -> bool {
    if !host.is_enum_class(cid) {
        return false;
    }
    let lc = method.to_ascii_lowercase();
    match &lc[..] {
        b"cases" => true,
        b"from" | b"tryfrom" => host
            .class_name(cid)
            .is_some_and(|n| host.reflect_is_a(&n, b"BackedEnum")),
        _ => false,
    }
}

/// spl_dual_it_get_method / spl_recursive_it_get_method: an SPL outer
/// iterator answers for the methods of its inner (or current sub-)
/// iterator, so method_exists() finds them.
fn spl_inner_has_method(host: &mut dyn Host, obj: &Value, method: &[u8], depth: u32) -> bool {
    let Some(name) = host.object_class_name(obj) else {
        return false;
    };
    let inner = if host.reflect_is_a(&name, b"IteratorIterator") {
        host.object_prop(obj, b"__it")
    } else if host.reflect_is_a(&name, b"RecursiveIteratorIterator") {
        let level = host
            .object_prop(obj, b"__level")
            .map(|l| conv::to_int(&deref(&l)));
        match (host.object_prop(obj, b"__its").map(|v| deref(&v)), level) {
            (Some(Value::Array(a)), Some(l)) => a.get(&rhp_values::ArrayKey::Int(l)).cloned(),
            _ => None,
        }
    } else {
        return false;
    };
    let Some(inner @ Value::Object(_)) = inner.map(|v| deref(&v)) else {
        return false;
    };
    let Ok(Some(cid)) = class_id_of(host, &inner) else {
        return false;
    };
    host.class_has_method(cid, method)
        || (depth < 64 && spl_inner_has_method(host, &inner, method, depth + 1))
}

fn fn_property_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(e) = object_or_class_error(host, "property_exists", &args[0]) {
        return Err(e);
    }
    let prop = conv::to_string_bytes(&deref(&args[1]));
    let found = match deref(&args[0]) {
        // SimpleXML's has_property handler: a child element (or attribute)
        // of that name.
        v @ Value::Object(_) if ext::simplexml::is_sxe_object(&v) => {
            let mut cb = PhpArray::new();
            cb.push(v.clone());
            cb.push(Value::str("__isset"));
            conv::to_bool(&host.call_callable(&Value::Array(cb), &[Value::str(prop)])?)
        }
        Value::Object(h) => {
            let cid = h.borrow().class;
            // An object matches on a declared member (instance or static) OR a
            // property reachable and set from the calling scope.
            host.class_has_property(cid, &prop)
                || internal_declared_property(host, cid, &prop)
                || host.object_prop_exists(&Value::Object(h.clone()), &prop)
        }
        Value::Str(s) => host
            .resolve_class_autoload(s.as_bytes())?
            .map(|cid| {
                host.class_has_property(cid, &prop) || internal_declared_property(host, cid, &prop)
            })
            .unwrap_or(false),
        _ => false,
    };
    Ok(Value::Bool(found))
}

/// A property an internal class declares but rhp serves from a handler
/// (DatePeriod, DOM, XMLReader).
fn internal_declared_property(host: &dyn Host, cid: u32, prop: &[u8]) -> bool {
    ext::date::has_virtual_property(host, cid, prop)
        || ext::dom::has_declared_property(host, cid, prop)
        || ext::xmlreader::has_declared_property(host, cid, prop)
}

/// Shared helper for `class_exists` / `interface_exists` / `enum_exists`: resolve
/// the name and test the resolved kind against `want`.
fn exists_of_kind(
    host: &mut dyn Host,
    args: &[Value],
    want: fn(rhp_values::ClassKind) -> bool,
) -> Result<bool, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    // `$autoload` defaults to true: a miss triggers registered autoloaders.
    let autoload = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(true);
    let cid = if autoload {
        host.resolve_class_autoload(&name)?
    } else {
        host.resolve_class(&name)
    };
    Ok(cid
        .and_then(|cid| host.class_kind(cid))
        .map(want)
        .unwrap_or(false))
}

fn fn_class_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use rhp_values::ClassKind;
    // Enums are classes too (`class_exists` is true for an enum name).
    Ok(Value::Bool(exists_of_kind(host, args, |k| {
        matches!(k, ClassKind::Class | ClassKind::Enum)
    })?))
}

fn fn_interface_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use rhp_values::ClassKind;
    Ok(Value::Bool(exists_of_kind(host, args, |k| {
        k == ClassKind::Interface
    })?))
}

fn fn_enum_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use rhp_values::ClassKind;
    Ok(Value::Bool(exists_of_kind(host, args, |k| {
        k == ClassKind::Enum
    })?))
}

fn fn_get_object_vars(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => {
            if let Some(a) = zend::object_handlers::properties_for(
                host,
                &h,
                zend::object_handlers::PropPurpose::GetObjectVars,
            )? {
                return Ok(Value::Array(a));
            }
            if let Some(v) = host.object_vars(&args[0]) {
                return Ok(v);
            }
            let mut arr = PhpArray::new();
            for (k, v) in h.borrow().props().iter() {
                arr.insert(rhp_values::ArrayKey::Str(k.clone()), v.clone());
            }
            Ok(Value::Array(arr))
        }
        Value::Closure(_) => Ok(Value::Array(PhpArray::new())),
        _ => Err(Thrown::type_error(
            "get_object_vars(): Argument #1 must be of type object",
        )),
    }
}

/// `get_mangled_object_vars()`: the property table under php's mangled
/// names, ignoring scope and the class's property hooks.
fn fn_get_mangled_object_vars(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => {
            // ARRAY_CAST: no lazy initialization.
            let h = rhp_values::lazy_target(&h);
            Ok(Value::Array(host.object_props_mangled(&h)))
        }
        v => Err(Thrown::type_error(format!(
            "get_mangled_object_vars(): Argument #1 ($object) must be of type object, {} given",
            v.type_name()
        ))),
    }
}

/// Extensions this engine reports as loaded, under php's own module names and
/// in php 8.5.8's `get_loaded_extensions()` order. These are the ones whose
/// functions/classes rhp actually provides. Every one is a module
/// (`registry::modules`); the test `modules_are_php_modules` lists the
/// modules left out.
pub(crate) const LOADED_EXTS: &[&str] = &[
    "Core",
    "date",
    "openssl",
    "pcre",
    "sqlite3",
    "zlib",
    "bcmath",
    "bz2",
    "ctype",
    "curl",
    "json",
    #[cfg(feature = "mbstring-libmbfl")]
    "mbstring",
    "SPL",
    "fileinfo",
    "filter",
    "gd",
    "gmp",
    "hash",
    "iconv",
    "intl",
    "session",
    "standard",
    "libxml",
    // Built in but disabled (php's CLI default; the CGI harness runs php
    // with opcache.enable=0): the functions exist and report "not started".
    "Zend OPcache",
    "pcntl",
    "PDO",
    "pdo_sqlite",
    "Phar",
    "posix",
    "random",
    "readline",
    "Reflection",
    "SimpleXML",
    "tokenizer",
    "dom",
    "xml",
    "xmlreader",
    "xmlwriter",
    "zip",
];

/// The extensions the running SAPI has loaded (pcntl only under the CLI and
/// CGI).
fn sapi_exts() -> impl Iterator<Item = &'static str> {
    LOADED_EXTS
        .iter()
        .copied()
        .filter(|e| crate::registry::module_loaded(e))
}

/// Every extension rhp reports as loaded under some SAPI ([`LOADED_EXTS`]),
/// whichever SAPI runs: `rhp-difftest gen readme` gives each its own row.
pub fn reported_extension_names() -> &'static [&'static str] {
    LOADED_EXTS
}

/// The loaded-extension list (php's module names), for phpinfo(INFO_MODULES).
pub(crate) fn loaded_extension_names() -> Vec<&'static str> {
    sapi_exts().collect()
}

/// The running SAPI's own module (php-cgi's `cgi-fcgi`), which
/// `get_loaded_extensions()` lists last.
fn sapi_ext() -> Option<&'static str> {
    crate::main::sapi::sapi_module()
        .and_then(|d| d.module)
        .map(|(name, _)| name)
}

/// The loaded extension matching `name` case-insensitively (php's module
/// lookup lowercases), if any.
pub(crate) fn loaded_ext(name: &[u8]) -> Option<&'static str> {
    let named = |e: &&str| e.as_bytes().eq_ignore_ascii_case(name);
    match LOADED_EXTS.iter().copied().find(named) {
        Some(e) => crate::registry::module_loaded(e).then_some(e),
        None => sapi_ext().filter(named),
    }
}

/// `phpversion($ext)`: most bundled extensions report PHP_VERSION.
pub(crate) fn ext_version(ext: &str) -> &'static str {
    match ext {
        "dom" => crate::version::EXT_DOM,
        "zip" => crate::version::EXT_ZIP,
        _ => crate::version::PHP_VERSION,
    }
}

fn fn_extension_loaded(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(loaded_ext(&arg_bytes(args, 0)).is_some()))
}

fn fn_get_loaded_extensions(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let zend = crate::zend::zpp::bool_param(
        host,
        args,
        0,
        "get_loaded_extensions",
        "zend_extensions",
        false,
    )?;
    let mut a = PhpArray::new();
    if !zend {
        for e in sapi_exts().chain(sapi_ext()) {
            a.push(Value::str(e));
        }
    } else {
        a.push(Value::str("Zend OPcache"));
    }
    Ok(Value::Array(a))
}

/// `get_extension_funcs($extension)`: the functions of a loaded module, in
/// php's order ("zend" names Core). php's list for the module, as far as
/// rhp has the functions (builtin or prelude) under the running SAPI, with
/// the SAPI's additional functions under `standard`; the SAPI's own module
/// lists its functions. An array, maybe empty, for a module php gives
/// functions; false for one it gives none or that is not loaded.
fn fn_get_extension_funcs(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "get_extension_funcs", "extension")?;
    let module = if name.eq_ignore_ascii_case(b"zend") {
        Some("Core")
    } else {
        loaded_ext(&name)
    };
    let Some(module) = module else {
        return Ok(Value::Bool(false));
    };
    let sapi = crate::main::sapi::sapi_module();
    let mut names: Vec<&str> = match sapi.and_then(|d| d.module) {
        Some((m, fns)) if m == module => fns.iter().map(|(n, _)| *n).collect(),
        _ => match FUNC_MODULES.iter().find(|(m, _)| *m == module) {
            Some((_, fns)) => fns.split(' ').collect(),
            None => return Ok(Value::Bool(false)),
        },
    };
    names.retain(|n| host.internal_function_exists(n.as_bytes()));
    if module == "standard" {
        for (n, _) in sapi.map_or(&[][..], |d| d.functions) {
            if !names.contains(n) && !crate::main::sapi::additional_left_out(n) {
                names.push(n);
            }
        }
    }
    let mut a = PhpArray::new();
    for n in names {
        a.push(Value::str(n));
    }
    Ok(Value::Array(a))
}

fn fn_function_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // A leading "\" is ignored, as in zif_function_exists.
    let name = arg_bytes(args, 0);
    let name = name.strip_prefix(b"\\").unwrap_or(&name);
    Ok(Value::Bool(host.function_exists(name)))
}

/// zend_forbid_dynamic_call: the scope functions refuse a dynamic call.
pub(crate) fn forbid_dynamic_call(host: &dyn Host, name: &str) -> Result<(), Thrown> {
    if host.call_is_dynamic() {
        return Err(Thrown::error(
            "Error",
            format!("Cannot call {name}() dynamically"),
        ));
    }
    Ok(())
}

/// php's `EX(prev_execute_data)` is top-level code (ZEND_CALL_CODE): a
/// callback some builtin made has that builtin's frame there instead.
fn called_from_code(host: &dyn Host) -> bool {
    host.current_arg_count().is_none() && !(host.call_is_dynamic() && host.called_back_by_builtin())
}

fn fn_func_num_args(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    if called_from_code(host) {
        return Err(Thrown::error(
            "Error",
            "func_num_args() must be called from a function context",
        ));
    }
    forbid_dynamic_call(host, "func_num_args")?;
    Ok(Value::Int(host.current_arg_count().unwrap_or(0)))
}

/// `get_defined_vars()`: a copy of the calling scope's variables.
fn fn_get_defined_vars(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    forbid_dynamic_call(host, "get_defined_vars")?;
    Ok(Value::Array(host.scope_vars()))
}

fn fn_func_get_args(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    if called_from_code(host) {
        return Err(Thrown::error(
            "Error",
            "func_get_args() cannot be called from the global scope",
        ));
    }
    forbid_dynamic_call(host, "func_get_args")?;
    let args = host.current_args().unwrap_or_default();
    let mut a = PhpArray::new();
    for v in args {
        a.push(v);
    }
    Ok(Value::Array(a))
}

fn fn_func_get_arg(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let i = conv::to_int(&deref(&args[0]));
    if i < 0 {
        return Err(Thrown::error(
            "ValueError",
            "func_get_arg(): Argument #1 ($position) must be greater than or equal to 0",
        ));
    }
    if called_from_code(host) {
        return Err(Thrown::error(
            "Error",
            "func_get_arg() cannot be called from the global scope",
        ));
    }
    forbid_dynamic_call(host, "func_get_arg")?;
    let n = host.current_arg_count().unwrap_or(0);
    if i >= n {
        return Err(Thrown::error("ValueError", 
            "func_get_arg(): Argument #1 ($position) must be less than the number of the arguments passed to the currently executed function",
        ));
    }
    Ok(host
        .current_args()
        .and_then(|a| a.into_iter().nth(i as usize))
        .unwrap_or(Value::Null))
}

/// `get_called_class()`: the late-static-binding class of the caller.
fn fn_get_called_class(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    match host.caller_called_scope().and_then(|c| host.class_name(c)) {
        Some(n) => Ok(Value::str(n)),
        None => Err(Thrown::error(
            "Error",
            "get_called_class() must be called from within a class",
        )),
    }
}

/// `debug_backtrace(int $options = DEBUG_BACKTRACE_PROVIDE_OBJECT, int $limit = 0)`.
fn fn_debug_backtrace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use rhp_values::ArrayKey as K;
    let options = args.first().map(|v| conv::to_int(&deref(v))).unwrap_or(1);
    let limit = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if let Some(Value::Array(t)) = host.backtrace(options & 1 != 0, options & 2 != 0) {
        if limit <= 0 {
            return Ok(Value::Array(t));
        }
        let mut out = PhpArray::new();
        for v in t.values().take(limit as usize) {
            out.push(v.clone());
        }
        return Ok(Value::Array(out));
    }
    let mut arr = PhpArray::new();
    for (func, class, file, line, this) in host.call_stack() {
        let mut e = PhpArray::new();
        if !file.is_empty() {
            e.insert(K::from_bytes(b"file"), Value::str(file));
            e.insert(K::from_bytes(b"line"), Value::Int(line as i64));
        }
        e.insert(K::from_bytes(b"function"), Value::str(func));
        if let Some(c) = class {
            e.insert(K::from_bytes(b"class"), Value::str(c));
            e.insert(K::from_bytes(b"type"), Value::str("->"));
            if let Some(obj) = this {
                e.insert(K::from_bytes(b"object"), obj);
            }
        }
        arr.push(Value::Array(e));
    }
    Ok(Value::Array(arr))
}

/// `debug_print_backtrace(int $options = 0, int $limit = 0)`: the trace in
/// getTraceAsString() format, without the `{main}` line.
fn fn_debug_print_backtrace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let options = args.first().map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let limit = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if let Some(Value::Array(t)) = host.backtrace(false, options & 2 != 0) {
        let mut frames = PhpArray::new();
        for v in t.values().take(if limit > 0 {
            limit as usize
        } else {
            usize::MAX
        }) {
            frames.push(v.clone());
        }
        let mut s = trace_to_string(host, &Value::Array(frames));
        // Strip the trailing "#N {main}".
        if let Some(pos) = s.iter().rposition(|&b| b == b'#') {
            s.truncate(pos);
        }
        host.write_out(&s);
        return Ok(Value::Null);
    }
    let mut out = Vec::new();
    for (i, (func, class, file, line, _this)) in host.call_stack().into_iter().enumerate() {
        let call = match &class {
            Some(c) => format!(
                "{}->{}",
                String::from_utf8_lossy(c),
                String::from_utf8_lossy(&func)
            ),
            None => String::from_utf8_lossy(&func).into_owned(),
        };
        out.extend_from_slice(
            format!(
                "#{} {}() called at [{}:{}]\n",
                i,
                call,
                String::from_utf8_lossy(&file),
                line
            )
            .as_bytes(),
        );
    }
    host.write_out(&out);
    Ok(Value::Null)
}

/// EG(error_reporting): the `error_reporting` directive, E_ALL while unset
/// (`@` lowers it to the fatal types while it runs).
pub fn error_reporting_level(host: &dyn Host) -> i64 {
    match host.ini_get(b"error_reporting") {
        // OnUpdateErrorReporting: atoi.
        Some(v) if !v.is_empty() => i64::from(crate::c_atoi(&v)),
        _ => crate::E_ALL,
    }
}

/// What `@` leaves of error_reporting (php 8): the fatal types.
pub const SILENCE_MASK: i64 = crate::E_ERROR
    | crate::E_CORE_ERROR
    | crate::E_COMPILE_ERROR
    | crate::E_USER_ERROR
    | crate::E_RECOVERABLE_ERROR
    | crate::E_PARSE;

/// `set_error_handler(?callable $callback, int $error_levels = E_ALL)`:
/// returns the previous handler (or null).
fn fn_set_handler(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = deref(&args[0]);
    let why = match &h {
        Value::Null => None,
        Value::Str(s) if !host.is_callable_value(&h) => Some(format!(
            "function \"{}\" not found or invalid function name",
            String::from_utf8_lossy(s.as_bytes())
        )),
        Value::Str(_) | Value::Array(_) | Value::Closure(_) | Value::Object(_) => None,
        _ => Some("no array or string given".to_string()),
    };
    if let Some(why) = why {
        return Err(Thrown::type_error(format!(
            "set_error_handler(): Argument #1 ($callback) must be a valid callback or null, {why}"
        )));
    }
    let mask = args
        .get(1)
        .map(|v| conv::to_int(&deref(v)))
        .unwrap_or(crate::E_ALL);
    Ok(host.set_error_handler(h, mask))
}

/// `set_exception_handler(?callable $callback)`: returns the previous handler.
fn fn_set_exception_handler(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = deref(&args[0]);
    let why = match &h {
        Value::Null => None,
        Value::Str(s) if !host.is_callable_value(&h) => Some(format!(
            "function \"{}\" not found or invalid function name",
            String::from_utf8_lossy(s.as_bytes())
        )),
        Value::Str(_) | Value::Array(_) | Value::Closure(_) | Value::Object(_) => None,
        _ => Some("no array or string given".to_string()),
    };
    if let Some(why) = why {
        return Err(Thrown::type_error(format!(
            "set_exception_handler(): Argument #1 ($callback) must be a valid callback or null, {why}"
        )));
    }
    Ok(host.set_exception_handler(h))
}

fn fn_restore_exception_handler(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.restore_exception_handler();
    Ok(Value::Bool(true))
}

fn fn_restore_handler(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.restore_error_handler();
    Ok(Value::Bool(true))
}

/// `trigger_error(string $message, int $error_level = E_USER_NOTICE)`.
fn fn_trigger_error(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = conv::to_string_bytes(&deref(&args[0]));
    let level = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(1024);
    match level {
        256 => {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                b"Passing E_USER_ERROR to trigger_error() is deprecated since 8.4, throw an exception or call exit with a string message instead",
            )?;
        }
        512 | 1024 | 16384 => {}
        _ => {
            return Err(Thrown::error(
                "ValueError",
                "trigger_error(): Argument #2 ($error_level) must be one of E_USER_ERROR, E_USER_WARNING, E_USER_NOTICE, or E_USER_DEPRECATED",
            ))
        }
    }
    host.emit_diagnostic(level, &msg)?;
    Ok(Value::Bool(true))
}

/// `error_reporting(?int $error_level = null): int`: the old level; sets a
/// new one when given.
fn fn_error_reporting(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let old = error_reporting_level(host);
    let new = crate::zend::zpp::int_or_null_param(host, args, 0, "error_reporting", "error_level")?;
    if let Some(n) = new {
        if n != old {
            host.ini_set(b"error_reporting", n.to_string().into_bytes());
        }
    }
    Ok(Value::Int(old))
}

fn fn_class_alias(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let class = str_param(host, args, 0, "class_alias", "class")?;
    let alias = str_param(host, args, 1, "class_alias", "alias")?;
    let autoload = bool_param(host, args, 2, "class_alias", "autoload", true)?;
    let id = if autoload {
        host.resolve_class_autoload(&class)?
    } else {
        host.resolve_class(&class)
    };
    match id {
        // The host warns when the alias name is taken.
        Some(id) => Ok(Value::Bool(host.class_alias(id, &alias)?)),
        None => {
            let msg = format!("Class \"{}\" not found", String::from_utf8_lossy(&class));
            host.emit_diagnostic(E_WARNING, msg.as_bytes())?;
            Ok(Value::Bool(false))
        }
    }
}

fn fn_get_defined_constants(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let categorize = bool_param(host, args, 0, "get_defined_constants", "categorize", false)?;
    let user = host.user_constants();
    let is_user = |n: &[u8]| user.iter().any(|(u, _)| u == n);
    let mut flat = PhpArray::new();
    let mut cats = PhpArray::new();
    // Constants a module registers at run time (ext/session's SID) come
    // last in its category.
    const RUNTIME: &[(&str, &str)] = &[("session", "SID")];
    for (module, names) in CONST_MODULES {
        let mut cat = PhpArray::new();
        let late = RUNTIME.iter().filter(|(m, _)| m == module).map(|(_, n)| *n);
        for name in names.split(' ').chain(late) {
            if is_user(name.as_bytes()) {
                continue;
            }
            if let Some(v) = host.get_const(name.as_bytes()) {
                cat.insert(ArrayKey::from_bytes(name.as_bytes()), v);
            }
        }
        if cat.is_empty() {
            continue;
        }
        if categorize {
            cats.insert(ArrayKey::from_bytes(module.as_bytes()), Value::Array(cat));
        } else {
            for (k, v) in cat.iter() {
                flat.insert(k.clone(), v.clone());
            }
        }
    }
    let mut ucat = PhpArray::new();
    for (k, v) in user {
        ucat.insert(ArrayKey::from_bytes(&k), v);
    }
    if categorize {
        if !ucat.is_empty() {
            cats.insert(ArrayKey::from_bytes(b"user"), Value::Array(ucat));
        }
        Ok(Value::Array(cats))
    } else {
        for (k, v) in ucat.iter() {
            flat.insert(k.clone(), v.clone());
        }
        Ok(Value::Array(flat))
    }
}

fn request_start() -> std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

/// `gc_status()` (zend_gc_get_status).
fn fn_gc_status(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let st = rhp_values::gc::status();
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: Value| a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    let n = |f: fn(&rhp_values::gc::Status) -> u32| Value::Int(st.as_ref().map_or(0, f) as i64);
    let secs =
        |f: fn(&rhp_values::gc::Status) -> u64| Value::Float(st.as_ref().map_or(0, f) as f64 / 1e9);
    put(
        "running",
        Value::Bool(st.as_ref().is_some_and(|s| s.running)),
    );
    put(
        "protected",
        Value::Bool(st.as_ref().is_some_and(|s| s.protected)),
    );
    put("full", Value::Bool(st.as_ref().is_some_and(|s| s.full)));
    put("runs", n(|s| s.runs));
    put("collected", n(|s| s.collected));
    put("threshold", n(|s| s.threshold));
    put("buffer_size", n(|s| s.buf_size));
    put("roots", n(|s| s.num_roots));
    put(
        "application_time",
        Value::Float(request_start().elapsed().as_secs_f64()),
    );
    put("collector_time", secs(|s| s.collector_time));
    put("destructor_time", secs(|s| s.dtor_time));
    put("free_time", secs(|s| s.free_time));
    Ok(Value::Array(a))
}

fn fn_gc_collect_cycles(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.gc_collect_cycles().map(Value::Int)
}

fn fn_gc_enable(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    crate::main::ini::alter_ini_entry(host, b"zend.enable_gc", b"1".to_vec(), "gc_enable")?;
    Ok(Value::Null)
}

fn fn_gc_disable(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    crate::main::ini::alter_ini_entry(host, b"zend.enable_gc", b"0".to_vec(), "gc_disable")?;
    Ok(Value::Null)
}

fn fn_gc_enabled(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(rhp_values::gc::enabled()))
}

fn fn_gc_mem_caches(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(0))
}

fn fn_zend_version(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(crate::version::ZEND_VERSION))
}

/// The platform `memcmp` over the first `n` bytes. php returns its result
/// as is, and the value differs by libc (macOS: the byte difference; glibc:
/// depends on the implementation), so call the same function.
fn memcmp_raw(a: &[u8], b: &[u8], n: usize) -> i64 {
    assert!(n <= a.len() && n <= b.len());
    if n == 0 {
        return 0;
    }
    // SAFETY: both slices hold at least `n` bytes (checked above).
    let r = unsafe { libc::memcmp(a.as_ptr().cast(), b.as_ptr().cast(), n) };
    r as i64
}

/// Case-insensitive variant: compares `tolower` of each byte.
fn memcmp_raw_ci(a: &[u8], b: &[u8], n: usize) -> i64 {
    for i in 0..n {
        let d = a[i].to_ascii_lowercase() as i64 - b[i].to_ascii_lowercase() as i64;
        if d != 0 {
            return d;
        }
    }
    0
}

/// Normalize to -1 / 0 / 1 (the equal-prefix / length-difference path).
fn norm(x: i64) -> i64 {
    match x.cmp(&0) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// `zend_binary_strcmp`: raw diff on first differing byte, clamped sign on the
/// equal-prefix (length-difference) path.
fn binary_strcmp(s1: &[u8], s2: &[u8]) -> i64 {
    let n = s1.len().min(s2.len());
    let r = memcmp_raw(s1, s2, n);
    if r != 0 {
        r
    } else {
        norm(s1.len() as i64 - s2.len() as i64)
    }
}

fn binary_strcasecmp(s1: &[u8], s2: &[u8]) -> i64 {
    let n = s1.len().min(s2.len());
    let r = memcmp_raw_ci(s1, s2, n);
    if r != 0 {
        r
    } else {
        norm(s1.len() as i64 - s2.len() as i64)
    }
}

/// `zend_binary_strncmp`: like strcmp but bounded to `length` bytes. The
/// equal-prefix path compares `min(length,len)` of each side.
pub(crate) fn binary_strncmp(s1: &[u8], s2: &[u8], length: usize) -> i64 {
    let n = length.min(s1.len()).min(s2.len());
    let r = memcmp_raw(s1, s2, n);
    if r != 0 {
        r
    } else {
        norm(length.min(s1.len()) as i64 - length.min(s2.len()) as i64)
    }
}

pub(crate) fn binary_strncasecmp(s1: &[u8], s2: &[u8], length: usize) -> i64 {
    let n = length.min(s1.len()).min(s2.len());
    let r = memcmp_raw_ci(s1, s2, n);
    if r != 0 {
        r
    } else {
        norm(length.min(s1.len()) as i64 - length.min(s2.len()) as i64)
    }
}

pub(crate) fn fn_strcmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(binary_strcmp(
        &conv::to_string_bytes(&args[0]),
        &conv::to_string_bytes(&args[1]),
    )))
}

pub(crate) fn fn_strcasecmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(binary_strcasecmp(
        &conv::to_string_bytes(&args[0]),
        &conv::to_string_bytes(&args[1]),
    )))
}

pub(crate) fn fn_strncmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = conv::to_int(&deref(&args[2]));
    if n < 0 {
        return Err(Thrown::error(
            "ValueError",
            "strncmp(): Argument #3 ($length) must be greater than or equal to 0",
        ));
    }
    Ok(Value::Int(binary_strncmp(
        &conv::to_string_bytes(&args[0]),
        &conv::to_string_bytes(&args[1]),
        n as usize,
    )))
}

pub(crate) fn fn_strncasecmp(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = conv::to_int(&deref(&args[2]));
    if n < 0 {
        return Err(Thrown::error(
            "ValueError",
            "strncasecmp(): Argument #3 ($length) must be greater than or equal to 0",
        ));
    }
    Ok(Value::Int(binary_strncasecmp(
        &conv::to_string_bytes(&args[0]),
        &conv::to_string_bytes(&args[1]),
        n as usize,
    )))
}

fn fn_get_resource_type(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        ref v @ Value::Resource(_) if host.resource_closed(v) => Ok(Value::str("Unknown")),
        Value::Resource(r) => Ok(Value::str(r.type_name)),
        other => Err(Thrown::type_error(format!(
            "get_resource_type(): Argument #1 ($resource) must be of type resource, {} given",
            crate::zend::zpp::zval_value_name(host, &other)
        ))),
    }
}

/// `get_resource_id($resource)`: the id `(int)` casts to, closed or not.
fn fn_get_resource_id(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Resource(r) => Ok(Value::Int(r.id as i64)),
        other => Err(Thrown::type_error(format!(
            "get_resource_id(): Argument #1 ($resource) must be of type resource, {} given",
            crate::zend::zpp::zval_value_name(host, &other)
        ))),
    }
}

/// The resource type names php registers (zend_fetch_list_dtor_id).
const RESOURCE_TYPES: &[&str] = &[
    "stream",
    "persistent stream",
    "stream-context",
    "stream filter",
    "stream bucket",
    "process",
    "Zip Directory",
    "Zip Entry",
];

/// `get_resources(?string $type = null)`: live resources keyed by id; with
/// a type, those of that type ("Unknown": the closed ones).
fn fn_get_resources(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ty = match args.first().map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(conv::to_string_bytes(&v)),
    };
    if let Some(t) = &ty {
        if t.as_slice() != b"Unknown" && !RESOURCE_TYPES.iter().any(|n| n.as_bytes() == t) {
            return Err(Thrown::error(
                "ValueError",
                "get_resources(): Argument #1 ($type) must be a valid resource type",
            ));
        }
    }
    let mut a = PhpArray::new();
    for r in host.live_resources() {
        let Value::Resource(h) = &r else { continue };
        let keep = match &ty {
            None => true,
            Some(t) if t.as_slice() == b"Unknown" => host.resource_closed(&r),
            Some(t) => !host.resource_closed(&r) && h.type_name.as_bytes() == t.as_slice(),
        };
        if keep {
            a.insert(rhp_values::ArrayKey::Int(h.id as i64), r.clone());
        }
    }
    Ok(Value::Array(a))
}

/// `get_error_handler()`: the current handler (EG(user_error_handler)).
fn fn_get_error_handler(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let h = host.set_error_handler(Value::Null, crate::E_ALL);
    host.restore_error_handler();
    Ok(h)
}

/// `get_exception_handler()`: the current handler.
fn fn_get_exception_handler(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let h = host.set_exception_handler(Value::Null);
    host.restore_exception_handler();
    Ok(h)
}

/// `exit()` / `die()` called as functions (`'exit'`, `call_user_func`): a
/// string status is printed, an int is the exit status. The `exit`
/// construct compiles to the Exit op instead.
fn fn_exit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first().map(rhp_values::ops::deref) {
        None => Err(Thrown::Exit(0)),
        Some(Value::Int(n)) => Err(Thrown::Exit(n as i32)),
        Some(v) => {
            let s = rhp_values::conv::to_string_bytes(&v);
            if !s.is_empty() {
                host.write_out(&s);
            }
            Err(Thrown::Exit(0))
        }
    }
}

/// ZEND_FUNCTION(clone): zpp gave an object and an array.
fn fn_clone(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let with = match args.get(1).map(deref) {
        Some(Value::Array(a)) => a,
        _ => PhpArray::new(),
    };
    host.clone_object(&deref(&args[0]), &with)
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("clone", fn_clone),
            ("exit", fn_exit),
            ("die", fn_exit),
            ("get_declared_classes", fn_get_declared_classes),
            ("get_declared_interfaces", fn_get_declared_interfaces),
            ("get_declared_traits", fn_get_declared_traits),
            ("trait_exists", fn_trait_exists),
            ("get_defined_functions", fn_get_defined_functions),
            ("get_class_methods", fn_get_class_methods),
            ("get_class_vars", fn_get_class_vars),
            ("get_included_files", fn_get_included_files),
            ("get_required_files", fn_get_included_files),
            ("define", fn_define),
            ("defined", fn_defined),
            ("strlen", fn_strlen),
            ("get_class", fn_get_class),
            ("get_parent_class", fn_get_parent_class),
            ("is_a", fn_is_a),
            ("is_subclass_of", fn_is_subclass_of),
            ("get_object_vars", fn_get_object_vars),
            ("get_mangled_object_vars", fn_get_mangled_object_vars),
            ("method_exists", fn_method_exists),
            ("property_exists", fn_property_exists),
            ("class_exists", fn_class_exists),
            ("interface_exists", fn_interface_exists),
            ("enum_exists", fn_enum_exists),
            ("extension_loaded", fn_extension_loaded),
            ("get_loaded_extensions", fn_get_loaded_extensions),
            ("get_extension_funcs", fn_get_extension_funcs),
            ("function_exists", fn_function_exists),
            ("func_num_args", fn_func_num_args),
            ("func_get_args", fn_func_get_args),
            ("get_defined_vars", fn_get_defined_vars),
            ("func_get_arg", fn_func_get_arg),
            ("get_called_class", fn_get_called_class),
            ("get_resource_id", fn_get_resource_id),
            ("get_resources", fn_get_resources),
            ("get_error_handler", fn_get_error_handler),
            ("get_exception_handler", fn_get_exception_handler),
            ("debug_backtrace", fn_debug_backtrace),
            ("debug_print_backtrace", fn_debug_print_backtrace),
            ("set_error_handler", fn_set_handler),
            ("restore_error_handler", fn_restore_handler),
            ("set_exception_handler", fn_set_exception_handler),
            ("restore_exception_handler", fn_restore_exception_handler),
            ("trigger_error", fn_trigger_error),
            ("user_error", fn_trigger_error),
            ("error_reporting", fn_error_reporting),
            ("gc_enable", fn_gc_enable),
            ("gc_disable", fn_gc_disable),
            ("gc_enabled", fn_gc_enabled),
            ("gc_collect_cycles", fn_gc_collect_cycles),
            ("class_alias", fn_class_alias),
            ("get_defined_constants", fn_get_defined_constants),
            ("gc_status", fn_gc_status),
            ("gc_mem_caches", fn_gc_mem_caches),
            ("zend_version", fn_zend_version),
            ("strcmp", fn_strcmp),
            ("strcasecmp", fn_strcasecmp),
            ("strncmp", fn_strncmp),
            ("strncasecmp", fn_strncasecmp),
            ("get_resource_type", fn_get_resource_type),
        ],
    );
}
