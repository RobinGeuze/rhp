//! The VM's side of the `Host` interface that builtins call back through.

use std::rc::Rc;

use rhp_builtins::zend::object_handlers::CastTarget;
use rhp_builtins::{self as builtins, Host};
use rhp_values::{
    ArrayKey, ClassId, ClassKind, ObjectHandle, PhpArray, PhpStr, Thrown, Value, Visibility,
};

use crate::calls::{is_engine_helper_name, user_visible_builtin};
use crate::classes::PropRes;
use crate::errors::is_docref_message;
use crate::streams::{res_id, MemMode, StreamState};
use crate::{classes, deref, Vm};

impl Vm {
    /// Whether a compiled property default satisfies `hit` (debug_zval_dump's
    /// interned / shadow-holder checks; a scan).
    fn is_prop_default(&self, hit: impl Fn(&Value) -> bool) -> bool {
        use rhp_bytecode::PropDefaultProto;
        self.decl_classes.iter().any(|d| {
            d.proto
                .props
                .iter()
                .any(|p| matches!(&p.default, PropDefaultProto::Const(v) if hit(v)))
        })
    }
}

impl Host for Vm {
    fn raise_diagnostic(&mut self, level: i64, msg: &[u8]) {
        // A builtin's `php_error_docref` ("func(): …") goes through
        // php_verror; engine and sapi errors do not, nor do the E_USER_*
        // messages trigger_error passes through as given.
        use builtins::{E_USER_DEPRECATED, E_USER_ERROR, E_USER_NOTICE, E_USER_WARNING};
        let user = E_USER_ERROR | E_USER_WARNING | E_USER_NOTICE | E_USER_DEPRECATED;
        if level & user != 0 || !is_docref_message(msg) {
            return self.raise_error(level, msg);
        }
        // php_verror: none while an out-of-memory error is pending.
        if self.mem.attention() {
            return;
        }
        let msg = self.docref_html(msg);
        self.raise_error(level, &msg);
    }

    fn fatal_error(&mut self, msg: &[u8]) -> Thrown {
        self.fatal_error_here(builtins::E_ERROR, msg)
    }

    fn shadow_holders(&self, v: &Value) -> usize {
        match v {
            Value::Ref(r) => self.globals.values().filter(|g| Rc::ptr_eq(g, r)).count(),
            Value::Object(h) => {
                let cls = h.borrow().class;
                self.classes[cls as usize]
                    .enum_cases
                    .iter()
                    .any(|c| Rc::ptr_eq(c, h)) as usize
            }
            // A property default: the compiled class and the linked one.
            Value::Array(a) => self
                .is_prop_default(|d| matches!(d, Value::Array(d) if d.data_ptr() == a.data_ptr()))
                as usize,
            Value::Str(s) => {
                self.is_prop_default(|d| matches!(d, Value::Str(d) if d.ptr_eq(s))) as usize
            }
            _ => 0,
        }
    }

    fn is_interned_str(&self, s: &PhpStr) -> bool {
        // A literal is a string constant of a compiled function (php's
        // compiler interns the literal table). Rare (debug_zval_dump), so
        // a scan.
        let lit = |p: &rhp_bytecode::FunctionProto| {
            p.consts
                .iter()
                .any(|c| matches!(c, Value::Str(c) if c.ptr_eq(s)))
        };
        self.stack.iter().any(|f| lit(&f.proto))
            || self.functions.iter().any(|f| lit(&f.proto))
            || self.closures.iter().any(|f| lit(&f.proto))
            || self.decl_classes.iter().any(|d| {
                d.proto.methods.iter().any(|m| lit(&m.proto))
                    || d.proto.enum_cases.iter().any(|c| c.name.ptr_eq(s))
            })
            || self.is_prop_default(|d| matches!(d, Value::Str(d) if d.ptr_eq(s)))
    }

    fn take_pending_throw(&mut self) -> Option<Thrown> {
        self.pending_throw.take()
    }

    fn result_unused(&self) -> bool {
        self.unused_call_depth == self.stack.len()
    }

    fn docref_diagnostic(&mut self, level: i64, msg: &[u8]) -> Result<(), Thrown> {
        self.ob_diag(level, msg);
        match self.pending_throw.take() {
            Some(t) => Err(t),
            None => Ok(()),
        }
    }

    fn set_error_handler(&mut self, handler: Value, mask: i64) -> Value {
        let prev = match self.error_handlers.last() {
            Some((Value::Bool(false), _)) | None => Value::Null,
            Some((h, _)) => h.clone(),
        };
        self.error_handlers.push((handler, mask));
        prev
    }

    fn restore_error_handler(&mut self) {
        self.error_handlers.pop();
    }

    fn caller_strict_types(&self) -> bool {
        // The user code calling the builtin: a prelude wrapper is internal.
        self.stack
            .iter()
            .rev()
            .find(|f| !f.proto.is_internal())
            .is_some_and(|f| f.proto.strict_types)
    }

    fn chain_thrown(&mut self, thrown: Thrown, previous: Thrown) -> Thrown {
        match previous {
            Thrown::Error { .. } | Thrown::Object(_) => {
                let thrown = self.materialize(thrown, None);
                let previous = self.materialize(previous, None);
                self.chain_previous(&thrown, previous);
                thrown
            }
            other => other,
        }
    }

    fn magic_fetch(&self) -> builtins::MagicFetch {
        let (frame, mode) = self.magic_fetch;
        if frame + 1 == self.stack.len() {
            mode
        } else {
            builtins::MagicFetch::Read
        }
    }

    fn arg_uses_strict_types(&self) -> bool {
        let Some(top) = self.stack.len().checked_sub(1) else {
            return false;
        };
        let strict = |i: usize| !self.is_internal_frame(i) && self.stack[i].proto.strict_types;
        // The builtin frame `i` is running, if php gives it one.
        let builtin_from = |i: usize| {
            self.native_calls
                .iter()
                .rev()
                .find(|n| n.caller == i)
                .is_some_and(|n| !n.frameless_icall)
        };
        // A running builtin is php's current frame: its caller is `top`.
        if builtin_from(top) {
            return strict(top);
        }
        // Else `top` runs the handler and php asks its caller, which is weak
        // when it is a builtin (a callback) or there is none.
        top.checked_sub(1)
            .is_some_and(|below| !builtin_from(below) && strict(below))
    }

    fn last_error(&self) -> Option<(i64, Vec<u8>, Vec<u8>, i64)> {
        self.last_error.clone()
    }

    fn last_error_trace(&self) -> Option<Value> {
        self.last_fatal_trace.clone().map(Value::Array)
    }

    fn clear_last_error(&mut self) {
        self.last_fatal_trace = None;
        self.last_error = None;
    }

    fn compare_cx(&mut self) -> Option<&mut dyn rhp_values::ops::CompareCx> {
        Some(self)
    }

    fn write_out(&mut self, bytes: &[u8]) {
        self.output_write(bytes);
    }

    fn output_control(&mut self, call: builtins::ObCall) -> Result<Value, Thrown> {
        self.output_control_call(call)
    }

    fn class_name(&self, class_id: u32) -> Option<Vec<u8>> {
        self.classes
            .get(class_id as usize)
            .map(|c| c.name.as_bytes().to_vec())
    }

    fn class_parent_name(&self, class_id: u32) -> Option<Vec<u8>> {
        let parent = self.classes.get(class_id as usize)?.parent?;
        Some(self.classes[parent as usize].name.as_bytes().to_vec())
    }

    fn is_enum_class(&self, class_id: u32) -> bool {
        self.classes
            .get(class_id as usize)
            .is_some_and(|c| c.kind == ClassKind::Enum)
    }

    fn not_serializable(&self, class_id: u32) -> bool {
        self.classes
            .get(class_id as usize)
            .is_some_and(|c| c.flags.not_serializable)
    }

