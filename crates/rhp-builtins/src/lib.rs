//! Native function + constant registry, and the narrow `Host` interface the
//! interpreter exposes to builtins.
//!
//! Builtins operate on owned `Value`s only — they never touch the arena/AST,
//! so this crate depends on `rhp-values` and `rhp-semantics` only, never on
//! the compiler or the VM (no dependency cycle).
//! The interpreter talks back through [`Host`].

use std::collections::HashMap;
use std::sync::OnceLock;

use rhp_values::conv;
use rhp_values::{PhpArray, Value};

pub use rhp_values::Thrown;

pub(crate) mod dump;
pub mod fdstream;
pub(crate) mod module;
pub mod object;
pub mod pcre;
pub mod signatures;
pub mod streams;
pub mod version;
pub mod zend;

pub use streams::{StatInfo, StreamWrapper, WrapperReg};

mod argparse;
/// The kind of an open stream resource (see [`Host::stream_kind`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamKind {
    /// A plain local file (real descriptor).
    File,
    /// A directory stream (`opendir`).
    Dir,
    /// An in-memory or wrapper-buffered stream (`php://memory`, phar entries).
    Memory,
    /// stdin/stdout/stderr/`php://output`.
    Stdio,
    /// An OS descriptor stream: a `proc_open` pipe, `php://stdin`.
    Pipe,
}

/// `stream_get_meta_data` information for a stream resource.
pub struct StreamMeta {
    pub wrapper_type: &'static str,
    pub stream_type: &'static str,
    pub mode: Vec<u8>,
    pub uri: Vec<u8>,
    pub unread_bytes: i64,
    pub seekable: bool,
    pub eof: bool,
    /// php's `blocked` (false after stream_set_blocking($s, false)).
    pub blocked: bool,
    /// php's `timed_out` (a socket read / write hit its timeout).
    pub timed_out: bool,
}

/// pcntl's asynchronous signal delivery: the VM polls
/// [`pcntl_interrupt_pending`] between instructions and, when set, calls
/// [`pcntl_dispatch`] to run the queued PHP signal handlers.
pub use ext::pcntl::{dispatch as pcntl_dispatch, interrupt_pending as pcntl_interrupt_pending};

mod arr_fns;
mod arraycmp_fns;
mod arrayset_fns;
#[rustfmt::skip]
mod const_modules;
mod class_fns;
mod crypt_fns;
mod datetime_fns;
mod dt_oop_fns;
mod ext;
mod file_fns;
mod filter_fns;
mod fs_fns;
mod html;
#[rustfmt::skip]
mod html_tables;
pub(crate) mod http;
mod inet;
mod info_fns;
mod ini_parse;
#[rustfmt::skip]
mod ini_table;
mod json_fns;
mod math_fns;
mod misc_fns;
mod opcache_fns;
mod parse_fns;
pub mod php_fns;
mod preg_fns;
mod printf_fns;
mod proc_fns;
mod query_fns;
pub mod rfc1867;
mod runtime_fns;
pub mod sapi;
mod serialize_fns;
mod socket_fns;
mod sort_fns;
mod stdext_fns;
mod str_fns;
mod strarr2_fns;
mod strcmp_fns;
mod stream_fns;
mod strsearch_fns;
mod strtotime_fns;
mod sysmisc_fns;
mod tzdb;
mod webenc_fns;
pub(crate) mod zip_ext;

/// The E_DEPRECATED message for reading a `#[\Deprecated]` extension
/// constant, if `name` is one.
pub fn constant_deprecation(name: &[u8]) -> Option<String> {
    ext::curl::constant_deprecation(name)
}

/// The engine's object free hook (`rhp_values::set_object_free_hook`), for
/// the per-object state still kept by object id.
pub fn object_freed(id: u32) {
    zip_ext::object_freed(id);
    sort_fns::object_freed(id);
    ext::openssl::object_freed(id);
    ext::curl::object_freed(id);
}

/// `zend_compare` from inside a builtin, with the host's object semantics.
pub fn host_compare(
    host: &mut dyn Host,
    a: &Value,
    b: &Value,
) -> Result<std::cmp::Ordering, Thrown> {
    match host.compare_cx() {
        Some(cx) => rhp_semantics::ops::compare(a, b, cx),
        None => rhp_semantics::ops::compare(a, b, &mut rhp_semantics::ops::NoCx),
    }
}

/// `$a == $b` from inside a builtin.
pub fn host_loose_eq(host: &mut dyn Host, a: &Value, b: &Value) -> Result<bool, Thrown> {
    Ok(host_compare(host, a, b)? == std::cmp::Ordering::Equal)
}

/// One userland output-control function ([`Host::output_control`]).
#[derive(Clone)]
pub enum ObCall {
    /// `ob_start($callback, $chunk_size, $flags)` (`callback` null: the
    /// "default output handler").
    Start {
        callback: Value,
        chunk_size: i64,
        flags: i64,
    },
    Flush,
    Clean,
    EndFlush,
    EndClean,
    GetFlush,
    GetClean,
    GetContents,
    GetLength,
    GetLevel,
    ListHandlers,
    GetStatus {
        full: bool,
    },
    ImplicitFlush(bool),
    /// `flush()`: sapi_flush.
    SapiFlush,
}

/// The interpreter surface a builtin may use. Implemented by the VM (and by
/// the minimal hosts in unit tests, which rely on the default bodies).
pub trait Host {
    /// Append bytes to the single output sink (stdout / output buffer).
    fn write_out(&mut self, bytes: &[u8]);

    /// A userland output-control call (main/output.c's `ob_*`, `flush`),
    /// arguments already parsed; the host owns the handler stack and raises
    /// php's notices. Default: unsupported (hosts without output buffering,
    /// e.g. the unit-test hosts).
    fn output_control(&mut self, _call: ObCall) -> Result<Value, Thrown> {
        Err(Thrown::unsupported(
            "output buffering",
            "this host has no output handler stack",
        ))
    }

    /// Raise a php diagnostic (`E_WARNING` = 2, `E_NOTICE` = 8, …) at the
    /// current line: php's `zend_error` — the user error handler
    /// (`set_error_handler`) first, else `error_get_last` + display per
    /// `error_reporting` / `@` / `display_errors` / `log_errors`. `msg`
    /// already carries any `func(): ` prefix. An exception thrown by a user
    /// handler is held by the host and surfaces when the running builtin
    /// returns (php's `EG(exception)`); see [`Host::emit_diagnostic`] to
    /// receive it at the call site instead. Default: dropped.
    fn raise_diagnostic(&mut self, _level: i64, _msg: &str) {}

    /// Take the exception a user error handler threw during the last
    /// [`Host::raise_diagnostic`], if any. Default: none.
    fn take_pending_throw(&mut self) -> Option<Thrown> {
        None
    }

    /// The last raised diagnostic for `error_get_last()`: (type, message,
    /// file, line). Default: none.
    fn last_error(&self) -> Option<(i64, Vec<u8>, Vec<u8>, i64)> {
        None
    }

    /// `error_clear_last()`.
    fn clear_last_error(&mut self) {}

    /// The engine's comparison context (`__toString`, internal compare
    /// handlers, notices) for loose comparisons inside builtins.
    fn compare_cx(&mut self) -> Option<&mut dyn rhp_semantics::ops::CompareCx> {
        None
    }

    /// A class's name by id (for `get_class` etc.). Default: not available.
    fn class_name(&self, _class_id: u32) -> Option<Vec<u8>> {
        None
    }

    /// A class's parent's name by id (for `get_parent_class`). Default: none.
    fn class_parent_name(&self, _class_id: u32) -> Option<Vec<u8>> {
        None
    }

    /// php's `zend_fetch_debug_backtrace` for `debug_backtrace()` (innermost
    /// first, the calling builtin's own frame skipped). Default: unavailable.
    fn backtrace(&self, _provide_object: bool, _ignore_args: bool) -> Option<Value> {
        None
    }

    /// Push a user exception handler (`set_exception_handler`; null clears),
    /// returning the previous one (or null). Default: not stored.
    fn set_exception_handler(&mut self, _handler: Value) -> Value {
        Value::Null
    }
    /// Pop back to the previous exception handler.
    fn restore_exception_handler(&mut self) {}

    /// Push a `set_error_handler` callback (null disables) handling the
    /// error types in `mask`; returns the previous handler or null.
    fn set_error_handler(&mut self, _handler: Value, _mask: i64) -> Value {
        Value::Null
    }
    /// `restore_error_handler`: pop back to the previous error handler.
    fn restore_error_handler(&mut self) {}

    /// Whether the code calling the running builtin declared
    /// `strict_types=1` (zpp then coerces no scalars). Default: no.
    fn caller_strict_types(&self) -> bool {
        false
    }

    /// php's `ZEND_ARG_USES_STRICT_TYPES()` as an object handler the engine
    /// runs (`do_operation`, `compare`) sees it: the strictness of the
    /// caller of php's current frame, which is the builtin running the
    /// handler if there is one, else the code running the operator. Weak
    /// when that caller is a builtin or there is none (the main script).
    /// Default: no.
    fn arg_uses_strict_types(&self) -> bool {
        false
    }

    /// `thrown` raised while `previous` is pending: php makes `previous`
    /// the new exception's previous one. An `exit` in `previous` wins.
    /// Default: `thrown` alone.
    fn chain_thrown(&mut self, thrown: Thrown, _previous: Thrown) -> Thrown {
        thrown
    }

    /// Inside an `@` region (error_reporting then reads as the fatal mask).
    fn silenced(&self) -> bool {
        false
    }

    /// A directive's current value (the per-request ini store shared with
    /// `ini_get`/`ini_set`; a null value reads as ""), or None when no loaded
    /// module registers it.
    fn ini_get(&self, name: &[u8]) -> Option<Vec<u8>> {
        runtime_fns::ini_value(name).map(Option::unwrap_or_default)
    }
    /// Store `value` for a known directive (no validation; engine-internal
    /// writes such as `error_reporting()`), returning the old value.
    fn ini_set(&mut self, name: &[u8], value: Vec<u8>) -> Option<Vec<u8>> {
        let old = self.ini_get(name)?;
        runtime_fns::ini_store(name, value);
        Some(old)
    }

    /// `register_shutdown_function`: queue a callback (with its arguments) for
    /// request shutdown. Default: refused (the host has no shutdown phase).
    fn register_shutdown_function(
        &mut self,
        _callback: Value,
        _args: Vec<Value>,
    ) -> Result<(), Thrown> {
        Err(Thrown::unsupported(
            "register_shutdown_function",
            "this host has no request shutdown",
        ))
    }

    /// Whether a resource has been closed (`fclose`d stream): `gettype` /
    /// `get_debug_type` then report `resource (closed)`. Default: no.
    fn resource_closed(&self, _res: &Value) -> bool {
        false
    }

    /// Whether a class id names an enum (for `var_dump` rendering). Default: no.
    fn is_enum_class(&self, _class_id: u32) -> bool {
        false
    }

    /// Whether objects of a class are `ZEND_ACC_NOT_SERIALIZABLE` (the
    /// Reflection objects, WeakReference, …). Default: no.
    fn not_serializable(&self, _class_id: u32) -> bool {
        false
    }

