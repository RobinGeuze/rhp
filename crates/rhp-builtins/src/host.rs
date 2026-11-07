//! The interpreter surface builtins call back into: the [`Host`] trait,
//! which the VM implements, and the types its methods take.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_values::{ObjectHandle, PhpArray, Value};

/// See [`Host::unserialize_prop`].
/// A variable of the running user code's scope, looked up by name.
pub enum ScopeVar {
    /// No such variable.
    Missing,
    /// A compiled variable (or symbol-table entry) that is undefined.
    Undef,
    /// Its value, dereferenced.
    Set(Value),
}

pub enum UnserProp {
    /// A declared property's storage key; `typed`: it has a type.
    Declared {
        storage: rhp_values::PhpStr,
        typed: bool,
    },
    /// A dynamic property's key.
    Dynamic(rhp_values::PhpStr),
    /// Unserialization fails.
    Fail,
}

use crate::main::streams::{ops, plain_wrapper};
use crate::main::{ini, rfc1867, sapi};
use crate::zend::object_handlers;
use crate::{StatInfo, StreamWrapper, Thrown};

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
    /// php_output_start_internal: an extension's handler `func` under
    /// `name`, with the standard flags.
    StartInternal {
        name: &'static str,
        func: crate::OutputHandlerFn,
        chunk_size: usize,
    },
    /// php_output_start_default: a buffer an internal function collects its
    /// output in (`Bool(false)`: not started; a fatal error may be pending).
    StartDefault,
    /// php_output_discard: drop the active buffer and its contents.
    Discard,
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
    /// Whether an output handler is running (php_output_handler_hook's
    /// GET_FLAGS succeeds): `bool`.
    HandlerRunning,
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
/// for the unit tests by `test_support::TestHost`. Every method is required
/// except the three whose bodies are shared: [`Host::emit_diagnostic`],
/// [`Host::ini_get`] and [`Host::ini_set`].
pub trait Host {
    /// Append bytes to the single output sink (stdout / output buffer).
    fn write_out(&mut self, bytes: &[u8]);

    /// A userland output-control call (main/output.c's `ob_*`, `flush`),
    /// arguments already parsed; the host owns the handler stack and raises
    /// php's notices.
    fn output_control(&mut self, call: ObCall) -> Result<Value, Thrown>;

    /// Raise a php diagnostic (`E_WARNING` = 2, `E_NOTICE` = 8, …) at the
    /// current line: php's `zend_error` — the user error handler
    /// (`set_error_handler`) first, else `error_get_last` + display per
    /// `error_reporting` / `@` / `display_errors` / `log_errors`. `msg` already
    /// carries any `func(): ` prefix. It is bytes, as php's messages are: a
    /// file name or argument in it may be any bytes. An exception thrown by a
    /// user handler is held by the host and surfaces when the running builtin
    /// returns (php's `EG(exception)`); see [`Host::emit_diagnostic`] to
    /// receive it at the call site instead.
    fn raise_diagnostic(&mut self, level: i64, msg: &[u8]);

    /// Take the exception a user error handler threw during the last
    /// [`Host::raise_diagnostic`], if any.
    fn take_pending_throw(&mut self) -> Option<Thrown>;

    /// Whether the running builtin was called as a whole expression
    /// statement, its result unused (php's `!USED_RET()`).
    fn result_unused(&self) -> bool {
        false
    }

    /// The last raised diagnostic for `error_get_last()`: (type, message, file,
    /// line).
    fn last_error(&self) -> Option<(i64, Vec<u8>, Vec<u8>, i64)>;

    /// `error_clear_last()`.
    fn clear_last_error(&mut self);

    /// The backtrace of the last fatal error (`fatal_error_backtraces`),
    /// `error_get_last()['trace']`.
    fn last_error_trace(&self) -> Option<Value> {
        None
    }

    /// The engine's comparison context (`__toString`, internal compare
    /// handlers, notices) for loose comparisons inside builtins.
    fn compare_cx(&mut self) -> Option<&mut dyn rhp_values::ops::CompareCx>;

    /// A class's name by id (for `get_class` etc.).
    fn class_name(&self, class_id: u32) -> Option<Vec<u8>>;

    /// A class's parent's name by id (for `get_parent_class`).
    fn class_parent_name(&self, class_id: u32) -> Option<Vec<u8>>;

    /// php's `zend_fetch_debug_backtrace` for `debug_backtrace()` (innermost
    /// first, the calling builtin's own frame skipped).
    fn backtrace(&mut self, provide_object: bool, ignore_args: bool) -> Option<Value>;

    /// Push a user exception handler (`set_exception_handler`; null clears),
    /// returning the previous one (or null).
    fn set_exception_handler(&mut self, handler: Value) -> Value;
    /// Pop back to the previous exception handler.
    fn restore_exception_handler(&mut self);

    /// Push a `set_error_handler` callback (null disables) handling the
    /// error types in `mask`; returns the previous handler or null.
    fn set_error_handler(&mut self, handler: Value, mask: i64) -> Value;
    /// `restore_error_handler`: pop back to the previous error handler.
    fn restore_error_handler(&mut self);

    /// Whether the code calling the running builtin declared `strict_types=1`
    /// (zpp then coerces no scalars).
    fn caller_strict_types(&self) -> bool;

    /// Write `v` through the by-reference argument `r` (php's
    /// ZEND_TRY_ASSIGN_REF_*): a reference bound to typed properties checks
    /// and coerces it, else the TypeError and nothing is written.
    fn assign_ref(&mut self, r: &rhp_values::PhpRef, v: Value) -> Result<(), Thrown> {
        *r.borrow_mut() = v;
        Ok(())
    }

