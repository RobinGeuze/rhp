//! The interpreter surface builtins call back into: the [`Host`] trait,
//! which the VM implements, and the types its methods take.

use rhp_values::ops::deref;
use rhp_values::{PhpArray, Value};

use crate::main::streams::{ops, plain_wrapper};
use crate::main::{ini, rfc1867, sapi, streams};
use crate::zend::object_handlers;
use crate::{lookup, StatInfo, StreamWrapper, Thrown};

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
    /// The TLS layer's `crypto` entry (an active session).
    pub crypto: Option<Value>,
    /// The wrapper's data (the http wrapper's response headers).
    pub wrapper_data: Option<Value>,
    /// What a data: (RFC 2397) stream's META_DATA_API adds (its media type
    /// and parameters); php then leaves out timed_out, blocked and eof.
    pub rfc2397: Option<Value>,
}

/// `zend_compare` from inside a builtin, with the host's object semantics.
pub fn host_compare(
    host: &mut dyn Host,
    a: &Value,
    b: &Value,
) -> Result<std::cmp::Ordering, Thrown> {
    match host.compare_cx() {
        Some(cx) => rhp_values::ops::compare(a, b, cx),
        None => rhp_values::ops::compare(a, b, &mut rhp_values::ops::NoCx),
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

/// The fetch a magic accessor serves ([`Host::magic_fetch`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MagicFetch {
    /// A read (`BP_VAR_R`), or anything else.
    #[default]
    Read,
    /// A write below the property or element (`BP_VAR_W`/`BP_VAR_RW`/
    /// `BP_VAR_UNSET`: `$o->p->q = …`, `$o->p[] = …`, `$o[k][] = …`,
    /// `&$o->p`, `unset($o->p->q)`).
    Write,
    /// `empty()`'s `__isset` or `offsetExists` (`check_empty`).
    Empty,
}

/// The interpreter surface a builtin may use. Implemented by the VM, and
/// for the unit tests by `test_support::TestHost`.
pub trait Host {
    /// Append bytes to the single output sink (stdout / output buffer).
    fn write_out(&mut self, bytes: &[u8]);

    /// A userland output-control call (main/output.c's `ob_*`, `flush`),
    /// arguments already parsed; the host owns the handler stack and raises
    /// php's notices. Default: unsupported (hosts without output buffering).
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
    fn compare_cx(&mut self) -> Option<&mut dyn rhp_values::ops::CompareCx> {
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

    /// How the engine fetches the property or element whose magic accessor
    /// (`__get`, `__isset`, `offsetGet`, `offsetExists`) is running, asked
    /// from that accessor's own frame (anything it calls sees a read):
    /// SimpleXML's handlers behave as php's `read_property` /
    /// `has_property` do for that fetch. Default: a plain read.
    fn magic_fetch(&self) -> MagicFetch {
        MagicFetch::Read
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
        ini::ini_value(name).map(Option::unwrap_or_default)
    }
    /// Store `value` for a known directive (no validation; engine-internal
    /// writes such as `error_reporting()`), returning the old value.
    fn ini_set(&mut self, name: &[u8], value: Vec<u8>) -> Option<Vec<u8>> {
        let old = self.ini_get(name)?;
        ini::ini_store(name, value);
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
    /// ([`object_handlers::registered`]) or an ancestor's. Default: none.
    fn object_handlers(&self, _class_id: u32) -> Option<&'static object_handlers::ObjectHandlers> {
        None
    }

    /// php's `zend_std_cast_object_tostring` for `obj`: `__toString()` for a
    /// string, true for bool, FAILURE (`Ok(None)`) otherwise; what a
    /// `cast_object` handler defers to. Default: no `__toString()` call.
    fn std_cast_object(
        &mut self,
        _obj: &rhp_values::ObjectHandle,
        target: object_handlers::CastTarget,
    ) -> Result<Option<Value>, Thrown> {
        Ok(object_handlers::std_cast_non_string(target))
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

    /// Read a named property off an object value (an enum case's `name` or
    /// `value`, an exception's `message`). Default: none.
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
    // registry still read local files; the VM overrides these to dispatch
    // through its per-run wrapper registry.

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

    /// Response header lines (status line first) of every hop of the last
    /// http(s) request; `None` before one sent a request.
    fn set_http_response_headers(&mut self, _headers: Option<Vec<Vec<u8>>>) {}
    fn http_response_headers(&self) -> Option<Vec<Vec<u8>>> {
        None
    }
    /// The `notification` callback of a stream context (or of a stream's
    /// context).
    fn stream_context_notifier(&self, _ctx: &Value) -> Option<Value> {
        None
    }
    /// Set (or drop) a context's `notification` callback.
    fn set_stream_context_notifier(&mut self, _ctx: &Value, _cb: Option<Value>) {}
    /// php's `decode_context_param`: the context resource itself, or a
    /// stream's context (a new one attached when it has none). `None` for
    /// any other value.
    fn resolve_stream_context(&mut self, _res: &Value) -> Option<Value> {
        None
    }

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
    /// [`Host::stream_open`] with the call's stream context (null: the
    /// default one), which a wrapper's own opener reads.
    fn stream_open_ctx(&mut self, path: &[u8], mode: &[u8], _ctx: &Value) -> Option<Value> {
        self.stream_open(path, mode)
    }
    /// A `stream` resource over an extension's stream ops.
    fn stream_from_ops(&mut self, _s: ops::OpsStream) -> Option<Value> {
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
        _info: plain_wrapper::SockInfo,
    ) -> Option<Value> {
        None
    }
    /// The socket state of a socket stream (`None` for any other stream).
    fn stream_sock_info(&self, _res: &Value) -> Option<plain_wrapper::SockInfo> {
        None
    }
    /// The descriptor stream behind a stream resource (sockets, pipes).
    fn stream_fd_mut(&mut self, _res: &Value) -> Option<&mut plain_wrapper::FdStream> {
        None
    }
    /// Register a descriptor stream as a stream resource whose context is
    /// `ctx` (php's `stream->ctx`). Default: unsupported.
    fn stream_insert_fd(
        &mut self,
        _stream: plain_wrapper::FdStream,
        _ctx: Option<Value>,
    ) -> Option<Value> {
        None
    }
    /// The context a stream was opened with (`PHP_STREAM_CONTEXT`).
    fn stream_context_of(&self, _res: &Value) -> Option<Value> {
        None
    }
    /// `zend_set_local_var_str`: assign a variable of the running user
    /// code's scope (a compiled variable, or the symbol table where there
    /// is one). False when the scope has no such variable.
    fn set_local_var(&mut self, _name: &[u8], _value: Value) -> bool {
        false
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
    /// `php_error_docref(NULL, level, msg)` raised outside a builtin of its
    /// own (an object or resource freed): prefixed with php's active
    /// function (the running builtin, else the innermost user function,
    /// `main` at top level, else `PHP Request Shutdown` in Unknown on line 0).
    fn docref_diagnostic(&mut self, level: i64, msg: &str) -> Result<(), Thrown> {
        self.emit_diagnostic(level, &format!("Unknown: {msg}"))
    }
}