    /// The object handlers of a class: its own registration
    /// ([`object::registered`]) or an ancestor's. Default: none.
    fn object_handlers(&self, _class_id: u32) -> Option<&'static object::ObjectHandlers> {
        None
    }

    /// php's `zend_std_cast_object_tostring` for `obj`: `__toString()` for a
    /// string, true for bool, FAILURE (`Ok(None)`) otherwise; what a
    /// `cast_object` handler defers to. Default: no `__toString()` call.
    fn std_cast_object(
        &mut self,
        _obj: &rhp_values::ObjectHandle,
        target: object::CastTarget,
    ) -> Result<Option<Value>, Thrown> {
        Ok(object::std_cast_non_string(target))
    }

    /// Whether the property slot `key` (storage key) of an object of class
    /// `class_id` is engine-internal state of a built-in class (the prelude's
    /// `__`-prefixed slots): php keeps such state outside the property table,
    /// so every property-table view (`var_dump`, `(array)`, `serialize`,
    /// `json_encode`, `get_object_vars`, `foreach`) skips it. Default: no.
    fn is_internal_slot(&self, _class_id: u32, _key: &rhp_values::PhpStr) -> bool {
        false
    }

    /// The uninitialized typed properties `var_dump` lists as
    /// `uninitialized(type)`: (mangled name, type, how many of the object's
    /// visible properties precede it). Default: none.
    fn uninit_props(
        &self,
        _obj: &rhp_values::ObjectHandle,
    ) -> Vec<(rhp_values::PhpStr, String, usize)> {
        Vec::new()
    }

    /// Define a runtime constant (`define`). False when the name is taken
    /// (the caller warns, as zend_register_constant does).
    fn define_const(&mut self, _name: &[u8], _value: Value) -> bool {
        false
    }

    /// Look up a constant by name (`constant`), or None if undefined.
    fn get_const(&self, _name: &[u8]) -> Option<Value> {
        None
    }

    /// A class constant's value (`defined`/`constant` with `Class::CONST`, incl.
    /// enum cases), or None if the class or constant does not exist.
    fn class_constant(&self, _class: &[u8], _name: &[u8]) -> Option<Value> {
        None
    }

    /// Like [`Host::class_constant`], but first evaluates the constant's
    /// initializer if it is still a pending constant expression (which can
    /// throw, e.g. on an undefined constant inside it).
    fn class_constant_eval(&mut self, class: &[u8], name: &[u8]) -> Result<Option<Value>, Thrown> {
        Ok(self.class_constant(class, name))
    }

    /// Run the destructors of objects nothing references any more, now: php
    /// destructs when the refcount reaches zero, rhp before the next
    /// statement. Default: nothing.
    fn run_released_destructors(&mut self) -> Result<(), Thrown> {
        Ok(())
    }

    /// Invoke a PHP callable (for `array_map`/`array_filter`/…). Default: refused.
    fn call_callable(&mut self, _callable: &Value, _args: &[Value]) -> Result<Value, Thrown> {
        Err(Thrown::unsupported(
            "callback",
            "callbacks are not available in this host",
        ))
    }

    /// Invoke a PHP callable with named arguments too (`call_user_func_array`
    /// with string keys). Default: refused when any argument is named.
    fn call_callable_named(
        &mut self,
        callable: &Value,
        args: &[Value],
        named: Vec<(rhp_values::PhpStr, Value)>,
    ) -> Result<Value, Thrown> {
        if !named.is_empty() {
            return Err(Thrown::unsupported(
                "named arguments",
                "not supported by this host",
            ));
        }
        self.call_callable(callable, args)
    }

    /// Whether a value is callable (`is_callable`): closures, function names,
    /// `[obj_or_class, method]` pairs, and `__invoke` objects. Default: closures only.
    fn is_callable_value(&self, callable: &Value) -> bool {
        matches!(deref(callable), Value::Closure(_))
    }

    /// Why a value is not a valid callback (zend_is_callable_error's message,
    /// e.g. `function "f" not found or invalid function name`), or None when
    /// it is callable. Default: derived from [`Host::is_callable_value`].
    fn callable_error(&self, callable: &Value) -> Option<String> {
        if self.is_callable_value(callable) {
            return None;
        }
        Some(match deref(callable) {
            Value::Str(s) => format!(
                "function \"{}\" not found or invalid function name",
                String::from_utf8_lossy(s.as_bytes())
            ),
            Value::Array(_) => "array callback must have exactly two members".to_string(),
            _ => "no array or string given".to_string(),
        })
    }

    /// Wrap a callable as a Closure (`Closure::fromCallable`) that resolves it
    /// in the calling scope. Default: no scope recorded.
    fn wrap_callable(&self, callable: Value) -> Value {
        Value::Closure(std::rc::Rc::new(rhp_values::ClosureData {
            fnidx: u32::MAX,
            captures: Vec::new(),
            bound_this: None,
            bound_class: None,
            bound_static: None,
            wrapped: Some(Box::new(callable)),
        }))
    }

    /// [`Host::callable_error`] after autoloading a class the callable names
    /// (zend_is_callable looks classes up with autoloading; a loader's
    /// exception propagates).
    fn check_callable(&mut self, callable: &Value) -> Result<Option<String>, Thrown> {
        Ok(self.callable_error(callable))
    }

    /// Resolve a class-like name to its opaque class id (for `class_exists`,
    /// `method_exists`/`property_exists` on a class-name string). Default: none.
    fn resolve_class(&self, _name: &[u8]) -> Option<u32> {
        None
    }

    /// Like `resolve_class` but invokes registered autoloaders on a miss (for
    /// `class_exists($name)` with `$autoload = true`, php's default). Default:
    /// falls back to the non-autoloading resolve.
    /// An exception thrown by an autoloader propagates.
    fn resolve_class_autoload(&mut self, name: &[u8]) -> Result<Option<u32>, Thrown> {
        Ok(self.resolve_class(name))
    }

    /// `class_alias`: register `alias` as another name for class `class_id`.
    /// Returns false if the alias name is already taken. Default: refused.
    fn class_alias(&mut self, _class_id: u32, _alias: &[u8]) -> bool {
        false
    }

    /// A class's relations for `class_parents` / `class_implements` /
    /// `class_uses`: (parent chain nearest-first, interfaces in php order,
    /// directly used traits). Default: none.
    #[allow(clippy::type_complexity)]
    fn class_relations(
        &self,
        _class_id: u32,
    ) -> Option<(Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>)> {
        None
    }

    /// The class scope of the code calling the builtin
    /// (`zend_get_executed_scope`), if any.
    fn caller_scope(&self) -> Option<u32> {
        None
    }

    /// An object's properties accessible from the calling code's class scope
    /// (`zend_check_property_access`), in property-table order — for
    /// `http_build_query` on objects. Default: every property.
    fn object_scoped_props(&self, obj: &Value) -> Vec<(rhp_values::PhpStr, Value)> {
        match deref(obj) {
            Value::Object(h) => h
                .borrow()
                .properties
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// User-defined constants (`define`/`const`) in definition order, for
    /// `get_defined_constants`. Default: none.
    fn user_constants(&self) -> Vec<(Vec<u8>, Value)> {
        Vec::new()
    }

    /// The request's header store (`SG(sapi_headers)`, `headers_sent`):
    /// `header()` and friends operate on it. Default: none (a host without a
    /// response; the header builtins then do nothing).
    fn response(&mut self) -> Option<&mut sapi::Response> {
        None
    }

    /// One `error_log()` / log_errors line for the SAPI's logger (php's
    /// `sapi_module.log_message`; no trailing newline). Default: stderr.
    fn log_message(&mut self, msg: &[u8]) {
        use std::io::Write;
        let mut e = std::io::stderr();
        let _ = e.write_all(msg);
        let _ = e.write_all(b"\n");
        let _ = e.flush();
    }

    /// The request headers (`getallheaders()`), in order. Default: none.
    fn request_headers(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        Vec::new()
    }

    /// The request a SAPI handed the engine (`SG(request_info)`), if any.
    fn request_info(&self) -> Option<&sapi::RequestInfo> {
        None
    }

    /// The request body's read state and uploads (`request_parse_body`,
    /// `is_uploaded_file`, `move_uploaded_file`). Default: none.
    fn body_state(&mut self) -> Option<&mut rfc1867::BodyState> {
        None
    }

    /// The [`ClassKind`] of a class id (class / interface / trait / enum), used by
    /// the `*_exists` family. Default: none.
    fn class_kind(&self, _class_id: u32) -> Option<rhp_values::ClassKind> {
        None
    }

    /// Whether a class id declares a method (case-insensitive, flattened over the
    /// inheritance chain). Default: false.
    fn class_has_method(&self, _class_id: u32, _method: &[u8]) -> bool {
        false
    }

    /// Whether a class id declares a property — instance or static
    /// (case-sensitive, flattened over the inheritance chain). Default: false.
    fn class_has_property(&self, _class_id: u32, _prop: &[u8]) -> bool {
        false
    }

    /// Whether a function exists (`function_exists`): a user function or a
    /// builtin. Default: builtins only (hosts without a user-function table).
    fn function_exists(&self, name: &[u8]) -> bool {
        lookup(&String::from_utf8_lossy(name)).is_some()
    }

    /// `Closure::bind`/`bindTo`: return a copy of `closure` with its `$this`
    /// rebound to `new_this` (object or null) and its scope set from `scope` (a
    /// class name string, an object, or null = keep/derive). Default: none.
    fn closure_bind(&self, _closure: &Value, _new_this: &Value, _scope: &Value) -> Option<Value> {
        None
    }

    /// Register an `spl_autoload_register` callable (optionally prepending).
    fn register_autoloader(&mut self, _callback: Value, _prepend: bool) {}

    /// The registered autoloader callables (`spl_autoload_functions`).
    fn autoload_functions(&self) -> Vec<Value> {
        Vec::new()
    }

    /// Trigger autoloading for a class name (`spl_autoload_call`).
    fn autoload_call(&mut self, _name: &[u8]) -> Result<(), Thrown> {
        Ok(())
    }

    /// The bcmath default scale (`bcscale()` with no argument), 0 at script
    /// start. Stored on the host so it resets per script run.
    fn bc_scale(&self) -> i64 {
        0
    }

    /// Set the bcmath default scale (`bcscale($scale)`).
    fn set_bc_scale(&mut self, _scale: i64) {}

    /// Instantiate a named class, then set the given properties (no
    /// constructor is run), as object-returning builtins do (`gmp_*`
    /// returning `GMP`, `simplexml_load_string`). An internal class gets its
    /// evaluated property defaults; a user class (a SimpleXMLElement
    /// subclass) is created without evaluating its defaults and without the
    /// slots it declares. `None` if the host has no such class or no object
    /// model, or when evaluating an internal class's defaults threw (the
    /// host throws that once the builtin returns).
    fn instantiate(&mut self, _class_name: &[u8], _props: Vec<(Vec<u8>, Value)>) -> Option<Value> {
        None
    }

    /// Read a named property off an object value (for builtins that accept their
    /// own object type back, e.g. `gmp_add(gmp_add(...), ...)`). Default: none.
    fn object_prop(&self, _obj: &Value, _name: &[u8]) -> Option<Value> {
        None
    }

    /// php's visibility-mangled name for an object's property storage key (as
    /// `(array)`, `serialize` and the dumpers see it): public/dynamic `name`,
    /// protected `"\0*\0name"`, private `"\0Class\0name"`. Hosts store private
    /// properties under their mangled key already; the default is the identity.
    fn mangled_prop_name(&self, _class_id: u32, key: &rhp_values::PhpStr) -> rhp_values::PhpStr {
        key.clone()
    }

    /// Store a property read by `unserialize` on `obj` like php's
    /// `process_nested_object_data`: a plain name, or one mangled for `*` / the
    /// object's own class, lands on the declared property of that name (at its
    /// storage key); any other name is kept as is. Returns false for a
    /// malformed mangled name (a leading NUL without a second one).
    fn unserialize_set_prop(&mut self, obj: &Value, key: &[u8], val: Value) -> bool {
        if let Value::Object(h) = obj {
            h.borrow_mut()
                .properties
                .insert(rhp_values::PhpStr::from_bytes(key.to_vec()), val);
        }
        true
    }

    /// `get_object_vars($obj)` as seen from the calling scope (unmangled names
    /// of the properties accessible there, in slot order). `None` = the host has
    /// no scope information; the builtin then returns every property.
    fn object_vars(&self, _obj: &Value) -> Option<Value> {
        None
    }

    /// Whether `$obj->name` currently exists as seen from the calling scope
    /// (the object half of `property_exists`). Default: a plain-name lookup.
    fn object_prop_exists(&self, obj: &Value, name: &[u8]) -> bool {
        match obj {
            Value::Object(h) => h
                .borrow()
                .properties
                .contains_key(&rhp_values::PhpStr::from_bytes(name.to_vec())),
            _ => false,
        }
    }

    /// The class name of an object value, if any (for `instanceof`-style checks
    /// inside builtins). Default: none.
    /// A closure's function name (`{closure:scope:line}`), for
    /// `is_callable`'s `$callable_name`. Default: unknown.
    fn closure_name(&self, _closure: &Value) -> Option<Vec<u8>> {
        None
    }

    fn object_class_name(&self, _obj: &Value) -> Option<Vec<u8>> {
        None
    }

    /// An object's properties keyed with php's visibility mangling (public
    /// `name`, protected `"\0*\0name"`, private `"\0Class\0name"`), in
    /// property-table order — what `(array)`, `serialize` and `json_encode`
    /// see. Default: the bare property names.
    fn object_props_mangled(&self, obj: &rhp_values::ObjectHandle) -> PhpArray {
        let mut a = PhpArray::new();
        for (k, v) in &obj.borrow().properties {
            a.insert(rhp_values::ArrayKey::from_bytes(k.as_bytes()), v.clone());
        }
        a
    }

    /// Build a `stdClass` object from named properties (for `json_decode` without
    /// `$associative`, `(object)` cast helpers, etc.). The default fallback — for
    /// hosts without an object model — returns an associative array instead.
    fn make_object(&mut self, props: Vec<(Vec<u8>, Value)>) -> Value {
        let mut a = rhp_values::PhpArray::new();
        for (k, v) in props {
            a.insert(rhp_values::ArrayKey::from_bytes(&k), v);
        }
        Value::Array(a)
    }

    // --- stream layer (file:// / phar:// / …) --------------------------------
    // Defaults implement only `file://` via std::fs, so hosts without a wrapper
    // registry (the unit-test hosts) still read local files; the VM overrides
    // these to dispatch through its per-run wrapper registry.

    /// Whole-file read through the stream layer (`file_get_contents`, include).
    fn stream_get_contents(&self, url: &[u8]) -> Option<Vec<u8>> {
        streams::default_get_contents(url)
    }

    /// Existence through the stream layer (`file_exists`).
    fn stream_exists(&self, url: &[u8]) -> bool {
        streams::default_exists(url)
    }

    /// Stat through the stream layer (`is_file`/`is_dir`/`filesize`).
    fn stream_stat(&self, url: &[u8]) -> Option<StatInfo> {
        streams::default_stat(url)
    }

    /// `lstat` through the stream layer (`lstat`/`is_link`/`filetype`).
    fn stream_lstat(&self, url: &[u8]) -> Option<StatInfo> {
        let (scheme, _) = streams::split_scheme(url);
        if scheme == b"file" {
            streams::FileWrapper.lstat(url)
        } else {
            None
        }
    }

    /// Directory listing through the stream layer (`opendir`/`scandir`).
    fn stream_opendir(&self, url: &[u8]) -> Option<Vec<Vec<u8>>> {
        let (scheme, _) = streams::split_scheme(url);
        if scheme == b"file" {
            streams::FileWrapper.opendir(url)
        } else {
            None
        }
    }

    /// Whole-file write through the stream layer (`file_put_contents`).
    fn stream_put_contents(&mut self, url: &[u8], data: &[u8]) -> Option<usize> {
        streams::default_put_contents(url, data)
    }

    /// The registered wrapper for a scheme, for wrapper-specific control ops
    /// (e.g. `Phar::mapPhar` reaching the `phar://` archive table). Default: none.
    fn stream_wrapper(&self, _scheme: &[u8]) -> Option<std::rc::Rc<dyn StreamWrapper>> {
        None
    }

    /// Registered stream-wrapper scheme names (for `stream_get_wrappers`).
    fn stream_wrapper_schemes(&self) -> Vec<Vec<u8>> {
        Vec::new()
    }

    /// The current PHP call stack for `debug_backtrace`/`debug_print_backtrace`:
    /// one entry per frame, innermost first, as
    /// `(function, class, file, line, this_object)` where file/line are the call
    /// site and `this_object` is the frame's `$this` (for `$trace['object']`).
    /// Default: empty.
    #[allow(clippy::type_complexity)]
    fn call_stack(&self) -> Vec<(Vec<u8>, Option<Vec<u8>>, Vec<u8>, u32, Option<Value>)> {
        Vec::new()
    }

    /// Create a stream context resource holding `options` (nested array like
    /// `['http' => [...], 'ssl' => [...]]`); returns the resource value.
    fn create_stream_context(&mut self, _options: Value) -> Value {
        Value::Bool(false)
    }
    /// The options array stored on a stream-context resource (or Null).
    fn stream_context_options(&self, _ctx: &Value) -> Value {
        Value::Null
    }
    /// `stream_context_set_option($ctx, $wrapper, $option, $value)`.
    fn stream_context_set_option(
        &mut self,
        _ctx: &Value,
        _wrapper: &[u8],
        _option: &[u8],
        _value: Value,
    ) {
    }

    /// Response header lines (status line first) from the last http(s) request.
    fn set_http_response_headers(&mut self, _headers: Vec<Vec<u8>>) {}
    fn http_response_headers(&self) -> Vec<Vec<u8>> {
        Vec::new()
    }
    fn clear_http_response_headers(&mut self) {}

    /// Number of arguments passed to the currently-executing user function
    /// (`func_num_args`). Default 0.
    fn current_arg_count(&self) -> i64 {
        0
    }
    /// The argument values passed to the currently-executing user function
    /// (`func_get_args`), best-effort (declared params only). Default empty.
    fn current_args(&self) -> Vec<Value> {
        Vec::new()
    }

    /// The filesystem path of the top-level script being executed (for
    /// `Phar::mapPhar`, which parses the running phar itself). Default: none.
    fn running_main_path(&self) -> Option<Vec<u8>> {
        None
    }

    /// The request's original `$_SERVER['argv']` (php's
    /// `PG(http_globals)[TRACK_VARS_SERVER]`, unaffected by script writes to
    /// `$_SERVER`), for `getopt`. Default: none.
    fn cli_argv(&self) -> Option<Vec<Vec<u8>>> {
        None
    }

    /// The file of the innermost user-code frame (php's
    /// `zend_get_executed_filename`), for `Phar::running` / `Phar::mapPhar`.
    /// Default: the main script.
    fn executing_file(&self) -> Option<Vec<u8>> {
        self.running_main_path()
    }

    // --- stream resources (fopen family) -------------------------------------
    // Streams are resource Values whose state lives in the VM; these are the
    // `fopen`/`fwrite`/… primitives. Defaults are no-ops (hosts without streams).

    /// `fopen($path, $mode)` → a resource, or None on failure.
    fn stream_open(&mut self, _path: &[u8], _mode: &[u8]) -> Option<Value> {
        None
    }
    /// `fwrite`/`fputs` → bytes written.
    fn stream_write(&mut self, _res: &Value, _data: &[u8]) -> Option<usize> {
        None
    }
    /// `fread($res, $len)` (len < 0 = read all remaining).
    fn stream_read(&mut self, _res: &Value, _len: i64) -> Option<Vec<u8>> {
        None
    }
    /// `fgets($res)` → one line (incl. newline), or None at EOF.
    fn stream_gets(&mut self, _res: &Value) -> Option<Vec<u8>> {
        None
    }
    /// `feof($res)`.
    fn stream_eof(&self, _res: &Value) -> bool {
        true
    }
    /// `fclose($res)` (flushes writable streams to their target).
    fn stream_close(&mut self, _res: &Value) -> bool {
        false
    }
    /// `fflush($res)`.
    fn stream_flush(&mut self, _res: &Value) -> bool {
        false
    }
    /// `rewind($res)`.
    fn stream_rewind(&mut self, _res: &Value) -> bool {
        false
    }
    /// What kind of live stream a resource is; `None` if it is not an open
    /// stream (closed, or not a stream resource).
    fn stream_kind(&self, _res: &Value) -> Option<StreamKind> {
        None
    }
    /// `fgets($res, $length)`: like `stream_gets` but reading at most
    /// `max` bytes (php's `$length - 1`).
    fn stream_gets_max(&mut self, res: &Value, _max: usize) -> Option<Vec<u8>> {
        self.stream_gets(res)
    }
    /// `fseek`: true on success.
    fn stream_seek(&mut self, _res: &Value, _offset: i64, _whence: i32) -> bool {
        false
    }
    /// `ftell`: None if the stream has no position.
    fn stream_tell(&mut self, _res: &Value) -> Option<i64> {
        None
    }
    /// `ftruncate`: None if the stream cannot be truncated at all.
    fn stream_truncate(&mut self, _res: &Value, _size: i64) -> Option<bool> {
        None
    }
    /// `fstat`.
    fn stream_fstat(&mut self, _res: &Value) -> Option<StatInfo> {
        None
    }
    /// The OS descriptor behind a stream (for `flock`).
    fn stream_fd(&self, _res: &Value) -> Option<i32> {
        None
    }
    /// `stream_get_meta_data` fields.
    fn stream_meta(&self, _res: &Value) -> Option<StreamMeta> {
        None
    }
    /// Consume `n` resource ids without creating a resource. php registers
    /// every internal stream it opens (file_get_contents, file(), copy, …) as
    /// a resource, so later resource ids skip past them.
    fn consume_resource_ids(&mut self, _n: u32) {}
    /// The default stream context (`stream_context_get_default`), allocated
    /// lazily as a resource the first time a stream function needs it — php
    /// does this on the first `fopen`/`file_get_contents`/`opendir`/…, which
    /// is visible in later resource ids.
    fn default_stream_context(&mut self) -> Value {
        Value::Null
    }
    /// `opendir`: a directory stream resource over `entries`.
    fn dir_open(&mut self, _url: &[u8], _entries: Vec<Vec<u8>>) -> Option<Value> {
        None
    }
    /// `readdir`: the next entry, or None at the end.
    fn dir_read(&mut self, _res: &Value) -> Option<Vec<u8>> {
        None
    }
    /// `rewinddir`.
    fn dir_rewind(&mut self, _res: &Value) {}
    /// The most recently opened directory stream (for `readdir()` with no arg).
    fn last_dir(&self) -> Option<Value> {
        None
    }

    // --- OS-level streams / process support (proc_open & friends) ------------

    /// Wrap an OS file descriptor (e.g. a pipe's parent end) as a `stream`
    /// resource. Default: unsupported (the fd is closed).
    fn stream_from_fd(&mut self, _fd: std::os::fd::OwnedFd) -> Option<Value> {
        None
    }
    /// A `dup(2)` of the OS fd behind a stream resource, for handing to a child
    /// process (`proc_open` descriptor given as a resource). `Err` carries
    /// php's stream-type name when the stream has no fd (e.g. `MEMORY`).
    fn stream_dup_fd(&mut self, _res: &Value) -> Result<std::os::fd::OwnedFd, &'static str> {
        Err("MEMORY")
    }
    /// The OS fd behind a stream, for `stream_select`, plus whether the stream
    /// already holds buffered unread data (then it counts as readable at once).
    fn stream_select_fd(&self, _res: &Value) -> Option<(std::os::fd::RawFd, bool)> {
        None
    }
    /// `stream_set_blocking($res, $enable)`.
    fn stream_set_blocking(&mut self, _res: &Value, _blocking: bool) -> bool {
        true
    }
    /// Wrap a socket descriptor as a socket `stream` resource (php's
    /// `php_netstream_data_t` stream). Default: unsupported.
    fn stream_from_socket(
        &mut self,
        _fd: std::os::fd::OwnedFd,
        _info: fdstream::SockInfo,
    ) -> Option<Value> {
        None
    }
    /// The socket state of a socket stream (`None` for any other stream).
    fn stream_sock_info(&self, _res: &Value) -> Option<fdstream::SockInfo> {
        None
    }
    /// `stream_set_timeout` (`None` = wait forever): false unless `res` is a
    /// socket stream.
    fn stream_set_timeout(&mut self, _res: &Value, _timeout: Option<(i64, i64)>) -> bool {
        false
    }
    /// Flush whatever the host holds back before the process forks
    /// (`pcntl_fork`), so the child does not write it a second time.
    fn prepare_fork(&mut self) {}
    /// Register an opaque native resource (e.g. a `process` handle) and return
    /// its resource Value. Default: unsupported (`Null`).
    fn resource_insert(
        &mut self,
        _type_name: &'static str,
        _data: Box<dyn std::any::Any>,
    ) -> Value {
        Value::Null
    }
    /// The native payload of a resource created by [`Host::resource_insert`].
    fn resource_get(&mut self, _res: &Value) -> Option<&mut dyn std::any::Any> {
        None
    }
    /// Remove (close) a native resource, returning its payload.
    fn resource_remove(&mut self, _res: &Value) -> Option<Box<dyn std::any::Any>> {
        None
    }
    /// Per-run state owned by an extension module (e.g. pcntl's signal-handler
    /// table), created on first use by `init`. Default: none (stateless host).
    fn module_state(
        &mut self,
        _key: &'static str,
        _init: fn() -> Box<dyn std::any::Any>,
    ) -> Option<&mut dyn std::any::Any> {
        None
    }
    /// `ReflectionMethod::invoke`: call `class::method` on `obj` (null for a
    /// static method) regardless of its visibility.
    fn reflect_invoke(
        &mut self,
        obj: &Value,
        class: &[u8],
        method: &[u8],
        args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        let mut cb = PhpArray::new();
        cb.push(match obj {
            Value::Null => Value::str(class.to_vec()),
            o => o.clone(),
        });
        cb.push(Value::str(method.to_vec()));
        self.call_callable(&Value::Array(cb), &args)
    }
    /// Is `res` a stream resource that is still open (not `fclose`d)?
    fn stream_is_open(&self, _res: &Value) -> bool {
        true
    }
    /// The mode a stream was opened with (`"r"`, `"w+b"`, ...), when known.
    fn stream_mode(&self, _res: &Value) -> Option<Vec<u8>> {
        None
    }

    // --- classes and reflection ---------------------------------------------

    /// Whether class `name` is `other` or has it as an ancestor.
    fn reflect_is_a(&self, _name: &[u8], _other: &[u8]) -> bool {
        false
    }
    /// `new C(...$args)` with (or without) running the constructor.
    fn reflect_new_instance(
        &mut self,
        _name: &[u8],
        _args: Vec<Value>,
        _run_ctor: bool,
    ) -> Result<Value, Thrown> {
        Err(Thrown::error(
            "ReflectionException",
            "reflection is not available in this host",
        ))
    }
    /// Set an object property bypassing visibility (`ReflectionProperty::setValue`),
    /// resolving `name` as class `class` would see it (its own private first).
    fn reflect_set_prop(
        &mut self,
        _obj: &Value,
        _class: &[u8],
        _name: &[u8],
        _val: Value,
    ) -> Result<(), Thrown> {
        Ok(())
    }
    /// Read an object property bypassing visibility (`ReflectionProperty::getValue`),
    /// resolving `name` as class `class` would see it (its own private first).
    fn reflect_get_prop(
        &mut self,
        obj: &Value,
        _class: &[u8],
        name: &[u8],
    ) -> Result<Option<Value>, Thrown> {
        Ok(self.object_prop(obj, name))
    }
    /// A Reflection / introspection metadata query (`op` names it; see the
    /// VM's `reflect` module): class / method / property / constant /
    /// attribute descriptors, declared-class and included-file lists, ….
    fn reflect_op(&mut self, _op: &[u8], _args: &[Value]) -> Result<Value, Thrown> {
        Ok(Value::Null)
    }
    /// An `opcache_*` function (`op` is the name without the prefix, the
    /// arguments already checked) answered by the host's compiled-script
    /// cache. `None`: the accelerator isn't started for this request.
    fn opcache_op(&mut self, _op: &str, _args: &[Value]) -> Option<Result<Value, Thrown>> {
        None
    }
    /// [`Host::raise_diagnostic`], returning the exception a user error
    /// handler threw (e.g. composer's `ErrorException`) so the builtin can
    /// stop right there with `?`.
    fn emit_diagnostic(&mut self, level: i64, message: &str) -> Result<(), Thrown> {
        self.raise_diagnostic(level, message);
        match self.take_pending_throw() {
            Some(t) => Err(t),
            None => Ok(()),
        }
    }
}

