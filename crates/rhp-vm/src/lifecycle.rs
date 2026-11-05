//! Engine construction, the global symbol table and superglobals, destructors
//! and the shutdown sequence.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rhp_builtins as builtins;
use rhp_bytecode::{FunctionProto, Program, Reg};
use rhp_values::{ObjectHandle, Thrown, Value};

use crate::lvalue::CursorRoot;
use crate::streams::{StreamState, StreamTable};
use crate::{deref, output, request, Frame, UnitFn, Vm};

/// The shutdown sequence's handler for an `exit` / uncaught throwable:
/// `Some(code)` ends the current step with that exit status.
type ShutdownThrow<'a> = dyn FnMut(&mut Vm, Thrown) -> Result<Option<i32>, Thrown> + 'a;

/// Resets the request state that builtins, the VM and the values keep in
/// thread-locals when the engine is dropped, after its own values: an idle
/// thread holds nothing of its last request (descriptors, handles, the
/// `putenv` overlay). The next `Vm::new` resets it again.
pub(crate) struct RequestStateReset;

impl Drop for RequestStateReset {
    fn drop(&mut self) {
        reset_request_state();
    }
}

/// Every thread-local that holds request state, back to a new request's
/// (docs/design/request-state.md).
fn reset_request_state() {
    builtins::reset_request_state();
    rhp_values::handles::reset_object_ids();
    crate::lvalue::reset_path_diags();
}

impl Vm {
    /// A fresh engine for one request under `sapi`: its name
    /// (`php_sapi_name()`, `PHP_SAPI`), ini defaults, header semantics and
    /// output sink. Set ini values, then compile and run.
    pub fn new(sapi: Box<dyn builtins::main::sapi::Sapi>) -> Self {
        // Builtin state is per request: never inherit a previous engine's.
        reset_request_state();
        rhp_values::mem::end_request();
        let name = sapi.name().to_string();
        builtins::main::sapi::set_sapi(&name, &sapi.ini_defaults());
        let mut constants = HashMap::new();
        for (name, value) in builtins::constants() {
            constants.insert(name.as_bytes().to_vec(), value);
        }
        constants.insert(b"PHP_SAPI".to_vec(), Value::str(name.into_bytes()));
        // Seed the stream-wrapper registry: the default `file://` plus every
        // module-provided wrapper (`phar://`, …).
        let mut wrappers: HashMap<Vec<u8>, Rc<dyn builtins::StreamWrapper>> = HashMap::new();
        wrappers.insert(b"file".to_vec(), builtins::main::streams::file_wrapper());
        wrappers.insert(
            b"glob".to_vec(),
            builtins::main::streams::glob_wrapper::glob_wrapper(),
        );
        for reg in builtins::stream_wrappers() {
            wrappers.insert(reg.scheme.as_bytes().to_vec(), (reg.factory)());
        }
        rhp_values::set_object_free_hook(builtins::object_freed);
        Vm {
            prop_guards: Default::default(),
            pending_magic_fetch: Default::default(),
            magic_fetch: Default::default(),
            output: output::Output::default(),
            stack: Vec::new(),
            units: Vec::new(),
            functions: Vec::new(),
            fn_names: HashMap::new(),
            fn_gen: 1,
            call_sites: Default::default(),
            closures: Vec::new(),
            decl_classes: Vec::new(),
            decl_runtime: std::collections::HashSet::new(),
            class_order: Vec::new(),
            constants,
            user_const_order: Vec::new(),
            max_depth: 1 << 16,
            native_calls: Vec::new(),
            exception_handlers: Vec::new(),
            shutdown_functions: Vec::new(),
            cursor_root: CursorRoot::Cv(0),
            cursor_steps: Default::default(),
            cursor_global: None,
            classes: Vec::new(),
            class_ids: Default::default(),
            globals: indexmap::IndexMap::new(),
            jit_globals: HashMap::new(),
            unset_typed_props: HashMap::new(),
            ref_globals: std::collections::HashSet::new(),
            statics: HashMap::new(),
            include_statics: HashMap::new(),
            generators: HashMap::new(),
            yield_out: None,
            gen_stack: Vec::new(),
            includer: None,
            fcc_scope: std::cell::Cell::new(None),
            fcc_this: std::cell::RefCell::new(None),
            included_files: indexmap::IndexSet::new(),
            internal_classes: 0,
            autoloaders: Vec::new(),
            in_autoload: Default::default(),
            inherit: Default::default(),
            pending_named: None,
            cuf_caller: None,
            dyn_call: false,
            dyn_raw_args: false,
            callable_dep_thrown: false,
            zpp_fcc: None,
            wrappers,
            main_script_path: None,
            streams: StreamTable::default(),
            readonly_reinit: Default::default(),
            hook_bypass: false,
            last_dir: None,
            default_context: None,
            next_resource_id: 1,
            stream_contexts: HashMap::new(),
            stream_notifiers: HashMap::new(),
            argv: Vec::new(),
            silence: 0,
            silence_stack: Vec::new(),
            last_error: None,
            error_handlers: Vec::new(),
            pending_throw: None,
            native_resources: HashMap::new(),
            stdin_reader: None,
            destructibles: Vec::new(),
            marked_destructed: Vec::new(),
            dtor_check: false,
            dtor_pending_checks: 0,
            ctor_frames: Vec::new(),
            finished_main: None,
            in_shutdown_scope: false,
            module_states: HashMap::new(),
            sapi: request::SapiState::new(sapi),
            opcache: None,
            preload_rec: None,
            preloaded: None,
            mem: rhp_values::mem::state(),
            delayed_oom: None,
            _request_state: RequestStateReset,
        }
    }