    fn object_handlers(
        &self,
        class_id: u32,
    ) -> Option<&'static builtins::zend::object_handlers::ObjectHandlers> {
        self.classes.get(class_id as usize)?.handlers
    }

    fn hooked_properties_for(
        &mut self,
        obj: &ObjectHandle,
        purpose: builtins::zend::object_handlers::PropPurpose,
    ) -> Result<Option<PhpArray>, Thrown> {
        use builtins::zend::object_handlers::PropPurpose as P;
        if purpose == P::Debug {
            let id = obj.borrow().id;
            if self.generators.contains_key(&id) {
                return Ok(Some(self.generator_debug_info(id)));
            }
        } else if obj.borrow().is_lazy() {
            return self.lazy_properties_for(obj, purpose);
        }
        self.hooked_view(obj, purpose)
    }

    fn lazy_props_object(&mut self, obj: &ObjectHandle) -> Result<ObjectHandle, Thrown> {
        Vm::lazy_props_object(self, obj)
    }

    fn std_cast_object(
        &mut self,
        obj: &ObjectHandle,
        target: CastTarget,
    ) -> Result<Option<Value>, Thrown> {
        self.std_cast(obj, target)
    }

    fn define_const(&mut self, name: &[u8], value: Value) -> bool {
        let key = builtins::zend::builtin_functions::const_key(name);
        if self.const_taken(&key) {
            return false;
        }
        self.constants.insert(key.into_owned(), value);
        self.user_const_order.push(name.to_vec());
        true
    }

    fn user_constants(&self) -> Vec<(Vec<u8>, Value)> {
        self.user_const_order
            .iter()
            .filter_map(|n| {
                let v = self
                    .constants
                    .get(&*builtins::zend::builtin_functions::const_key(n))?;
                Some((n.clone(), v.clone()))
            })
            .collect()
    }

    fn response(&mut self) -> Option<&mut builtins::main::sapi::Response> {
        Some(&mut self.sapi.response)
    }

    fn log_message(&mut self, msg: &[u8]) {
        self.sapi.sapi.log_message(msg);
    }

    fn request_headers(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.sapi
            .request
            .as_ref()
            .map(|r| r.headers.clone())
            .unwrap_or_default()
    }

    fn request_info(&self) -> Option<&builtins::main::sapi::RequestInfo> {
        self.sapi.request.as_ref()
    }

    fn body_state(&mut self) -> Option<&mut builtins::main::rfc1867::BodyState> {
        Some(&mut self.sapi.body)
    }

    fn get_const(&self, name: &[u8]) -> Option<Value> {
        self.constants
            .get(&*builtins::zend::builtin_functions::const_key(name))
            .cloned()
            // zend_get_special_const: true/false/null in any case.
            .or_else(|| match name.to_ascii_lowercase().as_slice() {
                b"true" => Some(Value::Bool(true)),
                b"false" => Some(Value::Bool(false)),
                b"null" => Some(Value::Null),
                _ => None,
            })
    }

    fn const_fetch_deprecation(&mut self, name: &[u8]) -> Result<(), Thrown> {
        let key = builtins::zend::builtin_functions::const_key(name).into_owned();
        if let Some(msg) = self.internal_const_deprecation(&key) {
            self.diag(builtins::E_DEPRECATED, &msg)?;
        }
        if !self.const_attrs.is_empty() {
            self.deprecated_user_constant(&key)?;
        }
        Ok(())
    }

    fn register_autoloader(&mut self, callback: Value, prepend: bool) {
        if prepend {
            self.autoloaders.insert(0, callback);
        } else {
            self.autoloaders.push(callback);
        }
    }

    fn autoload_functions(&self) -> Vec<Value> {
        self.autoloaders.clone()
    }

    fn autoload_call(&mut self, name: &[u8]) -> Result<(), Thrown> {
        self.try_autoload(name).map(|_| ())
    }

    // --- stream layer: dispatch to the per-run wrapper registry ---------------

    fn stream_get_contents(&self, url: &[u8]) -> Option<Vec<u8>> {
        if builtins::main::streams::is_data_url(url) {
            return match builtins::main::streams::parse_data_url(url) {
                Ok((data, _)) => Some(data),
                Err(e) => {
                    if !e.is_empty() {
                        builtins::main::streams::note_wrapper_error(e);
                    }
                    None
                }
            };
        }
        if url.eq_ignore_ascii_case(b"php://input") {
            return Some(self.request_body());
        }
        if url.eq_ignore_ascii_case(b"php://stdin") {
            // A fresh php://stdin stream (a dup of fd 0) read to EOF.
            let fd = builtins::main::streams::plain_wrapper::dup_raw(0)?;
            return Some(builtins::main::streams::plain_wrapper::FdStream::new(fd).read_all(-1));
        }
        self.wrapper_for(url)?.get_contents(url)
    }

    fn stream_exists(&self, url: &[u8]) -> bool {
        self.wrapper_for(url)
            .map(|w| w.exists(url))
            .unwrap_or(false)
    }

    fn stream_stat(&self, url: &[u8]) -> Option<builtins::StatInfo> {
        self.wrapper_for(url)?.stat(url)
    }

    fn stream_put_contents(&mut self, url: &[u8], data: &[u8]) -> Option<usize> {
        self.wrapper_for(url)?.put_contents(url, data)
    }

    fn stream_lstat(&self, url: &[u8]) -> Option<builtins::StatInfo> {
        self.wrapper_for(url)?.lstat(url)
    }

    fn stream_opendir(&self, url: &[u8]) -> Option<Vec<Vec<u8>>> {
        self.wrapper_for(url)?.opendir(url)
    }

    fn stream_wrapper(&self, scheme: &[u8]) -> Option<Rc<dyn builtins::StreamWrapper>> {
        self.wrappers.get(scheme).cloned()
    }

    fn stream_wrapper_schemes(&self) -> Vec<Vec<u8>> {
        // php's registration order (openssl's, zlib's, bz2's, standard's,
        // phar's, zip's MINIT); `php://`, data: and http(s):// are handled
        // outside the map.
        const ORDER: &[&[u8]] = &[
            b"https",
            b"ftps",
            b"compress.zlib",
            b"compress.bzip2",
            b"php",
            b"file",
            b"glob",
            b"data",
            b"http",
            b"ftp",
            b"phar",
            b"zip",
        ];
        let builtin = |s: &[u8]| outside_map_wrapper(s) && !self.wrappers_off.contains_key(s);
        let mut v: Vec<Vec<u8>> = ORDER
            .iter()
            .filter(|s| builtin(s) || self.wrappers.contains_key(**s))
            .map(|s| s.to_vec())
            .collect();
        let mut rest: Vec<Vec<u8>> = self
            .wrappers
            .keys()
            .filter(|k| !ORDER.contains(&k.as_slice()))
            .cloned()
            .collect();
        rest.sort();
        v.extend(rest);
        v
    }

    fn stream_wrapper_unregister(&mut self, scheme: &[u8]) -> bool {
        if let Some(w) = self.wrappers.remove(scheme) {
            // the original stays the one restore brings back
            self.wrappers_off.entry(scheme.to_vec()).or_insert(Some(w));
            return true;
        }
        if outside_map_wrapper(scheme) && !self.wrappers_off.contains_key(scheme) {
            self.wrappers_off.insert(scheme.to_vec(), None);
            return true;
        }
        false
    }

    fn stream_wrapper_restore(&mut self, scheme: &[u8]) -> builtins::WrapperRestore {
        match self.wrappers_off.remove(scheme) {
            Some(w) => {
                if let Some(w) = w {
                    self.wrappers.insert(scheme.to_vec(), w);
                }
                builtins::WrapperRestore::Restored
            }
            None if self.wrappers.contains_key(scheme) || outside_map_wrapper(scheme) => {
                builtins::WrapperRestore::Unchanged
            }
            None => builtins::WrapperRestore::NeverExisted,
        }
    }

    #[allow(clippy::type_complexity)]
    fn call_stack(&self) -> Vec<(Vec<u8>, Option<Vec<u8>>, Vec<u8>, u32, Option<Value>)> {
        // Innermost frame first. Each entry pairs the frame's function with the
        // CALL site (the caller frame's current file/line), matching php.
        let mut out = Vec::new();
        for i in (1..self.stack.len()).rev() {
            let f = &self.stack[i];
            let func = f.proto.name.as_bytes().to_vec();
            let class = f
                .defining_class
                .map(|c| self.class(c).name.as_bytes().to_vec());
            let this = self.current_this(i).map(Value::Object);
            let caller = i - 1;
            let file = self.stack[caller].proto.debug.file.as_bytes().to_vec();
            let line = self.current_line(caller);
            out.push((func, class, file, line, this));
        }
        out
    }

    fn create_stream_context(&mut self, options: Value) -> Value {
        let id = self.next_resource_id;
        self.next_resource_id += 1;
        self.stream_contexts.insert(id, options);
        self.listed_resource(id, "stream-context")
    }

    fn stream_context_options(&self, ctx: &Value) -> Value {
        if let Value::Resource(r) = ctx {
            // A stream stands for its context.
            let id = self.streams.context_of(r.id);
            if let Some(opts) = self.stream_contexts.get(&id) {
                return opts.clone();
            }
        }
        Value::Null
    }

    fn stream_context_set_option(
        &mut self,
        ctx: &Value,
        wrapper: &[u8],
        option: &[u8],
        value: Value,
    ) {
        let Value::Resource(r) = ctx else { return };
        let id = self.streams.context_of(r.id);
        let entry = self
            .stream_contexts
            .entry(id)
            .or_insert_with(|| Value::Array(PhpArray::new()));
        if !matches!(entry, Value::Array(_)) {
            *entry = Value::Array(PhpArray::new());
        }
        if let Value::Array(outer) = entry {
            let wk = ArrayKey::from_bytes(wrapper);
            let mut inner = match outer.get(&wk) {
                Some(Value::Array(a)) => a.clone(),
                _ => PhpArray::new(),
            };
            inner.insert(ArrayKey::from_bytes(option), value);
            outer.insert(wk, Value::Array(inner));
        }
    }

    fn current_arg_count(&self) -> Option<i64> {
        self.caller_function_frame().map(|f| f.argc as i64)
    }
    fn current_args(&self) -> Option<Vec<Value>> {
        let f = self.caller_function_frame()?;
        let mut out = Vec::with_capacity(f.argc);
        for p in &f.proto.params {
            if out.len() == f.argc {
                break;
            }
            let v = deref(&f.regs[p.cv as usize]);
            if !p.variadic {
                out.push(v);
            } else if let Value::Array(a) = &v {
                // The variadic array's positional entries (named extras
                // are not counted).
                for (k, v) in a.iter() {
                    if matches!(k, rhp_values::ArrayKey::Int(_)) {
                        out.push(deref(v));
                    }
                }
            }
        }
        out.extend(f.extra_args.iter().cloned());
        Some(out)
    }

    fn running_main_path(&self) -> Option<Vec<u8>> {
        self.main_script_path.clone()
    }

    fn cli_argv(&self) -> Option<Vec<Vec<u8>>> {
        Some(self.argv.clone())
    }

    fn executing_file(&self) -> Option<Vec<u8>> {
        // The innermost user-code frame.
        (0..self.stack.len())
            .rev()
            .find(|&i| !self.is_internal_frame(i))
            .map(|i| self.stack[i].proto.debug.file.as_bytes().to_vec())
            .or_else(|| self.main_script_path.clone())
    }

    fn global_var(&mut self, name: &[u8]) -> Option<Value> {
        let v = match self.globals.get(name) {
            Some(c) => crate::deref(&c.borrow()),
            None => self.jit_globals.get(name).cloned()?,
        };
        (!matches!(v, Value::Null)).then_some(v)
    }

    fn executing_line(&self) -> u32 {
        (0..self.stack.len())
            .rev()
            .find(|&i| !self.is_internal_frame(i))
            .map_or(0, |i| self.current_line(i))
    }

    fn stream_open(&mut self, path: &[u8], mode: &[u8]) -> Option<Value> {
        self.stream_open_ctx(path, mode, &Value::Null)
    }

    fn stream_open_ctx(&mut self, path: &[u8], mode: &[u8], ctx: &Value) -> Option<Value> {
        let writable = mode
            .iter()
            .any(|&b| matches!(b, b'w' | b'a' | b'x' | b'c' | b'+'));
        let append = mode.first() == Some(&b'a');
        if path == b"php://output" {
            let v = self.make_stream(StreamState::Stdout, "stream");
            if let Some(s) = res_id(&v).and_then(|id| self.streams.record_mut(id)) {
                s.output = true;
            }
            return Some(v);
        }
        let state = if path == b"php://stdout" {
            StreamState::Stdout
        } else if path == b"php://stderr" {
            StreamState::Stderr
        } else if path == b"php://stdin" {
            // php opens php://stdin on a dup() of fd 0: its own buffer.
            let v = self.make_stream(
                StreamState::Fd(builtins::main::streams::plain_wrapper::FdStream::new(
                    builtins::main::streams::plain_wrapper::dup_raw(0)?,
                )),
                "stream",
            );
            if let Some(s) = res_id(&v).and_then(|id| self.streams.record_mut(id)) {
                s.fd_name = Some((path.to_vec(), mode.to_vec()));
            }
            return Some(v);
        } else if path.eq_ignore_ascii_case(b"php://input") {
            // The request body: a read-only buffer, re-readable per open.
            return Some(self.make_stream(
                StreamState::Mem {
                    buf: self.request_body(),
                    pos: 0,
                    skew: 0,
                    eof: false,
                    temp: None,
                    mode: MemMode::ReadOnly,
                },
                "stream",
            ));
        } else if path == b"php://memory" || path.starts_with(b"php://temp") {
            let temp = path.starts_with(b"php://temp").then(|| path.to_vec());
            let is_temp = temp.is_some();
            // php_stream_mode_from_str: any "a" appends, else any "w" / "+"
            // is read/write, else read-only.
            let mem_mode = if mode.contains(&b'a') {
                MemMode::Append
            } else if mode.iter().any(|&c| c == b'w' || c == b'+') {
                MemMode::ReadWrite
            } else {
                MemMode::ReadOnly
            };
            let v = self.make_stream(
                StreamState::Mem {
                    buf: Vec::new(),
                    pos: 0,
                    skew: 0,
                    eof: false,
                    temp,
                    mode: mem_mode,
                },
                "stream",
            );
            if is_temp {
                // A temp stream wraps an inner memory stream, allocated right
                // after it (a second resource id).
                self.next_resource_id += 1;
            }
            return Some(v);
        } else if path.len() >= 9 && path[..9].eq_ignore_ascii_case(b"php://fd/") {
            // php_stream_url_wrap_php's fd/: a dup() of the descriptor
            let fd = builtins::main::streams::plain_wrapper::open_php_fd(&path[9..])?;
            let v = self.make_stream(
                StreamState::Fd(builtins::main::streams::plain_wrapper::FdStream::new(fd)),
                "stream",
            );
            if let Some(s) = res_id(&v).and_then(|id| self.streams.record_mut(id)) {
                s.fd_name = Some((path.to_vec(), mode.to_vec()));
            }
            return Some(v);
        } else if path.starts_with(b"php://") {
            return None; // other php:// streams unsupported
        } else if builtins::main::streams::is_data_url(path) {
            // php_stream_url_wrap_rfc2397: a TEMP stream over a MEMORY one
            let (data, meta) = match builtins::main::streams::parse_data_url(path) {
                Ok(v) => v,
                Err(e) => {
                    if !e.is_empty() {
                        builtins::main::streams::note_wrapper_error(e);
                    }
                    return None;
                }
            };
            let readonly = mode.first() == Some(&b'r') && mode.get(1) != Some(&b'+');
            let v = self.make_stream(
                StreamState::Mem {
                    buf: data,
                    pos: 0,
                    skew: 0,
                    eof: false,
                    temp: Some(path.to_vec()),
                    mode: if readonly {
                        MemMode::ReadOnly
                    } else {
                        MemMode::ReadWrite
                    },
                },
                "stream",
            );
            self.next_resource_id += 1;
            if let Some(s) = res_id(&v).and_then(|id| self.streams.record_mut(id)) {
                s.data_url = Some((Value::Array(meta), mode.to_vec(), path.to_vec()));
            }
            return Some(v);
        } else if builtins::main::streams::split_scheme(path).0 == b"file" {
            // Plain local file: a real descriptor (seek/tell/flock/ftruncate).
            StreamState::File(Box::new(builtins::main::streams::LocalFile::open(
                path, mode,
            )?))
        } else if let Some(opened) = self.open_wrapper_stream(path, mode, ctx) {
            return opened;
        } else if writable {
            let origin = (path.to_vec(), mode.to_vec());
            let zlib = builtins::main::streams::split_scheme(path).0 == b"compress.zlib";
            if zlib {
                // The zlib wrapper opens (creating / truncating) the inner
                // plain file right away: a resource of its own.
                let inner = &path[b"compress.zlib://".len()..];
                if !inner.windows(3).any(|w| w == b"://") {
                    if !builtins::main::fopen_wrappers::check_queued(inner, true) {
                        return None;
                    }
                    let fmode: &[u8] = if append { b"ab" } else { b"wb" };
                    let (flags, _, _) = builtins::main::streams::parse_fopen_mode(fmode)?;
                    builtins::zend::virtual_cwd::open(inner, flags, 0o666).ok()?;
                }
                self.next_resource_id += 1;
            }
            let buf = if append {
                self.wrapper_for(path)
                    .and_then(|w| w.get_contents(path))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let v = self.make_stream(
                StreamState::FileWrite {
                    buf,
                    url: path.to_vec(),
                },
                "stream",
            );
            if let Some(s) = res_id(&v).and_then(|id| self.streams.record_mut(id)) {
                s.origin = Some(origin);
            }
            return Some(v);
        } else {
            let data = self.wrapper_for(path).and_then(|w| w.get_contents(path))?;
            if builtins::main::streams::split_scheme(path).0 == b"compress.zlib" {
                self.next_resource_id += 1; // the inner plain-file stream
            }
            let v = self.make_stream(
                StreamState::Mem {
                    buf: data,
                    pos: 0,
                    skew: 0,
                    eof: false,
                    temp: None,
                    mode: MemMode::ReadWrite,
                },
                "stream",
            );
            if let Some(s) = res_id(&v).and_then(|id| self.streams.record_mut(id)) {
                s.origin = Some((path.to_vec(), mode.to_vec()));
            }
            return Some(v);
        };
        let res = self.make_stream(state, "stream");
        if let Some(s) = res_id(&res).and_then(|id| self.streams.record_mut(id)) {
            s.mode = Some(mode.to_vec());
        }
        Some(res)
    }

    fn stream_is_open(&self, res: &Value) -> bool {
        res_id(res).is_some_and(|id| self.streams.contains_key(&id))
    }

    fn stream_from_ops(&mut self, s: builtins::main::streams::ops::OpsStream) -> Option<Value> {
        Some(self.make_stream(StreamState::Ops(Box::new(s)), "stream"))
    }

    fn stream_mode(&self, res: &Value) -> Option<Vec<u8>> {
        let id = res_id(res)?;
        let rec = self.streams.record(id)?;
        if let Some(m) = rec.mode.as_ref().or(rec.origin.as_ref().map(|(_, m)| m)) {
            return Some(m.clone());
        }
        // Memory / temp streams (returned before the mode is recorded).
        match self.streams.get(&id)? {
            StreamState::Ops(s) => Some(s.mode.clone()),
            StreamState::Mem { mode, .. } => Some(
                match mode {
                    MemMode::ReadOnly => &b"rb"[..],
                    MemMode::Append => b"a+b",
                    MemMode::ReadWrite => b"w+b",
                }
                .to_vec(),
            ),
            _ => None,
        }
    }

    fn stream_write(&mut self, res: &Value, data: &[u8]) -> Option<usize> {
        let id = res_id(res)?;
        if self
            .streams
            .record(id)
            .is_some_and(|s| s.data_url.is_some())
        {
            // php_stream_rfc2397_ops has no write
            if data.is_empty() {
                return Some(0);
            }
            let func = self.native_calls.last().map_or("fwrite", |n| n.name);
            let msg = format!("{func}(): Stream is not writable");
            self.raise_error(builtins::E_NOTICE, &msg);
            return None;
        }
        enum Act {
            Out,
            Err,
            Buf,
            No,
        }
        let act = match self.streams.get_mut(&id)? {
            StreamState::Stdout => Act::Out,
            StreamState::Stderr => Act::Err,
            StreamState::Mem { .. } | StreamState::FileWrite { .. } => Act::Buf,
            StreamState::File(f) => return f.write(data),
            StreamState::Ops(s) => return s.write(data),
            StreamState::Fd(f) => {
                let r = f.write(data);
                let send_error = f.take_send_error();
                self.deliver_stream_events(res);
                if let Some(e) = send_error {
                    // php_sockop_write's notice, under the calling builtin.
                    let func = self.native_calls.last().map_or("fwrite", |n| n.name);
                    let msg = format!(
                        "{func}(): Send of {} bytes failed with errno={} {}",
                        e.count,
                        e.errno,
                        builtins::main::streams::strerror(e.errno)
                    );
                    self.raise_error(builtins::E_NOTICE, &msg);
                }
                return r;
            }
            StreamState::Stdin | StreamState::Dir { .. } => Act::No,
        };
        match act {
            Act::Out if self.streams.record(id).is_some_and(|s| s.output) => self.write_out(data),
            Act::Out => self.emit_raw_stdout(data),
            Act::Err => self.write_stderr(data),
            Act::Buf => match self.streams.get_mut(&id)? {
                StreamState::Mem {
                    mode: MemMode::ReadOnly,
                    ..
                } => return None,
                StreamState::Mem {
                    buf,
                    pos,
                    skew,
                    mode: MemMode::Append,
                    ..
                } => {
                    // Append mode writes at the end; php's position still
                    // just advances by the bytes written.
                    let tell = *pos as i64 + *skew + data.len() as i64;
                    buf.extend_from_slice(data);
                    *pos = buf.len();
                    *skew = tell - *pos as i64;
                }
                StreamState::Mem { buf, pos, .. } => {
                    // php://memory writes at the current position.
                    let end = *pos + data.len();
                    if end > buf.len() {
                        buf.resize(end, 0);
                    }
                    buf[*pos..end].copy_from_slice(data);
                    *pos = end;
                }
                StreamState::FileWrite { buf, .. } => buf.extend_from_slice(data),
                _ => {}
            },
            Act::No => return None,
        }
        Some(data.len())
    }

    fn stream_read(&mut self, res: &Value, len: i64) -> Option<Vec<u8>> {
        let id = res_id(res)?;
        if let Some(StreamState::Ops(s)) = self.streams.get_mut(&id) {
            let r = if len < 0 {
                Some(s.read_all())
            } else {
                s.read(len as usize)
            };
            self.raise_ops_stream_diags(id);
            return r;
        }
        match self.streams.get_mut(&id)? {
            StreamState::Mem {
                buf,
                pos,
                eof,
                temp,
                ..
            } => {
                let avail = buf.len().saturating_sub(*pos);
                let take = if len < 0 {
                    avail
                } else {
                    (len as usize).min(avail)
                };
                // _php_stream_read keeps reading memory streams until the
                // request is met, so a short read hits the end and sets eof.
                if len < 0 || take < len as usize {
                    *eof = true;
                }
                // A position past the end (fseek beyond EOF) reads nothing.
                let out = buf.get(*pos..*pos + take).unwrap_or_default().to_vec();
                *pos += take;
                // php://temp is a buffered stream over a memory stream: the
                // buffer fill drains the inner stream, so consuming the last
                // byte already leaves the stream at eof.
                if temp.is_some() && *pos >= buf.len() {
                    *eof = true;
                }
                Some(out)
            }
            StreamState::File(f) => {
                if len < 0 {
                    f.read_all()
                } else {
                    f.read(len as usize)
                }
            }
            StreamState::Fd(f) => {
                let r = if len < 0 {
                    Some(f.read_all(-1))
                } else {
                    f.read(len as usize)
                };
                self.deliver_stream_events(res);
                r
            }
            StreamState::Stdin => {
                let f = self.stdin_stream()?;
                if len < 0 {
                    Some(f.read_all(-1))
                } else {
                    f.read(len as usize)
                }
            }
            _ => Some(Vec::new()),
        }
    }

    fn stream_gets(&mut self, res: &Value) -> Option<Vec<u8>> {
        let r = self.stream_gets_impl(res, None);
        self.deliver_stream_events(res);
        r
    }

    fn stream_gets_max(&mut self, res: &Value, max: usize) -> Option<Vec<u8>> {
        let r = self.stream_gets_impl(res, Some(max));
        self.deliver_stream_events(res);
        r
    }

    fn stream_eof(&self, res: &Value) -> bool {
        match res_id(res).and_then(|id| self.streams.get(&id)) {
            // Wrapper-backed buffers (phar://, compress.zlib://, …) are
            // buffered streams in php: eof once everything has been consumed.
            Some(StreamState::Mem { buf, pos, .. })
                if res_id(res)
                    .and_then(|id| self.streams.record(id))
                    .is_some_and(|s| s.origin.is_some()) =>
            {
                *pos >= buf.len()
            }
            // php://memory and php://temp are unbuffered: eof is the flag a
            // read sets when it runs out of data.
            Some(StreamState::Mem { eof, .. }) => *eof,
            Some(StreamState::File(f)) => f.eof(),
            Some(StreamState::Fd(f)) => f.eof(),
            Some(StreamState::Ops(s)) => s.eof(),
            Some(StreamState::Stdin) => self.stdin_reader.as_ref().is_some_and(|f| f.eof()),
            Some(StreamState::Dir { .. }) => false,
            Some(StreamState::Stdout | StreamState::Stderr | StreamState::FileWrite { .. }) => {
                false
            }
            _ => true,
        }
    }

    fn backtrace(&self, provide_object: bool, ignore_args: bool) -> Option<Value> {
        // Leave out the running debug_backtrace()/debug_print_backtrace().
        let skip = self
            .native_calls
            .last()
            .is_some_and(|n| n.caller + 1 == self.stack.len());
        Some(Value::Array(self.build_trace_skip(
            provide_object,
            ignore_args,
            skip,
        )))
    }

    fn set_exception_handler(&mut self, handler: Value) -> Value {
        let prev = self
            .exception_handlers
            .last()
            .cloned()
            .unwrap_or(Value::Null);
        self.exception_handlers.push(handler);
        prev
    }

    fn restore_exception_handler(&mut self) {
        self.exception_handlers.pop();
    }

    fn register_shutdown_function(
        &mut self,
        callback: Value,
        args: Vec<Value>,
    ) -> Result<(), Thrown> {
        // php keeps the checked callable's fcc: a method callable bound to
        // the registering frame's `$this` keeps it.
        let callback = match (&callback, self.resolve_callable(&callback)) {
            (
                Value::Str(_) | Value::Array(_),
                Ok(crate::calls::Callee::Method(_, Some(this), _)),
            ) => {
                let (scope, called, _) = self.callable_scope();
                Value::Closure(Rc::new(rhp_values::ClosureData {
                    fnidx: u32::MAX,
                    captures: Vec::new(),
                    bound_this: Some(this),
                    bound_class: scope,
                    bound_static: called,
                    wrapped: Some(Box::new(callback.clone())),
                    // Not an object php makes: no handle.
                    id: 0,
                    statics: Default::default(),
                }))
            }
            _ => callback,
        };
        self.shutdown_functions.push((callback, args));
        Ok(())
    }

    fn stream_from_fd(&mut self, fd: std::os::fd::OwnedFd) -> Option<Value> {
        let st = StreamState::Fd(builtins::main::streams::plain_wrapper::FdStream::new(fd));
        Some(self.make_stream(st, "stream"))
    }

    fn stream_push_back(&mut self, res: &Value, data: &[u8]) -> bool {
        let Some(id) = res_id(res) else { return false };
        match self.streams.get_mut(&id) {
            Some(StreamState::Fd(f)) => f.push_back(data),
            Some(StreamState::File(f)) => f.push_back(data),
            Some(StreamState::Mem { pos, .. }) if *pos >= data.len() => *pos -= data.len(),
            _ => return false,
        }
        true
    }

    fn stream_set_mode(&mut self, res: &Value, mode: &[u8]) {
        if let Some(s) = res_id(res).and_then(|id| self.streams.record_mut(id)) {
            s.mode = Some(mode.to_vec());
        }
    }

    fn stream_from_socket(
        &mut self,
        fd: std::os::fd::OwnedFd,
        info: builtins::main::streams::plain_wrapper::SockInfo,
    ) -> Option<Value> {
        let blocked = info.blocked;
        let ty = if info.persistent {
            "persistent stream"
        } else {
            "stream"
        };
        let st =
            StreamState::Fd(builtins::main::streams::plain_wrapper::FdStream::new_socket(fd, info));
        let v = self.make_stream(st, ty);
        if !blocked {
            if let Some(s) = res_id(&v).and_then(|id| self.streams.record_mut(id)) {
                s.nonblocking = true;
            }
        }
        Some(v)
    }

    fn stream_sock_info(
        &self,
        res: &Value,
    ) -> Option<builtins::main::streams::plain_wrapper::SockInfo> {
        match self.streams.get(&res_id(res)?)? {
            StreamState::Fd(f) => f.sock().cloned(),
            _ => None,
        }
    }

    fn stream_fd_mut(
        &mut self,
        res: &Value,
    ) -> Option<&mut builtins::main::streams::plain_wrapper::FdStream> {
        match self.streams.get_mut(&res_id(res)?)? {
            StreamState::Fd(f) => Some(f),
            _ => None,
        }
    }

    fn stream_insert_fd(
        &mut self,
        stream: builtins::main::streams::plain_wrapper::FdStream,
        ctx: Option<Value>,
    ) -> Option<Value> {
        let (blocked, persistent) = stream
            .sock()
            .map_or((true, false), |s| (s.blocked, s.persistent));
        let ty = if persistent {
            "persistent stream"
        } else {
            "stream"
        };
        let v = self.make_stream(StreamState::Fd(stream), ty);
        if let Some(s) = res_id(&v).and_then(|id| self.streams.record_mut(id)) {
            s.nonblocking = !blocked;
            s.ctx = ctx.as_ref().and_then(res_id);
        }
        Some(v)
    }

    fn stream_context_notifier(&self, ctx: &Value) -> Option<Value> {
        let id = res_id(ctx)?;
        let id = self.streams.context_of(id);
        self.stream_notifiers.get(&id).cloned()
    }

    fn set_stream_context_notifier(&mut self, ctx: &Value, cb: Option<Value>) {
        let Some(id) = res_id(ctx) else { return };
        let id = self.streams.context_of(id);
        match cb {
            Some(cb) => {
                self.stream_notifiers.insert(id, cb);
            }
            None => {
                self.stream_notifiers.remove(&id);
            }
        }
    }

    fn resolve_stream_context(&mut self, res: &Value) -> Option<Value> {
        let id = res_id(res)?;
        if self.stream_contexts.contains_key(&id) {
            return Some(self.listed_resource(id, "stream-context"));
        }
        if !self.streams.contains_key(&id) {
            return None;
        }
        if let Some(c) = self.streams.record(id).and_then(|s| s.ctx) {
            return Some(self.listed_resource(c, "stream-context"));
        }
        // php_stream_context_alloc, attached to the stream
        let c = self.create_stream_context(Value::Array(PhpArray::new()));
        self.streams.record_mut(id)?.ctx = Some(res_id(&c)?);
        Some(c)
    }

    fn stream_context_of(&self, res: &Value) -> Option<Value> {
        // (A stream's context is listed by get_resources() while the stream
        // is open.)
        let id = self.streams.record(res_id(res)?)?.ctx?;
        Some(Value::Resource(Rc::new(rhp_values::ResourceData {
            id,
            type_name: "stream-context",
        })))
    }

    fn set_local_var(&mut self, name: &[u8], value: Value) -> bool {
        // zend_set_local_var_str: the innermost user code frame.
        let Some(top) = (0..self.stack.len())
            .rev()
            .find(|&i| !self.is_internal_frame(i))
        else {
            return false;
        };
        if self.stack[top].global_scope || self.stack[top].include_caller.is_some() {
            match self.var_var_slot(top, name) {
                Ok(cv) => self.stack[top].regs[cv as usize] = value,
                Err(cell) => *cell.borrow_mut() = value,
            }
            return true;
        }
        match self.var_var_lookup(top, name) {
            Some(Ok(cv)) => {
                self.stack[top].regs[cv as usize] = value;
                true
            }
            Some(Err(cell)) => {
                *cell.borrow_mut() = value;
                true
            }
            None => false,
        }
    }

    fn stream_set_timeout(&mut self, res: &Value, timeout: Option<(i64, i64)>) -> bool {
        let Some(id) = res_id(res) else { return false };
        match self.streams.get_mut(&id) {
            Some(StreamState::Fd(f)) => f.set_timeout(timeout),
            _ => false,
        }
    }

    fn prepare_fork(&mut self) {
        use std::io::Write;
        let _ = std::io::stderr().flush();
    }

    fn stream_dup_fd(&mut self, res: &Value) -> Result<std::os::fd::OwnedFd, &'static str> {
        use builtins::main::streams::plain_wrapper::dup_raw;
        let id = res_id(res).ok_or("Unknown")?;
        let fail = |o: Option<std::os::fd::OwnedFd>| o.ok_or("STDIO");
        let has_origin = self.streams.record(id).is_some_and(|s| s.origin.is_some());
        match self.streams.get_mut(&id).ok_or("Unknown")? {
            StreamState::Stdin => fail(dup_raw(0)),
            StreamState::Stdout => fail(dup_raw(1)),
            StreamState::Stderr => fail(dup_raw(2)),
            StreamState::Fd(f) => {
                f.sync_for_cast();
                fail(f.dup_fd())
            }
            StreamState::File(f) => {
                f.sync_for_cast();
                fail(builtins::main::streams::plain_wrapper::dup_raw(f.fd()))
            }
            StreamState::Mem { temp: Some(_), .. } => Err("TEMP"),
            _ if has_origin => Err("Unknown"),
            StreamState::Mem { .. } => Err("MEMORY"),
            _ => Err("Unknown"),
        }
    }

    fn stream_select_fd(&self, res: &Value) -> Option<(std::os::fd::RawFd, bool)> {
        let id = res_id(res)?;
        match self.streams.get(&id)? {
            StreamState::Stdin => Some((
                0,
                self.stdin_reader.as_ref().is_some_and(|f| f.has_buffered()),
            )),
            StreamState::Stdout => Some((1, false)),
            StreamState::Stderr => Some((2, false)),
            StreamState::Fd(f) => Some((f.raw_fd()?, f.has_buffered())),
            StreamState::File(f) => Some((f.fd(), f.unread() > 0)),
            _ => None,
        }
    }

    fn stream_set_blocking(&mut self, res: &Value, blocking: bool) -> bool {
        let Some(id) = res_id(res) else { return false };
        // A memory stream has no blocking option (php answers NOTIMPL, which
        // stream_set_blocking reports as success): `blocked` stays true.
        if let Some(s) = self.streams.record_mut(id) {
            if !matches!(s.state, StreamState::Mem { .. }) {
                s.nonblocking = !blocking;
            }
        }
        match self.streams.get_mut(&id) {
            Some(StreamState::Fd(f)) => f.set_blocking(blocking),
            Some(StreamState::Stdin) => {
                builtins::main::streams::plain_wrapper::set_fd_blocking(0, blocking)
            }
            Some(StreamState::Stdout) => {
                builtins::main::streams::plain_wrapper::set_fd_blocking(1, blocking)
            }
            Some(StreamState::Stderr) => {
                builtins::main::streams::plain_wrapper::set_fd_blocking(2, blocking)
            }
            Some(_) => true,
            None => false,
        }
    }

    fn resource_insert(&mut self, type_name: &'static str, data: Box<dyn std::any::Any>) -> Value {
        let id = self.next_resource_id;
        self.next_resource_id += 1;
        self.native_resources.insert(id, data);
        self.listed_resource(id, type_name)
    }

    fn resource_get(&mut self, res: &Value) -> Option<&mut dyn std::any::Any> {
        let id = res_id(res)?;
        self.native_resources.get_mut(&id).map(|b| b.as_mut())
    }

    fn resource_remove(&mut self, res: &Value) -> Option<Box<dyn std::any::Any>> {
        self.native_resources.remove(&res_id(res)?)
    }

    fn module_state(
        &mut self,
        key: &'static str,
        init: fn() -> Box<dyn std::any::Any>,
    ) -> Option<&mut dyn std::any::Any> {
        Some(self.module_states.entry(key).or_insert_with(init).as_mut())
    }

    fn resource_closed(&self, res: &Value) -> bool {
        // A resource is live while some table still holds its id.
        matches!(res, Value::Resource(r) if !(self.streams.contains_key(&r.id)
            || self.native_resources.contains_key(&r.id)
            || self.stream_contexts.contains_key(&r.id)))
    }

    fn stream_close(&mut self, res: &Value) -> bool {
        let Some(id) = res_id(res) else { return false };
        if self.last_dir.as_ref().and_then(res_id) == Some(id) {
            self.last_dir = None;
        }
        match self.free_stream(id) {
            Ok(closed) => closed,
            // php's EG(exception): thrown once the builtin returns.
            Err(t) => {
                self.pending_throw.get_or_insert(t);
                true
            }
        }
    }

    fn stream_flush(&mut self, res: &Value) -> bool {
        let Some(id) = res_id(res) else { return false };
        if let Some(StreamState::FileWrite { buf, url }) = self.streams.get(&id) {
            let (buf, url) = (buf.clone(), url.clone());
            return self
                .wrapper_for(&url)
                .map(|w| w.put_contents(&url, &buf))
                .is_some();
        }
        true
    }

    fn stream_rewind(&mut self, res: &Value) -> bool {
        match res_id(res).and_then(|id| self.streams.get_mut(&id)) {
            Some(StreamState::Mem { pos, skew, eof, .. }) => {
                *pos = 0;
                *skew = 0;
                *eof = false;
                true
            }
            Some(StreamState::File(f)) => f.rewind(),
            Some(StreamState::Fd(f)) => f.seek(0, 0),
            Some(StreamState::Ops(s)) => s.seek(0, 0) == 0,
            Some(StreamState::Stdout | StreamState::Stderr | StreamState::Stdin) => false,
            _ => true,
        }
    }

    fn stream_kind(&self, res: &Value) -> Option<builtins::StreamKind> {
        use builtins::StreamKind as K;
        Some(match self.streams.get(&res_id(res)?)? {
            StreamState::File(_) => K::File,
            StreamState::Fd(_) => K::Pipe,
            StreamState::Dir { .. } => K::Dir,
            StreamState::Mem { .. } | StreamState::FileWrite { .. } | StreamState::Ops(_) => {
                K::Memory
            }
            StreamState::Stdout | StreamState::Stderr | StreamState::Stdin => K::Stdio,
        })
    }

    fn stream_seek(&mut self, res: &Value, offset: i64, whence: i32) -> bool {
        let Some(id) = res_id(res) else { return false };
        match self.streams.get_mut(&id) {
            Some(StreamState::File(f)) => f.seek(offset, whence),
            Some(StreamState::Fd(f)) => f.seek(offset, whence),
            Some(StreamState::Ops(s)) => {
                let ok = s.seek(offset, whence) == 0;
                self.raise_ops_stream_diags(id);
                ok
            }
            Some(StreamState::Stdin) => match self.stdin_stream() {
                Some(f) => f.seek(offset, whence),
                None => false,
            },
            // php_plain_files_dirstream_rewind: any seek rewinds
            Some(StreamState::Dir { pos, .. }) => {
                *pos = 0;
                true
            }
            Some(StreamState::Mem {
                buf,
                pos,
                skew,
                eof,
                temp,
                ..
            }) => {
                // _php_stream_seek + php_stream_memory_seek: a negative
                // SEEK_SET fails without touching the stream; SEEK_CUR is
                // turned into a SEEK_SET past that check, so a negative
                // target reaches the memory stream, which resets its offset
                // to 0 and php's position to -1 (as any other failure does).
                // A temp stream (php://temp, data:) seeks its inner memory
                // stream with SEEK_SET, so there it fails untouched. Seeking
                // past the end is allowed (the next write zero-fills).
                let target = match whence {
                    0 if offset < 0 => return false,
                    0 => offset,
                    1 => {
                        let t = (*pos as i64 + *skew).saturating_add(offset);
                        if t < 0 && temp.is_some() {
                            return false;
                        }
                        t
                    }
                    2 => (buf.len() as i64).saturating_add(offset),
                    _ => {
                        *skew = 0;
                        return false;
                    }
                };
                if target < 0 {
                    *pos = 0;
                    *skew = -1;
                    return false;
                }
                *pos = target as usize;
                *skew = 0;
                *eof = false;
                true
            }
            _ => false,
        }
    }

    fn stream_tell(&mut self, res: &Value) -> Option<i64> {
        match self.streams.get(&res_id(res)?)? {
            StreamState::File(f) => Some(f.tell()),
            // php's ftell(): false while the stream position is -1.
            StreamState::Fd(f) => Some(f.tell()).filter(|&p| p != -1),
            StreamState::Ops(s) => Some(s.tell()).filter(|&p| p != -1),
            StreamState::Stdin => Some(self.stdin_reader.as_ref().map_or(0, |f| f.tell())),
            StreamState::Mem { pos, skew, .. } => {
                // ftell() is false while php's position is -1 (failed seek).
                let t = *pos as i64 + *skew;
                (t >= 0).then_some(t)
            }
            StreamState::FileWrite { buf, .. } => Some(buf.len() as i64),
            // php reads a `php_stream_dirent` (char[MAXPATHLEN] + d_type) per
            // entry, advancing the stream position by its size.
            StreamState::Dir { pos, .. } => {
                let dirent = if cfg!(target_os = "macos") {
                    1025
                } else {
                    4097
                };
                Some(*pos as i64 * dirent)
            }
            _ => Some(0),
        }
    }

    fn stream_truncate(&mut self, res: &Value, size: i64) -> Option<bool> {
        match self.streams.get_mut(&res_id(res)?)? {
            StreamState::File(f) => Some(f.truncate(size)),
            // A read-only memory stream refuses (TEMP_STREAM_READONLY).
            StreamState::Mem {
                mode: MemMode::ReadOnly,
                ..
            } => Some(false),
            StreamState::Mem { buf, pos, skew, .. } => {
                // php_stream_memory_set_option(TRUNCATE_SET_SIZE): shrinking
                // pulls the buffer offset back to the new end; php's stream
                // position (what ftell reports) is left alone.
                let size = size as usize;
                buf.resize(size, 0);
                if size < *pos {
                    *skew += (*pos - size) as i64;
                    *pos = size;
                }
                Some(true)
            }
            _ => None,
        }
    }

    fn stream_fstat(&mut self, res: &Value) -> Option<builtins::StatInfo> {
        let id = res_id(res)?;
        if let Some((uri, _)) = self.streams.record(id).and_then(|s| s.origin.as_ref()) {
            let uri = uri.clone();
            return self.wrapper_for(&uri)?.stat(&uri);
        }
        if let Some(StreamState::Ops(s)) = self.streams.get_mut(&id) {
            return s.stat();
        }
        match self.streams.get(&id)? {
            StreamState::File(f) => f.stat(),
            // php_stream_http_stream_stat fails rather than stat the socket
            StreamState::Fd(f) if f.sock().is_some_and(|s| s.wrapper == Some("http")) => None,
            StreamState::Fd(f) => builtins::main::streams::sys_fstat(f.raw_fd()?),
            StreamState::Stdin => builtins::main::streams::sys_fstat(0),
            StreamState::Stdout => builtins::main::streams::sys_fstat(1),
            StreamState::Stderr => builtins::main::streams::sys_fstat(2),
            StreamState::Mem { buf, .. } => Some(builtins::StatInfo {
                // php's memory-stream stat.
                is_regular: true,
                size: buf.len() as u64,
                mode: 0o100666,
                nlink: 1,
                rdev: -1,
                dev: 0xc,
                blksize: -1,
                blocks: -1,
                ..Default::default()
            }),
            _ => None,
        }
    }

    fn stream_fd(&self, res: &Value) -> Option<i32> {
        match self.streams.get(&res_id(res)?)? {
            StreamState::File(f) => Some(f.fd()),
            StreamState::Fd(f) => f.raw_fd(),
            StreamState::Stdin => Some(0),
            StreamState::Stdout => Some(1),
            StreamState::Stderr => Some(2),
            _ => None,
        }
    }

    fn stream_meta(&self, res: &Value) -> Option<builtins::StreamMeta> {
        use builtins::StreamMeta as M;
        let id = res_id(res)?;
        let meta = || -> Option<M> {
            if let Some((rfc, mode, uri)) =
                self.streams.record(id).and_then(|s| s.data_url.as_ref())
            {
                let eof = matches!(self.streams.get(&id)?, StreamState::Mem { eof: true, .. });
                return Some(M {
                    wrapper_type: "RFC2397",
                    stream_type: "RFC2397",
                    mode: mode.clone(),
                    uri: uri.clone(),
                    unread_bytes: 0,
                    seekable: true,
                    eof,
                    blocked: true,
                    timed_out: false,
                    crypto: None,
                    wrapper_data: None,
                    rfc2397: Some(rfc.clone()),
                });
            }
            if let Some((uri, mode)) = self.streams.record(id).and_then(|s| s.origin.as_ref()) {
                let (scheme, _) = builtins::main::streams::split_scheme(uri);
                let (wrapper_type, stream_type) = match scheme {
                    b"phar" => ("phar", "phar stream"),
                    b"compress.zlib" => ("ZLIB", "ZLIB"),
                    _ => ("user-space", "user-space"),
                };
                let (unread, eof) = match self.streams.get(&id)? {
                    StreamState::Mem { buf, pos, .. } => (0, *pos >= buf.len() && !buf.is_empty()),
                    _ => (0, false),
                };
                return Some(M {
                    wrapper_type,
                    stream_type,
                    mode: mode.clone(),
                    uri: uri.clone(),
                    unread_bytes: unread,
                    seekable: true,
                    eof,
                    blocked: true,
                    timed_out: false,
                    crypto: None,
                    wrapper_data: None,
                    rfc2397: None,
                });
            }
            Some(match self.streams.get(&id)? {
                StreamState::Ops(s) => M {
                    wrapper_type: s.wrapper_label.unwrap_or(""),
                    stream_type: s.label(),
                    mode: s.mode.clone(),
                    uri: s.uri.clone(),
                    unread_bytes: s.unread() as i64,
                    seekable: s.seekable(),
                    eof: s.eof_flag(),
                    blocked: true,
                    timed_out: false,
                    crypto: None,
                    wrapper_data: None,
                    rfc2397: None,
                },
                StreamState::File(f) => M {
                    wrapper_type: "plainfile",
                    stream_type: "STDIO",
                    mode: f.mode.clone(),
                    uri: f.uri.clone(),
                    unread_bytes: f.unread() as i64,
                    seekable: true,
                    eof: f.eof(),
                    blocked: true,
                    timed_out: false,
                    crypto: None,
                    wrapper_data: None,
                    rfc2397: None,
                },
                // A socket: php's META_DATA_API answer (timed_out, blocked,
                // eof from the socket layer), no wrapper / uri.
                StreamState::Fd(f) if f.sock().is_some() => {
                    let s = f.sock()?;
                    M {
                        wrapper_type: s.wrapper.unwrap_or(""),
                        stream_type: s.stream_type,
                        mode: s.mode.clone().unwrap_or_else(|| b"r+".to_vec()),
                        uri: s.uri.clone().unwrap_or_default(),
                        unread_bytes: f.unread() as i64,
                        seekable: false,
                        eof: f.eof_flag(),
                        blocked: s.blocked,
                        timed_out: s.timed_out,
                        crypto: f.crypto_meta(),
                        wrapper_data: s.wrapper_data.clone(),
                        rfc2397: None,
                    }
                }
                StreamState::Fd(f) => {
                    match self.streams.record(id).and_then(|s| s.fd_name.as_ref()) {
                        Some((uri, mode)) => M {
                            wrapper_type: "PHP",
                            stream_type: "STDIO",
                            mode: mode.clone(),
                            uri: uri.clone(),
                            unread_bytes: f.unread() as i64,
                            seekable: f.is_seekable(),
                            eof: f.eof(),
                            blocked: true,
                            timed_out: false,
                            crypto: None,
                            wrapper_data: None,
                            rfc2397: None,
                        },
                        // A pipe (proc_open, popen): no wrapper, no uri; the mode is
                        // popen's, else the descriptor's direction.
                        None => M {
                            wrapper_type: "",
                            stream_type: "STDIO",
                            mode: match self.streams.record(id).and_then(|s| s.mode.clone()) {
                                Some(m) => m,
                                None => match f
                                    .raw_fd()
                                    .map(builtins::main::streams::plain_wrapper::fd_access_mode)
                                {
                                    Some(Some(true)) => b"w".to_vec(),
                                    _ => b"r".to_vec(),
                                },
                            },
                            uri: Vec::new(),
                            unread_bytes: f.unread() as i64,
                            seekable: f.is_seekable(),
                            eof: f.eof(),
                            blocked: true,
                            timed_out: false,
                            crypto: None,
                            wrapper_data: None,
                            rfc2397: None,
                        },
                    }
                }
                StreamState::Dir { .. } => M {
                    wrapper_type: "plainfile",
                    stream_type: "dir",
                    mode: b"r".to_vec(),
                    uri: Vec::new(),
                    unread_bytes: 0,
                    seekable: true,
                    eof: false,
                    blocked: true,
                    timed_out: false,
                    crypto: None,
                    wrapper_data: None,
                    rfc2397: None,
                },
                StreamState::Mem {
                    eof, temp, mode, ..
                } => M {
                    wrapper_type: "PHP",
                    stream_type: if temp.is_some() { "TEMP" } else { "MEMORY" },
                    mode: match mode {
                        MemMode::ReadOnly => b"rb".to_vec(),
                        MemMode::Append => b"a+b".to_vec(),
                        MemMode::ReadWrite => b"w+b".to_vec(),
                    },
                    uri: temp.clone().unwrap_or_else(|| b"php://memory".to_vec()),
                    unread_bytes: 0,
                    seekable: true,
                    eof: *eof,
                    blocked: true,
                    timed_out: false,
                    crypto: None,
                    wrapper_data: None,
                    rfc2397: None,
                },
                StreamState::FileWrite { url, .. } => M {
                    wrapper_type: "plainfile",
                    stream_type: "STDIO",
                    mode: b"w".to_vec(),
                    uri: url.clone(),
                    unread_bytes: 0,
                    seekable: true,
                    eof: false,
                    blocked: true,
                    timed_out: false,
                    crypto: None,
                    wrapper_data: None,
                    rfc2397: None,
                },
                st @ (StreamState::Stdin | StreamState::Stdout | StreamState::Stderr) => {
                    let (fd, mode, uri): (i32, &[u8], &[u8]) = match st {
                        StreamState::Stdin => (0, b"rb", b"php://stdin"),
                        StreamState::Stdout => (1, b"wb", b"php://stdout"),
                        _ => (2, b"wb", b"php://stderr"),
                    };
                    // php: seekable unless a fifo or character device.
                    let seekable = builtins::main::streams::sys_fstat(fd).is_some_and(|s| {
                        let fmt = s.mode & 0o170000;
                        fmt != 0o010000 && fmt != 0o020000
                    });
                    M {
                        wrapper_type: "PHP",
                        stream_type: "STDIO",
                        mode: mode.to_vec(),
                        uri: uri.to_vec(),
                        unread_bytes: 0,
                        seekable,
                        eof: false,
                        blocked: true,
                        timed_out: false,
                        crypto: None,
                        wrapper_data: None,
                        rfc2397: None,
                    }
                }
            })
        };
        let mut m = meta()?;
        if self.streams.record(id).is_some_and(|s| s.nonblocking) {
            m.blocked = false;
        }
        Some(m)
    }

    fn dir_open(&mut self, url: &[u8], entries: Vec<Vec<u8>>) -> Option<Value> {
        let v = self.make_stream(
            StreamState::Dir {
                entries,
                pos: 0,
                url: url.to_vec(),
            },
            "stream",
        );
        self.last_dir = Some(v.clone());
        Some(v)
    }

    fn dir_read(&mut self, res: &Value) -> Option<Vec<u8>> {
        match self.streams.get_mut(&res_id(res)?)? {
            StreamState::Dir { entries, pos, .. } => {
                let e = entries.get(*pos).cloned();
                if e.is_some() {
                    *pos += 1;
                }
                e
            }
            _ => None,
        }
    }

    fn dir_rewind(&mut self, res: &Value) {
        let Some(id) = res_id(res) else { return };
        // php re-reads the directory on rewind (rewinddir(3)).
        let url = match self.streams.get(&id) {
            Some(StreamState::Dir { url, .. }) => url.clone(),
            _ => return,
        };
        let fresh = self.wrapper_for(&url).and_then(|w| w.opendir(&url));
        if let Some(StreamState::Dir { entries, pos, .. }) = self.streams.get_mut(&id) {
            if let Some(f) = fresh {
                *entries = f;
            }
            *pos = 0;
        }
    }

    fn consume_resource_ids(&mut self, n: u32) {
        self.next_resource_id += n;
    }

    fn default_stream_context(&mut self) -> Value {
        let id = match self.default_context {
            Some(id) => id,
            None => {
                let id = self.next_resource_id;
                self.next_resource_id += 1;
                self.stream_contexts
                    .insert(id, Value::Array(PhpArray::new()));
                self.default_context = Some(id);
                id
            }
        };
        self.listed_resource(id, "stream-context")
    }

    fn last_dir(&self) -> Option<Value> {
        let v = self.last_dir.as_ref()?;
        self.streams.get(&res_id(v)?)?;
        Some(v.clone())
    }

    // --- reflection -----------------------------------------------------------

    fn class_constant(&self, class: &[u8], name: &[u8]) -> Option<Value> {
        let cid = self.resolve_class_by_name(class).ok()?;
        self.classes[cid as usize]
            .constants
            .get(name)
            .map(|c| c.value.clone())
    }

    fn class_constant_eval(
        &mut self,
        class: &[u8],
        name: &[u8],
        silent: bool,
    ) -> Result<Option<Value>, Thrown> {
        let fail = |msg: String| -> Result<Option<Value>, Thrown> {
            if silent {
                Ok(None)
            } else {
                Err(Thrown::error("Error", msg))
            }
        };
        let s = String::from_utf8_lossy;
        // A builtin runs on its caller's frame.
        let (scope, called) = self
            .stack
            .last()
            .map_or((None, None), |f| (f.defining_class, f.called_class));
        let cid = if class.eq_ignore_ascii_case(b"self") {
            match scope {
                Some(c) => c,
                None => return fail("Cannot access \"self\" when no class scope is active".into()),
            }
        } else if class.eq_ignore_ascii_case(b"parent") {
            match scope.map(|c| self.classes[c as usize].parent) {
                None => {
                    return fail("Cannot access \"parent\" when no class scope is active".into())
                }
                Some(None) => {
                    return fail(
                        "Cannot access \"parent\" when current class scope has no parent".into(),
                    )
                }
                Some(Some(p)) => p,
            }
        } else if class.eq_ignore_ascii_case(b"static") {
            match called {
                Some(c) => c,
                None => {
                    return fail("Cannot access \"static\" when no class scope is active".into())
                }
            }
        } else {
            match Host::resolve_class_autoload(self, class)? {
                Some(c) => c,
                None => return fail(format!("Class \"{}\" not found", s(class))),
            }
        };
        let Some(c) = self.classes[cid as usize].constants.get(name) else {
            return fail(format!("Undefined constant {}::{}", s(class), s(name)));
        };
        let (vis, decl) = (c.visibility, c.declaring_class);
        let deprecated = c.deprecated.clone();
        if !self.check_access(scope, decl, vis) {
            let v = match vis {
                rhp_values::Visibility::Private => "private",
                rhp_values::Visibility::Protected => "protected",
                rhp_values::Visibility::Public => "public",
            };
            return fail(format!(
                "Cannot access {v} constant {}::{}",
                s(class),
                s(name)
            ));
        }
        if self.classes[cid as usize].kind == ClassKind::Trait {
            return fail(format!(
                "Cannot access trait constant {}::{} directly",
                s(class),
                s(name)
            ));
        }
        if let (Some(d), false) = (deprecated, silent) {
            self.class_const_deprecation(decl, name, &d)?;
        }
        if self.classes[cid as usize].enum_backing.is_some() {
            self.update_class_constants(cid)?;
        }
        self.class_const_value(cid, name, class, false)
    }

    fn reflect_is_a(&self, name: &[u8], other: &[u8]) -> bool {
        match (
            self.resolve_class_by_name(name),
            self.resolve_class_by_name(other),
        ) {
            (Ok(a), Ok(b)) => self.classes[a as usize].ancestors.contains(&b),
            _ => false,
        }
    }

    fn reflect_new_instance(
        &mut self,
        name: &[u8],
        args: Vec<Value>,
        run_ctor: bool,
    ) -> Result<Value, Thrown> {
        let cid = self.resolve_class_by_name(name)?;
        let entry = &self.classes[cid as usize];
        if entry.kind != ClassKind::Class || entry.flags.is_abstract {
            return Err(Thrown::error(
                "Error",
                format!(
                    "Cannot instantiate {} {}",
                    kind_word(entry.kind),
                    String::from_utf8_lossy(entry.name.as_bytes())
                ),
            ));
        }
        self.update_class_constants(cid)?;
        let obj = self.new_object(cid);
        let objval = Value::Object(obj);
        if run_ctor && self.lookup_method(cid, b"__construct").is_some() {
            let mut pair = PhpArray::new();
            pair.push(objval.clone());
            pair.push(Value::str("__construct"));
            self.call_callable_value(&Value::Array(pair), args)?;
        }
        Ok(objval)
    }

    fn reflect_set_prop(
        &mut self,
        obj: &Value,
        class: &[u8],
        name: &[u8],
        val: Value,
    ) -> Result<(), Thrown> {
        // ReflectionProperty::setValue: write_property under
        // EG(fake_scope) = the reflected class; an internal call, so
        // coercive typing.
        let scope = self.resolve_class_by_name(class).ok();
        let top = self.stack.len() - 1;
        let pname = PhpStr::from_bytes(name.to_vec());
        self.prop_store_as(top, deref(obj), pname, val, scope, scope, false)
    }

    fn reflect_get_prop(
        &mut self,
        obj: &Value,
        class: &[u8],
        name: &[u8],
    ) -> Result<Option<Value>, Thrown> {
        // read_property under EG(fake_scope) = the reflected class.
        let o = deref(obj);
        if !matches!(o, Value::Object(_)) {
            return Ok(None);
        }
        let scope = self.resolve_class_by_name(class).ok();
        let pname = PhpStr::from_bytes(name.to_vec());
        self.prop_fetch_as(o, pname, scope).map(Some)
    }

    fn is_internal_slot(&self, class_id: u32, key: &PhpStr) -> bool {
        self.internal_slot(class_id, key)
    }

    fn uninit_props(&self, obj: &ObjectHandle) -> Vec<(PhpStr, String, usize)> {
        let o = obj.borrow();
        let entry = self.class(o.class);
        let mut out = Vec::new();
        let mut before = 0usize;
        for (key, pe) in &entry.props {
            // A virtual property has no slot to dump.
            if self.internal_slot(o.class, key) || pe.virt {
                continue;
            }
            if o.properties.contains_key(key) {
                before += 1;
            } else if let Some(ty) = &pe.ty {
                out.push((
                    classes::mangled_prop_key(entry, key),
                    self.type_string_resolved(ty, Some(pe.declaring_class), Some(o.class)),
                    before,
                ));
            }
        }
        out
    }

    fn mangled_prop_name(&self, class_id: u32, key: &PhpStr) -> PhpStr {
        match self.classes.get(class_id as usize) {
            Some(entry) => classes::mangled_prop_key(entry, key),
            None => key.clone(),
        }
    }

    fn object_prop_exists(&self, obj: &Value, name: &[u8]) -> bool {
        let Value::Object(h) = deref(obj) else {
            return false;
        };
        let scope = self.stack.last().and_then(|f| f.defining_class);
        let b = h.borrow();
        let name = PhpStr::from_bytes(name.to_vec());
        match classes::resolve_prop(&self.classes, b.class, scope, &name) {
            PropRes::Slot(k) => b.properties.contains_key(&k),
            PropRes::Dynamic => b.properties.contains_key(&name),
            PropRes::Denied(_) => false,
        }
    }

    fn object_vars(&self, obj: &Value) -> Option<Value> {
        let Value::Object(h) = deref(obj) else {
            return None;
        };
        let scope = self.stack.last().and_then(|f| f.defining_class);
        let b = h.borrow();
        let mut arr = PhpArray::new();
        for (key, val) in &b.properties {
            if self.internal_slot(b.class, key) {
                continue;
            }
            if let Some(name) = classes::slot_accessible(&self.classes, b.class, scope, key) {
                // A reference held only by this slot reads as its value.
                let v = match val {
                    Value::Ref(rc) if Rc::strong_count(rc) == 1 => rc.borrow().clone(),
                    v => v.clone(),
                };
                arr.insert(ArrayKey::from_bytes(name.as_bytes()), v);
            }
        }
        Some(Value::Array(arr))
    }

    fn pow_op(&mut self, a: &Value, b: &Value) -> Result<Value, Thrown> {
        let (a, b) = (rhp_values::ops::deref(a), rhp_values::ops::deref(b));
        Vm::pow_op(self, &a, &b)
    }

    fn reflect_op(&mut self, op: &[u8], args: &[Value]) -> Result<Value, Thrown> {
        Vm::reflect_op(self, op, args)
    }

    fn opcache_op(&mut self, op: &str, args: &[Value]) -> Option<Result<Value, Thrown>> {
        Vm::opcache_op(self, op, args)
    }

    fn closure_name(&self, closure: &Value) -> Option<Vec<u8>> {
        match deref(closure) {
            Value::Closure(cd) if cd.wrapped.is_none() => self
                .closures
                .get(cd.fnidx as usize)
                .map(|f| f.proto.name.as_bytes().to_vec()),
            _ => None,
        }
    }

    fn closure_debug_info(&self, closure: &Value) -> Option<PhpArray> {
        match deref(closure) {
            Value::Closure(cd) => Some(Vm::closure_debug_info(self, &cd)),
            _ => None,
        }
    }

    fn current_closure(&self) -> Option<Value> {
        let f = self.stack.iter().rev().find(|f| !f.proto.is_internal())?;
        let cd = f.closure.as_ref()?;
        Some(Value::Closure(cd.clone()))
    }

    fn run_released_destructors(&mut self) -> Result<(), Thrown> {
        self.collect_released_now()
    }

    fn call_callable(&mut self, callable: &Value, args: &[Value]) -> Result<Value, Thrown> {
        // `call_user_func[_array]` compiled as a user call (see `cuf_caller`).
        let caller = self.cuf_caller.take();
        self.call_callable_value_mode(callable, args.to_vec(), caller)
    }

    fn call_callable_forward(&mut self, callable: &Value, args: &[Value]) -> Result<Value, Thrown> {
        self.cuf_caller.take();
        Vm::call_callable_forward(self, callable, args.to_vec())
    }

    fn call_callable_named(
        &mut self,
        callable: &Value,
        args: &[Value],
        named: Vec<(PhpStr, Value)>,
    ) -> Result<Value, Thrown> {
        let caller = self.cuf_caller.take();
        self.call_named(callable, args.to_vec(), named, caller)
    }

    fn resolve_class(&self, name: &[u8]) -> Option<u32> {
        self.resolve_class_by_name(name).ok()
    }

    fn resolve_class_autoload(&mut self, name: &[u8]) -> Result<Option<u32>, Thrown> {
        if let Ok(id) = self.resolve_class_by_name(name) {
            return Ok(Some(id));
        }
        // Run registered autoloaders; an exception one throws propagates.
        Ok(if self.try_autoload(name)? {
            self.resolve_class_by_name(name).ok()
        } else {
            None
        })
    }

    fn function_exists(&self, name: &[u8]) -> bool {
        self.fn_names.contains_key(&name.to_ascii_lowercase()) || user_visible_builtin(name)
    }

    fn internal_function_exists(&self, name: &[u8]) -> bool {
        user_visible_builtin(name)
            || self
                .fn_names
                .get(&name.to_ascii_lowercase())
                .is_some_and(|&i| self.functions[i].proto.is_internal())
    }

    fn closure_bind(
        &mut self,
        closure: &Value,
        new_this: &Value,
        scope: Option<&Value>,
    ) -> Result<Value, Thrown> {
        let scope = scope.cloned().unwrap_or_else(|| Value::str("static"));
        self.bind_closure(
            "Closure::bind",
            2,
            closure,
            new_this,
            Some(&scope),
            crate::closures::BindMode::Bind,
        )
    }

    fn class_alias(&mut self, class_id: u32, alias: &[u8]) -> Result<bool, Thrown> {
        let key = alias
            .strip_prefix(b"\\")
            .unwrap_or(alias)
            .to_ascii_lowercase();
        // zend_assert_valid_class_name(lcname, "a class alias")
        let tail = key.rsplit(|&b| b == b'\\').next().unwrap_or(&key);
        if crate::link_checks::is_reserved_class_name(tail) {
            let msg = format!(
                "Cannot use \"{}\" as a class alias as it is reserved",
                String::from_utf8_lossy(&key)
            );
            return Err(self.fatal_compile_error(msg.as_bytes()));
        }
        if key == b"_" {
            self.raise_error(
                rhp_values::E_DEPRECATED,
                "Using \"_\" as a class alias is deprecated since 8.4",
            );
        }
        if self.class_ids.contains_key(&key) {
            // zend_class_redeclaration_error_ex(E_WARNING, alias, ce)
            let ce = &self.classes[class_id as usize];
            let kind = crate::link_checks::kind_uc(ce.kind).to_ascii_lowercase();
            let alias = String::from_utf8_lossy(alias);
            let msg = if ce.decl_file.is_empty() {
                format!("Cannot redeclare {kind} {alias}")
            } else {
                format!(
                    "Cannot redeclare {kind} {alias} (previously declared in {}:{})",
                    String::from_utf8_lossy(ce.decl_file.as_bytes()),
                    ce.decl_line
                )
            };
            self.diag(rhp_values::E_WARNING, &msg)?;
            return Ok(false);
        }
        self.class_aliases
            .push((key.clone(), class_id, self.decl_classes.len()));
        self.class_ids.insert(key, class_id);
        Ok(true)
    }

    fn class_relations(&self, class_id: u32) -> Option<(Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>)> {
        let entry = self.classes.get(class_id as usize)?;
        let name = |id: ClassId| self.classes[id as usize].name.as_bytes().to_vec();
        let mut parents = Vec::new();
        let mut p = entry.parent;
        while let Some(pid) = p {
            parents.push(name(pid));
            p = self.classes[pid as usize].parent;
        }
        let ifaces = entry.interfaces.iter().map(|&i| name(i)).collect();
        let traits = entry.traits.iter().map(|&t| name(t)).collect();
        Some((parents, ifaces, traits))
    }

    fn caller_scope(&self) -> Option<u32> {
        // A builtin runs on its caller's frame.
        self.stack.last().and_then(|f| f.defining_class)
    }

    fn live_resources(&mut self) -> Vec<Value> {
        Vm::live_resources(self)
    }

    fn caller_called_scope(&self) -> Option<u32> {
        self.stack.last().and_then(|f| f.called_class)
    }

    fn object_scoped_props(&self, obj: &Value) -> Vec<(PhpStr, Value)> {
        let Value::Object(h) = deref(obj) else {
            return Vec::new();
        };
        // A builtin runs on its caller's frame: that frame's class is the scope.
        let scope = self.stack.last().and_then(|f| f.defining_class);
        let o = h.borrow();
        // Storage keys are mangled for privates: keep the slots `$obj->name`
        // reaches from the scope, under their plain names.
        o.properties
            .iter()
            .filter(|(k, _)| !self.internal_slot(o.class, k))
            .filter_map(|(k, v)| {
                classes::slot_accessible(&self.classes, o.class, scope, k).map(|n| (n, v.clone()))
            })
            .collect()
    }

    fn class_kind(&self, class_id: u32) -> Option<ClassKind> {
        self.classes.get(class_id as usize).map(|c| c.kind)
    }

    fn class_has_method(&self, class_id: u32, method: &[u8]) -> bool {
        match self.lookup_method(class_id, method) {
            // `__`-prefixed helpers of built-in (prelude) classes are engine
            // plumbing, invisible to php code; real magic methods stay.
            Some(m) => !(m.proto.is_internal() && is_engine_helper_name(method)),
            None => false,
        }
    }

    fn class_method_private_inherited(&self, class_id: u32, method: &[u8]) -> bool {
        self.lookup_method(class_id, method).is_some_and(|m| {
            m.visibility == rhp_values::Visibility::Private && m.defining_class != class_id
        })
    }

    fn class_has_property(&self, class_id: u32, prop: &[u8]) -> bool {
        // php's property_exists(): a declaration visible on the class, where a
        // parent's private does not count.
        match self.classes.get(class_id as usize) {
            Some(entry) => {
                let name = PhpStr::from_bytes(prop.to_vec());
                let inst = entry.prop_visible.get(&name).is_some_and(|k| {
                    let pe = &entry.props[k];
                    pe.visibility != rhp_values::Visibility::Private
                        || pe.declaring_class == class_id
                });
                inst || entry.static_props.get(prop).is_some_and(|sp| {
                    sp.visibility != rhp_values::Visibility::Private
                        || sp.declaring_class == class_id
                })
            }
            None => false,
        }
    }

    fn make_object(&mut self, props: Vec<(Vec<u8>, Value)>) -> Value {
        let Ok(cls) = self.resolve_class_by_name(b"stdClass") else {
            // No stdClass (shouldn't happen) — fall back to an array.
            let mut a = PhpArray::new();
            for (k, v) in props {
                a.insert(ArrayKey::from_bytes(&k), v);
            }
            return Value::Array(a);
        };
        let obj = self.new_object(cls);
        {
            let mut b = obj.borrow_mut();
            for (k, v) in props {
                b.properties.insert(rhp_values::PhpStr::from_bytes(k), v);
            }
        }
        Value::Object(obj)
    }

    fn instantiate(&mut self, class_name: &[u8], props: Vec<(Vec<u8>, Value)>) -> Option<Value> {
        let cls = self.resolve_class_by_name(class_name).ok()?;
        // php's internal classes always have their defaults evaluated. A
        // user class only gets here as a SimpleXMLElement subclass, which
        // php_sxe_object_new creates without zend_update_class_constants:
        // a failing constant expression throws only where it is used.
        let internal = self.is_internal_class(cls);
        if internal {
            if let Err(t) = self.update_class_constants(cls) {
                // php's EG(exception): thrown once the builtin returns.
                self.pending_throw.get_or_insert(t);
                return None;
            }
        }
        let obj = self.new_object(cls);
        if !internal {
            // sxe's property handlers never look at declared properties:
            // leave the subclass's slots out (as if unset) so every access
            // reaches the magic accessors, i.e. the XML.
            let declared: Vec<PhpStr> = self.classes[cls as usize]
                .props
                .iter()
                .filter(|(_, pe)| !self.is_internal_class(pe.declaring_class))
                .map(|(k, _)| k.clone())
                .collect();
            for k in declared {
                obj.borrow_mut().properties.shift_remove(&k);
                self.mark_typed_unset(&obj, &k);
            }
        }
        {
            let mut b = obj.borrow_mut();
            for (k, v) in props {
                b.properties.insert(rhp_values::PhpStr::from_bytes(k), v);
            }
        }
        Some(Value::Object(obj))
    }

    fn unserialize_prop(
        &mut self,
        obj: &ObjectHandle,
        key: &[u8],
    ) -> Result<builtins::UnserProp, Thrown> {
        use builtins::UnserProp;
        let cls = obj.borrow().class;
        let entry = &self.classes[cls as usize];
        // The declared slot php's property table has under `key`.
        let declared = |storage: &PhpStr| {
            let pe = &entry.props[storage];
            UnserProp::Declared {
                storage: storage.clone(),
                typed: pe.ty.is_some(),
            }
        };
        // Storage keys are php's names, but for protected properties.
        let k = PhpStr::from_bytes(key);
        if let Some(pe) = entry.props.get(&k) {
            if !pe.virt && pe.visibility != Visibility::Protected && !self.internal_slot(cls, &k) {
                return Ok(declared(&k));
            }
        }
        // is_property_visibility_changed
        if !entry.props.is_empty() {
            let name: &[u8] = if key.first() == Some(&0) {
                let bad = if key.len() < 3 || key[1] == 0 {
                    Some("Illegal member variable name")
                } else {
                    match key[1..key.len() - 1].iter().position(|&b| b == 0) {
                        Some(_) => None,
                        None => Some("Corrupt member variable name"),
                    }
                };
                if let Some(msg) = bad {
                    self.diag(builtins::E_NOTICE, msg)?;
                    return Ok(UnserProp::Fail);
                }
                let end = 1 + key[1..].iter().position(|&b| b == 0).unwrap_or(0);
                let owner = &key[1..end];
                if owner == b"*" || owner.eq_ignore_ascii_case(entry.name.as_bytes()) {
                    &key[end + 1..]
                } else {
                    b""
                }
            } else {
                key
            };
            let found = (!name.is_empty() || key.first() != Some(&0))
                .then(|| entry.prop_visible.get(&PhpStr::from_bytes(name)))
                .flatten()
                .filter(|k| !self.internal_slot(cls, k));
            if let Some(k) = found {
                let pe = &entry.props[k];
                if !pe.virt {
                    return Ok(declared(&k.clone()));
                }
                let mut msg =
                    b"unserialize(): Cannot unserialize value for virtual property ".to_vec();
                msg.extend_from_slice(self.classes[pe.declaring_class as usize].name.as_bytes());
                msg.extend_from_slice(b"::$");
                msg.extend_from_slice(key.split(|&b| b == 0).next().unwrap_or_default());
                self.emit_diagnostic(builtins::E_WARNING, &msg)?;
                return Ok(UnserProp::Fail);
            }
        }
        if obj.borrow().properties.contains_key(&k) {
            return Ok(UnserProp::Dynamic(k));
        }
        // A new dynamic property.
        let entry = &self.classes[cls as usize];
        let shown = if key.first() == Some(&0) {
            match key[1..].iter().position(|&b| b == 0) {
                Some(e) => &key[e + 2..],
                None => key,
            }
        } else {
            key
        };
        let cname = String::from_utf8_lossy(entry.name.as_bytes()).into_owned();
        let p = String::from_utf8_lossy(shown).into_owned();
        if entry.kind == ClassKind::Enum || entry.flags.is_readonly || entry.flags.no_dynamic {
            return Err(Thrown::error(
                "Error",
                format!("Cannot create dynamic property {cname}::${p}"),
            ));
        }
        if !entry.flags.allow_dynamic {
            self.diag(
                builtins::E_DEPRECATED,
                &format!("Creation of dynamic property {cname}::${p} is deprecated"),
            )?;
        }
        Ok(UnserProp::Dynamic(k))
    }

    fn unserialize_prop_type(
        &mut self,
        obj: &ObjectHandle,
        storage: &PhpStr,
        val: &Value,
        source: Option<(&ObjectHandle, &PhpStr)>,
    ) -> Result<Option<Value>, Thrown> {
        let cls = obj.borrow().class;
        let v = deref(val);
        let out = self.verify_prop_type(cls, storage, v.clone(), true)?;
        let converted = matches!((&v, &out), (Value::Int(_), Value::Float(_)));
        if !converted {
            return Ok(None);
        }
        let Some((src, skey)) = source else {
            return Ok(Some(out));
        };
        // zend_throw_ref_type_error_type: a typed reference cannot coerce.
        let pinfo = |vm: &Vm, c: ClassId, k: &PhpStr| {
            let pe = &vm.classes[c as usize].props[k];
            let ty = pe
                .ty
                .as_ref()
                .map(|t| vm.type_string_resolved(t, Some(pe.declaring_class), Some(c)))
                .unwrap_or_default();
            (
                String::from_utf8_lossy(vm.classes[pe.declaring_class as usize].name.as_bytes())
                    .into_owned(),
                String::from_utf8_lossy(pe.name.as_bytes()).into_owned(),
                ty,
            )
        };
        let scls = src.borrow().class;
        let (sc, sp, st) = pinfo(self, scls, skey);
        let (c, p, t) = pinfo(self, cls, storage);
        Err(Thrown::type_error(format!(
            "Reference with value of type {} held by property {sc}::${sp} of type {st} is not compatible with property {c}::${p} of type {t}",
            builtins::zval_value_name(self, &v)
        )))
    }

    fn prop_store(&mut self, obj: &ObjectHandle, storage: &PhpStr, val: Option<Value>) {
        let mut od = obj.borrow_mut();
        match val {
            Some(v) => *classes::prop_slot_mut(&self.classes, &mut od, storage.clone()) = v,
            None => {
                od.properties.shift_remove(storage);
            }
        }
    }

    fn mark_destructor_called(&mut self, obj: &ObjectHandle) {
        let ptr = Rc::as_ptr(obj) as usize;
        self.destructibles.retain(|d| Rc::as_ptr(d) as usize != ptr);
    }

    fn object_prop(&self, obj: &Value, name: &[u8]) -> Option<Value> {
        let h = match deref(obj) {
            Value::Object(h) => h,
            _ => return None,
        };
        let key = self.declared_prop_key(&h, name);
        let v = h.borrow().properties.get(&key).cloned();
        v
    }

    fn object_props_mangled(&self, obj: &ObjectHandle) -> PhpArray {
        match self.object_to_array_mangled(obj) {
            Value::Array(a) => a,
            _ => PhpArray::new(),
        }
    }

    fn reflect_invoke(
        &mut self,
        obj: &Value,
        class: &[u8],
        method: &[u8],
        args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        let cls = self.resolve_class_by_name(class)?;
        let shown = || {
            format!(
                "{}::{}",
                String::from_utf8_lossy(class),
                String::from_utf8_lossy(method)
            )
        };
        let Some(m) = self.lookup_method(cls, method) else {
            return Err(Thrown::error(
                "ReflectionException",
                format!("Method {}() does not exist", shown()),
            ));
        };
        if m.is_static {
            return self.call_sync(&m, None, cls, args);
        }
        match deref(obj) {
            Value::Object(h) => {
                let oc = h.borrow().class;
                if oc != m.defining_class && !self.class(oc).ancestors.contains(&m.defining_class) {
                    return Err(Thrown::error(
                        "ReflectionException",
                        "Given object is not an instance of the class this method was declared in",
                    ));
                }
                self.call_sync(&m, Some(h), oc, args)
            }
            _ => Err(Thrown::error(
                "ReflectionException",
                format!(
                    "Trying to invoke non static method {}() without an object",
                    shown()
                ),
            )),
        }
    }

    fn object_class_name(&self, obj: &Value) -> Option<Vec<u8>> {
        let h = match obj {
            Value::Object(h) => h,
            Value::Ref(r) => match &*r.borrow() {
                Value::Object(h) => return self.class_name(h.borrow().class),
                _ => return None,
            },
            _ => return None,
        };
        self.class_name(h.borrow().class)
    }

    fn is_callable_value(&self, callable: &Value) -> bool {
        self.resolve_callable(callable).is_ok()
    }

    fn callable_error(&self, callable: &Value) -> Option<String> {
        let e = self.resolve_callable(callable).err()?;
        Some(String::from_utf8_lossy(&e).into_owned())
    }

    fn check_callable(&mut self, callable: &Value) -> Result<Option<String>, Thrown> {
        Ok(self.check_callable_ex(callable, false)?.err())
    }

    fn wrap_callable(&self, callable: Value) -> Value {
        let (scope, called, _) = self.callable_scope();
        // zend_create_closure_from_callable binds the fcc's object.
        let this = match self.resolve_callable(&callable) {
            Ok(
                crate::calls::Callee::Method(_, this, _) | crate::calls::Callee::Magic(_, this, ..),
            ) => this,
            _ => None,
        };
        Value::Closure(Rc::new(rhp_values::ClosureData {
            fnidx: u32::MAX,
            captures: Vec::new(),
            bound_this: this,
            bound_class: scope,
            bound_static: called,
            wrapped: Some(Box::new(callable)),
            id: rhp_values::alloc_object_id(),
            statics: Default::default(),
        }))
    }
}

/// The word php uses in "Cannot instantiate {abstract class|interface|enum} X".
fn kind_word(kind: ClassKind) -> &'static str {
    match kind {
        ClassKind::Interface => "interface",
        ClassKind::Trait => "trait",
        ClassKind::Enum => "enum",
        ClassKind::Class => "abstract class",
    }
}

/// Built-in wrappers served outside the `wrappers` table (`php://`, `data:`,
/// `http(s)://`).
fn outside_map_wrapper(scheme: &[u8]) -> bool {
    matches!(scheme, b"php" | b"data" | b"http" | b"https")
}