/// `E_WARNING`.
pub const E_WARNING: i64 = 2;
/// `E_NOTICE`.
pub const E_NOTICE: i64 = 8;
/// `E_DEPRECATED`.
pub const E_DEPRECATED: i64 = 8192;

/// `php_error_docref(NULL, E_WARNING, msg)` from builtin `fname`.
pub fn warn(host: &mut dyn Host, fname: &str, msg: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(E_WARNING, &format!("{fname}(): {msg}"))
}

/// The refcount-0 destructor of a native (non-stream) resource, given its
/// payload: a process closes its pipes and reaps the child if it already
/// exited (proc_open_rsrc_dtor); anything else is just dropped.
pub fn native_resource_dtor(host: &mut dyn Host, data: Box<dyn std::any::Any>) {
    proc_fns::proc_dtor(host, data);
}

/// `php_error_docref1(NULL, arg, E_WARNING, msg)`: `fname(arg): msg`.
pub fn warn_arg(host: &mut dyn Host, fname: &str, arg: &[u8], msg: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(
        E_WARNING,
        &format!("{fname}({}): {msg}", String::from_utf8_lossy(arg)),
    )
}

/// php_stream_open_wrapper's failure report for `path` from builtin `fname`:
/// an unknown wrapper / bad php:// URL first, then "Failed to open stream:"
/// with the errno the failing syscall recorded (see [`streams::note_errno`]).
pub fn open_failed(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<(), Thrown> {
    let scheme = streams::split_scheme(path).0.to_ascii_lowercase();
    // Only the plain-files wrapper reports errno; others say "operation failed".
    let mut reason = if scheme == b"file" {
        streams::last_strerror()
    } else {
        "operation failed".to_string()
    };
    let known = host.stream_wrapper_schemes();
    if scheme == b"php" {
        warn(host, fname, "Invalid php:// URL specified")?;
        reason = "operation failed".into();
    } else if scheme != b"file"
        && !matches!(&scheme[..], b"http" | b"https" | b"data" | b"glob")
        && !known.iter().any(|k| k.eq_ignore_ascii_case(&scheme))
    {
        let name = String::from_utf8_lossy(&scheme).into_owned();
        warn(
            host,
            fname,
            &format!(
                "Unable to find the wrapper \"{name}\" - did you forget to enable it when you configured PHP?"
            ),
        )?;
        reason = streams::strerror(libc::ENOENT);
    }
    warn_arg(
        host,
        fname,
        path,
        &format!("Failed to open stream: {reason}"),
    )
}

/// `php_error_docref(NULL, E_NOTICE, msg)` from builtin `fname`.
pub fn notice(host: &mut dyn Host, fname: &str, msg: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(E_NOTICE, &format!("{fname}(): {msg}"))
}

/// `php_error_docref(NULL, E_DEPRECATED, msg)` from builtin `fname`.
pub fn deprecated(host: &mut dyn Host, fname: &str, msg: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(E_DEPRECATED, &format!("{fname}(): {msg}"))
}

/// The `#[\Deprecated]` notice php raises on calling a deprecated function:
/// `Function f() is deprecated since V, MSG`.
pub fn deprecated_fn(
    host: &mut dyn Host,
    fname: &str,
    since: &str,
    msg: &str,
) -> Result<(), Thrown> {
    host.emit_diagnostic(
        E_DEPRECATED,
        &format!("Function {fname}() is deprecated since {since}, {msg}"),
    )
}

/// A native builtin function over owned values.
pub type NativeFn = fn(&mut dyn Host, &[Value]) -> Result<Value, Thrown>;

/// Registry entry: a native function plus its arity bounds.
pub struct Builtin {
    pub name: &'static str,
    pub func: NativeFn,
    pub min_args: usize,
    /// `None` = variadic (no upper bound).
    pub max_args: Option<usize>,
}

macro_rules! builtins {
    ($($name:literal => $func:path, $min:literal, $max:expr);* $(;)?) => {{
        let mut m: HashMap<&'static str, Builtin> = HashMap::new();
        $(
            m.insert($name, Builtin { name: $name, func: $func, min_args: $min, max_args: $max });
        )*
        m
    }};
}

pub use php_fns::{apply_env_overrides, env_overrides};

/// The php version this engine reports (`PHP_VERSION`).
pub const PHP_VERSION: &str = version::PHP_VERSION;

/// A directive's current value: `None` = unknown directive, `Some(None)` =
/// registered with a null value.
pub fn ini_value(name: &[u8]) -> Option<Option<Vec<u8>>> {
    runtime_fns::ini_value(name)
}

/// C `atol`: leading blanks, sign, digits.
pub fn c_atol(s: &[u8]) -> i64 {
    runtime_fns::c_atol(s)
}

/// `zend_ini_parse_quantity`: `8M` → 8388608.
pub fn ini_parse_quantity(s: &[u8]) -> i64 {
    runtime_fns::ini_parse_quantity(s)
}

/// A directive's startup value (`-d`, else the SAPI's or the built-in
/// default): what php's PHP_INI_SYSTEM directives keep. `None` = unknown.
pub fn ini_startup_value(name: &[u8]) -> Option<Vec<u8>> {
    runtime_fns::ini_default_value(name).map(Option::unwrap_or_default)
}

pub use opcache_fns::{
    accel_log_dest, accel_log_line, asctime_local, preload_user_check, take_accel_log,
};

/// A startup ini setting (`php -d name=value`): any registered directive,
/// applied to the engine state it drives (precision, ...). Unknown names are
/// ignored. Call after [`reset_request_state`].
pub fn ini_startup_set(name: &[u8], value: &[u8]) {
    runtime_fns::ini_startup(name, value);
}

pub use runtime_fns::{apply_memory_limits, take_shutdown_diagnostics, take_startup_diagnostics};

/// Reset every builtin's per-request state (ini overrides, default timezone,
/// precision, strtok/preg/json/mbstring state, process title, putenv
/// overlay). Engines call this on construction so state never leaks from one
/// script run to the next on the same thread.
pub fn reset_request_state() {
    ext::libxml::reset_state();
    runtime_fns::reset_state();
    sysmisc_fns::reset_state();
    preg_fns::reset_state();
    json_fns::reset_state();
    ext::mbstring::reset_state();
    strsearch_fns::reset_state();
    php_fns::reset_state();
    stdext_fns::reset_state();
    sapi::reset_state();
    zip_ext::reset_state();
    ext::curl::reset_state();
    ext::openssl::reset_state();
    ext::bz2::reset_state();
    ext::intl::error_reset();
    rhp_values::reset_dropped_streams();
}

/// The builtin function table, keyed by lowercase name (PHP function names are
/// case-insensitive).
pub fn registry() -> &'static HashMap<&'static str, Builtin> {
    static REG: OnceLock<HashMap<&'static str, Builtin>> = OnceLock::new();
    REG.get_or_init(|| {
        let mut m = builtins! {
            "var_dump"       => fn_var_dump, 1, None;
            "print_r"        => fn_print_r, 1, Some(2);
            "var_export"     => fn_var_export, 1, Some(2);
            "gettype"        => fn_gettype, 1, Some(1);
            "get_debug_type" => fn_get_debug_type, 1, Some(1);
            "is_numeric"     => fn_is_numeric, 1, Some(1);
            "is_int"         => fn_is_int, 1, Some(1);
            "is_integer"     => fn_is_int, 1, Some(1);
            "is_long"        => fn_is_int, 1, Some(1);
            "is_float"       => fn_is_float, 1, Some(1);
            "is_double"      => fn_is_float, 1, Some(1);
            "is_string"      => fn_is_string, 1, Some(1);
            "is_bool"        => fn_is_bool, 1, Some(1);
            "is_null"        => fn_is_null, 1, Some(1);
            "is_array"       => fn_is_array, 1, Some(1);
            "is_scalar"      => fn_is_scalar, 1, Some(1);
            "is_object"      => fn_is_object, 1, Some(1);
            "is_callable"    => fn_is_callable, 1, Some(3);
            "define"         => fn_define, 2, Some(3);
            "defined"        => fn_defined, 1, Some(1);
            "constant"       => fn_constant, 1, Some(1);
            "intval"         => fn_intval, 1, Some(2);
            "floatval"       => fn_floatval, 1, Some(1);
            "doubleval"      => fn_floatval, 1, Some(1);
            "strval"         => fn_strval, 1, Some(1);
            "boolval"        => fn_boolval, 1, Some(1);
            "strlen"         => fn_strlen, 1, Some(1);
            "count"          => fn_count, 1, Some(2);
            "sizeof"         => fn_sizeof, 1, Some(2);
            "intdiv"         => fn_intdiv, 2, Some(2);
            "str_repeat"     => fn_str_repeat, 2, Some(2);
            "abs"            => fn_abs, 1, Some(1);
            "max"            => fn_max, 1, None;
            "min"            => fn_min, 1, None;
            "array_sum"      => fn_array_sum, 1, Some(1);
            "array_product"  => fn_array_product, 1, Some(1);
            "array_keys"     => fn_array_keys, 1, Some(3);
            "is_nan"         => fn_is_nan, 1, Some(1);
            "is_infinite"    => fn_is_infinite, 1, Some(1);
            "is_finite"      => fn_is_finite, 1, Some(1);
            "array_values"   => fn_array_values, 1, Some(1);
            "array_merge"    => fn_array_merge, 0, None;
            "array_reverse"  => fn_array_reverse, 1, Some(2);
            "range"          => fn_range, 2, Some(3);
            "array_map"      => fn_array_map, 2, None;
            "array_filter"   => fn_array_filter, 1, Some(3);
            "array_reduce"   => fn_array_reduce, 2, Some(3);
            "call_user_func" => fn_call_user_func, 1, None;
            "call_user_func_array" => fn_call_user_func_array, 2, Some(2);
            "in_array"       => fn_in_array, 2, Some(3);
            "array_key_exists" => fn_array_key_exists, 2, Some(2);
            "implode"        => fn_implode, 1, Some(2);
            "join"           => fn_implode, 1, Some(2);
            "explode"        => fn_explode, 2, Some(3);
            "strtoupper"     => fn_strtoupper, 1, Some(1);
            "strtolower"     => fn_strtolower, 1, Some(1);
            "str_contains"   => fn_str_contains, 2, Some(2);
            "ord"            => fn_ord, 1, Some(1);
            "chr"            => fn_chr, 1, Some(1);
            "get_class"      => fn_get_class, 0, Some(1);
            "get_parent_class" => fn_get_parent_class, 0, Some(1);
            "is_a"           => fn_is_a, 2, Some(3);
            "is_subclass_of" => fn_is_subclass_of, 2, Some(3);
            "get_object_vars" => fn_get_object_vars, 1, Some(1);
            "get_mangled_object_vars" => fn_get_mangled_object_vars, 1, Some(1);
            "method_exists"  => fn_method_exists, 2, Some(2);
            "property_exists" => fn_property_exists, 2, Some(2);
            "class_exists"   => fn_class_exists, 1, Some(2);
            "interface_exists" => fn_interface_exists, 1, Some(2);
            "enum_exists"    => fn_enum_exists, 1, Some(2);
            "iterator_to_array" => fn_iterator_to_array, 1, Some(2);
        };
        for (_, register) in builtin_modules() {
            register(&mut m);
        }
        m
    })
}

type Registrar = Box<dyn Fn(&mut HashMap<&'static str, Builtin>)>;

/// Every builtin module's registration, in registry order (a later module
/// would override an earlier one's function of the same name; the
/// `no_duplicate_builtins` test keeps names unique).
fn builtin_modules() -> Vec<(&'static str, Registrar)> {
    let mut v: Vec<(&'static str, Registrar)> = vec![
        ("str_fns", Box::new(str_fns::register)),
        ("math_fns", Box::new(math_fns::register)),
        ("arr_fns", Box::new(arr_fns::register)),
        ("misc_fns", Box::new(misc_fns::register)),
        ("file_fns", Box::new(file_fns::register)),
    ];
    // Discrete extensions (php-src ext/<name>/ layout): each a self-contained
    // module owning its functions/constants/classes/wrappers.
    for md in ext::modules() {
        v.push((md.name, Box::new(move |m| (md.functions)(m))));
    }
    let rest: Vec<(&'static str, Registrar)> = vec![
        ("webenc_fns", Box::new(webenc_fns::register)),
        ("html", Box::new(html::register)),
        ("datetime_fns", Box::new(datetime_fns::register)),
        ("dt_oop_fns", Box::new(dt_oop_fns::register)),
        ("strtotime_fns", Box::new(strtotime_fns::register)),
        ("strarr2_fns", Box::new(strarr2_fns::register)),
        ("sort_fns", Box::new(sort_fns::register)),
        ("serialize_fns", Box::new(serialize_fns::register)),
        ("filter_fns", Box::new(filter_fns::register)),
        ("strcmp_fns", Box::new(strcmp_fns::register)),
        ("printf_fns", Box::new(printf_fns::register)),
        ("arrayset_fns", Box::new(arrayset_fns::register)),
        ("preg_fns", Box::new(preg_fns::register)),
        ("json_fns", Box::new(json_fns::register)),
        ("parse_fns", Box::new(parse_fns::register)),
        ("php_fns", Box::new(php_fns::register)),
        ("info_fns", Box::new(info_fns::register)),
        ("stream_fns", Box::new(stream_fns::register)),
        ("socket_fns", Box::new(socket_fns::register)),
        ("strsearch_fns", Box::new(strsearch_fns::register)),
        ("query_fns", Box::new(query_fns::register)),
        ("runtime_fns", Box::new(runtime_fns::register)),
        ("class_fns", Box::new(class_fns::register)),
        ("sapi", Box::new(sapi::register)),
        ("rfc1867", Box::new(rfc1867::register)),
        ("opcache_fns", Box::new(opcache_fns::register)),
        ("ini_parse", Box::new(ini_parse::register)),
        ("arraycmp_fns", Box::new(arraycmp_fns::register)),
        ("sysmisc_fns", Box::new(sysmisc_fns::register)),
        ("stdext_fns", Box::new(stdext_fns::register)),
        ("crypt_fns", Box::new(crypt_fns::register)),
        ("proc_fns", Box::new(proc_fns::register)),
        ("fs_fns", Box::new(fs_fns::register)),
    ];
    v.extend(rest);
    v
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    /// Branches developed in parallel must not register the same builtin
    /// twice: the later registration silently shadows the earlier one.
    #[test]
    fn no_duplicate_builtins() {
        let mut owner: HashMap<&'static str, &'static str> = HashMap::new();
        let mut dups = Vec::new();
        for (module, register) in builtin_modules() {
            let mut m = HashMap::new();
            register(&mut m);
            for name in m.keys() {
                if let Some(prev) = owner.insert(name, module) {
                    dups.push(format!("{name}: {prev} and {module}"));
                }
            }
        }
        dups.sort();
        assert!(dups.is_empty(), "duplicate builtins:\n{}", dups.join("\n"));
    }

    /// Every constant has one definition. The engines load `constants()`
    /// into a map, so a second definition silently overrides the first, and
    /// a version bump made in one copy (version.rs) would not take effect.
    #[test]
    fn no_duplicate_constants() {
        let mut seen = std::collections::HashSet::new();
        let mut dups: Vec<&str> = constants()
            .into_iter()
            .map(|(name, _)| name)
            .filter(|name| !seen.insert(*name))
            .collect();
        dups.sort();
        assert!(dups.is_empty(), "constants defined twice: {dups:?}");
    }
}

/// Case-insensitive builtin lookup.
pub fn lookup(name: &str) -> Option<&'static Builtin> {
    let lower = name.to_ascii_lowercase();
    lookup_lc(&lower)
}

/// [`lookup`] for a name that is already lowercase (no allocation).
pub fn lookup_lc(name_lc: &str) -> Option<&'static Builtin> {
    let b = registry().get(name_lc)?;
    if matches!(name_lc.as_bytes().first(), Some(b'a' | b'c' | b'g'))
        && sapi::builtin_hidden(name_lc)
    {
        return None;
    }
    Some(b)
}