    /// Whether `r` is bound to typed properties (ZEND_REF_HAS_TYPE_SOURCES).
    fn ref_is_typed(&self, r: &rhp_values::PhpRef) -> bool {
        let _ = r;
        false
    }

    /// Make the typed property `key` of `obj`, whose slot holds `r`, a type
    /// source of that reference (ZEND_REF_ADD_TYPE_SOURCE).
    fn add_ref_type_source(
        &mut self,
        r: &rhp_values::PhpRef,
        obj: &ObjectHandle,
        key: &rhp_values::PhpStr,
    ) {
        let _ = (r, obj, key);
    }

    /// How the engine fetches the property or element whose magic accessor
    /// (`__get`, `__isset`, `offsetGet`, `offsetExists`) is running, asked from
    /// that accessor's own frame (anything it calls sees a read): SimpleXML's
    /// handlers behave as php's `read_property` / `has_property` do for that
    /// fetch.
    fn magic_fetch(&self) -> MagicFetch;

    /// php's `ZEND_ARG_USES_STRICT_TYPES()` as an object handler the engine
    /// runs (`do_operation`, `compare`) sees it: the strictness of the caller
    /// of php's current frame, which is the builtin running the handler if
    /// there is one, else the code running the operator. Weak when that caller
    /// is a builtin or there is none (the main script).
    fn arg_uses_strict_types(&self) -> bool;

    /// gc_collect_cycles(): run the cycle collector, returning how many
    /// values it freed.
    fn gc_collect_cycles(&mut self) -> Result<i64, Thrown> {
        Ok(0)
    }

    /// `thrown` raised while `previous` is pending: php makes `previous` the
    /// new exception's previous one. An `exit` in `previous` wins.
    fn chain_thrown(&mut self, thrown: Thrown, previous: Thrown) -> Thrown;

