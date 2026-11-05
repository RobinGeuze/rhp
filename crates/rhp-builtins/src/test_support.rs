//! The host the unit tests call builtins with.
//!
//! [`TestHost`] keeps what a builtin writes and the diagnostics it raises,
//! and runs callbacks through a Rust closure. It implements every [`Host`]
//! method the VM implements, as a host without an engine behind it: no
//! classes, constants, streams or resources, and only `file://` paths.

use std::any::Any;
use std::os::fd::{OwnedFd, RawFd};
use std::rc::Rc;

use rhp_values::ops::deref;
use rhp_values::ops::CompareCx;
use rhp_values::{ArrayKey, ClassKind, ClosureData, ObjectHandle, PhpArray, PhpStr, Value};

use crate::main::rfc1867::BodyState;
use crate::main::sapi::{RequestInfo, Response};
use crate::main::streams;
use crate::main::streams::ops::OpsStream;
use crate::main::streams::plain_wrapper::{FdStream, SockInfo};
use crate::main::streams::{FileWrapper, StatInfo, StreamWrapper};
use crate::zend::object_handlers::{CastTarget, ObjectHandlers};
use crate::{Host, MagicFetch, NativeFn, ObCall, StreamKind, StreamMeta, Thrown};

/// A callback for [`TestHost::with_callback`].
type Callback<'a> = Box<dyn FnMut(&[Value]) -> Value + 'a>;

#[derive(Default)]
pub(crate) struct TestHost<'a> {
    /// What the builtin wrote.
    pub out: Vec<u8>,
    /// The diagnostics it raised, as php displays them
    /// (`Warning: f(): msg`).
    pub diags: Vec<String>,
    /// Runs `call_callable`; while set, every value is callable.
    callback: Option<Callback<'a>>,
}

impl<'a> TestHost<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    /// A host whose callbacks all run `f` with the call's arguments.
    pub fn with_callback(f: impl FnMut(&[Value]) -> Value + 'a) -> Self {
        TestHost {
            callback: Some(Box::new(f)),
            ..Self::default()
        }
    }
}

/// Call `f`, returning its result and the diagnostics it raised.
pub(crate) fn run(f: NativeFn, args: &[Value]) -> (Result<Value, Thrown>, Vec<String>) {
    let mut h = TestHost::new();
    let r = f(&mut h, args);
    (r, h.diags)
}