/// `(PHP_OS, PHP_OS_FAMILY)` for the platform this engine was built for.
fn php_os() -> (&'static str, &'static str) {
    if cfg!(target_os = "macos") {
        ("Darwin", "Darwin")
    } else if cfg!(target_os = "linux") {
        ("Linux", "Linux")
    } else if cfg!(target_os = "windows") {
        ("WINNT", "Windows")
    } else if cfg!(target_os = "freebsd") {
        ("FreeBSD", "BSD")
    } else if cfg!(target_os = "openbsd") {
        ("OpenBSD", "BSD")
    } else if cfg!(target_os = "netbsd") {
        ("NetBSD", "BSD")
    } else if cfg!(target_os = "solaris") {
        ("SunOS", "Solaris")
    } else {
        ("Unknown", "Unknown")
    }
}

/// Engine constants (`PHP_EOL`, `PHP_INT_MAX`, …).
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("PHP_EOL", Value::str("\n")),
        ("PHP_INT_MAX", Value::Int(i64::MAX)),
        ("PHP_INT_MIN", Value::Int(i64::MIN)),
        ("PHP_INT_SIZE", Value::Int(8)),
        ("PHP_FLOAT_EPSILON", Value::Float(f64::EPSILON)),
        ("PHP_FLOAT_MAX", Value::Float(f64::MAX)),
        ("PHP_FLOAT_MIN", Value::Float(f64::MIN_POSITIVE)),
        ("PHP_VERSION", Value::str(version::PHP_VERSION)),
        ("PHP_MAJOR_VERSION", Value::Int(version::PHP_MAJOR_VERSION)),
        ("PHP_MINOR_VERSION", Value::Int(version::PHP_MINOR_VERSION)),
        (
            "PHP_RELEASE_VERSION",
            Value::Int(version::PHP_RELEASE_VERSION),
        ),
        ("PHP_EXTRA_VERSION", Value::str(version::PHP_EXTRA_VERSION)),
        ("PHP_VERSION_ID", Value::Int(version::PHP_VERSION_ID)),
        ("PHP_SAPI", Value::str("cli")),
        // php reports its own executable; so do we (composer spawns PHP_BINARY).
        (
            "PHP_BINARY",
            Value::str(
                std::env::current_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
        ),
        // php bakes in its build host's OS (uname -s) and family.
        ("PHP_OS", Value::str(php_os().0)),
        ("PHP_OS_FAMILY", Value::str(php_os().1)),
        ("ZEND_THREAD_SAFE", Value::Bool(false)),
        ("ZEND_DEBUG_BUILD", Value::Bool(false)),
        ("ZEND_VM_KIND", Value::str(version::ZEND_VM_KIND)),
        ("TRUE", Value::Bool(true)),
        ("FALSE", Value::Bool(false)),
        ("NULL", Value::Null),
        ("PHP_FD_SETSIZE", Value::Int(1024)),
        ("UPLOAD_ERR_OK", Value::Int(0)),
        ("UPLOAD_ERR_INI_SIZE", Value::Int(1)),
        ("UPLOAD_ERR_FORM_SIZE", Value::Int(2)),
        ("UPLOAD_ERR_PARTIAL", Value::Int(3)),
        ("UPLOAD_ERR_NO_FILE", Value::Int(4)),
        ("UPLOAD_ERR_NO_TMP_DIR", Value::Int(6)),
        ("UPLOAD_ERR_CANT_WRITE", Value::Int(7)),
        ("UPLOAD_ERR_EXTENSION", Value::Int(8)),
        // php-cli only (the VM drops it under other SAPIs, like STDIN).
        ("PHP_CLI_PROCESS_TITLE", Value::Bool(true)),
        ("PHP_DEBUG", Value::Bool(false)),
        ("PHP_ZTS", Value::Bool(false)),
        ("ICONV_VERSION", Value::str(version::ICONV_VERSION)),
        ("ICONV_IMPL", Value::str("libiconv")),
        ("ICONV_MIBENUM_ASCII", Value::Int(3)),
        ("DATE_ATOM", Value::str("Y-m-d\\TH:i:sP")),
        ("DATE_RFC3339", Value::str("Y-m-d\\TH:i:sP")),
        ("DATE_W3C", Value::str("Y-m-d\\TH:i:sP")),
        ("DATE_ISO8601", Value::str("Y-m-d\\TH:i:sO")),
        ("DATE_COOKIE", Value::str("l, d-M-Y H:i:s T")),
        ("DATE_RFC2822", Value::str("D, d M Y H:i:s O")),
        ("DATE_RSS", Value::str("D, d M Y H:i:s O")),
        ("DATE_RFC822", Value::str("D, d M y H:i:s O")),
        ("DATE_RFC850", Value::str("l, d-M-y H:i:s T")),
        ("DATE_RFC1036", Value::str("D, d M y H:i:s O")),
        ("DATE_RFC1123", Value::str("D, d M Y H:i:s O")),
        // LIBXML_* live with the libxml module (ext/libxml).
        ("FILE_USE_INCLUDE_PATH", Value::Int(1)),
        ("FILE_IGNORE_NEW_LINES", Value::Int(2)),
        ("FILE_SKIP_EMPTY_LINES", Value::Int(4)),
        ("FILE_APPEND", Value::Int(8)),
        ("FILE_NO_DEFAULT_CONTEXT", Value::Int(16)),
        ("FILE_TEXT", Value::Int(0)),
        ("FILE_BINARY", Value::Int(0)),
        // SEEK_* come from fs_fns::constants().
        ("LOCK_SH", Value::Int(1)),
        ("LOCK_EX", Value::Int(2)),
        ("LOCK_UN", Value::Int(3)),
        ("LOCK_NB", Value::Int(4)),
        (
            "MB_ONIGURUMA_VERSION",
            Value::str(version::ONIGURUMA_VERSION),
        ),
        ("DIRECTORY_SEPARATOR", Value::str("/")),
        ("PATH_SEPARATOR", Value::str(":")),
        ("PHP_MAXPATHLEN", Value::Int(libc::PATH_MAX as i64)), // MAXPATHLEN: 1024 on macOS, 4096 on Linux
        ("PHP_BINDIR", Value::str(version::BINDIR)),
        ("PHP_PREFIX", Value::str(version::PREFIX)),
        ("PHP_EXTENSION_DIR", Value::str(version::EXTENSION_DIR)),
        ("DEFAULT_INCLUDE_PATH", Value::str(version::INCLUDE_PATH)),
        ("PHP_BUILD_DATE", Value::str(version::BUILD_DATE)),
        ("PHP_BUILD_PROVIDER", Value::str(version::BUILD_PROVIDER)),
        ("PEAR_INSTALL_DIR", Value::str(version::PEAR_INSTALL_DIR)),
        (
            "PEAR_EXTENSION_DIR",
            Value::str(version::PEAR_EXTENSION_DIR),
        ),
        ("PHP_SBINDIR", Value::str(version::SBINDIR)),
        ("PHP_MANDIR", Value::str(version::MANDIR)),
        ("PHP_LIBDIR", Value::str(version::LIBDIR)),
        ("PHP_DATADIR", Value::str(version::DATADIR)),
        ("PHP_SYSCONFDIR", Value::str(version::SYSCONFDIR)),
        ("PHP_LOCALSTATEDIR", Value::str(version::LOCALSTATEDIR)),
        (
            "PHP_CONFIG_FILE_PATH",
            Value::str(version::CONFIG_FILE_PATH),
        ),
        (
            "PHP_CONFIG_FILE_SCAN_DIR",
            Value::str(version::CONFIG_FILE_SCAN_DIR),
        ),
        ("PHP_SHLIB_SUFFIX", Value::str(version::SHLIB_SUFFIX)),
        ("LC_CTYPE", Value::Int(0)),
        ("LC_NUMERIC", Value::Int(1)),
        ("LC_TIME", Value::Int(2)),
        ("LC_COLLATE", Value::Int(3)),
        ("LC_MONETARY", Value::Int(4)),
        ("LC_MESSAGES", Value::Int(5)),
        ("LC_ALL", Value::Int(6)),
        ("INI_SCANNER_NORMAL", Value::Int(0)),
        ("INI_SCANNER_RAW", Value::Int(1)),
        ("INI_SCANNER_TYPED", Value::Int(2)),
        ("PHP_QUERY_RFC1738", Value::Int(1)),
        ("PHP_QUERY_RFC3986", Value::Int(2)),
        ("PHP_URL_SCHEME", Value::Int(0)),
        ("PHP_URL_HOST", Value::Int(1)),
        ("PHP_URL_PORT", Value::Int(2)),
        ("PHP_URL_USER", Value::Int(3)),
        ("PHP_URL_PASS", Value::Int(4)),
        ("PHP_URL_PATH", Value::Int(5)),
        ("PHP_URL_QUERY", Value::Int(6)),
        ("PHP_URL_FRAGMENT", Value::Int(7)),
        ("PATHINFO_DIRNAME", Value::Int(1)),
        ("PATHINFO_BASENAME", Value::Int(2)),
        ("PATHINFO_EXTENSION", Value::Int(4)),
        ("PATHINFO_FILENAME", Value::Int(8)),
        ("PATHINFO_ALL", Value::Int(15)),
        ("DEBUG_BACKTRACE_PROVIDE_OBJECT", Value::Int(1)),
        ("DEBUG_BACKTRACE_IGNORE_ARGS", Value::Int(2)),
        ("E_ERROR", Value::Int(1)),
        ("E_PARSE", Value::Int(4)),
        ("E_CORE_ERROR", Value::Int(16)),
        ("E_CORE_WARNING", Value::Int(32)),
        ("E_COMPILE_ERROR", Value::Int(64)),
        ("E_COMPILE_WARNING", Value::Int(128)),
        ("E_USER_ERROR", Value::Int(256)),
        ("E_USER_WARNING", Value::Int(512)),
        ("E_USER_NOTICE", Value::Int(1024)),
        ("E_STRICT", Value::Int(2048)),
        ("E_RECOVERABLE_ERROR", Value::Int(4096)),
        ("E_DEPRECATED", Value::Int(8192)),
        ("E_USER_DEPRECATED", Value::Int(16384)),
        ("NAN", Value::Float(f64::NAN)),
        ("INF", Value::Float(f64::INFINITY)),
        ("E_ALL", Value::Int(30719)),
        ("E_WARNING", Value::Int(2)),
        ("E_NOTICE", Value::Int(8)),
        ("COUNT_NORMAL", Value::Int(0)),
        ("COUNT_RECURSIVE", Value::Int(1)),
        // Placeholders: the VM binds the stream resources when a request
        // starts (php-cli), or removes them (other SAPIs).
        ("STDIN", Value::Int(0)),
        ("STDOUT", Value::Int(1)),
        ("STDERR", Value::Int(2)),
        ("CONNECTION_ABORTED", Value::Int(1)),
        ("CONNECTION_NORMAL", Value::Int(0)),
        ("CONNECTION_TIMEOUT", Value::Int(2)),
        ("STR_PAD_RIGHT", Value::Int(1)),
        ("STR_PAD_LEFT", Value::Int(0)),
        ("STR_PAD_BOTH", Value::Int(2)),
        ("SORT_REGULAR", Value::Int(0)),
        ("SORT_NUMERIC", Value::Int(1)),
        ("SORT_STRING", Value::Int(2)),
        ("SORT_DESC", Value::Int(3)),
        ("SORT_ASC", Value::Int(4)),
        ("SORT_LOCALE_STRING", Value::Int(5)),
        ("SORT_NATURAL", Value::Int(6)),
        ("SORT_FLAG_CASE", Value::Int(8)),
        ("MB_CASE_UPPER", Value::Int(0)),
        ("MB_CASE_LOWER", Value::Int(1)),
        ("MB_CASE_TITLE", Value::Int(2)),
        ("MB_CASE_FOLD", Value::Int(3)),
        ("MB_CASE_UPPER_SIMPLE", Value::Int(4)),
        ("MB_CASE_LOWER_SIMPLE", Value::Int(5)),
        ("MB_CASE_TITLE_SIMPLE", Value::Int(6)),
        ("MB_CASE_FOLD_SIMPLE", Value::Int(7)),
        ("FILTER_DEFAULT", Value::Int(516)),
        ("FILTER_UNSAFE_RAW", Value::Int(516)),
        ("FILTER_VALIDATE_INT", Value::Int(257)),
        ("FILTER_VALIDATE_BOOLEAN", Value::Int(258)),
        ("FILTER_VALIDATE_BOOL", Value::Int(258)),
        ("FILTER_VALIDATE_FLOAT", Value::Int(259)),
        ("FILTER_VALIDATE_REGEXP", Value::Int(272)),
        ("FILTER_VALIDATE_DOMAIN", Value::Int(277)),
        ("FILTER_VALIDATE_URL", Value::Int(273)),
        ("FILTER_VALIDATE_EMAIL", Value::Int(274)),
        ("FILTER_VALIDATE_IP", Value::Int(275)),
        ("FILTER_VALIDATE_MAC", Value::Int(276)),
        ("FILTER_SANITIZE_ENCODED", Value::Int(514)),
        ("FILTER_SANITIZE_SPECIAL_CHARS", Value::Int(515)),
        ("FILTER_SANITIZE_EMAIL", Value::Int(517)),
        ("FILTER_SANITIZE_URL", Value::Int(518)),
        ("FILTER_SANITIZE_NUMBER_INT", Value::Int(519)),
        ("FILTER_SANITIZE_NUMBER_FLOAT", Value::Int(520)),
        ("FILTER_SANITIZE_FULL_SPECIAL_CHARS", Value::Int(522)),
        ("FILTER_SANITIZE_ADD_SLASHES", Value::Int(523)),
        ("FILTER_FLAG_NONE", Value::Int(0)),
        ("FILTER_NULL_ON_FAILURE", Value::Int(134217728)),
        ("FILTER_FLAG_ALLOW_OCTAL", Value::Int(1)),
        ("FILTER_FLAG_ALLOW_HEX", Value::Int(2)),
        ("FILTER_FLAG_ALLOW_THOUSAND", Value::Int(8192)),
        ("FILTER_FLAG_ALLOW_SCIENTIFIC", Value::Int(16384)),
        ("FILTER_FLAG_IPV4", Value::Int(1048576)),
        ("FILTER_FLAG_IPV6", Value::Int(2097152)),
        ("FILTER_FLAG_NO_PRIV_RANGE", Value::Int(8388608)),
        ("FILTER_FLAG_NO_RES_RANGE", Value::Int(4194304)),
        ("FILTER_FLAG_PATH_REQUIRED", Value::Int(262144)),
        ("FILTER_FLAG_QUERY_REQUIRED", Value::Int(524288)),
        ("FILTER_FLAG_HOSTNAME", Value::Int(1048576)),
        ("FILTER_FLAG_EMAIL_UNICODE", Value::Int(1048576)),
        ("ENT_QUOTES", Value::Int(3)),
        ("ENT_COMPAT", Value::Int(2)),
        ("ENT_NOQUOTES", Value::Int(0)),
        ("ENT_HTML401", Value::Int(0)),
        ("ENT_HTML5", Value::Int(48)),
        ("ENT_XML1", Value::Int(16)),
        ("ENT_XHTML", Value::Int(32)),
        ("ENT_IGNORE", Value::Int(4)),
        ("ENT_SUBSTITUTE", Value::Int(8)),
        ("ENT_DISALLOWED", Value::Int(128)),
        ("HTML_SPECIALCHARS", Value::Int(0)),
        ("HTML_ENTITIES", Value::Int(1)),
        ("PHP_ROUND_HALF_UP", Value::Int(1)),
        ("PHP_ROUND_HALF_DOWN", Value::Int(2)),
        ("PHP_ROUND_HALF_EVEN", Value::Int(3)),
        ("PHP_ROUND_HALF_ODD", Value::Int(4)),
        ("ARRAY_FILTER_USE_KEY", Value::Int(2)),
        ("ARRAY_FILTER_USE_BOTH", Value::Int(1)),
        ("M_PI", Value::Float(std::f64::consts::PI)),
        ("M_E", Value::Float(std::f64::consts::E)),
        ("M_SQRT2", Value::Float(std::f64::consts::SQRT_2)),
        ("PHP_FLOAT_DIG", Value::Int(15)),
        // main/php_output.h
        ("PHP_OUTPUT_HANDLER_START", Value::Int(0x01)),
        ("PHP_OUTPUT_HANDLER_WRITE", Value::Int(0x00)),
        ("PHP_OUTPUT_HANDLER_FLUSH", Value::Int(0x04)),
        ("PHP_OUTPUT_HANDLER_CLEAN", Value::Int(0x02)),
        ("PHP_OUTPUT_HANDLER_FINAL", Value::Int(0x08)),
        ("PHP_OUTPUT_HANDLER_CONT", Value::Int(0x00)),
        ("PHP_OUTPUT_HANDLER_END", Value::Int(0x08)),
        ("PHP_OUTPUT_HANDLER_CLEANABLE", Value::Int(0x10)),
        ("PHP_OUTPUT_HANDLER_FLUSHABLE", Value::Int(0x20)),
        ("PHP_OUTPUT_HANDLER_REMOVABLE", Value::Int(0x40)),
        ("PHP_OUTPUT_HANDLER_STDFLAGS", Value::Int(0x70)),
        ("PHP_OUTPUT_HANDLER_STARTED", Value::Int(0x1000)),
        ("PHP_OUTPUT_HANDLER_DISABLED", Value::Int(0x2000)),
        ("PHP_OUTPUT_HANDLER_PROCESSED", Value::Int(0x4000)),
    ]
    .into_iter()
    .chain(preg_fns::preg_constants())
    .chain(json_fns::json_constants())
    .chain(info_fns::constants())
    .chain(fs_fns::constants())
    .chain(stdext_fns::constants())
    .chain(crypt_fns::constants())
    .chain(socket_fns::constants())
    // Per-extension constants (GMP_*, PASSWORD_*, ZLIB_*, …) live with their module.
    .chain(ext::modules().into_iter().flat_map(|md| (md.constants)()))
    .collect()
}