    /// In the traces of `thrown` and its previous exceptions, put `with` in
    /// place of every argument that is the array stored at `data_ptr`
    /// (`PhpArray::data_ptr`). php's traces share the argument's table, so
    /// a function that keeps changing it in place changes what they show.
    fn trace_replace_array(&mut self, thrown: &Thrown, data_ptr: usize, with: &Value) {
        let _ = (thrown, data_ptr, with);
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
    /// request shutdown.
    fn register_shutdown_function(
        &mut self,
        callback: Value,
        args: Vec<Value>,
    ) -> Result<(), Thrown>;

    /// Whether a resource has been closed (`fclose`d stream): `gettype` /
    /// `get_debug_type` then report `resource (closed)`.
    fn resource_closed(&self, res: &Value) -> bool;

    /// The live resources in id order (`get_resources`).
    fn live_resources(&mut self) -> Vec<Value>;

    /// Whether a class id names an enum (for `var_dump` rendering).
    fn is_enum_class(&self, class_id: u32) -> bool;

    /// Whether objects of a class are `ZEND_ACC_NOT_SERIALIZABLE` (the
    /// Reflection objects, WeakReference, …).
    fn not_serializable(&self, class_id: u32) -> bool;

    /// The object handlers of a class: its own registration
    /// ([`object_handlers::registered`]) or an ancestor's.
    fn object_handlers(&self, class_id: u32) -> Option<&'static object_handlers::ObjectHandlers>;

    /// php's zend_hooked_object_build_properties: the table a class with
    /// property hooks shows for `purpose` (`get_object_vars`, `json_encode`,
    /// `var_export`), its hooked properties read through their get hooks;
    /// `None` for other classes and purposes.
    fn hooked_properties_for(
        &mut self,
        _obj: &ObjectHandle,
        _purpose: object_handlers::PropPurpose,
    ) -> Result<Option<PhpArray>, Thrown> {
        Ok(None)
    }

    /// php's get_properties on a lazy object: initialize it; the object
    /// whose properties to read (an initialized proxy's instance).
    fn lazy_props_object(&mut self, obj: &ObjectHandle) -> Result<ObjectHandle, Thrown> {
        Ok(obj.clone())
    }

    /// php's `zend_std_cast_object_tostring` for `obj`: `__toString()` for a
    /// string, true for bool, FAILURE (`Ok(None)`) otherwise; what a
    /// `cast_object` handler defers to.
    fn std_cast_object(
        &mut self,
        obj: &rhp_values::ObjectHandle,
        target: object_handlers::CastTarget,
    ) -> Result<Option<Value>, Thrown>;

    /// Whether the property slot `key` (storage key) of an object of class
    /// `class_id` is engine-internal state of a built-in class (the prelude's
    /// `__`-prefixed slots): php keeps such state outside the property table,
    /// so every property-table view (`var_dump`, `(array)`, `serialize`,
    /// `json_encode`, `get_object_vars`, `foreach`) skips it.
    fn is_internal_slot(&self, class_id: u32, key: &rhp_values::PhpStr) -> bool;

    /// The uninitialized typed properties `var_dump` lists as
    /// `uninitialized(type)`: (mangled name, type, how many of the object's
    /// visible properties precede it).
    fn uninit_props(
        &self,
        obj: &rhp_values::ObjectHandle,
    ) -> Vec<(rhp_values::PhpStr, String, usize)>;

    /// Define a runtime constant (`define`). False when the name is taken
    /// (the caller warns, as zend_register_constant does).
    fn define_const(&mut self, name: &[u8], value: Value) -> bool;

    /// Look up a constant by name (`constant`), or None if undefined.
    fn get_const(&self, name: &[u8]) -> Option<Value>;

    /// Raise the deprecation of fetching constant `name` (CONST_DEPRECATED),
    /// when it has one.
    fn const_fetch_deprecation(&mut self, _name: &[u8]) -> Result<(), Thrown> {
        Ok(())
    }

    /// A class constant's value (`defined`/`constant` with `Class::CONST`, incl.
    /// enum cases), or None if the class or constant does not exist.
    fn class_constant(&self, class: &[u8], name: &[u8]) -> Option<Value>;

    /// zend_get_class_constant_ex for `constant()` / `defined()`: `class`
    /// may be `self`/`parent`/`static` (the caller's), is autoloaded, and
    /// the constant must be visible from the caller's scope; its pending
    /// initializer is evaluated. With `silent`, a missing class or
    /// constant or an inaccessible one is `Ok(None)`; otherwise php's
    /// Error.
    fn class_constant_eval(
        &mut self,
        class: &[u8],
        name: &[u8],
        silent: bool,
    ) -> Result<Option<Value>, Thrown>;

    /// Run the destructors of objects nothing references any more, now: php
    /// destructs when the refcount reaches zero, rhp before the next statement.
    fn run_released_destructors(&mut self) -> Result<(), Thrown>;

    /// Invoke a PHP callable (for `array_map`/`array_filter`/…).
    fn call_callable(&mut self, callable: &Value, args: &[Value]) -> Result<Value, Thrown>;

    /// Invoke a PHP callable with named arguments too (`call_user_func_array`
    /// with string keys).
    fn call_callable_named(
        &mut self,
        callable: &Value,
        args: &[Value],
        named: Vec<(rhp_values::PhpStr, Value)>,
    ) -> Result<Value, Thrown>;

    /// [`Host::call_callable`] forwarding the caller's called scope to a
    /// static method (`forward_static_call`).
    fn call_callable_forward(&mut self, callable: &Value, args: &[Value]) -> Result<Value, Thrown>;

    /// Whether a value is callable (`is_callable`): closures, function names,
    /// `[obj_or_class, method]` pairs, and `__invoke` objects.
    fn is_callable_value(&self, callable: &Value) -> bool;

    /// Why a value is not a valid callback (zend_is_callable_error's message,
    /// e.g. `function "f" not found or invalid function name`), or None when it
    /// is callable. A String: every caller puts it in an exception message.
    fn callable_error(&self, callable: &Value) -> Option<String>;

    /// Wrap a callable as a Closure (`Closure::fromCallable`) that resolves it
    /// in the calling scope.
    fn wrap_callable(&self, callable: Value) -> Value;

    /// [`Host::callable_error`] after autoloading a class the callable names
    /// (zend_is_callable looks classes up with autoloading; a loader's
    /// exception propagates).
    fn check_callable(&mut self, callable: &Value) -> Result<Option<String>, Thrown>;

    /// Resolve a class-like name to its opaque class id (for `class_exists`,
    /// `method_exists`/`property_exists` on a class-name string).
    fn resolve_class(&self, name: &[u8]) -> Option<u32>;

    /// Like `resolve_class` but invokes registered autoloaders on a miss (for
    /// `class_exists($name)` with `$autoload = true`, php's default). An
    /// exception thrown by an autoloader propagates.
    fn resolve_class_autoload(&mut self, name: &[u8]) -> Result<Option<u32>, Thrown>;

    /// `class_alias`: register `alias` as another name for class `class_id`.
    /// Returns false if the alias name is already taken; a reserved name is
    /// a fatal error (zend_register_class_alias_ex).
    fn class_alias(&mut self, class_id: u32, alias: &[u8]) -> Result<bool, Thrown>;

    /// A class's relations for `class_parents` / `class_implements` /
    /// `class_uses`: (parent chain nearest-first, interfaces in php order,
    /// directly used traits). Default: none.
    #[allow(clippy::type_complexity)]
    fn class_relations(&self, class_id: u32) -> Option<(Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>)>;

    /// The class scope of the code calling the builtin
    /// (`zend_get_executed_scope`), if any.
    fn caller_scope(&self) -> Option<u32>;

    /// clone(): copy `obj` (running `__clone`), then write `with` to the
    /// copy from the calling scope.
    fn clone_object(&mut self, obj: &Value, with: &PhpArray) -> Result<Value, Thrown> {
        let _ = (obj, with);
        Err(Thrown::error("Error", "clone() needs the engine"))
    }

    /// The late-static-binding class of the code calling the builtin
    /// (`zend_get_called_scope`), if any.
    fn caller_called_scope(&self) -> Option<u32>;

    /// An object's properties accessible from the calling code's class scope
    /// (`zend_check_property_access`), in property-table order — for
    /// `http_build_query` on objects.
    fn object_scoped_props(&self, obj: &Value) -> Vec<(rhp_values::PhpStr, Value)>;

    /// User-defined constants (`define`/`const`) in definition order, for
    /// `get_defined_constants`.
    fn user_constants(&self) -> Vec<(Vec<u8>, Value)>;

    /// The request's header store (`SG(sapi_headers)`, `headers_sent`):
    /// `header()` and friends operate on it.
    fn response(&mut self) -> Option<&mut sapi::Response>;

    /// One `error_log()` / log_errors line for the SAPI's logger (php's
    /// `sapi_module.log_message`; no trailing newline).
    fn log_message(&mut self, msg: &[u8]);

    /// The request headers (`getallheaders()`), in order.
    fn request_headers(&self) -> Vec<(Vec<u8>, Vec<u8>)>;

    /// The request a SAPI handed the engine (`SG(request_info)`), if any.
    fn request_info(&self) -> Option<&sapi::RequestInfo>;

    /// The request body's read state and uploads (`request_parse_body`,
    /// `is_uploaded_file`, `move_uploaded_file`).
    fn body_state(&mut self) -> Option<&mut rfc1867::BodyState>;

    /// The [`ClassKind`] of a class id (class / interface / trait / enum), used
    /// by the `*_exists` family.
    fn class_kind(&self, class_id: u32) -> Option<rhp_values::ClassKind>;

    /// Whether a class id declares a method (case-insensitive, flattened over
    /// the inheritance chain).
    fn class_has_method(&self, class_id: u32, method: &[u8]) -> bool;

    /// Whether a class's method `method` is private and declared by
    /// another class (a parent's private method, which `method_exists()`
    /// on a class name does not count).
    fn class_method_private_inherited(&self, class_id: u32, method: &[u8]) -> bool;

    /// Whether a class id declares a property — instance or static
    /// (case-sensitive, flattened over the inheritance chain).
    fn class_has_property(&self, class_id: u32, prop: &[u8]) -> bool;

    /// Whether a function exists (`function_exists`): a user function or a
    /// builtin.
    fn function_exists(&self, name: &[u8]) -> bool;

    /// Whether `name` is an internal function under the running SAPI: a
    /// builtin or a prelude function, not a user function.
    fn internal_function_exists(&self, name: &[u8]) -> bool;

    /// `Closure::bind($closure, $newThis, $newScope)`: a new Closure with
    /// `$this` and the scope rebound, or null (with php's warning) for a
    /// binding php refuses.
    fn closure_bind(
        &mut self,
        closure: &Value,
        new_this: &Value,
        scope: Option<&Value>,
    ) -> Result<Value, Thrown>;

    /// Register an `spl_autoload_register` callable (optionally prepending).
    fn register_autoloader(&mut self, callback: Value, prepend: bool);

    /// The registered autoloader callables (`spl_autoload_functions`).
    fn autoload_functions(&self) -> Vec<Value>;

    /// Trigger autoloading for a class name (`spl_autoload_call`).
    fn autoload_call(&mut self, name: &[u8]) -> Result<(), Thrown>;

    /// Instantiate a named class, then set the given properties (no
    /// constructor is run), as object-returning builtins do (`gmp_*`
    /// returning `GMP`, `simplexml_load_string`). An internal class gets its
    /// evaluated property defaults; a user class (a SimpleXMLElement
    /// subclass) is created without evaluating its defaults and without the
    /// slots it declares. `None` if the host has no such class or no object
    /// model, or when evaluating an internal class's defaults threw (the
    /// host throws that once the builtin returns).
    fn instantiate(&mut self, class_name: &[u8], props: Vec<(Vec<u8>, Value)>) -> Option<Value>;

    /// Read a named property off an object value (an enum case's `name` or
    /// `value`, an exception's `message`).
    fn object_prop(&self, obj: &Value, name: &[u8]) -> Option<Value>;

    /// php's visibility-mangled name for an object's property storage key (as
    /// `(array)`, `serialize` and the dumpers see it): public/dynamic `name`,
    /// protected `"\0*\0name"`, private `"\0Class\0name"`. Hosts store private
    /// properties under their mangled key already.
    fn mangled_prop_name(&self, class_id: u32, key: &rhp_values::PhpStr) -> rhp_values::PhpStr;

    /// process_nested_object_data's lookup of property `key` read by
    /// `unserialize` for `obj`: a declared property (also through a plain or
    /// `*` / own-class mangled name), else a dynamic one (creating it raises
    /// php's deprecation, or throws for classes without dynamic properties).
    /// `UnserProp::Fail` after a malformed mangled name or the virtual
    /// property warning.
    fn unserialize_prop(&mut self, obj: &ObjectHandle, key: &[u8]) -> Result<UnserProp, Thrown> {
        let _ = obj;
        Ok(UnserProp::Dynamic(rhp_values::PhpStr::from_bytes(
            key.to_vec(),
        )))
    }

    /// zend_verify_prop_assignable_by_ref (strict) for the declared property
    /// `storage` of `obj`. `source`: the value is a reference already bound
    /// to that typed property, so it must match exactly. Returns the value
    /// converted (int to float), if it changed.
    fn unserialize_prop_type(
        &mut self,
        obj: &ObjectHandle,
        storage: &rhp_values::PhpStr,
        val: &Value,
        source: Option<(&ObjectHandle, &rhp_values::PhpStr)>,
    ) -> Result<Option<Value>, Thrown> {
        let _ = (obj, storage, val, source);
        Ok(None)
    }

    /// Write (`Some`) or remove (`None`) the property slot `storage` of
    /// `obj`, keeping declared slots in declaration order.
    fn prop_store(&mut self, obj: &ObjectHandle, storage: &rhp_values::PhpStr, val: Option<Value>) {
        let mut o = obj.borrow_mut();
        match val {
            Some(v) => {
                o.properties.insert(storage.clone(), v);
            }
            None => {
                o.remove_prop(storage);
            }
        }
    }

    /// GC_ADD_FLAGS(obj, IS_OBJ_DESTRUCTOR_CALLED): its destructor never runs.
    fn mark_destructor_called(&mut self, _obj: &ObjectHandle) {}

    /// `get_object_vars($obj)` as seen from the calling scope (unmangled names
    /// of the properties accessible there, in slot order). `None` = the host has
    /// no scope information; the builtin then returns every property.
    fn object_vars(&self, obj: &Value) -> Option<Value>;

    /// Whether `$obj->name` currently exists as seen from the calling scope
    /// (the object half of `property_exists`).
    fn object_prop_exists(&self, obj: &Value, name: &[u8]) -> bool;

    /// A closure's function name (`{closure:scope:line}`), for
    /// `is_callable`'s `$callable_name`.
    fn closure_name(&self, closure: &Value) -> Option<Vec<u8>>;

    /// What `var_dump`/`print_r` show for a Closure
    /// (zend_closure_get_debug_info): name/file/line/static/this/parameter,
    /// or function/this/parameter for a first-class callable.
    fn closure_debug_info(&self, closure: &Value) -> Option<rhp_values::PhpArray>;

    /// The Closure the innermost user frame runs (`Closure::getCurrent()`),
    /// if that frame is a closure's.
    fn current_closure(&self) -> Option<Value>;

    /// A `Fiber` method, or the object's destruction (Zend/zend_fibers.c;
    /// the engine owns the stack switching).
    fn fiber(
        &mut self,
        op: crate::zend::fibers::FiberOp,
        this: Option<&rhp_values::ObjectHandle>,
        args: &[Value],
    ) -> Result<Value, Thrown> {
        let _ = (op, this, args);
        Err(Thrown::error("Error", "Fibers are not available"))
    }

    /// The class name of an object value, if any (for `instanceof`-style checks
    /// inside builtins).
    fn object_class_name(&self, obj: &Value) -> Option<Vec<u8>>;

    /// An object's properties keyed with php's visibility mangling (public
    /// `name`, protected `"\0*\0name"`, private `"\0Class\0name"`), in
    /// property-table order — what `(array)`, `serialize` and `json_encode`
    /// see.
    fn object_props_mangled(&self, obj: &rhp_values::ObjectHandle) -> PhpArray;

    /// Build a `stdClass` object from named properties (for `json_decode`
    /// without `$associative`, `(object)` cast helpers, etc.).
    fn make_object(&mut self, props: Vec<(Vec<u8>, Value)>) -> Value;

    // --- stream layer (file:// / phar:// / …) --------------------------------
    // The VM dispatches these through its per-run wrapper registry.

    /// Whole-file read through the stream layer (`file_get_contents`, include).
    fn stream_get_contents(&self, url: &[u8]) -> Option<Vec<u8>>;

    /// Existence through the stream layer (`file_exists`).
    fn stream_exists(&self, url: &[u8]) -> bool;

    /// Stat through the stream layer (`is_file`/`is_dir`/`filesize`).
    fn stream_stat(&self, url: &[u8]) -> Option<StatInfo>;

    /// `lstat` through the stream layer (`lstat`/`is_link`/`filetype`).
    fn stream_lstat(&self, url: &[u8]) -> Option<StatInfo>;

    /// Directory listing through the stream layer (`opendir`/`scandir`).
    fn stream_opendir(&self, url: &[u8]) -> Option<Vec<Vec<u8>>>;

    /// Whole-file write through the stream layer (`file_put_contents`).
    fn stream_put_contents(&mut self, url: &[u8], data: &[u8]) -> Option<usize>;

    /// The registered wrapper for a scheme, for wrapper-specific control ops
    /// (e.g. `Phar::mapPhar` reaching the `phar://` archive table).
    fn stream_wrapper(&self, scheme: &[u8]) -> Option<std::rc::Rc<dyn StreamWrapper>>;

    /// Registered stream-wrapper scheme names (for `stream_get_wrappers`).
    fn stream_wrapper_schemes(&self) -> Vec<Vec<u8>>;

    /// `stream_wrapper_unregister`: drop `scheme` from this request's
    /// wrapper table; false when it is not registered.
    fn stream_wrapper_unregister(&mut self, _scheme: &[u8]) -> bool {
        false
    }

    /// `stream_wrapper_restore`: put the built-in wrapper for `scheme` back.
    fn stream_wrapper_restore(&mut self, _scheme: &[u8]) -> WrapperRestore {
        WrapperRestore::NeverExisted
    }

    /// The current PHP call stack for `debug_backtrace`/`debug_print_backtrace`:
    /// one entry per frame, innermost first, as
    /// `(function, class, file, line, this_object)` where file/line are the call
    /// site and `this_object` is the frame's `$this` (for `$trace['object']`).
    /// Default: empty.
    #[allow(clippy::type_complexity)]
    fn call_stack(&self) -> Vec<(Vec<u8>, Option<Vec<u8>>, Vec<u8>, u32, Option<Value>)>;

    /// Create a stream context resource holding `options` (nested array like
    /// `['http' => [...], 'ssl' => [...]]`); returns the resource value.
    fn create_stream_context(&mut self, options: Value) -> Value;
    /// The options array stored on a stream-context resource (or Null).
    fn stream_context_options(&self, ctx: &Value) -> Value;
    /// `stream_context_set_option($ctx, $wrapper, $option, $value)`.
    fn stream_context_set_option(
        &mut self,
        ctx: &Value,
        wrapper: &[u8],
        option: &[u8],
        value: Value,
    );

    /// The `notification` callback of a stream context (or of a stream's
    /// context).
    fn stream_context_notifier(&self, ctx: &Value) -> Option<Value>;
    /// Set (or drop) a context's `notification` callback.
    fn set_stream_context_notifier(&mut self, ctx: &Value, cb: Option<Value>);
    /// php's `decode_context_param`: the context resource itself, or a
    /// stream's context (a new one attached when it has none). `None` for
    /// any other value.
    fn resolve_stream_context(&mut self, res: &Value) -> Option<Value>;

    /// Number of arguments passed to the calling function
    /// (`func_num_args`); None when the caller is top-level code (the
    /// script, an include or eval: php's ZEND_CALL_CODE).
    fn current_arg_count(&self) -> Option<i64>;
    /// The arguments passed to the calling function (`func_get_args`),
    /// with the declared parameters' current values; None as for
    /// [`Host::current_arg_count`].
    fn current_args(&self) -> Option<Vec<Value>>;

    /// The filesystem path of the top-level script being executed (for
    /// `Phar::mapPhar`, which parses the running phar itself).
    fn running_main_path(&self) -> Option<Vec<u8>>;

    /// The request's original `$_SERVER['argv']` (php's
    /// `PG(http_globals)[TRACK_VARS_SERVER]`, unaffected by script writes to
    /// `$_SERVER`), for `getopt`.
    fn cli_argv(&self) -> Option<Vec<Vec<u8>>>;

    /// The file of the innermost user-code frame (php's
    /// `zend_get_executed_filename`), for `Phar::running` / `Phar::mapPhar`.
    fn executing_file(&self) -> Option<Vec<u8>>;
    /// Global variable `name` from the global symbol table (an
    /// auto-global is armed first), None when unset.
    fn global_var(&mut self, _name: &[u8]) -> Option<Value> {
        None
    }
    /// zend_exception_error(thrown, level): report a throwable as uncaught
    /// ("Fatal error: Uncaught ..." for E_ERROR, "Warning: ..." for
    /// E_WARNING).
    fn exception_error(&mut self, _thrown: Thrown, _level: i64) {}
    /// The line of the innermost user-code frame (php's
    /// `zend_get_executed_lineno`), 0 outside one.
    fn executing_line(&self) -> u32 {
        0
    }

    // --- stream resources (fopen family) -------------------------------------
    // Streams are resource Values whose state lives in the VM; these are the
    // `fopen`/`fwrite`/… primitives.

    /// `fopen($path, $mode)` → a resource, or None on failure.
    fn stream_open(&mut self, path: &[u8], mode: &[u8]) -> Option<Value>;
    /// [`Host::stream_open`] with the call's stream context (null: the
    /// default one), which a wrapper's own opener reads.
    fn stream_open_ctx(&mut self, path: &[u8], mode: &[u8], ctx: &Value) -> Option<Value>;
    /// A `stream` resource over an extension's stream ops.
    fn stream_from_ops(&mut self, s: ops::OpsStream) -> Option<Value>;
    /// `fwrite`/`fputs` → bytes written.
    fn stream_write(&mut self, res: &Value, data: &[u8]) -> Option<usize>;
    /// `fread($res, $len)` (len < 0 = read all remaining).
    fn stream_read(&mut self, res: &Value, len: i64) -> Option<Vec<u8>>;
    /// `fgets($res)` → one line (incl. newline), or None at EOF.
    fn stream_gets(&mut self, res: &Value) -> Option<Vec<u8>>;
    /// `feof($res)`.
    fn stream_eof(&self, res: &Value) -> bool;
    /// `fclose($res)` (flushes writable streams to their target).
    fn stream_close(&mut self, res: &Value) -> bool;
    /// `fflush($res)`.
    fn stream_flush(&mut self, res: &Value) -> bool;
    /// `rewind($res)`.
    fn stream_rewind(&mut self, res: &Value) -> bool;
    /// What kind of live stream a resource is; `None` if it is not an open
    /// stream (closed, or not a stream resource).
    fn stream_kind(&self, res: &Value) -> Option<StreamKind>;
    /// `fgets($res, $length)`: like `stream_gets` but reading at most
    /// `max` bytes (php's `$length - 1`).
    fn stream_gets_max(&mut self, res: &Value, max: usize) -> Option<Vec<u8>>;
    /// `fseek`: true on success.
    fn stream_seek(&mut self, res: &Value, offset: i64, whence: i32) -> bool;
    /// `ftell`: None if the stream has no position.
    fn stream_tell(&mut self, res: &Value) -> Option<i64>;
    /// `ftruncate`: None if the stream cannot be truncated at all.
    fn stream_truncate(&mut self, res: &Value, size: i64) -> Option<bool>;
    /// `fstat`.
    fn stream_fstat(&mut self, res: &Value) -> Option<StatInfo>;
    /// The OS descriptor behind a stream (for `flock`).
    fn stream_fd(&self, res: &Value) -> Option<i32>;
    /// `stream_get_meta_data` fields.
    fn stream_meta(&self, res: &Value) -> Option<StreamMeta>;
    /// Consume `n` resource ids without creating a resource. php registers
    /// every internal stream it opens (file_get_contents, file(), copy, …) as
    /// a resource, so later resource ids skip past them.
    fn consume_resource_ids(&mut self, n: u32);
    /// The default stream context (`stream_context_get_default`), allocated
    /// lazily as a resource the first time a stream function needs it — php
    /// does this on the first `fopen`/`file_get_contents`/`opendir`/…, which
    /// is visible in later resource ids.
    fn default_stream_context(&mut self) -> Value;
    /// `opendir`: a directory stream resource over `entries`.
    fn dir_open(&mut self, url: &[u8], entries: Vec<Vec<u8>>) -> Option<Value>;
    /// `readdir`: the next entry, or None at the end.
    fn dir_read(&mut self, res: &Value) -> Option<Vec<u8>>;
    /// `rewinddir`.
    fn dir_rewind(&mut self, res: &Value);
    /// The most recently opened directory stream (for `readdir()` with no arg).
    fn last_dir(&self) -> Option<Value>;

    // --- OS-level streams / process support (proc_open & friends) ------------

    /// Wrap an OS file descriptor (e.g. a pipe's parent end) as a `stream`
    /// resource.
    fn stream_from_fd(&mut self, fd: std::os::fd::OwnedFd) -> Option<Value>;
    /// Record the mode a stream was opened with (`stream_get_meta_data`'s
    /// `mode`), for streams opened without a path (`popen`).
    fn stream_set_mode(&mut self, _res: &Value, _mode: &[u8]) {}
    /// Return bytes just read from `res` to the front of its read buffer
    /// (php leaves them buffered); false when the stream cannot.
    fn stream_push_back(&mut self, _res: &Value, _data: &[u8]) -> bool {
        false
    }
    /// A `dup(2)` of the OS fd behind a stream resource, for handing to a child
    /// process (`proc_open` descriptor given as a resource). `Err` carries
    /// php's stream-type name when the stream has no fd (e.g. `MEMORY`).
    fn stream_dup_fd(&mut self, res: &Value) -> Result<std::os::fd::OwnedFd, &'static str>;
    /// The OS fd behind a stream, for `stream_select`, plus whether the stream
    /// already holds buffered unread data (then it counts as readable at once).
    fn stream_select_fd(&self, res: &Value) -> Option<(std::os::fd::RawFd, bool)>;
    /// `stream_set_blocking($res, $enable)`.
    fn stream_set_blocking(&mut self, res: &Value, blocking: bool) -> bool;
    /// Wrap a socket descriptor as a socket `stream` resource (php's
    /// `php_netstream_data_t` stream).
    fn stream_from_socket(
        &mut self,
        fd: std::os::fd::OwnedFd,
        info: plain_wrapper::SockInfo,
    ) -> Option<Value>;
    /// The socket state of a socket stream (`None` for any other stream).
    fn stream_sock_info(&self, res: &Value) -> Option<plain_wrapper::SockInfo>;
    /// The descriptor stream behind a stream resource (sockets, pipes).
    fn stream_fd_mut(&mut self, res: &Value) -> Option<&mut plain_wrapper::FdStream>;
    /// Register a descriptor stream as a stream resource whose context is `ctx`
    /// (php's `stream->ctx`).
    fn stream_insert_fd(
        &mut self,
        stream: plain_wrapper::FdStream,
        ctx: Option<Value>,
    ) -> Option<Value>;
    /// The context a stream was opened with (`PHP_STREAM_CONTEXT`).
    fn stream_context_of(&self, res: &Value) -> Option<Value>;
    /// `zend_set_local_var_str`: assign a variable of the running user
    /// code's scope (a compiled variable, or the symbol table where there
    /// is one). False when the scope has no such variable.
    fn set_local_var(&mut self, name: &[u8], value: Value) -> bool;
    /// The running builtin was called dynamically (a callback, `$f()`,
    /// call_user_func): php's ZEND_CALL_DYNAMIC.
    fn call_is_dynamic(&self) -> bool {
        false
    }
    /// The running builtin is a callback another builtin (or the engine,
    /// with no frame running) invoked: php's previous frame is not user
    /// code.
    fn called_back_by_builtin(&self) -> bool {
        false
    }
    /// get_defined_vars(): a copy of the running user code's variables.
    fn scope_vars(&mut self) -> PhpArray {
        PhpArray::new()
    }
    /// A variable of the running user code's scope.
    fn scope_var(&mut self, _name: &[u8]) -> ScopeVar {
        ScopeVar::Missing
    }
    /// Assign a variable of the running user code's scope (created when
    /// missing; a reference it holds is written through).
    fn scope_assign(&mut self, _name: &[u8], _value: Value) -> Result<(), Thrown> {
        Ok(())
    }
    /// Make a variable of the running user code's scope an alias of `cell`.
    fn scope_bind(&mut self, _name: &[u8], _cell: Rc<RefCell<Value>>) {}
    /// `$this` of the running user code (compact('this')).
    fn scope_this(&self) -> Option<Value> {
        None
    }
    /// `stream_set_timeout` (`None` = wait forever): false unless `res` is a
    /// socket stream.
    fn stream_set_timeout(&mut self, res: &Value, timeout: Option<(i64, i64)>) -> bool;
    /// Flush whatever the host holds back before the process forks
    /// (`pcntl_fork`), so the child does not write it a second time.
    fn prepare_fork(&mut self);
    /// Register an opaque native resource (e.g. a `process` handle) and return
    /// its resource Value.
    fn resource_insert(&mut self, type_name: &'static str, data: Box<dyn std::any::Any>) -> Value;
    /// The native payload of a resource created by [`Host::resource_insert`].
    fn resource_get(&mut self, res: &Value) -> Option<&mut dyn std::any::Any>;
    /// Remove (close) a native resource, returning its payload.
    fn resource_remove(&mut self, res: &Value) -> Option<Box<dyn std::any::Any>>;
    /// Per-run state owned by an extension module (e.g. pcntl's signal-handler
    /// table), created on first use by `init`.
    fn module_state(
        &mut self,
        key: &'static str,
        init: fn() -> Box<dyn std::any::Any>,
    ) -> Option<&mut dyn std::any::Any>;
    /// `ReflectionMethod::invoke`: call `class::method` on `obj` (null for a
    /// static method) regardless of its visibility. `args` is the argument
    /// array (string keys are named arguments).
    fn reflect_invoke(
        &mut self,
        obj: &Value,
        class: &[u8],
        method: &[u8],
        args: Value,
    ) -> Result<Value, Thrown>;
    /// Is `res` a stream resource that is still open (not `fclose`d)?
    fn stream_is_open(&self, res: &Value) -> bool;
    /// The mode a stream was opened with (`"r"`, `"w+b"`, ...), when known.
    fn stream_mode(&self, res: &Value) -> Option<Vec<u8>>;
    /// Attach `filter` to stream `res`'s write (else read) chain, at the end
    /// (else the front), as `stream_filter_append` / `_prepend` do; with
    /// `register` it gets a `stream filter` resource. `None` when appending
    /// to the read chain failed on the buffered data (the filter is freed).
    fn stream_filter_add(
        &mut self,
        _res: &Value,
        _filter: crate::main::streams::filter::Filter,
        _write: bool,
        _append: bool,
        _register: bool,
    ) -> Option<Value> {
        None
    }
    /// `stream_filter_remove`: flush the filter, take it off its chain and
    /// free it. Err for a resource that is not a live stream filter.
    fn stream_filter_remove(&mut self, _filter: &Value) -> Result<bool, Thrown> {
        Ok(false)
    }
    /// A line read (`stream_get_line`) starts (`done` false) or ended on
    /// `res`: php's read buffer of a memory stream fills by chunks then.
    fn stream_note_line_read(&mut self, _res: &Value, _done: bool) {}
    /// `php_stream_get_record` where the host runs it itself (a filtered
    /// stream): `Some(None)` is false; `None` leaves it to the caller.
    fn stream_get_record(
        &mut self,
        _res: &Value,
        _maxlen: usize,
        _delim: &[u8],
    ) -> Option<Option<Vec<u8>>> {
        None
    }
    /// The name php's docref messages use for the running function
    /// (`get_active_function_name`, or the include kind while an include
    /// opens its file).
    fn active_function_name(&self) -> String {
        "Unknown".to_string()
    }
    /// Hold `t` as php's EG(exception), thrown once the builtin returns.
    fn set_pending_throw(&mut self, _t: Thrown) {}
    /// Whether an exception is held (EG(exception)).
    fn has_pending_throw(&self) -> bool {
        false
    }
    /// `has_property(ZEND_PROPERTY_EXISTS)` with the object's own class as
    /// scope (a set, possibly null, declared or dynamic property).
    fn object_prop_exists_as(&self, obj: &Value, name: &[u8]) -> bool {
        self.object_prop_exists(obj, name)
    }

    // --- classes and reflection ---------------------------------------------

    /// Whether class `name` is `other` or has it as an ancestor.
    fn reflect_is_a(&self, name: &[u8], other: &[u8]) -> bool;
    /// `new C(...$args)` with (or without) running the constructor.
    fn reflect_new_instance(
        &mut self,
        name: &[u8],
        args: Vec<Value>,
        run_ctor: bool,
    ) -> Result<Value, Thrown>;
    /// Set an object property bypassing visibility (`ReflectionProperty::setValue`),
    /// resolving `name` as class `class` would see it (its own private first).
    fn reflect_set_prop(
        &mut self,
        obj: &Value,
        class: &[u8],
        name: &[u8],
        val: Value,
    ) -> Result<(), Thrown>;
    /// Read an object property bypassing visibility (`ReflectionProperty::getValue`),
    /// resolving `name` as class `class` would see it (its own private first).
    fn reflect_get_prop(
        &mut self,
        obj: &Value,
        class: &[u8],
        name: &[u8],
    ) -> Result<Option<Value>, Thrown>;
    /// A Reflection / introspection metadata query (`op` names it; see the
    /// VM's `reflect` module): class / method / property / constant /
    /// attribute descriptors, declared-class and included-file lists, ….
    fn reflect_op(&mut self, op: &[u8], args: &[Value]) -> Result<Value, Thrown>;
    /// An `opcache_*` function (`op` is the name without the prefix, the
    /// arguments already checked) answered by the host's compiled-script
    /// cache. `None`: the accelerator isn't started for this request.
    fn opcache_op(&mut self, op: &str, args: &[Value]) -> Option<Result<Value, Thrown>>;
    /// [`Host::raise_diagnostic`], returning the exception a user error
    /// handler threw (e.g. composer's `ErrorException`) so the builtin can
    /// stop right there with `?`.
    fn emit_diagnostic(&mut self, level: i64, message: &[u8]) -> Result<(), Thrown> {
        self.raise_diagnostic(level, message);
        match self.take_pending_throw() {
            Some(t) => Err(t),
            None => Ok(()),
        }
    }
    /// An E_ERROR a builtin raises (zend_error_noreturn): reported at the
    /// innermost user frame with the backtrace, then the request bails out
    /// with the returned value.
    fn fatal_error(&mut self, msg: &[u8]) -> Thrown {
        Thrown::error("Error", String::from_utf8_lossy(msg).into_owned())
    }
    /// Whether php holds `s` as an interned string: a string literal of
    /// the compiled code (debug_zval_dump prints `interned`).
    fn is_interned_str(&self, _s: &rhp_values::PhpStr) -> bool {
        false
    }
    /// Holders of `v` rhp keeps where php has one slot, left out of the
    /// refcounts var_dump and debug_zval_dump show: a global variable's
    /// cell (held by the globals table and by main's CV), an enum case
    /// (held by its class constant and the case list).
    fn shadow_holders(&self, _v: &Value) -> usize {
        0
    }
    /// `php_error_docref(NULL, level, msg)` raised outside a builtin of its
    /// own (an object or resource freed): prefixed with php's active
    /// function (the running builtin, else the innermost user function,
    /// `main` at top level, else `PHP Request Shutdown` in Unknown on line 0).
    fn docref_diagnostic(&mut self, level: i64, msg: &[u8]) -> Result<(), Thrown>;
    /// `pow_function` (the `**` operator) with the engine's diagnostics and
    /// object handlers.
    fn pow_op(&mut self, a: &Value, b: &Value) -> Result<Value, Thrown> {
        use rhp_values::ops;
        let (x, y) = ops::arith_pair(a, b, "**")?;
        if ops::pow_base0_neg_exp(x, y) {
            self.emit_diagnostic(
                crate::E_DEPRECATED,
                b"Power of base 0 and negative exponent is deprecated",
            )?;
        }
        Ok(ops::pow_num(x, y))
    }
}

/// What [`Host::stream_wrapper_restore`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapperRestore {
    /// No built-in wrapper has that name.
    NeverExisted,
    /// The built-in wrapper is still registered.
    Unchanged,
    /// The built-in wrapper is registered again.
    Restored,
}

/// A list of positional arguments as an argument array.
pub fn list_value(args: Vec<Value>) -> Value {
    let mut a = rhp_values::PhpArray::new();
    for v in args {
        a.push(v);
    }
    Value::Array(a)
}