impl Host for TestHost<'_> {
    fn write_out(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }
    fn output_control(&mut self, _call: ObCall) -> Result<Value, Thrown> {
        Err(Thrown::unsupported(
            "output buffering",
            "this host has no output handler stack",
        ))
    }

    // --- diagnostics -------------------------------------------------------

    fn raise_diagnostic(&mut self, level: i64, msg: &[u8]) {
        let msg = String::from_utf8_lossy(msg);
        let kind = match level {
            crate::E_WARNING => "Warning",
            crate::E_NOTICE => "Notice",
            crate::E_DEPRECATED => "Deprecated",
            _ => "Unknown",
        };
        self.diags.push(format!("{kind}: {msg}"));
    }
    fn take_pending_throw(&mut self) -> Option<Thrown> {
        None
    }
    fn last_error(&self) -> Option<(i64, Vec<u8>, Vec<u8>, i64)> {
        None
    }
    fn clear_last_error(&mut self) {}
    fn set_exception_handler(&mut self, _handler: Value) -> Value {
        Value::Null
    }
    fn restore_exception_handler(&mut self) {}
    fn set_error_handler(&mut self, _handler: Value, _mask: i64) -> Value {
        Value::Null
    }
    fn restore_error_handler(&mut self) {}
    fn chain_thrown(&mut self, thrown: Thrown, _previous: Thrown) -> Thrown {
        thrown
    }
    fn silenced(&self) -> bool {
        false
    }
    fn docref_diagnostic(&mut self, level: i64, msg: &[u8]) -> Result<(), Thrown> {
        self.emit_diagnostic(level, &[b"Unknown: ", msg].concat())
    }
    fn log_message(&mut self, msg: &[u8]) {
        use std::io::Write;
        let mut e = std::io::stderr();
        let _ = e.write_all(msg);
        let _ = e.write_all(b"\n");
        let _ = e.flush();
    }

    // --- the calling code --------------------------------------------------

    fn compare_cx(&mut self) -> Option<&mut dyn CompareCx> {
        None
    }
    fn backtrace(&self, _provide_object: bool, _ignore_args: bool) -> Option<Value> {
        None
    }
    #[allow(clippy::type_complexity)]
    fn call_stack(&self) -> Vec<(Vec<u8>, Option<Vec<u8>>, Vec<u8>, u32, Option<Value>)> {
        Vec::new()
    }
    fn caller_strict_types(&self) -> bool {
        false
    }
    fn arg_uses_strict_types(&self) -> bool {
        false
    }
    fn magic_fetch(&self) -> MagicFetch {
        MagicFetch::Read
    }
    fn caller_scope(&self) -> Option<u32> {
        None
    }
    fn current_arg_count(&self) -> i64 {
        0
    }
    fn current_args(&self) -> Vec<Value> {
        Vec::new()
    }
    fn set_local_var(&mut self, _name: &[u8], _value: Value) -> bool {
        false
    }
    fn running_main_path(&self) -> Option<Vec<u8>> {
        None
    }
    fn executing_file(&self) -> Option<Vec<u8>> {
        self.running_main_path()
    }
    fn cli_argv(&self) -> Option<Vec<Vec<u8>>> {
        None
    }
    fn run_released_destructors(&mut self) -> Result<(), Thrown> {
        Ok(())
    }
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
    fn prepare_fork(&mut self) {}
    fn opcache_op(&mut self, _op: &str, _args: &[Value]) -> Option<Result<Value, Thrown>> {
        None
    }
    fn module_state(
        &mut self,
        _key: &'static str,
        _init: fn() -> Box<dyn Any>,
    ) -> Option<&mut dyn Any> {
        None
    }

    // --- callables ---------------------------------------------------------

    fn call_callable(&mut self, _callable: &Value, args: &[Value]) -> Result<Value, Thrown> {
        match &mut self.callback {
            Some(f) => Ok(f(args)),
            None => Err(Thrown::unsupported(
                "callback",
                "callbacks are not available in this host",
            )),
        }
    }
    fn call_callable_named(
        &mut self,
        callable: &Value,
        args: &[Value],
        named: Vec<(PhpStr, Value)>,
    ) -> Result<Value, Thrown> {
        if !named.is_empty() {
            return Err(Thrown::unsupported(
                "named arguments",
                "not supported by this host",
            ));
        }
        self.call_callable(callable, args)
    }
    fn is_callable_value(&self, callable: &Value) -> bool {
        self.callback.is_some() || matches!(deref(callable), Value::Closure(_))
    }
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
    fn check_callable(&mut self, callable: &Value) -> Result<Option<String>, Thrown> {
        Ok(self.callable_error(callable))
    }
    fn wrap_callable(&self, callable: Value) -> Value {
        Value::Closure(Rc::new(ClosureData {
            fnidx: u32::MAX,
            captures: Vec::new(),
            bound_this: None,
            bound_class: None,
            bound_static: None,
            wrapped: Some(Box::new(callable)),
        }))
    }
    fn closure_name(&self, _closure: &Value) -> Option<Vec<u8>> {
        None
    }
    fn closure_bind(&self, _closure: &Value, _new_this: &Value, _scope: &Value) -> Option<Value> {
        None
    }
    fn function_exists(&self, name: &[u8]) -> bool {
        crate::lookup(&String::from_utf8_lossy(name)).is_some()
    }
    fn internal_function_exists(&self, name: &[u8]) -> bool {
        self.function_exists(name)
    }

    // --- constants ---------------------------------------------------------

    fn define_const(&mut self, _name: &[u8], _value: Value) -> bool {
        false
    }
    fn get_const(&self, _name: &[u8]) -> Option<Value> {
        None
    }
    fn user_constants(&self) -> Vec<(Vec<u8>, Value)> {
        Vec::new()
    }
    fn class_constant(&self, _class: &[u8], _name: &[u8]) -> Option<Value> {
        None
    }
    fn class_constant_eval(&mut self, class: &[u8], name: &[u8]) -> Result<Option<Value>, Thrown> {
        Ok(self.class_constant(class, name))
    }

    // --- classes -----------------------------------------------------------

    fn class_name(&self, _class_id: u32) -> Option<Vec<u8>> {
        None
    }
    fn class_parent_name(&self, _class_id: u32) -> Option<Vec<u8>> {
        None
    }
    fn class_kind(&self, _class_id: u32) -> Option<ClassKind> {
        None
    }
    fn class_has_method(&self, _class_id: u32, _method: &[u8]) -> bool {
        false
    }
    fn class_has_property(&self, _class_id: u32, _prop: &[u8]) -> bool {
        false
    }
    #[allow(clippy::type_complexity)]
    fn class_relations(
        &self,
        _class_id: u32,
    ) -> Option<(Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>)> {
        None
    }
    fn is_enum_class(&self, _class_id: u32) -> bool {
        false
    }
    fn not_serializable(&self, _class_id: u32) -> bool {
        false
    }
    fn object_handlers(&self, _class_id: u32) -> Option<&'static ObjectHandlers> {
        None
    }
    fn resolve_class(&self, _name: &[u8]) -> Option<u32> {
        None
    }
    fn resolve_class_autoload(&mut self, name: &[u8]) -> Result<Option<u32>, Thrown> {
        Ok(self.resolve_class(name))
    }
    fn class_alias(&mut self, _class_id: u32, _alias: &[u8]) -> Result<bool, Thrown> {
        Ok(false)
    }
    fn register_autoloader(&mut self, _callback: Value, _prepend: bool) {}
    fn autoload_functions(&self) -> Vec<Value> {
        Vec::new()
    }
    fn autoload_call(&mut self, _name: &[u8]) -> Result<(), Thrown> {
        Ok(())
    }
    fn instantiate(&mut self, _class_name: &[u8], _props: Vec<(Vec<u8>, Value)>) -> Option<Value> {
        None
    }

    // --- objects -----------------------------------------------------------

    fn std_cast_object(
        &mut self,
        _obj: &ObjectHandle,
        target: CastTarget,
    ) -> Result<Option<Value>, Thrown> {
        Ok(crate::zend::object_handlers::std_cast_non_string(target))
    }
    fn is_internal_slot(&self, _class_id: u32, _key: &PhpStr) -> bool {
        false
    }
    fn uninit_props(&self, _obj: &ObjectHandle) -> Vec<(PhpStr, String, usize)> {
        Vec::new()
    }
    fn object_scoped_props(&self, obj: &Value) -> Vec<(PhpStr, Value)> {
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
    fn object_prop(&self, _obj: &Value, _name: &[u8]) -> Option<Value> {
        None
    }
    fn mangled_prop_name(&self, _class_id: u32, key: &PhpStr) -> PhpStr {
        key.clone()
    }
    fn unserialize_set_prop(&mut self, obj: &Value, key: &[u8], val: Value) -> bool {
        if let Value::Object(h) = obj {
            h.borrow_mut()
                .properties
                .insert(PhpStr::from_bytes(key.to_vec()), val);
        }
        true
    }
    fn object_vars(&self, _obj: &Value) -> Option<Value> {
        None
    }
    fn object_prop_exists(&self, obj: &Value, name: &[u8]) -> bool {
        match obj {
            Value::Object(h) => h
                .borrow()
                .properties
                .contains_key(&PhpStr::from_bytes(name.to_vec())),
            _ => false,
        }
    }
    fn object_class_name(&self, _obj: &Value) -> Option<Vec<u8>> {
        None
    }
    fn object_props_mangled(&self, obj: &ObjectHandle) -> PhpArray {
        let mut a = PhpArray::new();
        for (k, v) in &obj.borrow().properties {
            a.insert(ArrayKey::from_bytes(k.as_bytes()), v.clone());
        }
        a
    }
    /// No object model: an array with the properties.
    fn make_object(&mut self, props: Vec<(Vec<u8>, Value)>) -> Value {
        let mut a = PhpArray::new();
        for (k, v) in props {
            a.insert(ArrayKey::from_bytes(&k), v);
        }
        Value::Array(a)
    }

    // --- reflection --------------------------------------------------------

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
    fn reflect_is_a(&self, _name: &[u8], _other: &[u8]) -> bool {
        false
    }
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
    fn reflect_set_prop(
        &mut self,
        _obj: &Value,
        _class: &[u8],
        _name: &[u8],
        _val: Value,
    ) -> Result<(), Thrown> {
        Ok(())
    }
    fn reflect_get_prop(
        &mut self,
        obj: &Value,
        _class: &[u8],
        name: &[u8],
    ) -> Result<Option<Value>, Thrown> {
        Ok(self.object_prop(obj, name))
    }
    fn reflect_op(&mut self, _op: &[u8], _args: &[Value]) -> Result<Value, Thrown> {
        Ok(Value::Null)
    }

    // --- the request -------------------------------------------------------

    fn response(&mut self) -> Option<&mut Response> {
        None
    }
    fn request_headers(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        Vec::new()
    }
    fn request_info(&self) -> Option<&RequestInfo> {
        None
    }
    fn body_state(&mut self) -> Option<&mut BodyState> {
        None
    }

    // --- the stream layer: file:// only ------------------------------------

    fn stream_get_contents(&self, url: &[u8]) -> Option<Vec<u8>> {
        streams::default_get_contents(url)
    }
    fn stream_exists(&self, url: &[u8]) -> bool {
        streams::default_exists(url)
    }
    fn stream_stat(&self, url: &[u8]) -> Option<StatInfo> {
        streams::default_stat(url)
    }
    fn stream_lstat(&self, url: &[u8]) -> Option<StatInfo> {
        match streams::split_scheme(url).0 {
            b"file" => FileWrapper.lstat(url),
            _ => None,
        }
    }
    fn stream_opendir(&self, url: &[u8]) -> Option<Vec<Vec<u8>>> {
        match streams::split_scheme(url).0 {
            b"file" => FileWrapper.opendir(url),
            _ => None,
        }
    }
    fn stream_put_contents(&mut self, url: &[u8], data: &[u8]) -> Option<usize> {
        streams::default_put_contents(url, data)
    }
    fn stream_wrapper(&self, _scheme: &[u8]) -> Option<Rc<dyn StreamWrapper>> {
        None
    }
    fn stream_wrapper_schemes(&self) -> Vec<Vec<u8>> {
        Vec::new()
    }

    // --- stream contexts ---------------------------------------------------

    fn create_stream_context(&mut self, _options: Value) -> Value {
        Value::Bool(false)
    }
    fn stream_context_options(&self, _ctx: &Value) -> Value {
        Value::Null
    }
    fn stream_context_set_option(
        &mut self,
        _ctx: &Value,
        _wrapper: &[u8],
        _option: &[u8],
        _value: Value,
    ) {
    }
    fn stream_context_notifier(&self, _ctx: &Value) -> Option<Value> {
        None
    }
    fn set_stream_context_notifier(&mut self, _ctx: &Value, _cb: Option<Value>) {}
    fn resolve_stream_context(&mut self, _res: &Value) -> Option<Value> {
        None
    }
    fn default_stream_context(&mut self) -> Value {
        Value::Null
    }
    fn stream_context_of(&self, _res: &Value) -> Option<Value> {
        None
    }

    // --- stream and directory resources: none ------------------------------

    fn stream_open(&mut self, _path: &[u8], _mode: &[u8]) -> Option<Value> {
        None
    }
    fn stream_open_ctx(&mut self, path: &[u8], mode: &[u8], _ctx: &Value) -> Option<Value> {
        self.stream_open(path, mode)
    }
    fn stream_from_ops(&mut self, _s: OpsStream) -> Option<Value> {
        None
    }
    fn stream_write(&mut self, _res: &Value, _data: &[u8]) -> Option<usize> {
        None
    }
    fn stream_read(&mut self, _res: &Value, _len: i64) -> Option<Vec<u8>> {
        None
    }
    fn stream_gets(&mut self, _res: &Value) -> Option<Vec<u8>> {
        None
    }
    fn stream_gets_max(&mut self, res: &Value, _max: usize) -> Option<Vec<u8>> {
        self.stream_gets(res)
    }
    fn stream_eof(&self, _res: &Value) -> bool {
        true
    }
    fn stream_close(&mut self, _res: &Value) -> bool {
        false
    }
    fn stream_flush(&mut self, _res: &Value) -> bool {
        false
    }
    fn stream_rewind(&mut self, _res: &Value) -> bool {
        false
    }
    fn stream_kind(&self, _res: &Value) -> Option<StreamKind> {
        None
    }
    fn stream_seek(&mut self, _res: &Value, _offset: i64, _whence: i32) -> bool {
        false
    }
    fn stream_tell(&mut self, _res: &Value) -> Option<i64> {
        None
    }
    fn stream_truncate(&mut self, _res: &Value, _size: i64) -> Option<bool> {
        None
    }
    fn stream_fstat(&mut self, _res: &Value) -> Option<StatInfo> {
        None
    }
    fn stream_fd(&self, _res: &Value) -> Option<i32> {
        None
    }
    fn stream_meta(&self, _res: &Value) -> Option<StreamMeta> {
        None
    }
    fn stream_is_open(&self, _res: &Value) -> bool {
        true
    }
    fn stream_mode(&self, _res: &Value) -> Option<Vec<u8>> {
        None
    }
    fn stream_set_timeout(&mut self, _res: &Value, _timeout: Option<(i64, i64)>) -> bool {
        false
    }
    fn stream_set_blocking(&mut self, _res: &Value, _blocking: bool) -> bool {
        true
    }
    fn stream_from_fd(&mut self, _fd: OwnedFd) -> Option<Value> {
        None
    }
    fn stream_dup_fd(&mut self, _res: &Value) -> Result<OwnedFd, &'static str> {
        Err("MEMORY")
    }
    fn stream_select_fd(&self, _res: &Value) -> Option<(RawFd, bool)> {
        None
    }
    fn stream_from_socket(&mut self, _fd: OwnedFd, _info: SockInfo) -> Option<Value> {
        None
    }
    fn stream_sock_info(&self, _res: &Value) -> Option<SockInfo> {
        None
    }
    fn stream_fd_mut(&mut self, _res: &Value) -> Option<&mut FdStream> {
        None
    }
    fn stream_insert_fd(&mut self, _stream: FdStream, _ctx: Option<Value>) -> Option<Value> {
        None
    }
    fn consume_resource_ids(&mut self, _n: u32) {}
    fn dir_open(&mut self, _url: &[u8], _entries: Vec<Vec<u8>>) -> Option<Value> {
        None
    }
    fn dir_read(&mut self, _res: &Value) -> Option<Vec<u8>> {
        None
    }
    fn dir_rewind(&mut self, _res: &Value) {}
    fn last_dir(&self) -> Option<Value> {
        None
    }
    fn resource_closed(&self, _res: &Value) -> bool {
        false
    }
    fn resource_insert(&mut self, _type_name: &'static str, _data: Box<dyn Any>) -> Value {
        Value::Null
    }
    fn resource_get(&mut self, _res: &Value) -> Option<&mut dyn Any> {
        None
    }
    fn resource_remove(&mut self, _res: &Value) -> Option<Box<dyn Any>> {
        None
    }
}