/// Concatenated PHP-source class declarations contributed by extension modules
/// (GMP, Phar, …). The compiler compiles these into the prelude alongside the
/// engine's own core classes.
pub fn module_prelude() -> String {
    let mut s = String::new();
    s.push_str(fs_fns::PRELUDE_PHP);
    s.push('\n');
    s.push_str(stdext_fns::PRELUDE_PHP);
    s.push('\n');
    for md in ext::modules() {
        if !md.prelude_php.is_empty() {
            s.push_str(md.prelude_php);
            s.push('\n');
        }
    }
    s
}

/// Built-in stream wrappers contributed by extension modules (e.g. `phar://`),
/// installed into the VM's per-run registry at startup.
pub fn stream_wrappers() -> Vec<WrapperReg> {
    ext::modules()
        .into_iter()
        .flat_map(|md| (md.wrappers)())
        .collect()
}

/// If `name` is a deliberately-refused high-risk function, return why.
pub fn is_refused(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "extract" => Some("writes arbitrary variables into the local scope"),
        "compact" => Some("resolves variable names dynamically from strings"),
        "create_function" => Some("compiles a string into a function (eval-equivalent)"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// builtin implementations
// ---------------------------------------------------------------------------

use rhp_semantics::{deref, ops};

fn fn_var_dump(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    for a in args {
        let r = dump::resolve_views(host, a, object::PropPurpose::Debug);
        if r.is_ok() {
            dump::var_dump(host, &mut out, a, 0);
        }
        dump::clear_views();
        if let Err(t) = r {
            host.write_out(&out);
            return Err(t);
        }
    }
    host.write_out(&out);
    Ok(Value::Null)
}

fn fn_print_r(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    let r = dump::resolve_views(host, &args[0], object::PropPurpose::Debug);
    if r.is_ok() {
        dump::print_r(host, &mut out, &args[0], 0);
    }
    dump::clear_views();
    r?;
    let return_mode = args.get(1).map(conv::to_bool).unwrap_or(false);
    if return_mode {
        Ok(Value::str(out))
    } else {
        host.write_out(&out);
        Ok(Value::Bool(true))
    }
}

fn fn_var_export(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    let r = dump::resolve_views(host, &args[0], object::PropPurpose::VarExport);
    let circular = if r.is_ok() {
        dump::var_export(host, &mut out, &args[0], 1)
    } else {
        0
    };
    dump::clear_views();
    r?;
    for _ in 0..circular {
        host.emit_diagnostic(E_WARNING, "var_export does not handle circular references")?;
    }
    let return_mode = args.get(1).map(conv::to_bool).unwrap_or(false);
    if return_mode {
        Ok(Value::str(out))
    } else {
        host.write_out(&out);
        Ok(Value::Null)
    }
}

fn fn_gettype(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    if host.resource_closed(&v) {
        return Ok(Value::str("resource (closed)"));
    }
    Ok(Value::str(v.type_name()))
}

fn fn_get_debug_type(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = match deref(&args[0]) {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Closure(_) => "Closure",
        o @ Value::Object(_) => {
            // php: the class name. An anonymous class is named
            // `Prefix@anonymous\0file:line$n`; only the part before the NUL shows.
            let cname = host.object_class_name(&o).unwrap_or_default();
            let end = cname.iter().position(|&b| b == 0).unwrap_or(cname.len());
            return Ok(Value::str(cname[..end].to_vec()));
        }
        ref v @ Value::Resource(ref r) => {
            if host.resource_closed(v) {
                return Ok(Value::str("resource (closed)"));
            }
            return Ok(Value::str(format!("resource ({})", r.type_name)));
        }
        Value::Ref(_) => unreachable!(),
    };
    Ok(Value::str(name))
}

fn fn_is_numeric(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(conv::is_numeric(&args[0])))
}

macro_rules! is_type {
    ($name:ident, $pat:pat) => {
        fn $name(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            Ok(Value::Bool(matches!(deref(&args[0]), $pat)))
        }
    };
}
is_type!(fn_is_int, Value::Int(_));
is_type!(fn_is_float, Value::Float(_));
is_type!(fn_is_string, Value::Str(_));
is_type!(fn_is_bool, Value::Bool(_));
is_type!(fn_is_null, Value::Null);
is_type!(fn_is_array, Value::Array(_));
is_type!(fn_is_object, Value::Object(_) | Value::Closure(_));

fn fn_is_nan(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        matches!(deref(&args[0]), Value::Float(f) if f.is_nan()),
    ))
}
fn fn_is_infinite(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        matches!(deref(&args[0]), Value::Float(f) if f.is_infinite()),
    ))
}
fn fn_is_finite(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        matches!(deref(&args[0]), Value::Float(f) if f.is_finite()),
    ))
}