    /// Start the request in `dir` (php-cgi's script directory, an embedder's
    /// document root); relative paths, `getcwd()` and child processes use
    /// it. The process's working directory never changes: each thread's
    /// engine has its own (`rhp_builtins::zend::virtual_cwd`). `false` when `dir` isn't
    /// a directory.
    pub fn set_cwd(&mut self, dir: &[u8]) -> bool {
        builtins::zend::virtual_cwd::set_cwd(dir)
    }

    /// A startup ini setting (`-d key=value`); unknown directives are ignored.
    pub fn set_ini(&mut self, name: &[u8], value: &[u8]) {
        builtins::ini_startup_set(name, value);
    }

    /// Set the CLI argument vector (`$argv`; `argv[0]` is the script name).
    pub fn set_argv(&mut self, argv: Vec<Vec<u8>>) {
        self.argv = argv;
    }

    /// Get or create the shared global cell for `name` in the global symbol
    /// table, used for {main}'s variables, superglobals, `global` and
    /// `$GLOBALS[...]`.
    pub(crate) fn global_cell(&mut self, name: &[u8]) -> Rc<RefCell<Value>> {
        if let Some(c) = self.globals.get(name) {
            return c.clone();
        }
        let init = self.jit_globals.remove(name).unwrap_or(Value::Null);
        let c = Rc::new(RefCell::new(init));
        self.globals.insert(name.to_vec(), c.clone());
        c
    }

    /// Point global `name` at `cell` (after the variable was unset or
    /// re-pointed with `=&`): the symbol-table entry is replaced, and every
    /// live global-scope frame ({main}, top-level includes) follows.
    pub(crate) fn rebind_global(&mut self, name: &[u8], cell: Rc<RefCell<Value>>) {
        self.globals.insert(name.to_vec(), cell.clone());
        for f in self.stack.iter_mut().filter(|f| f.global_scope) {
            if let Some(i) = f
                .proto
                .cv_var_names
                .iter()
                .position(|n| n.as_bytes() == name)
            {
                f.regs[i] = Value::Ref(cell.clone());
            }
        }
    }

    /// After CV `cv` of frame `top` was overwritten as a slot (unset, `=&`),
    /// keep the global symbol table in step when `top` runs in global scope.
    pub(crate) fn sync_global_cv(&mut self, top: usize, cv: Reg) {
        let f = &self.stack[top];
        if !f.global_scope || cv as usize >= f.proto.cv_var_names.len() {
            return;
        }
        let name = f.proto.cv_var_names[cv as usize].as_bytes().to_vec();
        if name.is_empty() || name == b"this" || is_auto_global(&name) {
            return;
        }
        let cell = match &self.stack[top].regs[cv as usize] {
            Value::Ref(rc) => {
                self.ref_globals.insert(name.clone());
                rc.clone()
            }
            v => {
                self.ref_globals.remove(&name);
                Rc::new(RefCell::new(v.clone()))
            }
        };
        self.rebind_global(&name, cell);
    }