fn fn_is_callable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    if let Some(Value::Ref(rc)) = args.get(2) {
        *rc.borrow_mut() = Value::str(callable_name(host, &v));
    }
    // `$syntax_only`: only the callable's shape is checked.
    if args.get(1).is_some_and(|f| conv::to_bool(&deref(f))) {
        match &v {
            Value::Str(_) | Value::Closure(_) => return Ok(Value::Bool(true)),
            Value::Array(a) if a.len() == 2 => {
                let parts: Vec<Value> = a.iter().map(|(_, x)| deref(x)).collect();
                return Ok(Value::Bool(
                    matches!(parts[0], Value::Str(_) | Value::Object(_))
                        && matches!(parts[1], Value::Str(_)),
                ));
            }
            Value::Object(_) => {}
            _ => return Ok(Value::Bool(false)),
        }
    }
    Ok(Value::Bool(host.check_callable(&v)?.is_none()))
}

/// zend_get_callable_name_ex: `is_callable`'s `$callable_name`.
fn callable_name(host: &dyn Host, v: &Value) -> Vec<u8> {
    let class_of = |h: &rhp_values::ObjectHandle| {
        let cls = h.borrow().class;
        host.class_name(cls).unwrap_or_default()
    };
    match v {
        Value::Array(a) if a.len() == 2 => {
            let parts: Vec<Value> = a.iter().map(|(_, x)| deref(x)).collect();
            let method = match &parts[1] {
                Value::Str(m) => m.as_bytes().to_vec(),
                _ => return b"Array".to_vec(),
            };
            let class = match &parts[0] {
                Value::Str(c) => c.as_bytes().to_vec(),
                Value::Object(h) => class_of(h),
                _ => return b"Array".to_vec(),
            };
            [&class[..], b"::", &method[..]].concat()
        }
        Value::Array(_) => b"Array".to_vec(),
        Value::Closure(cd) => match &cd.wrapped {
            // A first-class callable names what it wraps (`strlen`, `A::m`).
            Some(w) => callable_name(host, &deref(w)),
            None => host
                .closure_name(v)
                .unwrap_or_else(|| b"Closure::__invoke".to_vec()),
        },
        Value::Object(h) => [&class_of(h)[..], b"::__invoke"].concat(),
        other => conv::to_string_bytes(other),
    }
}

/// zpp's `f` check: "F(): Argument #N ($callback) must be a valid callback, …".
pub fn require_callback(
    host: &mut dyn Host,
    func: &str,
    n: usize,
    callable: &Value,
) -> Result<(), Thrown> {
    match host.check_callable(callable)? {
        None => Ok(()),
        Some(why) => Err(Thrown::type_error(format!(
            "{func}(): Argument #{n} ($callback) must be a valid callback, {why}"
        ))),
    }
}

fn fn_define(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    let value = deref(&args[1]).clone();
    if args.get(2).is_some_and(|v| conv::to_bool(&deref(v))) {
        host.emit_diagnostic(
            E_WARNING,
            "define(): Argument #3 ($case_insensitive) is ignored since declaration of case-insensitive constants is no longer supported",
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
pub fn const_redefined_message(name: &[u8]) -> String {
    let key = const_key(name);
    let shown = &key[..key.iter().position(|&b| b == 0).unwrap_or(key.len())];
    format!(
        "Constant {} already defined, this will be an error in PHP 9",
        String::from_utf8_lossy(shown)
    )
}

/// Resolve a constant by name: a class constant `Class::CONST` (incl. enum cases)
/// or a global constant. A leading `\` is skipped, as zend_get_constant_ex
/// does.
fn resolve_const(host: &mut dyn Host, name: &[u8]) -> Result<Option<Value>, Thrown> {
    let name = name.strip_prefix(b"\\").unwrap_or(name);
    if let Some(pos) = find_sub(name, b"::") {
        host.class_constant_eval(&name[..pos], &name[pos + 2..])
    } else {
        Ok(host.get_const(name))
    }
}

fn fn_defined(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    Ok(Value::Bool(resolve_const(host, &name)?.is_some()))
}

fn fn_constant(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    resolve_const(host, &name)?.ok_or_else(|| {
        let shown = name.strip_prefix(b"\\").unwrap_or(&name);
        Thrown::error(
            "Error",
            format!("Undefined constant \"{}\"", String::from_utf8_lossy(shown)),
        )
    })
}
is_type!(
    fn_is_scalar,
    Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_)
);

fn fn_intval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    let base = args.get(1).map(|b| conv::to_int(&deref(b))).unwrap_or(10);
    // zval_get_long: the object's cast_object.
    if let Some(n) = object::object_to_number(host, &v, object::CastTarget::Int)? {
        return Ok(Value::Int(conv::to_int(&n)));
    }
    // `base` applies only to a string argument; base 10 is plain integer conversion.
    if base == 10 {
        return Ok(Value::Int(conv::to_int(&v)));
    }
    let Value::Str(s) = &v else {
        return Ok(Value::Int(conv::to_int(&v)));
    };
    Ok(Value::Int(intval_base(s.as_bytes(), base)))
}

/// Parse a string in an explicit base, matching PHP's `intval($s, $base)`:
/// skips leading whitespace, an optional sign, and (for base 0/16) a `0x` / (0/8)
/// `0o`/`0` / (0/2) `0b` prefix; stops at the first invalid digit.
fn intval_base(bytes: &[u8], mut base: i64) -> i64 {
    let mut i = 0;
    while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    let neg = match bytes.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    // Base 0: auto-detect from the prefix.
    let rest = &bytes[i..];
    let (digits_start, detected) = detect_prefix(rest, base);
    i += digits_start;
    if base == 0 {
        base = detected;
    }
    if !(2..=36).contains(&base) {
        return 0;
    }
    let mut acc: i64 = 0;
    while i < bytes.len() {
        let d = match bytes[i] {
            b'0'..=b'9' => (bytes[i] - b'0') as i64,
            b'a'..=b'z' => (bytes[i] - b'a' + 10) as i64,
            b'A'..=b'Z' => (bytes[i] - b'A' + 10) as i64,
            _ => break,
        };
        if d >= base {
            break;
        }
        acc = acc.saturating_mul(base).saturating_add(d);
        i += 1;
    }
    if neg {
        -acc
    } else {
        acc
    }
}

/// Returns (bytes-consumed-by-prefix, detected-base) for base 0/16/8/2 prefixes.
fn detect_prefix(rest: &[u8], base: i64) -> (usize, i64) {
    match rest {
        [b'0', b'x' | b'X', ..] if base == 0 || base == 16 => (2, 16),
        [b'0', b'b' | b'B', ..] if base == 0 || base == 2 => (2, 2),
        [b'0', b'o' | b'O', ..] if base == 0 || base == 8 => (2, 8),
        [b'0', ..] if base == 0 => (1, 8),
        _ => (0, if base == 0 { 10 } else { base }),
    }
}

fn fn_floatval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // zval_get_double: the object's cast_object.
    if let Some(f) = object::object_to_number(host, &args[0], object::CastTarget::Float)? {
        return Ok(Value::Float(conv::to_float(&f)));
    }
    Ok(Value::Float(conv::to_float(&args[0])))
}

fn fn_strval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // zval_get_string: objects go through their cast handler (`__toString`).
    let v = deref(&args[0]);
    if let Value::Object(h) = &v {
        let mut cb = PhpArray::new();
        cb.push(v.clone());
        cb.push(Value::str("__toString"));
        let cb = Value::Array(cb);
        if host.is_callable_value(&cb) {
            return Ok(Value::str(conv::to_string_bytes(
                &host.call_callable(&cb, &[])?,
            )));
        }
        let class = host.class_name(h.borrow().class).unwrap_or_default();
        return Err(Thrown::error(
            "Error",
            format!(
                "Object of class {} could not be converted to string",
                String::from_utf8_lossy(&class)
            ),
        ));
    }
    Ok(Value::str(conv::to_string_bytes(&args[0])))
}

fn fn_boolval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(object::is_true(host, &args[0])?))
}

fn fn_strlen(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // Without copying the string (it may be most of memory_limit).
    let n = match &args[0] {
        Value::Str(s) => s.as_bytes().len(),
        v => conv::to_string_bytes(v).len(),
    };
    Ok(Value::Int(n as i64))
}

/// php_count_recursive: an array met again through a reference warns
/// "Recursion detected" and counts 0.
fn count_recursive(
    host: &mut dyn Host,
    fname: &str,
    a: &rhp_values::PhpArray,
    seen: &mut Vec<usize>,
) -> Result<i64, Thrown> {
    if seen.contains(&a.data_ptr()) {
        host.emit_diagnostic(E_WARNING, &format!("{fname}(): Recursion detected"))?;
        return Ok(0);
    }
    seen.push(a.data_ptr());
    let mut n = a.len() as i64;
    for (_, v) in a.iter() {
        if let Value::Array(sub) = deref(v) {
            n += count_recursive(host, fname, &sub, seen)?;
        }
    }
    seen.pop();
    Ok(n)
}

fn fn_count(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    count_impl(host, args, "count")
}

/// sizeof() is count() under its own name, which its errors carry.
fn fn_sizeof(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    count_impl(host, args, "sizeof")
}

fn count_impl(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let mode = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if mode != 0 && mode != 1 {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #2 ($mode) must be either COUNT_NORMAL or COUNT_RECURSIVE"
            ),
        ));
    }
    match deref(&args[0]) {
        Value::Array(a) if mode == 1 => Ok(Value::Int(count_recursive(
            host,
            fname,
            &a,
            &mut Vec::new(),
        )?)),
        Value::Array(a) => Ok(Value::Int(a.len() as i64)),
        // A Countable object: count($obj) calls $obj->count().
        obj @ Value::Object(_) => {
            let callable = Value::Array({
                let mut a = rhp_values::PhpArray::new();
                a.push(obj.clone());
                a.push(Value::str("count"));
                a
            });
            if host.is_callable_value(&callable) {
                let r = host.call_callable(&callable, &[])?;
                return Ok(Value::Int(conv::to_int(&deref(&r))));
            }
            Err(Thrown::type_error(format!(
                "{fname}(): Argument #1 ($value) must be of type Countable|array, {} given",
                argparse::value_name(host, &obj)
            )))
        }
        other => Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($value) must be of type Countable|array, {} given",
            other.type_name_lower()
        ))),
    }
}

fn fn_intdiv(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = conv::to_int(&args[0]);
    let b = conv::to_int(&args[1]);
    if b == 0 {
        return Err(Thrown::error("DivisionByZeroError", "Division by zero"));
    }
    if a == i64::MIN && b == -1 {
        return Err(Thrown::error(
            "ArithmeticError",
            "Division of PHP_INT_MIN by -1 is not an integer",
        ));
    }
    Ok(Value::Int(a / b))
}

fn fn_str_repeat(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let n = conv::to_int(&args[1]).max(0) as usize;
    // zend_string_safe_falloc(len, mult, 0): php refuses past memory_limit.
    if !s.is_empty()
        && n != 0
        && !rhp_values::mem::reserve_safe(s.len(), n, rhp_values::mem::zstr_size(0))
    {
        return Ok(Value::Null);
    }
    Ok(Value::str(s.repeat(n)))
}

fn fn_abs(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match conv::to_number(&deref(&args[0])).0 {
        conv::Num::Int(n) => Ok(match n.checked_abs() {
            Some(v) => Value::Int(v),
            None => Value::Float((n as f64).abs()),
        }),
        conv::Num::Float(f) => Ok(Value::Float(f.abs())),
    }
}

fn fn_max(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    extremum(host, args, true)
}
fn fn_min(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    extremum(host, args, false)
}

/// `(double)l` converts back to `l` exactly (`zend_dval_to_lval_silent`).
fn long_is_exact_double(l: i64) -> bool {
    let d = l as f64;
    (-(2f64.powi(63))..2f64.powi(63)).contains(&d) && d as i64 == l
}

/// php's `min`/`max`: the array form (`zend_hash_minmax`), the two-argument
/// frameless form, and the variadic loop with its long/double fast paths.
fn extremum(host: &mut dyn Host, args: &[Value], want_max: bool) -> Result<Value, Thrown> {
    use std::cmp::Ordering;
    let name = if want_max { "max" } else { "min" };
    let args: Vec<Value> = args.iter().map(deref).collect();
    if args.len() == 1 {
        let Value::Array(a) = &args[0] else {
            return Err(Thrown::type_error(format!(
                "{name}(): Argument #1 ($value) must be of type array, {} given",
                type_name_of(host, &args[0])
            )));
        };
        let mut it = a.iter().map(|(_, v)| deref(v));
        let Some(mut res) = it.next() else {
            return Err(Thrown::error(
                "ValueError",
                format!("{name}(): Argument #1 ($value) must contain at least one element"),
            ));
        };
        for zv in it {
            let c = host_compare(host, &res, &zv)?;
            if (want_max && c == Ordering::Less) || (!want_max && c == Ordering::Greater) {
                res = zv;
            }
        }
        return Ok(res);
    }
    if args.len() == 2 {
        // ZEND_FRAMELESS_FUNCTION(min|max, 2)
        let (lhs, rhs) = (&args[0], &args[1]);
        let pick = |keep_lhs: bool| if keep_lhs { lhs.clone() } else { rhs.clone() };
        let dbl = |l: f64, r: f64| if want_max { l >= r } else { l < r };
        return Ok(match (lhs, rhs) {
            (Value::Int(l), Value::Int(r)) => pick(if want_max { l >= r } else { l < r }),
            (Value::Int(l), Value::Float(r)) if long_is_exact_double(*l) => {
                pick(dbl(*l as f64, *r))
            }
            (Value::Float(l), Value::Float(r)) => pick(dbl(*l, *r)),
            (Value::Float(l), Value::Int(r)) if long_is_exact_double(*r) => {
                pick(dbl(*l, *r as f64))
            }
            _ => {
                let c = host_compare(host, lhs, rhs)?;
                pick(if want_max {
                    c != Ordering::Less
                } else {
                    c == Ordering::Less
                })
            }
        });
    }
    #[derive(PartialEq)]
    enum Mode {
        Long(i64),
        Double(f64),
        Generic,
    }
    let mut best = 0usize;
    let mut mode = match &args[0] {
        Value::Int(l) => Mode::Long(*l),
        Value::Float(d) => Mode::Double(*d),
        _ => Mode::Generic,
    };
    let better_d = |cur: f64, v: f64| if want_max { cur < v } else { cur > v };
    for i in 1..args.len() {
        if let Mode::Long(cur) = mode {
            match &args[i] {
                Value::Int(v) => {
                    if (want_max && cur < *v) || (!want_max && cur > *v) {
                        mode = Mode::Long(*v);
                        best = i;
                    }
                    continue;
                }
                Value::Float(_) if long_is_exact_double(cur) => mode = Mode::Double(cur as f64),
                _ => mode = Mode::Generic,
            }
        }
        if let Mode::Double(cur) = mode {
            match &args[i] {
                Value::Float(v) => {
                    if better_d(cur, *v) {
                        mode = Mode::Double(*v);
                        best = i;
                    }
                    continue;
                }
                Value::Int(v) if long_is_exact_double(*v) => {
                    if better_d(cur, *v as f64) {
                        mode = Mode::Double(*v as f64);
                        best = i;
                    }
                    continue;
                }
                _ => mode = Mode::Generic,
            }
        }
        let c = host_compare(host, &args[i], &args[best])?;
        if (want_max && c == Ordering::Greater) || (!want_max && c == Ordering::Less) {
            best = i;
        }
    }
    Ok(args[best].clone())
}

fn fn_array_sum(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    array_binop(host, args, "array_sum", "Addition", ops::add, 0)
}

fn fn_array_product(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    array_binop(host, args, "array_product", "Multiplication", ops::mul, 1)
}

/// php_array_binop: `op` over the entries, objects by their
/// `cast_object(_IS_NUMBER)`. An entry the operator refuses is skipped
/// with a warning (a resource counts as its id, a non-numeric string as 0).
fn array_binop(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    op_name: &str,
    op: fn(&Value, &Value) -> Result<Value, Thrown>,
    initial: i64,
) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($array) must be of type array"
        )));
    };
    let mut acc = Value::Int(initial);
    for (_, v) in a.iter() {
        let v = deref(v);
        let operand = match &v {
            Value::Object(_) | Value::Closure(_) => {
                match object::cast(host, &v, object::CastTarget::Number)? {
                    Some(n @ (Value::Int(_) | Value::Float(_))) => Some(n),
                    _ => None,
                }
            }
            Value::Array(_) => None,
            Value::Resource(r) => {
                acc = op(&acc, &Value::Int(r.id as i64))?;
                None
            }
            Value::Str(s) => match conv::scan_number(s.as_bytes()) {
                None => {
                    acc = op(&acc, &Value::Int(0))?;
                    None
                }
                Some(_) => {
                    if conv::numeric_string(s.as_bytes()).is_none() {
                        host.emit_diagnostic(E_WARNING, "A non-numeric value encountered")?;
                    }
                    Some(v.clone())
                }
            },
            _ => Some(v.clone()),
        };
        match operand {
            Some(n) => acc = op(&acc, &n)?,
            None => {
                let ty = type_name_of(host, &v);
                warn(
                    host,
                    fname,
                    &format!("{op_name} is not supported on type {ty}"),
                )?
            }
        }
    }
    Ok(acc)
}

fn fn_array_keys(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error(
            "array_keys(): Argument #1 ($array) must be of type array",
        ));
    };
    // Optional search value: keep only keys whose value matches (loose by
    // default, strict `===` when the third argument is true).
    let search = args.get(1).map(deref);
    let strict = args
        .get(2)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    let key_val = |k: &rhp_values::ArrayKey| match k {
        rhp_values::ArrayKey::Int(n) => Value::Int(*n),
        rhp_values::ArrayKey::Str(s) => Value::Str(s.clone()),
    };
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let keep = match &search {
            None => true,
            Some(needle) => {
                if strict {
                    ops::strict_eq(&deref(v), needle)
                } else {
                    host_loose_eq(host, &deref(v), needle)?
                }
            }
        };
        if keep {
            out.push(key_val(k));
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_values(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error(
            "array_values(): Argument #1 ($array) must be of type array",
        ));
    };
    let mut out = PhpArray::new();
    for (_, v) in a.iter() {
        out.push(v.clone());
    }
    Ok(Value::Array(out))
}

fn fn_array_merge(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = PhpArray::new();
    for arg in args {
        let Value::Array(a) = deref(arg) else {
            return Err(Thrown::type_error(
                "array_merge(): Arguments must be of type array",
            ));
        };
        for (k, v) in a.iter() {
            match k {
                // integer keys are renumbered; string keys overwrite
                rhp_values::ArrayKey::Int(_) => out.push(v.clone()),
                rhp_values::ArrayKey::Str(_) => out.insert(k.clone(), v.clone()),
            }
        }
    }
    Ok(Value::Array(out))
}

pub(crate) fn key_to_value(k: &rhp_values::ArrayKey) -> Value {
    match k {
        rhp_values::ArrayKey::Int(n) => Value::Int(*n),
        rhp_values::ArrayKey::Str(s) => Value::Str(s.clone()),
    }
}

fn as_array(v: &Value, who: &str) -> Result<PhpArray, Thrown> {
    match deref(v) {
        Value::Array(a) => Ok(a),
        _ => Err(Thrown::type_error(format!(
            "{who}(): argument must be of type array"
        ))),
    }
}