    /// Make CV `cv` of frame `top` a reference to `rc` (a by-ref foreach
    /// variable), keeping the global symbol table in step.
    pub(crate) fn bind_ref_cv(&mut self, top: usize, cv: Reg, rc: Rc<RefCell<Value>>) {
        self.reg_set(top, cv, Value::Ref(rc));
        self.sync_global_cv(top, cv);
    }

    /// Bind every named CV of a global-scope frame to its global cell.
    pub(crate) fn bind_global_scope(&mut self, frame: &mut Frame) {
        frame.global_scope = true;
        let proto = frame.proto.clone();
        for (i, name) in proto.cv_var_names.iter().enumerate() {
            let n = name.as_bytes();
            if n.is_empty() || n == b"this" || is_auto_global(n) {
                continue;
            }
            frame.regs[i] = Value::Ref(self.global_cell(n));
        }
    }

    /// `$GLOBALS` read: a by-value copy of the global symbol table (php 8.1+).
    /// Variables never assigned (null cells) are left out, as php skips its
    /// undefined (IS_UNDEF) entries.
    pub(crate) fn globals_array(&self) -> Value {
        use rhp_values::{ArrayKey, PhpArray};
        let mut a = PhpArray::new();
        for (k, c) in &self.globals {
            let v = c.borrow();
            if matches!(*v, Value::Null) {
                continue;
            }
            a.insert(ArrayKey::from_bytes(k), deref(&v));
        }
        Value::Array(a)
    }