fn fn_array_map(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cb = &args[0];
    let cb_null = matches!(deref(cb), Value::Null);
    let arrays: Vec<PhpArray> = args[1..]
        .iter()
        .map(|a| as_array(a, "array_map"))
        .collect::<Result<_, _>>()?;
    if arrays.len() == 1 {
        // single array: preserve keys
        let mut out = PhpArray::new();
        for (k, v) in arrays[0].iter() {
            let r = if cb_null {
                v.clone()
            } else {
                host.call_callable(cb, std::slice::from_ref(v))?
            };
            out.insert(k.clone(), r);
        }
        return Ok(Value::Array(out));
    }
    // multiple arrays: zip by position, reindex
    let max_len = arrays.iter().map(|a| a.len()).max().unwrap_or(0);
    let mut out = PhpArray::new();
    for i in 0..max_len {
        let tuple: Vec<Value> = arrays
            .iter()
            .map(|a| {
                a.get_index(i)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null)
            })
            .collect();
        if cb_null {
            let mut t = PhpArray::new();
            for v in tuple {
                t.push(v);
            }
            out.push(Value::Array(t));
        } else {
            out.push(host.call_callable(cb, &tuple)?);
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_filter(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = as_array(&args[0], "array_filter")?;
    let cb = args.get(1).filter(|c| !matches!(deref(c), Value::Null));
    let mode = args.get(2).map(conv::to_int).unwrap_or(0);
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let keep = match cb {
            None => object::is_true(host, v)?,
            Some(cb) => {
                let call_args: Vec<Value> = match mode {
                    2 => vec![key_to_value(k)],            // ARRAY_FILTER_USE_KEY
                    1 => vec![v.clone(), key_to_value(k)], // ARRAY_FILTER_USE_BOTH
                    _ => vec![v.clone()],
                };
                let r = host.call_callable(cb, &call_args)?;
                object::is_true(host, &r)?
            }
        };
        if keep {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

fn fn_array_reduce(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = as_array(&args[0], "array_reduce")?;
    let cb = &args[1];
    let mut acc = args.get(2).map(deref).unwrap_or(Value::Null);
    for (_, v) in a.iter() {
        acc = host.call_callable(cb, &[acc, v.clone()])?;
    }
    Ok(acc)
}

fn fn_call_user_func(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    require_callback(host, "call_user_func", 1, &args[0])?;
    host.call_callable(&args[0], &args[1..])
}

fn fn_call_user_func_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    require_callback(host, "call_user_func_array", 1, &args[0])?;
    let params = as_array(&args[1], "call_user_func_array")?;
    let mut argv: Vec<Value> = Vec::new();
    let mut named: Vec<(rhp_values::PhpStr, Value)> = Vec::new();
    for (k, v) in params.iter() {
        match k {
            rhp_values::ArrayKey::Int(_) if !named.is_empty() => {
                return Err(Thrown::error(
                    "Error",
                    "Cannot use positional argument after named argument",
                ))
            }
            rhp_values::ArrayKey::Int(_) => argv.push(v.clone()),
            rhp_values::ArrayKey::Str(s) => named.push((s.clone(), v.clone())),
        }
    }
    if named.is_empty() {
        return host.call_callable(&args[0], &argv);
    }
    host.call_callable_named(&args[0], &argv, named)
}

fn fn_array_reverse(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[0]) else {
        return Err(Thrown::type_error(
            "array_reverse(): Argument #1 ($array) must be of type array",
        ));
    };
    let preserve = args.get(1).map(conv::to_bool).unwrap_or(false);
    let mut out = PhpArray::new();
    let entries: Vec<_> = a.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (k, v) in entries.into_iter().rev() {
        match k {
            rhp_values::ArrayKey::Int(_) if !preserve => out.push(v),
            _ => out.insert(k, v),
        }
    }
    Ok(Value::Array(out))
}

/// range()'s RANGE_CHECK_{LONG,DOUBLE}_INIT_ARRAY: a ValueError when the
/// result would exceed HT_MAX_SIZE elements. (php prints the lower bound
/// as start and the higher as end, whatever the direction.)
fn range_size_check(
    start: conv::Num,
    end: conv::Num,
    step: f64,
    float: bool,
) -> Result<(), Thrown> {
    const HT_MAX_SIZE: u64 = 0x4000_0000;
    let msg = "The supplied range exceeds the maximum array size by";
    if let (false, conv::Num::Int(a), conv::Num::Int(b)) = (float, start, end) {
        let (lo, hi) = (a.min(b), a.max(b));
        let step = step as u64;
        let size = (hi as u64).wrapping_sub(lo as u64) / step;
        if size >= HT_MAX_SIZE - 1 {
            return Err(Thrown::error(
                "ValueError",
                format!(
                    "{msg} {} elements: start={lo}, end={hi}, step={step}. Calculated size: {size}. Maximum size: {HT_MAX_SIZE}.",
                    size - (HT_MAX_SIZE - 1)
                ),
            ));
        }
        return Ok(());
    }
    let (a, b) = (start.as_f64(), end.as_f64());
    let (lo, hi) = (a.min(b), a.max(b));
    let size = (hi - lo) / step + 1.0;
    if size >= HT_MAX_SIZE as f64 {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{msg} {:.1} elements: start={lo:.1}, end={hi:.1}, step={step:.1}. Max size: {HT_MAX_SIZE}",
                size - HT_MAX_SIZE as f64
            ),
        ));
    }
    Ok(())
}

fn fn_range(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // Numeric ranges only (char ranges deferred).
    let start = conv::to_number(&deref(&args[0])).0;
    let end = conv::to_number(&deref(&args[1])).0;
    let step = args
        .get(2)
        .map(|v| conv::to_number(&deref(v)).0.as_f64().abs())
        .unwrap_or(1.0);
    let step = if step == 0.0 { 1.0 } else { step };
    let mut out = PhpArray::new();
    let use_float = matches!(start, conv::Num::Float(_))
        || matches!(end, conv::Num::Float(_))
        || step.fract() != 0.0;
    let (s, e) = (start.as_f64(), end.as_f64());
    range_size_check(start, end, step, use_float)?;
    // php sizes the result up front, fallibly.
    let count = ((s - e).abs() / step).floor() + 1.0;
    if count.is_finite()
        && !rhp_values::mem::reserve_packed_array(count.min(u32::MAX as f64) as usize)
    {
        return Ok(Value::Null);
    }
    if s <= e {
        let mut x = s;
        while x <= e + 1e-9 {
            out.push(if use_float {
                Value::Float(x)
            } else {
                Value::Int(x as i64)
            });
            x += step;
        }
    } else {
        let mut x = s;
        while x >= e - 1e-9 {
            out.push(if use_float {
                Value::Float(x)
            } else {
                Value::Int(x as i64)
            });
            x -= step;
        }
    }
    Ok(Value::Array(out))
}

fn fn_in_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let needle = deref(&args[0]);
    let Value::Array(a) = deref(&args[1]) else {
        return Err(Thrown::type_error(
            "in_array(): Argument #2 ($haystack) must be of type array",
        ));
    };
    let strict = args.get(2).map(conv::to_bool).unwrap_or(false);
    for (_, v) in a.iter() {
        let hit = if strict {
            ops::strict_eq(&needle, v)
        } else {
            host_loose_eq(host, &needle, v)?
        };
        if hit {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn fn_array_key_exists(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Array(a) = deref(&args[1]) else {
        return Err(Thrown::type_error(
            "array_key_exists(): Argument #2 ($array) must be of type array",
        ));
    };
    let Some(key) = rhp_values::ArrayKey::from_value(&args[0]) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(a.get(&key).is_some()))
}

fn fn_implode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // implode($array) or implode($glue, $array)
    let (glue, arr) = match (deref(&args[0]), args.get(1).map(deref)) {
        (Value::Array(a), _) => (Vec::new(), a),
        (g, Some(Value::Array(a))) => (conv::to_string_bytes(&g), a),
        _ => {
            return Err(Thrown::type_error(
                "implode(): Argument must be of type array",
            ))
        }
    };
    let parts: Vec<std::borrow::Cow<[u8]>> = arr
        .iter()
        .map(|(_, v)| match v {
            Value::Str(s) => std::borrow::Cow::Borrowed(s.as_bytes()),
            v => std::borrow::Cow::Owned(conv::to_string_bytes(v)),
        })
        .collect();
    let len: usize = parts.iter().map(|p| p.len()).sum();
    // zend_string_safe_falloc(n - 1, glue_len, len): the glue repeated per
    // element can dwarf the input, so php checks the limit first.
    if parts.len() > 1
        && !rhp_values::mem::reserve_safe(
            parts.len() - 1,
            glue.len(),
            rhp_values::mem::zstr_size(len),
        )
    {
        return Ok(Value::Null);
    }
    let mut out = Vec::with_capacity(len + glue.len() * parts.len().saturating_sub(1));
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(&glue);
        }
        out.extend_from_slice(p);
    }
    Ok(Value::str(out))
}

fn fn_explode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let delim = conv::to_string_bytes(&deref(&args[0]));
    let s = conv::to_string_bytes(&deref(&args[1]));
    if delim.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "explode(): Argument #1 ($separator) cannot be empty",
        ));
    }
    // All pieces, in order.
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut start = 0;
    while let Some(pos) = find_sub(&s[start..], &delim) {
        parts.push(s[start..start + pos].to_vec());
        start += pos + delim.len();
    }
    parts.push(s[start..].to_vec());

    let limit = args.get(2).map(|v| conv::to_int(&deref(v)));
    let n = parts.len();
    let pieces: Vec<Vec<u8>> = match limit {
        None => parts,
        // Positive: at most `limit` pieces; the last keeps the remainder.
        Some(l) if l > 0 => {
            let l = l as usize;
            if l >= n {
                parts
            } else {
                let mut out: Vec<Vec<u8>> = parts[..l - 1].to_vec();
                out.push(parts[l - 1..].join(&delim[..]));
                out
            }
        }
        // Zero is treated as 1: the whole string as one piece.
        Some(0) => vec![s.clone()],
        // Negative: drop the last `|limit|` pieces.
        Some(l) => {
            let drop = (-l) as usize;
            if drop >= n {
                Vec::new()
            } else {
                parts[..n - drop].to_vec()
            }
        }
    };
    let mut arr = PhpArray::new();
    for p in pieces {
        arr.push(Value::str(p));
    }
    Ok(Value::Array(arr))
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

fn fn_strtoupper(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut b = conv::to_string_bytes(&deref(&args[0]));
    b.make_ascii_uppercase();
    Ok(Value::str(b))
}

fn fn_strtolower(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut b = conv::to_string_bytes(&deref(&args[0]));
    b.make_ascii_lowercase();
    Ok(Value::str(b))
}

fn fn_str_contains(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let haystack = conv::to_string_bytes(&deref(&args[0]));
    let needle = conv::to_string_bytes(&deref(&args[1]));
    Ok(Value::Bool(
        needle.is_empty() || find_sub(&haystack, &needle).is_some(),
    ))
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
            "Calling get_class() without arguments is deprecated",
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
        other => Err(Thrown::type_error(format!(
            "get_class(): Argument #1 ($object) must be of type object, {} given",
            argparse::value_name(host, &other)
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
            // Subtype but NOT the same class (is_subclass_of is exclusive).
            let same = norm_class(&n) == norm_class(&target);
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
                "Calling get_parent_class() without arguments is deprecated",
            )?;
            Ok(host
                .caller_scope()
                .and_then(|c| host.class_parent_name(c))
                .map(Value::str)
                .unwrap_or(Value::Bool(false)))
        }
        Some(other) => Err(Thrown::type_error(format!(
            "get_parent_class(): Argument #1 ($object_or_class) must be an object or a valid class name, {} given",
            argparse::value_name(host, &other)
        ))),
    }
}

/// Resolve the class id named by an argument: an object yields its own class,
/// a string resolves via the host's class table.
fn class_id_of(host: &mut dyn Host, v: &Value) -> Result<Option<u32>, Thrown> {
    Ok(match deref(v) {
        Value::Object(h) => Some(h.borrow().class),
        Value::Str(s) => host.resolve_class_autoload(s.as_bytes())?,
        _ => None,
    })
}

fn fn_method_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = conv::to_string_bytes(&deref(&args[1]));
    let found = class_id_of(host, &args[0])?
        .map(|cid| host.class_has_method(cid, &method))
        .unwrap_or(false);
    Ok(Value::Bool(found))
}

fn fn_property_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
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
                || host.object_prop_exists(&Value::Object(h.clone()), &prop)
        }
        Value::Str(s) => host
            .resolve_class_autoload(s.as_bytes())?
            .map(|cid| host.class_has_property(cid, &prop))
            .unwrap_or(false),
        _ => false,
    };
    Ok(Value::Bool(found))
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

fn fn_get_object_vars(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => {
            if let Some(a) = object::properties_for(host, &h, object::PropPurpose::GetObjectVars)? {
                return Ok(Value::Array(a));
            }
            if let Some(v) = host.object_vars(&args[0]) {
                return Ok(v);
            }
            let mut arr = PhpArray::new();
            for (k, v) in h.borrow().properties.iter() {
                arr.insert(rhp_values::ArrayKey::Str(k.clone()), v.clone());
            }
            Ok(Value::Array(arr))
        }
        _ => Err(Thrown::type_error(
            "get_object_vars(): Argument #1 must be of type object",
        )),
    }
}

/// `get_mangled_object_vars()`: the property table under php's mangled
/// names, ignoring scope and the class's property hooks.
fn fn_get_mangled_object_vars(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match deref(&args[0]) {
        Value::Object(h) => Ok(Value::Array(host.object_props_mangled(&h))),
        v => Err(Thrown::type_error(format!(
            "get_mangled_object_vars(): Argument #1 ($object) must be of type object, {} given",
            v.type_name()
        ))),
    }
}

fn fn_ord(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let b = conv::to_string_bytes(&deref(&args[0]));
    Ok(Value::Int(b.first().copied().unwrap_or(0) as i64))
}

fn fn_chr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = conv::to_int(&deref(&args[0])).rem_euclid(256) as u8;
    Ok(Value::str(vec![n]))
}

// small helper on Value for error messages
trait TypeNameLower {
    fn type_name_lower(&self) -> &'static str;
}
impl TypeNameLower for Value {
    fn type_name_lower(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) | Value::Closure(_) => "object",
            Value::Resource(_) => "resource",
            Value::Ref(r) => r.borrow().type_name_lower(),
        }
    }
}

/// `zend_zval_value_name`: the class name for objects, `true`/`false` for
/// bools, else the type name.
pub fn type_name_of(host: &dyn Host, v: &Value) -> String {
    match deref(v) {
        Value::Bool(true) => "true".into(),
        Value::Bool(false) => "false".into(),
        Value::Closure(_) => "Closure".into(),
        o @ Value::Object(_) => host
            .object_class_name(&o)
            .map(|n| String::from_utf8_lossy(&n).into_owned())
            .unwrap_or_else(|| "object".into()),
        other => other.type_name_lower().into(),
    }
}

/// A host that records diagnostics the way php displays them
/// (`Warning: f(): msg`), for unit-testing the sites that emit them.
#[cfg(test)]
pub(crate) mod diag_test {
    use super::{Host, Thrown, Value};

    #[derive(Default)]
    pub struct DiagHost {
        pub out: Vec<u8>,
        pub diags: Vec<String>,
    }

    impl Host for DiagHost {
        fn write_out(&mut self, bytes: &[u8]) {
            self.out.extend_from_slice(bytes);
        }
        fn raise_diagnostic(&mut self, level: i64, message: &str) {
            let kind = match level {
                super::E_WARNING => "Warning",
                super::E_NOTICE => "Notice",
                super::E_DEPRECATED => "Deprecated",
                _ => "Unknown",
            };
            self.diags.push(format!("{kind}: {message}"));
        }
    }

    /// Call `f`, returning its result and the diagnostics it emitted.
    pub fn run(f: crate::NativeFn, args: &[Value]) -> (Result<Value, Thrown>, Vec<String>) {
        let mut h = DiagHost::default();
        let r = f(&mut h, args);
        (r, h.diags)
    }
}