    /// Populate the superglobal global cells ($_SERVER, $argv, …) for a CLI run.
    fn seed_superglobals(&mut self) {
        use rhp_values::{ArrayKey, PhpArray};
        if self.sapi.request.is_some() {
            self.seed_request_superglobals();
            return;
        }
        let argv_arr = {
            let mut a = PhpArray::new();
            for arg in &self.argv {
                a.push(Value::str(arg.clone()));
            }
            a
        };
        let argc = self.argv.len() as i64;
        let script = self.argv.first().cloned().unwrap_or_default();

        let mut server = PhpArray::new();
        for (k, v) in std::env::vars() {
            server.insert(
                ArrayKey::from_bytes(k.as_bytes()),
                Value::str(v.into_bytes()),
            );
        }
        server.insert(
            ArrayKey::from_bytes(b"argv"),
            Value::Array(argv_arr.clone()),
        );
        server.insert(ArrayKey::from_bytes(b"argc"), Value::Int(argc));
        server.insert(
            ArrayKey::from_bytes(b"PHP_SELF"),
            Value::str(script.clone()),
        );
        server.insert(
            ArrayKey::from_bytes(b"SCRIPT_NAME"),
            Value::str(script.clone()),
        );
        // `-r` code / a script read from stdin has no script file.
        let script_fn = self.main_script_path.clone().unwrap_or_default();
        server.insert(
            ArrayKey::from_bytes(b"SCRIPT_FILENAME"),
            Value::str(script_fn),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        server.insert(ArrayKey::from_bytes(b"REQUEST_TIME"), Value::Int(now));
        server.insert(
            ArrayKey::from_bytes(b"REQUEST_TIME_FLOAT"),
            Value::Float(now as f64),
        );

        let set = |vm: &mut Vm, name: &[u8], v: Value| {
            *vm.global_cell(name).borrow_mut() = v;
        };
        // php's CLI order: argv/argc (register_argc_argv), the request
        // auto-globals, then $_SERVER; $_ENV/$_REQUEST are created on first use.
        set(self, b"argv", Value::Array(argv_arr));
        set(self, b"argc", Value::Int(argc));
        for empty in [&b"_GET"[..], b"_POST", b"_COOKIE", b"_FILES"] {
            set(self, empty, Value::Array(PhpArray::new()));
        }
        set(self, b"_SERVER", Value::Array(server));
        let mut env = PhpArray::new();
        for (k, v) in std::env::vars() {
            env.insert(
                ArrayKey::from_bytes(k.as_bytes()),
                Value::str(v.into_bytes()),
            );
        }
        self.jit_globals.insert(b"_ENV".to_vec(), Value::Array(env));
        for lazy in [&b"_REQUEST"[..], b"_SESSION"] {
            self.jit_globals
                .insert(lazy.to_vec(), Value::Array(PhpArray::new()));
        }
    }

    /// Bind a frame's superglobal CVs to their shared global cells.
    pub(crate) fn bind_superglobals(&mut self, frame: &mut Frame) {
        if frame.proto.superglobals.is_empty() {
            return;
        }
        for (cv, name) in frame.proto.superglobals.clone() {
            if name.as_bytes() == b"GLOBALS" {
                continue; // `$GLOBALS` compiles to its own opcodes
            }
            let cell = self.global_cell(name.as_bytes());
            frame.regs[cv as usize] = Value::Ref(cell);
        }
    }

    /// Set (or override) an engine constant, e.g. the SAPI's `PHP_BINARY`.
    pub fn set_constant(&mut self, name: &str, value: Value) {
        self.constants.insert(name.as_bytes().to_vec(), value);
    }

    /// Record the top-level script's filesystem path (for `Phar::mapPhar`).
    pub fn set_main_script_path(&mut self, path: impl Into<Vec<u8>>) {
        self.main_script_path = Some(path.into());
    }

    /// A fatal error bailed out (php_error_cb's
    /// zend_objects_store_mark_destructed): no object alive now runs its
    /// destructor. Objects created later (by shutdown functions) still do.
    pub(crate) fn mark_objects_destructed(&mut self) {
        let marked = std::mem::take(&mut self.destructibles);
        self.marked_destructed.extend(marked);
    }

    /// Remember a new object whose class defines `__destruct` or has a
    /// native `dtor_obj` / `free_obj_host`.
    pub(crate) fn track_destructible(&mut self, h: &ObjectHandle) {
        let cls = h.borrow().class;
        let native = self
            .class(cls)
            .handlers
            .is_some_and(|t| t.dtor_obj.is_some() || t.free_obj_host.is_some());
        if native || self.lookup_method(cls, b"__destruct").is_some() {
            self.destructibles.push(h.clone());
        }
    }

    /// Destruct `h` (already removed from the tracking list, so it runs at
    /// most once): the class's native `dtor_obj` first, then `__destruct`
    /// (zend_objects_destroy_object), then `free_obj_host` unless the
    /// destructor kept a reference.
    fn call_destructor(&mut self, h: ObjectHandle) -> Result<(), Thrown> {
        let cls = h.borrow().class;
        let handlers = self.class(cls).handlers;
        let native = match handlers.and_then(|t| t.dtor_obj) {
            Some(f) => f(self, &h),
            None => Ok(()),
        };
        let mut kept = false;
        if let Some(m) = self.lookup_method(cls, b"__destruct") {
            let before = Rc::strong_count(&h);
            self.call_sync(&m, Some(h.clone()), cls, Vec::new())?;
            kept = Rc::strong_count(&h) > before;
        }
        let freed = match handlers.and_then(|t| t.free_obj_host) {
            Some(f) if !kept => f(self, &h),
            _ => Ok(()),
        };
        native.and(freed)
    }

    /// Destruct every tracked object nothing else references any more (php
    /// destructs on the refcount reaching zero), in creation order.
    pub(crate) fn collect_released(&mut self) -> Result<(), Thrown> {
        // A full scan is O(n): with many live destructible objects, scan only
        // once enough release events have accumulated (amortized O(1)).
        self.dtor_pending_checks += 1;
        if self.destructibles.len() > 256
            && self.dtor_pending_checks < self.destructibles.len() / 64
        {
            return Ok(());
        }
        self.dtor_pending_checks = 0;
        self.collect_released_now()
    }

    /// Destruct every tracked object nothing references any more (a full
    /// scan, repeated until no destructor releases further objects).
    pub(crate) fn collect_released_now(&mut self) -> Result<(), Thrown> {
        loop {
            if !self.destructibles.iter().any(|h| Rc::strong_count(h) == 1) {
                return Ok(());
            }
            let (released, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.destructibles)
                .into_iter()
                .partition(|h| Rc::strong_count(h) == 1);
            self.destructibles = kept;
            let mut it = released.into_iter();
            while let Some(h) = it.next() {
                if let Err(e) = self.call_destructor(h) {
                    // Keep the not-yet-destructed ones tracked.
                    let rest: Vec<_> = it.collect();
                    self.destructibles.splice(0..0, rest);
                    return Err(e);
                }
            }
        }
    }

    /// php_request_shutdown: 1. the `register_shutdown_function` callbacks,
    /// in order (including ones registered meanwhile; an `exit` or uncaught
    /// throwable stops the rest), 2. destructors — globals holding the only
    /// reference to an object first (in reverse order), then every remaining
    /// object in creation order, 3. flush all output buffers. `on_throw`
    /// handles an `exit` / uncaught throwable (`Some(code)`: it ends that
    /// step with this exit status). Runs with {main}'s globals in scope.
    fn shutdown_sequence(
        &mut self,
        mut status: i32,
        on_throw: &mut ShutdownThrow,
    ) -> Result<i32, Thrown> {
        self.oom_reset();
        // 1. shutdown functions
        let mut i = 0;
        while i < self.shutdown_functions.len() {
            let (cb, args) = self.shutdown_functions[i].clone();
            i += 1;
            self.enter_shutdown_scope();
            let r = self.call_callable_value(&cb, args);
            // zend_call_function's interrupt check after the call.
            if let Err(t) = r.and_then(|_| self.oom_interrupt()) {
                self.enter_shutdown_scope();
                if let Some(c) = on_throw(self, t)? {
                    status = c;
                    break;
                }
            }
        }
        // php_free_shutdown_functions is step 7: the callbacks (and their
        // object handles) live until after the destructors.
        let _registered = std::mem::take(&mut self.shutdown_functions);
        // 2. destructors
        self.enter_shutdown_scope();
        if let Err(t) = self
            .shutdown_destructors()
            .and_then(|_| self.oom_interrupt())
        {
            self.enter_shutdown_scope();
            if let Some(c) = on_throw(self, t)? {
                status = c;
            }
        }
        self.destructibles.clear();
        self.marked_destructed.clear();
        // 3. php_output_end_all: finalize each buffer into the one below it
        // (an exception from a handler is fatal here).
        self.enter_shutdown_scope();
        self.output_end_all();
        if let Some(t) = self.pending_throw.take() {
            self.enter_shutdown_scope();
            if let Some(c) = on_throw(self, t)? {
                status = c;
            }
            self.output_end_all();
        }
        self.shutdown_functions.clear();
        self.stack.clear();
        Ok(status)
    }

    /// The stack shutdown code runs on: just {main}'s frame (so `global`
    /// still reaches the script's variables), shown in traces as php's
    /// `[internal function]` caller. Frames left above it by an earlier
    /// shutdown step are dropped.
    pub(crate) fn enter_shutdown_scope(&mut self) {
        if self.in_shutdown_scope && !self.stack.is_empty() {
            self.stack.truncate(1);
            return;
        }
        self.stack.clear();
        match self.finished_main.take() {
            Some(mut main) => {
                let mut p = (*main.proto).clone();
                p.debug.file = rhp_values::PhpStr::from("");
                p.flags.insert(rhp_bytecode::ProtoFlags::SHUTDOWN_SCOPE);
                main.pc = p.code.len();
                // {main}'s temporaries are dead once it finished (php frees
                // TMP/VAR slots as it goes): only its variables keep values.
                let ncv = (p.num_cvs as usize).min(main.regs.len());
                for r in &mut main.regs[ncv..] {
                    *r = Value::Null;
                }
                main.proto = Rc::new(p);
                self.stack.push(main);
            }
            None => self.push_internal_main(),
        }
        self.in_shutdown_scope = true;
    }

    /// php's `zend_call_destructors`.
    fn shutdown_destructors(&mut self) -> Result<(), Thrown> {
        self.dtor_pending_checks = usize::MAX / 2;
        self.collect_released()?;
        // Globals holding the sole reference to an object, last first, until
        // nothing changes (php's `zval_call_destructor` sweep over the global
        // symbol table).
        loop {
            let mut changed = false;
            let names: Vec<Vec<u8>> = self.globals.keys().rev().cloned().collect();
            for name in names {
                let Some(cell) = self.globals.get(&name[..]).cloned() else {
                    continue;
                };
                // A global that is a PHP reference (made one by `global`,
                // `=&`, by-ref passing, or aliased beyond the table and the
                // global-scope frames' own slots) is not an object.
                if self.ref_globals.contains(&name) {
                    continue;
                }
                let frame_slots = self
                    .stack
                    .iter()
                    .flat_map(|f| f.regs.iter())
                    .filter(|v| matches!(v, Value::Ref(r) if Rc::ptr_eq(r, &cell)))
                    .count();
                if Rc::strong_count(&cell) != 2 + frame_slots {
                    continue; // (+1: our clone)
                }
                // A Closure is an object too: freeing the last one releases
                // what it captured.
                let sole_closure =
                    matches!(&*cell.borrow(), Value::Closure(c) if Rc::strong_count(c) == 1);
                if sole_closure {
                    self.globals.shift_remove(&name[..]);
                    let v = std::mem::replace(&mut *cell.borrow_mut(), Value::Null);
                    drop(v);
                    self.collect_released_now()?;
                    changed = true;
                    continue;
                }
                let h = match &*cell.borrow() {
                    Value::Object(h) => h.clone(),
                    _ => continue,
                };
                let ptr = Rc::as_ptr(&h) as usize;
                let idx = self
                    .destructibles
                    .iter()
                    .position(|d| Rc::as_ptr(d) as usize == ptr);
                // owners: the global + (if tracked) our list + our clone
                if Rc::strong_count(&h) != 2 + idx.is_some() as usize {
                    continue;
                }
                drop(h);
                self.globals.shift_remove(&name[..]);
                let v = std::mem::replace(&mut *cell.borrow_mut(), Value::Null);
                if let Some(i) = idx {
                    let h = self.destructibles.remove(i);
                    self.call_destructor(h)?;
                }
                drop(v);
                // Objects only the freed one held die with it.
                self.collect_released_now()?;
                changed = true;
            }
            if !changed {
                break;
            }
        }
        // Then every object still alive, in creation order (objects created
        // by destructors included). From here on freed handles are not
        // reused (zend_objects_store_call_destructors).
        rhp_values::handles::stop_object_id_reuse();
        while !self.destructibles.is_empty() {
            let batch = std::mem::take(&mut self.destructibles);
            let mut it = batch.into_iter();
            while let Some(h) = it.next() {
                if let Err(e) = self.call_destructor(h) {
                    let rest: Vec<_> = it.collect();
                    self.destructibles.splice(0..0, rest);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Execute a compiled program; returns the process exit code.
    fn run(&mut self, prog: &Program) -> Result<i32, Thrown> {
        // The request's memory accounting starts here, with its limits
        // (compiling the main script doesn't count, as with opcache).
        builtins::apply_memory_limits();
        rhp_values::mem::begin_request();
        // Bind the standard stream resources (CLI SAPI) once.
        if self.streams.is_empty() {
            // Forget drops left over from a previous VM on this thread.
            rhp_values::take_dropped_streams();
            let stdin = self.make_stream(StreamState::Stdin, "stream");
            let stdout = self.make_stream(StreamState::Stdout, "stream");
            let stderr = self.make_stream(StreamState::Stderr, "stream");
            if builtins::main::sapi::sapi_name() == "cli" {
                self.constants.insert(b"STDIN".to_vec(), stdin);
                self.constants.insert(b"STDOUT".to_vec(), stdout);
                self.constants.insert(b"STDERR".to_vec(), stderr);
            } else {
                // Only php-cli defines the standard stream constants (web
                // code tests `defined('STDIN')` to detect a console).
                for k in [
                    &b"STDIN"[..],
                    b"STDOUT",
                    b"STDERR",
                    b"PHP_CLI_PROCESS_TITLE",
                ] {
                    self.constants.remove(k);
                }
            }
            // php_request_startup: the output_handler / output_buffering
            // buffer opens before php_hash_environment builds the request
            // superglobals (whose warnings then land in it).
            self.output_startup();
            self.seed_superglobals();
        }
        self.emit_compile_diags(prog);
        // Install (hoist) all declared functions, keyed case-insensitively.
        for (idx, proto) in prog.functions.iter().enumerate() {
            // Engine helpers are builtins, not prelude functions, so a user
            // function may take any helper's name.
            debug_assert!(
                idx >= prog.num_prelude_functions
                    || !crate::calls::is_helper_function(proto.name.as_bytes())
            );
            self.functions.push(UnitFn {
                proto: proto.clone(),
                unit: 0,
            });
            self.add_fn_name(proto.name.as_bytes().to_ascii_lowercase(), idx);
        }
        // The main script is unit 0: register its anonymous closure /
        // arrow-function protos (indexed by fnidx).
        self.decl_classes.clear();
        self.decl_runtime.clear();
        let unit = self.add_unit(prog);
        debug_assert_eq!(unit, 0);
        // Link the built-in prelude classes, then hoist the user's top-level
        // classes (those whose dependencies are available); the rest, and
        // nested declarations, are declared when execution reaches them.
        // (Engine data php has before a request starts: not counted.)
        rhp_values::mem::uncounted(|| {
            self.install_prelude(&prog.classes[..prog.num_prelude_classes])
        })?;
        self.internal_classes = self.classes.len();
        let top = self.register_decl_classes(prog, unit);
        // Preloaded functions and classes exist before the script compiles.
        self.install_preload();
        self.hoist_decl_classes(top)?;
        let mut main_frame = Frame::new(prog.main.clone(), unit, 0);
        self.bind_superglobals(&mut main_frame);
        // {main}'s variables are the global symbol table's entries.
        self.bind_global_scope(&mut main_frame);
        self.stack.push(main_frame);
        // zend_execute_script raises an out-of-memory error still pending
        // when the script ends.
        match self.run_loop().and_then(|_| self.oom_interrupt()) {
            Ok(()) => Ok(self.sapi.startup_status),
            Err(Thrown::Exit(code)) => Ok(code),
            Err(other) => Err(other),
        }
    }

    /// Push php's "no user code running" frame: callbacks invoked after the
    /// script (exception handler, shutdown functions) are called from
    /// `[internal function]`.
    fn push_internal_main(&mut self) {
        let mut main = FunctionProto::new(rhp_values::PhpStr::from("{main}"));
        main.debug.file = rhp_values::PhpStr::from("");
        main.flags.insert(rhp_bytecode::ProtoFlags::INTERNAL);
        self.stack.push(Frame::new(Rc::new(main), 0, 0));
    }

    /// Run the script and php's request shutdown (php_execute_script +
    /// php_request_shutdown): an uncaught throwable goes to the exception
    /// handler or is reported, then the shutdown functions run (a fatal or
    /// `exit` in one stops the rest), then every output buffer is flushed.
    /// Returns the exit status; `Err` only for an engine refusal.
    pub fn run_main(&mut self, prog: &Program) -> Result<i32, Thrown> {
        let status = match self.run(prog) {
            Ok(c) => c,
            Err(t) => {
                self.enter_shutdown_scope();
                if t.is_catchable() {
                    // The unwound frames' locals were destroyed on the way out
                    // (php runs their destructors before the exception reaches
                    // the handler or the fatal report).
                    self.dtor_pending_checks = usize::MAX / 2;
                    let _ = self.collect_released();
                }
                self.top_level_throw(t)?.unwrap_or(0)
            }
        };
        let status = self.shutdown_sequence(status, &mut |vm, t| vm.top_level_throw(t))?;
        // php_request_shutdown sends the headers if no output did: a throw
        // from the header_register_callback callback is reported there.
        self.sapi_send_headers();
        if let Some(t) = self.pending_throw.take() {
            return Ok(self.top_level_throw(t)?.unwrap_or(255));
        }
        Ok(status)
    }
}

/// php's auto-globals (`$_SERVER`, …, `$GLOBALS`): bound by name in every
/// scope, never as a plain global-scope variable.
fn is_auto_global(name: &[u8]) -> bool {
    matches!(
        name,
        b"GLOBALS"
            | b"_SERVER"
            | b"_GET"
            | b"_POST"
            | b"_REQUEST"
            | b"_SESSION"
            | b"_ENV"
            | b"_COOKIE"
            | b"_FILES"
    )
}
