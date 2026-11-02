//! The register bytecode VM.
//!
//! Executes a `rhp_bytecode::Program`. Every arithmetic/compare/concat/cast
//! opcode is a thin wrapper over the verified `rhp_semantics::ops` / value
//! `conv` functions, and calls go through `rhp_builtins` — so the VM and the
//! tree-walking oracle are semantically identical by construction, and any
//! differential mismatch localizes to a compiler-lowering bug (spec §2.3).
//!
//! Phase-1 scope: a single `{main}` frame, builtin calls, no user functions /
//! exceptions yet (the explicit call stack + exception table are in place for
//! those phases).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use classes::{Deny, PropRes};
use rhp_builtins::{self as builtins, Host};
use rhp_bytecode::{
    ClassProto, ClassRef, CmpKind, ConstInitProto, Default as ParamDefault, FunctionProto,
    IncludeKind, Instr, NIdx, Program, PropDefaultProto, Reg,
};
use rhp_semantics::{cast_to_array, inc_dec_value, ops};
use rhp_values::{
    conv, ArrayKey, ClassId, ClassKind, ObjectData, ObjectHandle, PhpArray, PhpStr, Thrown, Value,
};

/// A pending lvalue step (an evaluated key, an append, or an object property).
#[derive(Clone)]
enum CursorStep {
    Key(Value),
    Append,
    Prop(rhp_values::PhpStr),
}

/// Where a cursor path first meets a property php routes through `__get`
/// (missing or inaccessible on its object, with an unguarded `__get`).
enum MagicSplit {
    /// No such property: the plain path functions handle the whole path.
    Plain,
    /// The final step is such a property of this object.
    Leaf(ObjectHandle, PhpStr),
    /// Step `.0` (not the last) is such a property: writes below it go into
    /// whatever `__get` returns.
    Nested(usize, ObjectHandle, PhpStr),
    /// Step `.0` (a key / append) indexes this ArrayAccess object: the leaf
    /// is `offsetSet`/`offsetUnset`, a deeper path goes through `offsetGet`.
    Offset(usize, ObjectHandle),
}

/// How an ArrayAccess element is fetched below a nested write (php's
/// BP_VAR_W / BP_VAR_RW / BP_VAR_UNSET).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DimFetch {
    W,
    Rw,
    Unset,
}

/// SAPI-injected callback that parses + compiles an included file's source into
/// a `Program`, keeping the VM compiler-agnostic (see the `includer` field).
type Includer = Box<dyn FnMut(&str, &[u8]) -> Result<Program, IncludeError>>;

/// Why the SAPI's includer could not produce a `Program`.
#[derive(Debug)]
pub enum IncludeError {
    /// A syntax error: becomes a catchable `ParseError` at (file, `line`).
    Parse { message: String, line: u32 },
    /// Anything else (a compile-time Error, an unsupported construct, …).
    Thrown(Thrown),
}

impl From<Thrown> for IncludeError {
    fn from(t: Thrown) -> Self {
        IncludeError::Thrown(t)
    }
}

/// The virtual machine: output sink + call stack + global tables.
pub struct Vm {
    out: Vec<u8>,
    /// Active magic-accessor guards: (object id, accessor, property name).
    prop_guards: std::collections::HashSet<(u32, &'static [u8], PhpStr)>,
    /// Write output directly to stdout instead of `out` (the CLI).
    stdout_passthrough: bool,
    /// Output-buffering stack (`ob_start`): `echo`/`print` write to the top
    /// buffer when non-empty, else straight to `out`.
    ob_stack: Vec<Vec<u8>>,
    stack: Vec<Frame>,
    functions: Vec<Rc<FunctionProto>>,
    fn_names: HashMap<Vec<u8>, usize>, // lowercase name -> index into `functions`
    closures: Vec<Rc<FunctionProto>>,  // anonymous closure protos, indexed by `fnidx`
    /// Flat table of user class declarations (`class_base + DeclareClass`
    /// operand): each file's top-level class-likes then its nested ones, with
    /// whether the declaration has already happened (hoisted or executed).
    decl_classes: Vec<(ClassProto, bool)>,
    /// `decl_classes` slots declared by an executed `DeclareClass` (not
    /// early-bound): running that declaration again is a redeclaration.
    decl_runtime: std::collections::HashSet<usize>,
    constants: HashMap<Vec<u8>, Value>,
    /// User-defined constant names in definition order (`get_defined_constants`).
    user_const_order: Vec<Vec<u8>>,
    max_depth: usize,
    /// Builtin calls in progress (innermost last), for exception traces.
    native_calls: Vec<NativeCall>,
    /// `set_exception_handler` stack (innermost last; null = none).
    exception_handlers: Vec<Value>,
    /// `register_shutdown_function` callbacks with their arguments, in order.
    shutdown_functions: Vec<(Value, Vec<Value>)>,
    /// CLI mode: unbuffered stdout is written to the process's stdout before
    /// anything goes to stderr, so the two streams interleave as in php.
    direct_stdout: bool,
    // lvalue cursor: a root in the current frame (CV or a static-prop cell) +
    // pending path steps. Cursor opcodes are emitted contiguously, so a single
    // cursor never nests.
    cursor_root: CursorRoot,
    cursor_steps: Vec<CursorStep>,
    // The global's name when the open cursor is rooted at `$GLOBALS[name]`
    // (`$GLOBALS[name] = &$v` rebinds the symbol-table entry itself).
    cursor_global: Option<Vec<u8>>,
    // OOP class table (see `classes` module).
    classes: Vec<classes::ClassEntry>,
    class_ids: HashMap<Vec<u8>, ClassId>, // lowercase name -> id
    next_object_id: u32,
    // The global symbol table (php's EG(symbol_table)), in insertion order:
    // one shared cell per global variable. {main}'s CVs, top-level included
    // files, `global $x` and `$GLOBALS[...]` all alias these cells.
    globals: indexmap::IndexMap<Vec<u8>, Rc<RefCell<Value>>>,
    // Auto-globals php creates on first use (auto_globals_jit): their initial
    // value, moved into `globals` when a script first references the name.
    jit_globals: HashMap<Vec<u8>, Value>,
    // Typed property slots that were `unset()` (object id -> storage keys):
    // php clears a slot's IS_PROP_UNINIT flag on unset, after which an absent
    // slot defers to the magic accessors; a never-initialized one does not.
    unset_typed_props: HashMap<u32, Vec<PhpStr>>,
    // Globals that became PHP references (`global $x`, `=&`, by-ref passing):
    // php's shutdown sweep only destroys plain (non-reference) object globals.
    ref_globals: std::collections::HashSet<Vec<u8>>,
    // `static $x`: persistent cells keyed by (function proto pointer, var name).
    statics: HashMap<StaticKey, Rc<RefCell<Value>>>,
    // Non-constant class const / static-prop / property-default initializers,
    // evaluated after all classes link (so cross-class/enum refs resolve).
    // Generator runtime state, keyed by the Generator object's id.
    generators: HashMap<u32, GenState>,
    // Set by the `Yield` opcode to hand (key, value, resume-dst) to the driver.
    yield_out: Option<(Value, Value, Reg)>,
    // Stack of generator ids currently being advanced (innermost last).
    gen_stack: Vec<u32>,
    // Default scale for the bcmath builtins (`bcscale()`); 0 at script start.
    bc_scale: i64,
    // `include`/`require`: a callback (injected by the SAPI) that parses+compiles
    // an included file's source into a Program, keeping the VM compiler-agnostic.
    includer: Option<Includer>,
    /// The scope a first-class callable was created in, while its wrapped
    /// callable is resolved (php resolves `$this->priv(...)` at creation).
    fcc_scope: std::cell::Cell<Option<(Option<ClassId>, Option<ClassId>)>>,
    // Resolved real paths already included (for the `_once` variants).
    included_files: std::collections::HashSet<Vec<u8>>,
    // Registered `spl_autoload_register` callables, in call order.
    autoloaders: Vec<Value>,
    /// Lowercased class names whose autoload is in progress (php's
    /// `EG(in_autoload)`): a nested lookup of the same name does not re-enter
    /// the autoloaders.
    in_autoload: std::collections::HashSet<Vec<u8>>,
    /// Named-argument binding for the frame about to be pushed: the skipped
    /// parameter mask and the extra named arguments collected by a variadic
    /// parameter. Set by a call instruction right before the push.
    pending_named: Option<(u64, Vec<(PhpStr, Value)>)>,
    /// Set while a statically resolved `call_user_func[_array]()` builtin runs:
    /// the caller's `strict_types`, consumed by its single callback dispatch.
    cuf_caller: Option<bool>,
    // Stream wrappers by scheme (`file`, `phar`, …), installed at startup from
    // the module list; runtime-mutable later via `stream_wrapper_register`.
    wrappers: HashMap<Vec<u8>, Rc<dyn builtins::StreamWrapper>>,
    // Filesystem path of the top-level script (for `Phar::mapPhar`).
    main_script_path: Option<Vec<u8>>,
    // Open stream resources (fopen family), keyed by resource id.
    streams: HashMap<u32, StreamState>,
    /// Streams opened on `php://output`: writes go through the output
    /// buffers like `echo` (unlike `php://stdout`).
    output_streams: std::collections::HashSet<u32>,
    /// Readonly properties already reinitialized inside a `__clone`
    /// (object id, storage key): once each.
    readonly_reinit: std::collections::HashSet<(u32, PhpStr)>,
    /// Streams switched to non-blocking mode (stream_get_meta_data's `blocked`).
    nonblocking: std::collections::HashSet<u32>,
    /// Descriptor streams opened by name (`php://stdin`): their uri and mode.
    fd_names: HashMap<u32, (Vec<u8>, Vec<u8>)>,
    // The fopen() mode of each stream opened by stream_open.
    stream_modes: HashMap<u32, Vec<u8>>,
    next_resource_id: u32,
    // The most recently opened directory stream (`readdir()` with no handle).
    // The last opened directory handle (php's DIRG(default_dir), which holds
    // a reference so the handle stays open).
    last_dir: Option<Value>,
    // The lazily-allocated default stream context resource id.
    default_context: Option<u32>,
    // (uri, mode) of streams opened through a non-local wrapper (phar://…),
    // for fstat / stream_get_meta_data.
    stream_origins: HashMap<u32, (Vec<u8>, Vec<u8>)>,
    // Stream-context resources (`stream_context_create`), id -> options array.
    stream_contexts: HashMap<u32, Value>,
    // Response header lines from the last http(s) request (status line first).
    last_response_headers: Vec<Vec<u8>>,
    // Open ZipArchive handles, id -> parsed archive.
    // CLI arguments ($argv / $_SERVER['argv']); argv[0] is the script name.
    argv: Vec<Vec<u8>>,
    // `@` nesting depth: diagnostics are recorded but not displayed while > 0.
    silence: u32,
    // Active `@` regions: (frame depth, silence level to restore).
    silence_stack: Vec<(usize, u32)>,
    // The last raised diagnostic (`error_get_last`): type, message, file, line.
    last_error: Option<(i64, Vec<u8>, Vec<u8>, i64)>,
    /// `set_error_handler` stack (innermost last): (callback or null, mask).
    error_handlers: Vec<(Value, i64)>,
    /// A user error handler is running (php unsets the handler meanwhile).
    /// An exception a user error handler threw while a builtin (or the
    /// engine) raised a diagnostic: php's pending `EG(exception)`, thrown
    /// once the builtin returns.
    pending_throw: Option<Thrown>,
    // Opaque native resources (e.g. `proc_open` process handles), by id.
    native_resources: HashMap<u32, Box<dyn std::any::Any>>,
    // The `STDIN` constant's reader: a buffered stream over a dup of fd 0
    // (created on first read).
    stdin_reader: Option<builtins::fdstream::FdStream>,
    // Live objects whose class has a `__destruct` (strong handles, creation
    // order). An entry whose only owner is this list has been released by the
    // program: its destructor runs at the next check (`dtor_check`), as php's
    // refcount release would. Whatever is left runs at request shutdown.
    destructibles: Vec<ObjectHandle>,
    // Set when a reference to an object may have been dropped (a frame
    // returned, a variable holding an object was overwritten or unset).
    dtor_check: bool,
    // Checks requested since the last full scan of `destructibles` (scans of a
    // long list are rationed to keep the cost amortized).
    dtor_pending_checks: usize,
    // Constructor frames in flight: (stack index, object pointer). A
    // constructor that throws leaves its object marked destructed (php's
    // `zend_object_store_ctor_failed`): its destructor never runs.
    ctor_frames: Vec<(usize, usize)>,
    // The {main} frame once the script finished (returned, exited or threw):
    // kept so shutdown functions and destructors still see the globals.
    finished_main: Option<Frame>,
    // The shutdown sequence's stack ({main} as an internal frame) is set up.
    in_shutdown_scope: bool,
    // A fatal error bailed out (php_error_cb's zend_objects_store_mark_destructed):
    // objects with an id below this never run their destructor (shutdown
    // functions still run). 0: no bailout.
    destructed_below: u32,
    // Per-run extension state (`Host::module_state`), by module key.
    module_states: HashMap<&'static str, Box<dyn std::any::Any>>,
}

/// Whether `name` is a builtin visible to user code. `__`-prefixed builtins
/// are engine-internal helpers behind prelude classes (no php function
/// starts with `__`), so function_exists/is_callable don't report them.
fn user_visible_builtin(name: &[u8]) -> bool {
    !name.starts_with(b"__") && builtins::lookup(&String::from_utf8_lossy(name)).is_some()
}

/// The shutdown sequence's handler for an `exit` / uncaught throwable:
/// `Some(code)` ends the current step with that exit status.
type ShutdownThrow<'a> = dyn FnMut(&mut Vm, Thrown) -> Result<Option<i32>, Thrown> + 'a;

/// php's memory-stream mode (`TEMP_STREAM_READONLY` / `_APPEND` / default).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MemMode {
    ReadWrite,
    ReadOnly,
    Append,
}

/// The backing state of an open stream resource.
enum StreamState {
    /// Standard output / `php://output` — writes to the VM output sink.
    Stdout,
    /// Standard error — writes to the process stderr.
    Stderr,
    /// Standard input (reads return EOF for now).
    Stdin,
    /// An in-memory / preloaded-file buffer (`php://memory`, a file opened for
    /// reading): read from `pos`, writes append.
    /// `skew` is php's `stream->position` minus the buffer offset: they
    /// diverge after a failed seek (position -1) or a shrinking ftruncate,
    /// which moves only the memory stream's own offset.
    Mem {
        buf: Vec<u8>,
        pos: usize,
        skew: i64,
        /// php's `stream->eof`: set by a read that found no more data,
        /// cleared by a successful seek.
        eof: bool,
        /// The URI of a `php://temp` stream (a TEMP stream wrapping a
        /// MEMORY one); `None` for memory / wrapper-backed buffers.
        temp: Option<Vec<u8>>,
        /// php's `TEMP_STREAM_*` mode (read-only, append, read/write).
        mode: MemMode,
    },
    /// A file opened for writing: buffered, flushed to the wrapper on
    /// flush/close. `append` keeps existing content. (Non-local wrappers.)
    FileWrite { buf: Vec<u8>, url: Vec<u8> },
    /// A plain local file with a real descriptor.
    File(Box<builtins::streams::LocalFile>),
    /// A directory stream (`opendir`): the entries snapshot + read position.
    Dir {
        entries: Vec<Vec<u8>>,
        pos: usize,
        url: Vec<u8>,
    },
    /// An OS file descriptor stream: a `proc_open` pipe's parent end or
    /// `php://stdin` (a dup of fd 0).
    Fd(builtins::fdstream::FdStream),
}

/// Status of a generator.
#[derive(PartialEq, Eq, Clone, Copy)]
enum GenStatus {
    NotStarted,
    Suspended,
    Running,
    Finished,
}

/// A generator's suspended execution state.
struct GenState {
    /// The suspended frame (taken out while running). `None` while running/finished.
    frame: Option<Frame>,
    status: GenStatus,
    current_key: Value,
    current_val: Value,
    return_val: Value,
    /// Next auto-increment integer key for keyless `yield`.
    auto_key: i64,
    /// Register in the frame that receives the `send()` value on resume.
    resume_dst: Reg,
    /// php's ZEND_GENERATOR_AT_FIRST_YIELD: started, and not resumed since
    /// (only then can it be rewound).
    at_first_yield: bool,
}

mod classes;
mod compare;

/// A function-static slot: (proto address, class it runs as, variable name).
type StaticKey = (usize, Option<ClassId>, Vec<u8>);

/// Named call arguments, in call order.
type NamedArgs = Vec<(PhpStr, Value)>;

/// Where an lvalue cursor is rooted.
#[derive(Clone)]
enum CursorRoot {
    /// A register (compiled variable) in the current frame.
    Cv(Reg),
    /// A shared static-property cell.
    Cell(Rc<RefCell<Value>>),
}

impl Host for Vm {
    fn raise_diagnostic(&mut self, level: i64, msg: &str) {
        self.raise_error(level, msg);
    }

    fn take_pending_throw(&mut self) -> Option<Thrown> {
        self.pending_throw.take()
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

    fn silenced(&self) -> bool {
        self.silence > 0
    }

    fn last_error(&self) -> Option<(i64, Vec<u8>, Vec<u8>, i64)> {
        self.last_error.clone()
    }

    fn clear_last_error(&mut self) {
        self.last_error = None;
    }

    fn compare_cx(&mut self) -> Option<&mut dyn rhp_semantics::ops::CompareCx> {
        Some(self)
    }

    fn write_out(&mut self, bytes: &[u8]) {
        match self.ob_stack.last_mut() {
            Some(buf) => buf.extend_from_slice(bytes),
            None => self.emit_stdout(bytes),
        }
    }

    fn ob_start(&mut self) {
        self.ob_stack.push(Vec::new());
    }

    fn ob_get_contents(&self) -> Option<Vec<u8>> {
        self.ob_stack.last().cloned()
    }

    fn ob_get_level(&self) -> usize {
        self.ob_stack.len()
    }

    /// Pop the top buffer and return its contents (`ob_get_clean`).
    fn ob_get_clean(&mut self) -> Option<Vec<u8>> {
        self.ob_stack.pop()
    }

    /// Pop the top buffer and discard it (`ob_end_clean`).
    fn ob_end_clean(&mut self) -> bool {
        self.ob_stack.pop().is_some()
    }

    /// Pop the top buffer and write its contents to the next sink (`ob_end_flush`).
    fn ob_end_flush(&mut self) -> bool {
        match self.ob_stack.pop() {
            Some(buf) => {
                self.write_out(&buf);
                true
            }
            None => false,
        }
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

    fn define_const(&mut self, name: &[u8], value: Value) -> bool {
        if self.constants.contains_key(name) {
            return false;
        }
        self.constants.insert(name.to_vec(), value);
        self.user_const_order.push(name.to_vec());
        true
    }

    fn user_constants(&self) -> Vec<(Vec<u8>, Value)> {
        self.user_const_order
            .iter()
            .filter_map(|n| self.constants.get(n).map(|v| (n.clone(), v.clone())))
            .collect()
    }

    fn output_started(&self) -> bool {
        !self.out.is_empty()
    }

    fn get_const(&self, name: &[u8]) -> Option<Value> {
        self.constants.get(name).cloned()
    }

    fn bc_scale(&self) -> i64 {
        self.bc_scale
    }

    fn set_bc_scale(&mut self, scale: i64) {
        self.bc_scale = scale;
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
        if url.eq_ignore_ascii_case(b"php://stdin") {
            // A fresh php://stdin stream (a dup of fd 0) read to EOF.
            let fd = builtins::fdstream::dup_raw(0)?;
            return Some(builtins::fdstream::FdStream::new(fd).read_all(-1));
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
        // The `php://` pseudo-wrapper is handled specially (not in the map).
        let mut v: Vec<Vec<u8>> = vec![b"php".to_vec()];
        v.extend(self.wrappers.keys().cloned());
        v
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
        Value::Resource(Rc::new(rhp_values::ResourceData {
            id,
            type_name: "stream-context",
        }))
    }

    fn stream_context_options(&self, ctx: &Value) -> Value {
        if let Value::Resource(r) = ctx {
            if let Some(opts) = self.stream_contexts.get(&r.id) {
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
        let entry = self
            .stream_contexts
            .entry(r.id)
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

    fn set_http_response_headers(&mut self, headers: Vec<Vec<u8>>) {
        self.last_response_headers = headers;
    }
    fn http_response_headers(&self) -> Vec<Vec<u8>> {
        self.last_response_headers.clone()
    }
    fn clear_http_response_headers(&mut self) {
        self.last_response_headers.clear();
    }

    fn current_arg_count(&self) -> i64 {
        self.stack.last().map(|f| f.argc as i64).unwrap_or(0)
    }
    fn current_args(&self) -> Vec<Value> {
        let Some(f) = self.stack.last() else {
            return Vec::new();
        };
        let n = (f.argc).min(f.proto.params.len());
        let mut out = Vec::new();
        for p in f.proto.params.iter().take(n) {
            out.push(deref(&f.regs[p.cv as usize]).clone());
        }
        out.extend(f.extra_args.iter().cloned());
        out
    }

    fn running_main_path(&self) -> Option<Vec<u8>> {
        self.main_script_path.clone()
    }

    fn has_streams(&self) -> bool {
        true
    }

    fn supports_signals(&self) -> bool {
        true
    }

    fn stream_open(&mut self, path: &[u8], mode: &[u8]) -> Option<Value> {
        let writable = mode
            .iter()
            .any(|&b| matches!(b, b'w' | b'a' | b'x' | b'c' | b'+'));
        let append = mode.first() == Some(&b'a');
        if path == b"php://output" {
            let v = self.make_stream(StreamState::Stdout, "stream");
            if let Some(id) = res_id(&v) {
                self.output_streams.insert(id);
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
                StreamState::Fd(builtins::fdstream::FdStream::new(
                    builtins::fdstream::dup_raw(0)?,
                )),
                "stream",
            );
            if let Some(id) = res_id(&v) {
                self.fd_names.insert(id, (path.to_vec(), mode.to_vec()));
            }
            return Some(v);
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
        } else if path.starts_with(b"php://") {
            return None; // other php:// streams unsupported
        } else if builtins::streams::split_scheme(path).0 == b"file" {
            // Plain local file: a real descriptor (seek/tell/flock/ftruncate).
            StreamState::File(Box::new(builtins::streams::LocalFile::open(path, mode)?))
        } else if writable {
            let origin = (path.to_vec(), mode.to_vec());
            let zlib = builtins::streams::split_scheme(path).0 == b"compress.zlib";
            if zlib {
                // The zlib wrapper opens (creating / truncating) the inner
                // plain file right away: a resource of its own.
                let inner = &path[b"compress.zlib://".len()..];
                if !inner.windows(3).any(|w| w == b"://") {
                    let mut o = std::fs::OpenOptions::new();
                    if append {
                        o.append(true).create(true);
                    } else {
                        o.write(true).create(true).truncate(!append);
                    }
                    o.open(String::from_utf8_lossy(inner).as_ref()).ok()?;
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
            if let Some(id) = res_id(&v) {
                self.stream_origins.insert(id, origin);
            }
            return Some(v);
        } else {
            let data = self.wrapper_for(path).and_then(|w| w.get_contents(path))?;
            if builtins::streams::split_scheme(path).0 == b"compress.zlib" {
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
            if let Some(id) = res_id(&v) {
                self.stream_origins
                    .insert(id, (path.to_vec(), mode.to_vec()));
            }
            return Some(v);
        };
        let res = self.make_stream(state, "stream");
        if let Some(id) = res_id(&res) {
            self.stream_modes.insert(id, mode.to_vec());
        }
        Some(res)
    }

    fn stream_is_open(&self, res: &Value) -> bool {
        res_id(res).is_some_and(|id| self.streams.contains_key(&id))
    }

    fn stream_mode(&self, res: &Value) -> Option<Vec<u8>> {
        let id = res_id(res)?;
        if let Some(m) = self
            .stream_modes
            .get(&id)
            .or_else(|| self.stream_origins.get(&id).map(|(_, m)| m))
        {
            return Some(m.clone());
        }
        // Memory / temp streams (returned before the mode is recorded).
        match self.streams.get(&id)? {
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
            StreamState::Fd(f) => return f.write(data),
            StreamState::Stdin | StreamState::Dir { .. } => Act::No,
        };
        match act {
            Act::Out if self.output_streams.contains(&id) => self.write_out(data),
            Act::Out => self.emit_stdout(data),
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
        match self.streams.get_mut(&id)? {
            StreamState::Mem { buf, pos, eof, .. } => {
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
                if len < 0 {
                    Some(f.read_all(-1))
                } else {
                    f.read(len as usize)
                }
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
        self.stream_gets_impl(res, None)
    }

    fn stream_gets_max(&mut self, res: &Value, max: usize) -> Option<Vec<u8>> {
        self.stream_gets_impl(res, Some(max))
    }

    fn stream_eof(&self, res: &Value) -> bool {
        match res_id(res).and_then(|id| self.streams.get(&id)) {
            // Wrapper-backed buffers (phar://, compress.zlib://, …) are
            // buffered streams in php: eof once everything has been consumed.
            Some(StreamState::Mem { buf, pos, .. })
                if res_id(res).is_some_and(|id| self.stream_origins.contains_key(&id)) =>
            {
                *pos >= buf.len()
            }
            // php://memory and php://temp are unbuffered: eof is the flag a
            // read sets when it runs out of data.
            Some(StreamState::Mem { eof, .. }) => *eof,
            Some(StreamState::File(f)) => f.eof(),
            Some(StreamState::Fd(f)) => f.eof(),
            Some(StreamState::Stdin) => self.stdin_reader.as_ref().is_some_and(|f| f.eof()),
            Some(StreamState::Dir { .. }) => false,
            Some(StreamState::Stdout | StreamState::Stderr | StreamState::FileWrite { .. }) => {
                false
            }
            _ => true,
        }
    }

    fn backtrace(&self, provide_object: bool, ignore_args: bool) -> Option<Value> {
        let full = self.build_trace_ex(provide_object, ignore_args);
        // Drop the running debug_backtrace()/debug_print_backtrace() frame itself.
        let skip = usize::from(
            self.native_calls
                .last()
                .is_some_and(|n| n.caller + 1 == self.stack.len()),
        );
        let mut out = PhpArray::new();
        for (_, v) in full.iter().skip(skip) {
            out.push(v.clone());
        }
        Some(Value::Array(out))
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
        self.shutdown_functions.push((callback, args));
        Ok(())
    }

    fn stream_from_fd(&mut self, fd: std::os::fd::OwnedFd) -> Option<Value> {
        let st = StreamState::Fd(builtins::fdstream::FdStream::new(fd));
        Some(self.make_stream(st, "stream"))
    }

    fn stream_dup_fd(&mut self, res: &Value) -> Result<std::os::fd::OwnedFd, &'static str> {
        use builtins::fdstream::dup_raw;
        let id = res_id(res).ok_or("Unknown")?;
        let fail = |o: Option<std::os::fd::OwnedFd>| o.ok_or("STDIO");
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
                fail(builtins::fdstream::dup_raw(f.fd()))
            }
            StreamState::Mem { temp: Some(_), .. } => Err("TEMP"),
            _ if self.stream_origins.contains_key(&id) => Err("Unknown"),
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
        if self.streams.contains_key(&id) {
            if blocking {
                self.nonblocking.remove(&id);
            } else {
                self.nonblocking.insert(id);
            }
        }
        match self.streams.get_mut(&id) {
            Some(StreamState::Fd(f)) => f.set_blocking(blocking),
            Some(StreamState::Stdin) => builtins::fdstream::set_fd_blocking(0, blocking),
            Some(StreamState::Stdout) => builtins::fdstream::set_fd_blocking(1, blocking),
            Some(StreamState::Stderr) => builtins::fdstream::set_fd_blocking(2, blocking),
            Some(_) => true,
            None => false,
        }
    }

    fn resource_insert(&mut self, type_name: &'static str, data: Box<dyn std::any::Any>) -> Value {
        let id = self.next_resource_id;
        self.next_resource_id += 1;
        self.native_resources.insert(id, data);
        Value::Resource(Rc::new(rhp_values::ResourceData { id, type_name }))
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
        self.free_stream(id)
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
            StreamState::Mem { .. } | StreamState::FileWrite { .. } => K::Memory,
            StreamState::Stdout | StreamState::Stderr | StreamState::Stdin => K::Stdio,
        })
    }

    fn stream_seek(&mut self, res: &Value, offset: i64, whence: i32) -> bool {
        let Some(id) = res_id(res) else { return false };
        match self.streams.get_mut(&id) {
            Some(StreamState::File(f)) => f.seek(offset, whence),
            Some(StreamState::Fd(f)) => f.seek(offset, whence),
            Some(StreamState::Stdin) => match self.stdin_stream() {
                Some(f) => f.seek(offset, whence),
                None => false,
            },
            Some(StreamState::Mem {
                buf,
                pos,
                skew,
                eof,
                ..
            }) => {
                // _php_stream_seek + php_stream_memory_seek: a negative
                // SEEK_SET fails without touching the stream; SEEK_CUR is
                // resolved against php's position; any other failure resets
                // the buffer offset to 0 and php's position to -1. Seeking
                // past the end is allowed (the next write zero-fills).
                let target = match whence {
                    0 if offset < 0 => return false,
                    0 => offset,
                    1 => (*pos as i64 + *skew).saturating_add(offset),
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
        if let Some((uri, _)) = self.stream_origins.get(&id) {
            let uri = uri.clone();
            return self.wrapper_for(&uri)?.stat(&uri);
        }
        match self.streams.get(&id)? {
            StreamState::File(f) => f.stat(),
            StreamState::Fd(f) => builtins::streams::sys_fstat(f.raw_fd()?),
            StreamState::Stdin => builtins::streams::sys_fstat(0),
            StreamState::Stdout => builtins::streams::sys_fstat(1),
            StreamState::Stderr => builtins::streams::sys_fstat(2),
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
            if let Some((uri, mode)) = self.stream_origins.get(&id) {
                let (scheme, _) = builtins::streams::split_scheme(uri);
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
                });
            }
            Some(match self.streams.get(&id)? {
                StreamState::File(f) => M {
                    wrapper_type: "plainfile",
                    stream_type: "STDIO",
                    mode: f.mode.clone(),
                    uri: f.uri.clone(),
                    unread_bytes: f.unread() as i64,
                    seekable: true,
                    eof: f.eof(),
                    blocked: true,
                },
                StreamState::Fd(f) => match self.fd_names.get(&id) {
                    Some((uri, mode)) => M {
                        wrapper_type: "PHP",
                        stream_type: "STDIO",
                        mode: mode.clone(),
                        uri: uri.clone(),
                        unread_bytes: 0,
                        seekable: f.tell() != -1,
                        eof: f.eof(),
                        blocked: true,
                    },
                    // A pipe (proc_open, popen): no wrapper, no uri; the mode is
                    // the descriptor's direction.
                    None => M {
                        wrapper_type: "",
                        stream_type: "STDIO",
                        mode: match f.raw_fd().map(builtins::fdstream::fd_access_mode) {
                            Some(Some(true)) => b"w".to_vec(),
                            _ => b"r".to_vec(),
                        },
                        uri: Vec::new(),
                        unread_bytes: 0,
                        seekable: f.tell() != -1,
                        eof: f.eof(),
                        blocked: true,
                    },
                },
                StreamState::Dir { .. } => M {
                    wrapper_type: "plainfile",
                    stream_type: "dir",
                    mode: b"r".to_vec(),
                    uri: Vec::new(),
                    unread_bytes: 0,
                    seekable: true,
                    eof: false,
                    blocked: true,
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
                },
                st @ (StreamState::Stdin | StreamState::Stdout | StreamState::Stderr) => {
                    let (fd, mode, uri): (i32, &[u8], &[u8]) = match st {
                        StreamState::Stdin => (0, b"rb", b"php://stdin"),
                        StreamState::Stdout => (1, b"wb", b"php://stdout"),
                        _ => (2, b"wb", b"php://stderr"),
                    };
                    // php: seekable unless a fifo or character device.
                    let seekable = builtins::streams::sys_fstat(fd).is_some_and(|s| {
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
                    }
                }
            })
        };
        let mut m = meta()?;
        if self.nonblocking.contains(&id) {
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
        Value::Resource(Rc::new(rhp_values::ResourceData {
            id,
            type_name: "stream-context",
        }))
    }

    fn last_dir(&self) -> Option<Value> {
        let v = self.last_dir.as_ref()?;
        self.streams.get(&res_id(v)?)?;
        Some(v.clone())
    }

    // --- reflection -----------------------------------------------------------

    fn reflect_meta(&self, name: &[u8]) -> Option<Value> {
        let cid = self.resolve_class_by_name(name).ok()?;
        let entry = &self.classes[cid as usize];
        let full = entry.name.as_bytes().to_vec();
        let (short, ns) = match full.iter().rposition(|&b| b == b'\\') {
            Some(p) => (full[p + 1..].to_vec(), full[..p].to_vec()),
            None => (full.clone(), Vec::new()),
        };
        let mut a = PhpArray::new();
        a.insert(ArrayKey::from_bytes(b"name"), Value::str(full.clone()));
        a.insert(ArrayKey::from_bytes(b"shortName"), Value::str(short));
        a.insert(ArrayKey::from_bytes(b"namespaceName"), Value::str(ns));
        a.insert(
            ArrayKey::from_bytes(b"isAbstract"),
            Value::Bool(entry.flags.is_abstract),
        );
        a.insert(
            ArrayKey::from_bytes(b"isInterface"),
            Value::Bool(entry.kind == ClassKind::Interface),
        );
        a.insert(
            ArrayKey::from_bytes(b"isEnum"),
            Value::Bool(entry.kind == ClassKind::Enum),
        );
        a.insert(
            ArrayKey::from_bytes(b"isFinal"),
            Value::Bool(entry.flags.is_final),
        );
        a.insert(
            ArrayKey::from_bytes(b"isInstantiable"),
            Value::Bool(entry.kind == ClassKind::Class && !entry.flags.is_abstract),
        );
        a.insert(
            ArrayKey::from_bytes(b"parent"),
            match entry.parent {
                Some(p) => Value::str(self.classes[p as usize].name.as_bytes().to_vec()),
                None => Value::Bool(false),
            },
        );
        let mut ifaces = PhpArray::new();
        for &anc in &entry.interfaces {
            ifaces.push(Value::str(
                self.classes[anc as usize].name.as_bytes().to_vec(),
            ));
        }
        a.insert(ArrayKey::from_bytes(b"interfaces"), Value::Array(ifaces));
        let mut methods = PhpArray::new();
        for m in entry.methods.values() {
            methods.push(Value::str(m.proto.name.as_bytes().to_vec()));
        }
        a.insert(ArrayKey::from_bytes(b"methods"), Value::Array(methods));
        let mut consts = PhpArray::new();
        for (k, c) in &entry.constants {
            consts.insert(ArrayKey::from_bytes(k), c.value.clone());
        }
        a.insert(ArrayKey::from_bytes(b"constants"), Value::Array(consts));
        // Reflection lists properties most-derived class first (php's order),
        // while `entry.props` is the parent-first object layout `var_dump` uses.
        let mut props = PhpArray::new();
        let mut chain = vec![cid];
        while let Some(p) = self.classes[*chain.last().unwrap() as usize].parent {
            chain.push(p);
        }
        let rank = |c: ClassId| chain.iter().position(|&x| x == c).unwrap_or(chain.len());
        // Only the declaration each name resolves to on this class, minus a
        // parent's private (php's getProperties() never lists those).
        let mut keys: Vec<(usize, &PhpStr)> = entry
            .props
            .iter()
            .filter(|(k, p)| {
                entry.prop_visible.get(&p.name) == Some(*k)
                    && (p.visibility != rhp_values::Visibility::Private || p.declaring_class == cid)
            })
            .map(|(_, p)| (rank(p.declaring_class), &p.name))
            .collect();
        keys.sort_by_key(|&(r, _)| r); // stable: keeps declaration order within a class
        for (_, k) in keys {
            props.push(Value::str(k.as_bytes().to_vec()));
        }
        a.insert(ArrayKey::from_bytes(b"properties"), Value::Array(props));
        Some(Value::Array(a))
    }

    fn class_constant(&self, class: &[u8], name: &[u8]) -> Option<Value> {
        let cid = self.resolve_class_by_name(class).ok()?;
        self.classes[cid as usize]
            .constants
            .get(name)
            .map(|c| c.value.clone())
    }

    fn class_constant_eval(&mut self, class: &[u8], name: &[u8]) -> Result<Option<Value>, Thrown> {
        let Ok(cid) = self.resolve_class_by_name(class) else {
            return Ok(None);
        };
        if self.classes[cid as usize].enum_backing.is_some() {
            self.update_class_constants(cid)?;
        }
        self.class_const_value(cid, name, class, false)
    }

    fn update_class_constants(&mut self, class: &[u8]) -> Result<(), Thrown> {
        match self.resolve_class_by_name(class) {
            Ok(cid) => Vm::update_class_constants(self, cid),
            Err(_) => Ok(()),
        }
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

    fn reflect_set_prop(&mut self, obj: &Value, class: &[u8], name: &[u8], val: Value) -> bool {
        if let Value::Object(h) = deref(obj) {
            let key = self.reflect_prop_key(&h, class, name);
            let mut od = h.borrow_mut();
            *classes::prop_slot_mut(&self.classes, &mut od, key) = val;
            true
        } else {
            false
        }
    }

    fn reflect_get_prop(&self, obj: &Value, class: &[u8], name: &[u8]) -> Option<Value> {
        match deref(obj) {
            Value::Object(h) => {
                let key = self.reflect_prop_key(&h, class, name);
                let v = h.borrow().properties.get(&key).map(deref);
                v
            }
            _ => None,
        }
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
            if self.internal_slot(o.class, key) {
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

    fn reflect_class_constants(&self, class: &[u8]) -> Option<Value> {
        use rhp_values::Visibility;
        let cid = self.resolve_class_by_name(class).ok()?;
        let mut arr = PhpArray::new();
        for (name, c) in &self.classes[cid as usize].constants {
            let mut d = PhpArray::new();
            d.insert(ArrayKey::from_bytes(b"name"), Value::str(name.clone()));
            d.insert(ArrayKey::from_bytes(b"value"), c.value.clone());
            d.insert(
                ArrayKey::from_bytes(b"public"),
                Value::Bool(c.visibility == Visibility::Public),
            );
            d.insert(
                ArrayKey::from_bytes(b"protected"),
                Value::Bool(c.visibility == Visibility::Protected),
            );
            d.insert(
                ArrayKey::from_bytes(b"private"),
                Value::Bool(c.visibility == Visibility::Private),
            );
            arr.push(Value::Array(d));
        }
        Some(Value::Array(arr))
    }

    fn reflect_method_params(&self, class: &[u8], method: &[u8]) -> Option<Value> {
        let cid = self.resolve_class_by_name(class).ok()?;
        let m = self.lookup_method(cid, method)?;
        Some(self.params_to_value(&m.proto))
    }

    fn reflect_callable_params(&self, callable: &Value) -> Option<Value> {
        let proto = match deref(callable) {
            Value::Closure(cd) if cd.wrapped.is_none() => {
                self.closures.get(cd.fnidx as usize)?.clone()
            }
            Value::Str(s) => {
                let i = *self.fn_names.get(&s.as_bytes().to_ascii_lowercase())?;
                self.functions[i].clone()
            }
            _ => return None,
        };
        Some(self.params_to_value(&proto))
    }

    fn call_callable(&mut self, callable: &Value, args: &[Value]) -> Result<Value, Thrown> {
        // `call_user_func[_array]` compiled as a user call (see `cuf_caller`).
        let caller = self.cuf_caller.take();
        self.call_callable_value_mode(callable, args.to_vec(), caller)
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

    fn closure_bind(&self, closure: &Value, new_this: &Value, scope: &Value) -> Option<Value> {
        let cd = match deref(closure) {
            Value::Closure(cd) => cd,
            _ => return None,
        };
        let new_this_h = match deref(new_this) {
            Value::Object(h) => Some(h),
            _ => None,
        };
        // php's do_closure_bind: an object's class, `"static"` = keep the
        // closure's scope, a class name, or null = no scope. The called scope
        // (`static::`) is the new `$this`'s class, else the new scope.
        let scope_class = match deref(scope) {
            Value::Null => None,
            Value::Object(h) => Some(h.borrow().class),
            v => {
                let name = conv::to_string_bytes(&v);
                if name == b"static" {
                    cd.bound_class
                } else {
                    self.resolve_class_by_name(&name).ok().or(cd.bound_class)
                }
            }
        };
        let called = new_this_h
            .as_ref()
            .map(|h| h.borrow().class)
            .or(scope_class);
        Some(Value::Closure(Rc::new(rhp_values::ClosureData {
            fnidx: cd.fnidx,
            captures: cd.captures.clone(),
            bound_this: new_this_h,
            bound_class: scope_class,
            bound_static: called,
            wrapped: cd.wrapped.clone(),
        })))
    }

    fn class_alias(&mut self, class_id: u32, alias: &[u8]) -> bool {
        let key = alias
            .strip_prefix(b"\\")
            .unwrap_or(alias)
            .to_ascii_lowercase();
        if self.class_ids.contains_key(&key) {
            return false;
        }
        self.class_ids.insert(key, class_id);
        true
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
            Some(m) => !(is_prelude_proto(&m.proto) && is_engine_helper_name(method)),
            None => false,
        }
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
        self.update_class_constants(cls).ok()?;
        let obj = self.new_object(cls);
        {
            let mut b = obj.borrow_mut();
            for (k, v) in props {
                b.properties.insert(rhp_values::PhpStr::from_bytes(k), v);
            }
        }
        Some(Value::Object(obj))
    }

    fn unserialize_set_prop(&mut self, obj: &Value, key: &[u8], val: Value) -> bool {
        let Value::Object(h) = deref(obj) else {
            return false;
        };
        let cls = h.borrow().class;
        let entry = &self.classes[cls as usize];
        let declared = |name: &[u8]| {
            entry
                .prop_visible
                .get(&PhpStr::from_bytes(name.to_vec()))
                .cloned()
        };
        let storage = if key.first() == Some(&0) {
            let Some(end) = key[1..].iter().position(|&b| b == 0) else {
                return false;
            };
            let (owner, name) = (&key[1..end + 1], &key[end + 2..]);
            let own = owner == b"*" || owner.eq_ignore_ascii_case(entry.name.as_bytes());
            own.then(|| declared(name))
                .flatten()
                .unwrap_or_else(|| PhpStr::from_bytes(key.to_vec()))
        } else {
            declared(key).unwrap_or_else(|| PhpStr::from_bytes(key.to_vec()))
        };
        let mut od = h.borrow_mut();
        *classes::prop_slot_mut(&self.classes, &mut od, storage) = val;
        true
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
        self.resolve_callable(callable).err()
    }

    fn check_callable(&mut self, callable: &Value) -> Result<Option<String>, Thrown> {
        self.autoload_callable_class(callable)?;
        Ok(self.resolve_callable(callable).err())
    }

    fn wrap_callable(&self, callable: Value) -> Value {
        let (scope, called, _) = self.callable_scope();
        Value::Closure(Rc::new(rhp_values::ClosureData {
            fnidx: u32::MAX,
            captures: Vec::new(),
            bound_this: None,
            bound_class: scope,
            bound_static: called,
            wrapped: Some(Box::new(callable)),
        }))
    }
}

/// A resolved callable (see `Vm::resolve_callable`).
enum Callee {
    Closure(Rc<rhp_values::ClosureData>),
    Function(Rc<FunctionProto>),
    Builtin(String),
    /// A method with its `$this` (None: static) and called class.
    Method(classes::ResolvedMethod, Option<ObjectHandle>, ClassId),
    /// `__call` (with `$this`) / `__callStatic` for an unknown method name.
    Magic(
        classes::ResolvedMethod,
        Option<ObjectHandle>,
        ClassId,
        Vec<u8>,
    ),
    /// An enum's built-in `cases`/`from`/`tryFrom` (lowercased).
    EnumStatic(ClassId, Vec<u8>),
}

/// PG(display_errors) (main.c php_get_display_errors_mode).
enum DisplayErrors {
    Off,
    Stdout,
    Stderr,
}

fn display_errors_mode(v: &[u8]) -> DisplayErrors {
    let lc = v.to_ascii_lowercase();
    match lc.as_slice() {
        b"on" | b"yes" | b"true" | b"stdout" => DisplayErrors::Stdout,
        b"stderr" => DisplayErrors::Stderr,
        _ => match builtins::php_fns::atol(v) as u8 {
            0 => DisplayErrors::Off,
            2 => DisplayErrors::Stderr,
            _ => DisplayErrors::Stdout,
        },
    }
}

/// zend_ini_parse_bool: "true"/"yes"/"on" (any case) or a nonzero number.
fn ini_bool(v: &[u8]) -> bool {
    let lc = v.to_ascii_lowercase();
    matches!(lc.as_slice(), b"true" | b"yes" | b"on") || builtins::php_fns::atol(v) != 0
}

/// One activation record.
struct Frame {
    proto: Rc<FunctionProto>,
    regs: Vec<Value>,
    pc: usize,
    /// Caller register that receives this frame's return value (unused for main).
    ret_dst: Reg,
    /// The class whose body this is (`self::`/`parent::`/`__CLASS__`); None outside methods.
    defining_class: Option<ClassId>,
    /// The late-static-binding class (`static::`); None outside methods.
    called_class: Option<ClassId>,
    /// What each running `finally` block (keyed by its start pc) must do when
    /// it finishes: re-raise, return, or jump on (php's per-finally fast_call).
    pending: Vec<(u32, FinallyCont)>,
    /// A synchronous (re-entrant) call: when this frame returns, `do_return`
    /// hands the value back to the Rust caller instead of delivering to a reg.
    capture: bool,
    /// Number of positional arguments actually passed (for non-constant parameter
    /// default-init guards in the prologue).
    argc: usize,
    /// Parameters below `argc` that a named-argument call skipped (bit i =
    /// parameter i): their defaults still apply, as if not passed.
    holes: u64,
    /// Arguments passed beyond the declared parameters (non-variadic callee):
    /// kept for `func_get_args()` and exception-trace `args`.
    extra_args: Vec<Value>,
    /// `include`/`require`/`eval` frame: the construct name shown in traces.
    include_kind: Option<&'static str>,
    /// Runs in the global scope ({main}, or a file included from it): its
    /// CVs are the global symbol table's cells.
    global_scope: bool,
}

/// A builtin (native) call in progress, for exception traces: php shows the
/// internal function as its own trace frame, called from `caller`'s line, and
/// any user callback it invokes as `[internal function]`.
struct NativeCall {
    name: &'static str,
    /// The running builtin's argument slice, borrowed rather than copied so
    /// tracking a call costs nothing until a trace is actually built.
    /// Invariant: the entry is pushed just before the builtin is called and
    /// popped right after it returns, while the owning `Vec` (a local of
    /// `run_builtin`, only shared-borrowed by the builtin) is alive and
    /// unmodified — so the pointer is valid whenever it is in `native_calls`.
    args: *const [Value],
    /// Index of the frame that made the call.
    caller: usize,
}

/// Result of unwinding: a handler was installed, or the exception escaped the
/// current (sub)run and should propagate to the Rust caller.
enum UnwindResult {
    Caught,
    Escaped(Thrown),
}

/// The innermost try/catch with a `finally` whose protected range holds `pc`
/// (and, for a jump, does not hold `target`): its finally block's start pc.
fn enclosing_finally(proto: &FunctionProto, pc: u32, target: Option<u32>) -> Option<u32> {
    proto.exception_table.iter().find_map(|r| {
        let f = r.finally?;
        let holds = |p: u32| p >= r.start && p < r.end;
        (holds(pc) && !target.is_some_and(holds)).then_some(f)
    })
}

/// What to do after a `finally` block completes.
enum FinallyCont {
    /// An exception was in flight (no catch matched); re-raise after finally.
    Rethrow(Thrown),
    /// A `return` crossed out of the try/catch; complete it after finally.
    Return(Value),
    /// A `break`/`continue` crossed out of the try/catch; jump after finally.
    Jump(u32),
}

impl Frame {
    fn new(proto: Rc<FunctionProto>, ret_dst: Reg) -> Self {
        let regs = vec![Value::Null; proto.num_registers as usize];
        Frame {
            proto,
            regs,
            pc: 0,
            ret_dst,
            defining_class: None,
            called_class: None,
            pending: Vec::new(),
            capture: false,
            argc: 0,
            holes: 0,
            extra_args: Vec::new(),
            include_kind: None,
            global_scope: false,
        }
    }
}

impl Default for Vm {
    fn default() -> Self {
        Vm::new()
    }
}

impl Vm {
    /// Resolve a (possibly namespaced) function reference. Tries the primary
    /// `name`, then `fallback` (the global name) if set, against user functions;
    /// returns the user-function index if found, plus the lowercased name to use
    /// for a builtin dispatch / refusal check (the last candidate tried, which is
    /// the global name inside a namespace since builtins are not namespaced).
    fn resolve_fn(&self, name: NIdx, fallback: NIdx, names: &[PhpStr]) -> (Option<usize>, Vec<u8>) {
        let p = names[name as usize].as_bytes().to_ascii_lowercase();
        if let Some(&i) = self.fn_names.get(&p) {
            return (Some(i), p);
        }
        if fallback != rhp_bytecode::NO_NAME {
            let f = names[fallback as usize].as_bytes().to_ascii_lowercase();
            let idx = self.fn_names.get(&f).copied();
            return (idx, f);
        }
        (None, p)
    }

    pub fn new() -> Self {
        // Builtin state is per request: never inherit a previous engine's.
        builtins::reset_request_state();
        let mut constants = HashMap::new();
        for (name, value) in builtins::constants() {
            constants.insert(name.as_bytes().to_vec(), value);
        }
        // Seed the stream-wrapper registry: the default `file://` plus every
        // module-provided wrapper (`phar://`, …).
        let mut wrappers: HashMap<Vec<u8>, Rc<dyn builtins::StreamWrapper>> = HashMap::new();
        wrappers.insert(b"file".to_vec(), builtins::streams::file_wrapper());
        for reg in builtins::stream_wrappers() {
            wrappers.insert(reg.scheme.as_bytes().to_vec(), (reg.factory)());
        }
        rhp_values::set_object_free_hook(builtins::zip_ext::object_freed);
        Vm {
            out: Vec::new(),
            prop_guards: Default::default(),
            stdout_passthrough: false,
            ob_stack: Vec::new(),
            stack: Vec::new(),
            functions: Vec::new(),
            fn_names: HashMap::new(),
            closures: Vec::new(),
            decl_classes: Vec::new(),
            decl_runtime: std::collections::HashSet::new(),
            constants,
            user_const_order: Vec::new(),
            max_depth: 1 << 16,
            native_calls: Vec::new(),
            exception_handlers: Vec::new(),
            shutdown_functions: Vec::new(),
            direct_stdout: false,
            cursor_root: CursorRoot::Cv(0),
            cursor_steps: Vec::new(),
            cursor_global: None,
            classes: Vec::new(),
            class_ids: HashMap::new(),
            next_object_id: 1,
            globals: indexmap::IndexMap::new(),
            jit_globals: HashMap::new(),
            unset_typed_props: HashMap::new(),
            ref_globals: std::collections::HashSet::new(),
            statics: HashMap::new(),
            generators: HashMap::new(),
            yield_out: None,
            gen_stack: Vec::new(),
            bc_scale: 0,
            includer: None,
            fcc_scope: std::cell::Cell::new(None),
            included_files: std::collections::HashSet::new(),
            autoloaders: Vec::new(),
            in_autoload: Default::default(),
            pending_named: None,
            cuf_caller: None,
            wrappers,
            main_script_path: None,
            streams: HashMap::new(),
            output_streams: Default::default(),
            readonly_reinit: Default::default(),
            nonblocking: Default::default(),
            fd_names: HashMap::new(),
            last_dir: None,
            default_context: None,
            stream_origins: HashMap::new(),
            stream_modes: HashMap::new(),
            next_resource_id: 1,
            stream_contexts: HashMap::new(),
            last_response_headers: Vec::new(),
            argv: Vec::new(),
            silence: 0,
            silence_stack: Vec::new(),
            last_error: None,
            error_handlers: Vec::new(),
            pending_throw: None,
            native_resources: HashMap::new(),
            stdin_reader: None,
            destructibles: Vec::new(),
            dtor_check: false,
            dtor_pending_checks: 0,
            ctor_frames: Vec::new(),
            finished_main: None,
            in_shutdown_scope: false,
            destructed_below: 0,
            module_states: HashMap::new(),
        }
    }

    /// Raise a php diagnostic (`E_WARNING`, `E_NOTICE`, …): record it for
    /// `error_get_last()` and display it (CLI `display_errors=1`) unless an
    /// `@` is in effect.
    /// php's `zend_error` for a non-fatal diagnostic of type `level`: the
    /// user error handler (when its mask covers `level`; called even under
    /// `@`, where `error_reporting()` reads as the fatal mask), and unless it
    /// returns false, nothing else; otherwise php_error_cb: `error_get_last`
    /// and the display / log step (masked by `error_reporting` and `@`). An
    /// exception from the handler becomes `pending_throw`.
    /// An `E_COMPILE_ERROR` raised while executing (e.g. a runtime class
    /// redeclaration): never seen by a user error handler, displayed with
    /// php 8.5's backtrace, then the request bails out (exit status 255).
    pub(crate) fn fatal_compile_error(&mut self, msg: &str) -> Thrown {
        let (file, line) = match (0..self.stack.len())
            .rev()
            .find(|&i| !is_prelude_proto(&self.stack[i].proto))
        {
            None => (Vec::new(), 0),
            Some(i) => (self.current_file(i), self.current_line(i) as i64),
        };
        self.last_error = Some((64, msg.as_bytes().to_vec(), file.clone(), line));
        let trace = Value::Array(self.build_trace_ex(false, false));
        let trace = builtins::php_fns::trace_to_string(self, &trace);
        self.display_error_trace(64, "Fatal error", msg.as_bytes(), &file, line, &trace);
        self.destructed_below = self.next_object_id;
        Thrown::Exit(255)
    }

    pub(crate) fn raise_error(&mut self, level: i64, msg: &str) {
        // The innermost user frame: prelude (built-in PHP-source) code has no
        // line map, and php reports internal functions at their caller.
        let (file, line) = match (0..self.stack.len())
            .rev()
            .find(|&i| !is_prelude_proto(&self.stack[i].proto))
        {
            None => (Vec::new(), 0),
            Some(i) => (self.current_file(i), self.current_line(i) as i64),
        };
        if self.pending_throw.is_some() {
            return; // php skips diagnostics while an exception is pending
        }
        let handler = match self.error_handlers.last() {
            Some((h, mask)) if !matches!(h, Value::Null | Value::Bool(false)) => {
                (mask & level != 0).then(|| h.clone())
            }
            _ => None,
        };
        if let Some(h) = handler {
            let args = vec![
                Value::Int(level),
                Value::str(msg.as_bytes().to_vec()),
                Value::str(file.clone()),
                Value::Int(line),
            ];
            // php's EG(user_error_handler) swap: while the handler runs there
            // is none (`false` marks php's UNDEF); it comes back afterwards
            // only if the handler did not install another one.
            let idx = self.error_handlers.len() - 1;
            self.error_handlers[idx].0 = Value::Bool(false);
            let r = self.call_callable_value(&h, args);
            if self.error_handlers.len() == idx + 1
                && matches!(self.error_handlers[idx].0, Value::Bool(false))
            {
                self.error_handlers[idx].0 = h;
            }
            match r {
                Ok(Value::Bool(false)) => {} // fall through to php_error_cb
                Ok(_) => return,
                Err(t) => {
                    self.pending_throw = Some(t);
                    return;
                }
            }
        }
        self.last_error = Some((level, msg.as_bytes().to_vec(), file.clone(), line));
        if level == 256 {
            // E_USER_ERROR is fatal: displayed with the backtrace, then the
            // request bails out (shutdown functions still run; exit 255).
            let trace = Value::Array(self.build_trace_ex(false, false));
            let trace = builtins::php_fns::trace_to_string(self, &trace);
            self.display_error_trace(level, "Fatal error", msg.as_bytes(), &file, line, &trace);
            self.destructed_below = self.next_object_id;
            self.pending_throw = Some(Thrown::Exit(255));
            return;
        }
        if self.silence > 0 && level & SILENCE_KEEPS == 0 {
            return;
        }
        let label = match level {
            1 | 16 | 64 | 256 => "Fatal error",
            2 | 32 | 128 | 512 => "Warning",
            8 | 1024 => "Notice",
            8192 | 16384 => "Deprecated",
            4096 => "Recoverable fatal error",
            _ => "Unknown error",
        };
        self.display_error(level, label, msg.as_bytes(), &file, line);
    }

    /// Set the CLI argument vector (`$argv`; `argv[0]` is the script name).
    /// A startup ini setting (`-d key=value`); unknown directives are ignored.
    pub fn set_ini(&mut self, name: &[u8], value: &[u8]) {
        builtins::ini_startup_set(name, value);
    }

    pub fn set_argv(&mut self, argv: Vec<Vec<u8>>) {
        self.argv = argv;
    }

    /// Get or create the shared global cell for `name` in the global symbol
    /// table, used for {main}'s variables, superglobals, `global` and
    /// `$GLOBALS[...]`.
    fn global_cell(&mut self, name: &[u8]) -> Rc<RefCell<Value>> {
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
    fn rebind_global(&mut self, name: &[u8], cell: Rc<RefCell<Value>>) {
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
    fn sync_global_cv(&mut self, top: usize, cv: Reg) {
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
    fn bind_ref_cv(&mut self, top: usize, cv: Reg, rc: Rc<RefCell<Value>>) {
        self.reg_set(top, cv, Value::Ref(rc));
        self.sync_global_cv(top, cv);
    }

    /// Bind every named CV of a global-scope frame to its global cell.
    fn bind_global_scope(&mut self, frame: &mut Frame) {
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
    fn globals_array(&self) -> Value {
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
        let script_fn = self.main_script_path.clone().unwrap_or(script.clone());
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

    /// Whether `v` is an instance of the named class/interface (or a Closure for
    /// the `Closure` type). Unresolvable target class → lenient `true` (the class
    /// may be autoloadable; we don't reject on a name we can't see).
    /// Read property `pname` off `o` (visibility-checked, `__get` fallback).
    fn prop_fetch(&mut self, top: usize, o: Value, pname: PhpStr) -> Result<Value, Thrown> {
        Ok(match o {
            Value::Object(h) => {
                let cls = h.borrow().class;
                let scope = self.stack[top].defining_class;
                let mut absent_slot = None;
                let denied = match classes::resolve_prop(&self.classes, cls, scope, &pname) {
                    PropRes::Slot(k) => {
                        if let Some(v) = h.borrow().properties.get(&k) {
                            return Ok(deref(v));
                        }
                        // Skip __get() for an uninitialized typed property.
                        if self.uninit_typed(&h, &k) {
                            if let Some(e) = self.typed_slot(cls, &k) {
                                return Err(e);
                            }
                        }
                        absent_slot = Some(k);
                        None
                    }
                    PropRes::Dynamic => {
                        if let Some(v) = h.borrow().properties.get(&pname) {
                            return Ok(deref(v));
                        }
                        None
                    }
                    PropRes::Denied(vis) => Some(vis),
                };
                match self.magic(&h, cls, b"__get", &pname) {
                    Some(getter) => {
                        let arg = Value::str(pname.as_bytes().to_vec());
                        self.call_magic(&getter, &h, cls, b"__get", &pname, vec![arg])?
                    }
                    None => match denied {
                        Some(vis) => {
                            return Err(classes::prop_access_err(
                                &self.classes,
                                cls,
                                pname.as_bytes(),
                                vis,
                            ))
                        }
                        None => {
                            if let Some(e) =
                                absent_slot.as_ref().and_then(|k| self.typed_slot(cls, k))
                            {
                                return Err(e);
                            }
                            let msg = format!(
                                "Undefined property: {}::${}",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                String::from_utf8_lossy(pname.as_bytes())
                            );
                            self.diag(2, &msg)?;
                            Value::Null
                        }
                    },
                }
            }
            Value::Closure(_) => {
                let msg = format!(
                    "Undefined property: Closure::${}",
                    String::from_utf8_lossy(pname.as_bytes())
                );
                self.diag(2, &msg)?;
                Value::Null
            }
            other => {
                let msg = format!(
                    "Attempt to read property \"{}\" on {}",
                    String::from_utf8_lossy(pname.as_bytes()),
                    match other {
                        Value::Bool(true) => "true",
                        Value::Bool(false) => "false",
                        ref o => type_lc(o),
                    }
                );
                self.diag(2, &msg)?;
                Value::Null
            }
        })
    }

    /// `isset($o->name)`: a set, non-null property reachable from the scope,
    /// else `__isset`, else false.
    fn prop_isset(&mut self, top: usize, o: Value, pname: PhpStr) -> Result<bool, Thrown> {
        let Value::Object(h) = o else {
            return Ok(false);
        };
        let cls = h.borrow().class;
        let scope = self.stack[top].defining_class;
        let present = match classes::resolve_prop(&self.classes, cls, scope, &pname) {
            // Skip __isset() for an uninitialized typed property.
            PropRes::Slot(k) if self.uninit_typed(&h, &k) => return Ok(false),
            PropRes::Slot(k) => h.borrow().properties.get(&k).cloned(),
            PropRes::Dynamic => h.borrow().properties.get(&pname).cloned(),
            PropRes::Denied(_) => None,
        };
        Ok(match present {
            // A declared/dynamic property: set iff non-null.
            Some(v) => !matches!(deref(&v), Value::Null),
            None => match self.magic(&h, cls, b"__isset", &pname) {
                Some(m) => {
                    let nm = Value::str(pname.as_bytes().to_vec());
                    let r = self.call_magic(&m, &h, cls, b"__isset", &pname, vec![nm])?;
                    conv::to_bool(&r)
                }
                None => false,
            },
        })
    }

    /// `empty($o->name)` (zend_std_has_property, ZEND_PROPERTY_NOT_EMPTY): a
    /// reachable set property's falsiness, else `__isset` and — only if it
    /// says yes — `__get`; inaccessible / missing without magic is empty.
    fn prop_empty(&mut self, top: usize, o: Value, pname: PhpStr) -> Result<bool, Thrown> {
        let Value::Object(h) = o else {
            return Ok(true);
        };
        let cls = h.borrow().class;
        let scope = self.stack[top].defining_class;
        let present = match classes::resolve_prop(&self.classes, cls, scope, &pname) {
            PropRes::Slot(k) if self.uninit_typed(&h, &k) => return Ok(true),
            PropRes::Slot(k) => h.borrow().properties.get(&k).cloned(),
            PropRes::Dynamic => h.borrow().properties.get(&pname).cloned(),
            PropRes::Denied(_) => None,
        };
        if let Some(v) = present {
            return Ok(!conv::to_bool(&deref(&v)));
        }
        let nm = || Value::str(pname.as_bytes().to_vec());
        let Some(m) = self.magic(&h, cls, b"__isset", &pname) else {
            return Ok(true);
        };
        let r = self.call_magic(&m, &h, cls, b"__isset", &pname, vec![nm()])?;
        if !conv::to_bool(&r) {
            return Ok(true);
        }
        match self.magic(&h, cls, b"__get", &pname) {
            Some(g) => {
                let v = self.call_magic(&g, &h, cls, b"__get", &pname, vec![nm()])?;
                Ok(!conv::to_bool(&deref(&v)))
            }
            None => Ok(true),
        }
    }

    /// `$o->name` in BP_VAR_IS mode (an isset / empty / `??` container): the
    /// reachable set property, else `__isset` then (if it says yes) `__get`,
    /// else null — never an access error (zend_std_read_property, BP_VAR_IS).
    fn prop_fetch_is(&mut self, top: usize, o: Value, pname: PhpStr) -> Result<Value, Thrown> {
        let Value::Object(h) = o else {
            return Ok(Value::Null);
        };
        let cls = h.borrow().class;
        let scope = self.stack[top].defining_class;
        let present = match classes::resolve_prop(&self.classes, cls, scope, &pname) {
            PropRes::Slot(k) if self.uninit_typed(&h, &k) => return Ok(Value::Null),
            PropRes::Slot(k) => h.borrow().properties.get(&k).cloned(),
            PropRes::Dynamic => h.borrow().properties.get(&pname).cloned(),
            PropRes::Denied(_) => None,
        };
        if let Some(v) = present {
            return Ok(deref(&v));
        }
        let nm = || Value::str(pname.as_bytes().to_vec());
        if let Some(m) = self.magic(&h, cls, b"__isset", &pname) {
            let r = self.call_magic(&m, &h, cls, b"__isset", &pname, vec![nm()])?;
            if !conv::to_bool(&r) {
                return Ok(Value::Null);
            }
        }
        // (No `__isset`, or inside this property's own `__isset`: `__get`.)
        match self.magic(&h, cls, b"__get", &pname) {
            Some(g) => self.call_magic(&g, &h, cls, b"__get", &pname, vec![nm()]),
            None => Ok(Value::Null),
        }
    }

    /// `unset($o->name)`: remove the property reachable from the scope, else
    /// `__unset`, else an access error for an inaccessible declaration.
    fn prop_unset(&mut self, top: usize, o: Value, pname: PhpStr) -> Result<(), Thrown> {
        let Value::Object(h) = o else {
            return Ok(());
        };
        let cls = h.borrow().class;
        if self.is_directory_class(cls) && self.class(cls).prop_visible.contains_key(&pname) {
            return Err(Thrown::error(
                "Error",
                format!(
                    "Cannot unset readonly property Directory::${}",
                    String::from_utf8_lossy(pname.as_bytes())
                ),
            ));
        }
        let scope = self.stack[top].defining_class;
        let res = classes::resolve_prop(&self.classes, cls, scope, &pname);
        let key = match &res {
            PropRes::Slot(k) => Some(k.clone()),
            PropRes::Dynamic => Some(pname.clone()),
            PropRes::Denied(_) => None,
        };
        let present = key
            .as_ref()
            .is_some_and(|k| h.borrow().properties.contains_key(k));
        // zend_std_unset_property: an initialized readonly property can never
        // be unset; otherwise the (explicit or implicit) set visibility rules.
        if let PropRes::Slot(k) = &res {
            if let Some(pe) = self.class(cls).props.get(k).filter(|pe| pe.readonly) {
                if present {
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Cannot unset readonly property {}::${}",
                            String::from_utf8_lossy(self.class(pe.declaring_class).name.as_bytes()),
                            String::from_utf8_lossy(pe.name.as_bytes())
                        ),
                    ));
                }
            }
            let called = self.stack[top].called_class;
            if let Err(e) = classes::set_access_ok(&self.classes, cls, scope, k) {
                return Err(classes::set_access_err(&self.classes, called, "unset", e));
            }
            // A public readonly property is implicitly protected(set).
            if pe_public_implicit_set(self, cls, k) {
                let pe = &self.class(cls).props[k];
                let decl = pe.declaring_class;
                let related = scope.is_some_and(|s| {
                    s == decl
                        || self.class(s).ancestors.contains(&decl)
                        || self.class(decl).ancestors.contains(&s)
                });
                if !related {
                    let e = (rhp_values::Visibility::Protected, decl, pe.name.clone());
                    return Err(classes::set_access_err(&self.classes, called, "unset", e));
                }
            }
        }
        let was_uninit = matches!(&res, PropRes::Slot(k) if self.uninit_typed(&h, k));
        if present {
            let key = key.unwrap();
            h.borrow_mut().properties.shift_remove(&key);
            self.mark_typed_unset(&h, &key);
        } else if was_uninit {
            // Reset IS_PROP_UNINIT, bypassing __unset().
            self.mark_typed_unset(&h, key.as_ref().unwrap());
        } else if let Some(m) = self.magic(&h, cls, b"__unset", &pname) {
            let nm = Value::str(pname.as_bytes().to_vec());
            self.call_magic(&m, &h, cls, b"__unset", &pname, vec![nm])?;
        } else if let PropRes::Denied(vis) = res {
            return Err(classes::prop_access_err(
                &self.classes,
                cls,
                pname.as_bytes(),
                vis,
            ));
        }
        Ok(())
    }

    /// Is the declared slot `key` of `h` an uninitialized typed property
    /// that was never `unset()` (php's IS_PROP_UNINIT)? Such a slot never
    /// routes through the magic accessors: writes go to the slot, reads are
    /// the "must not be accessed before initialization" Error.
    fn uninit_typed(&self, h: &ObjectHandle, key: &PhpStr) -> bool {
        let b = h.borrow();
        if b.properties.contains_key(key) {
            return false;
        }
        let typed = self
            .classes
            .get(b.class as usize)
            .and_then(|e| e.props.get(key))
            .is_some_and(|pe| pe.ty.is_some());
        typed
            && !self
                .unset_typed_props
                .get(&b.id)
                .is_some_and(|ks| ks.contains(key))
    }

    /// `unset()` of the declared typed slot `key`: from now on an absent
    /// slot defers to the magic accessors.
    fn mark_typed_unset(&mut self, h: &ObjectHandle, key: &PhpStr) {
        let b = h.borrow();
        let typed = self
            .classes
            .get(b.class as usize)
            .and_then(|e| e.props.get(key))
            .is_some_and(|pe| pe.ty.is_some());
        if typed {
            let ks = self.unset_typed_props.entry(b.id).or_default();
            if !ks.contains(key) {
                ks.push(key.clone());
            }
        }
    }

    /// The magic property accessor `which` (`__get`/`__set`/`__isset`/
    /// `__unset`) for `name` on `h`, unless that same accessor is already
    /// running for that property on that object (php's property guards: inside
    /// `__get('x')`, `$this->x` is a plain property access).
    fn magic(
        &self,
        h: &ObjectHandle,
        cls: ClassId,
        which: &'static [u8],
        name: &PhpStr,
    ) -> Option<classes::ResolvedMethod> {
        let id = h.borrow().id;
        if self.prop_guards.contains(&(id, which, name.clone())) {
            return None;
        }
        self.lookup_method(cls, which)
    }

    /// Run a magic accessor with its property guard held.
    fn call_magic(
        &mut self,
        m: &classes::ResolvedMethod,
        h: &ObjectHandle,
        cls: ClassId,
        which: &'static [u8],
        name: &PhpStr,
        args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        let guard = (h.borrow().id, which, name.clone());
        self.prop_guards.insert(guard.clone());
        let r = self.call_sync(m, Some(h.clone()), cls, args);
        self.prop_guards.remove(&guard);
        r
    }

    /// php's static-property visibility check: a non-public static is reachable
    /// only from its declaring class (private) / a related class (protected);
    /// the error names the class the access was spelled against.
    fn check_static_prop_access(
        &self,
        top: usize,
        cls: ClassId,
        pname: &[u8],
    ) -> Result<(), Thrown> {
        if let Some(sp) = self.class(cls).static_props.get(pname) {
            if !self.static_prop_accessible(self.stack[top].defining_class, sp) {
                return Err(classes::prop_access_err(
                    &self.classes,
                    cls,
                    pname,
                    Deny::Vis(sp.visibility),
                ));
            }
        }
        Ok(())
    }

    /// Write `val` to property `pname` of `o` (visibility-checked, `__set` fallback).
    fn prop_store(
        &mut self,
        top: usize,
        o: Value,
        pname: PhpStr,
        val: Value,
    ) -> Result<(), Thrown> {
        if let Value::Object(h) = o {
            let cls = h.borrow().class;
            if self.is_directory_class(cls) {
                // Directory: readonly properties, no dynamic properties.
                let p = String::from_utf8_lossy(pname.as_bytes());
                return Err(Thrown::error(
                    "Error",
                    if self.class(cls).prop_visible.contains_key(&pname) {
                        format!("Cannot modify readonly property Directory::${p}")
                    } else {
                        format!("Cannot create dynamic property Directory::${p}")
                    },
                ));
            }
            let scope = self.stack[top].defining_class;
            let res = classes::resolve_prop(&self.classes, cls, scope, &pname);
            let is_dyn = matches!(res, PropRes::Dynamic);
            let (key, denied) = match res {
                PropRes::Slot(k) => (k, None),
                PropRes::Dynamic => (pname.clone(), None),
                PropRes::Denied(vis) => (pname.clone(), Some(vis)),
            };
            let present = denied.is_none() && h.borrow().properties.contains_key(&key);
            // Writes to uninitialized typed properties bypass __set().
            let uninit = !present && denied.is_none() && !is_dyn && self.uninit_typed(&h, &key);
            if denied.is_none() {
                if let Err(e) = classes::set_access_ok(&self.classes, cls, scope, &key) {
                    // An unset asymmetric property still defers to `__set`.
                    if !present && !uninit {
                        if let Some(setter) = self.magic(&h, cls, b"__set", &pname) {
                            let nm = Value::str(pname.as_bytes().to_vec());
                            self.call_magic(&setter, &h, cls, b"__set", &pname, vec![nm, val])?;
                            return Ok(());
                        }
                    }
                    let called = self.stack[top].called_class;
                    return Err(classes::set_access_err(&self.classes, called, "modify", e));
                }
            }
            if !present && !uninit {
                // Inaccessible, or not currently set (dynamic / unset declared):
                // `__set` intercepts when defined.
                if let Some(setter) = self.magic(&h, cls, b"__set", &pname) {
                    let nm = Value::str(pname.as_bytes().to_vec());
                    self.call_magic(&setter, &h, cls, b"__set", &pname, vec![nm, val])?;
                    return Ok(());
                }
                if let Some(vis) = denied {
                    return Err(classes::prop_access_err(
                        &self.classes,
                        cls,
                        pname.as_bytes(),
                        vis,
                    ));
                }
                if is_dyn {
                    self.check_dynamic_prop(top, cls, &pname)?;
                }
            }
            if is_dyn && present && self.class(cls).kind == ClassKind::Enum {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot modify readonly property {}::${}",
                        String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                        String::from_utf8_lossy(pname.as_bytes())
                    ),
                ));
            }
            let val = if is_dyn {
                val
            } else {
                let present = h.borrow().properties.contains_key(&key);
                self.check_readonly_write(&h, &key, present, scope)?;
                let strict = self.stack[top].proto.strict_types;
                self.verify_prop_type(cls, &key, val, strict)?
            };
            let mut od = h.borrow_mut();
            let slot = classes::prop_slot_mut(&self.classes, &mut od, key);
            match slot {
                Value::Ref(rc) => {
                    let rc = rc.clone();
                    drop(od);
                    *rc.borrow_mut() = val;
                }
                other => *other = val,
            }
            return Ok(());
        }
        Err(Thrown::error(
            "Error",
            format!(
                "Attempt to assign property \"{}\" on {}",
                String::from_utf8_lossy(pname.as_bytes()),
                match &o {
                    Value::Bool(true) => "true",
                    Value::Bool(false) => "false",
                    other => type_lc(other),
                }
            ),
        ))
    }

    /// Creating a dynamic property (zend_forbidden_dynamic_property /
    /// zend_deprecated_dynamic_property): an Error on enums and readonly
    /// classes, else deprecated unless the class (or a parent) allows it.
    fn check_dynamic_prop(
        &mut self,
        top: usize,
        cls: ClassId,
        pname: &PhpStr,
    ) -> Result<(), Thrown> {
        let e = self.class(cls);
        let cname = String::from_utf8_lossy(e.name.as_bytes()).into_owned();
        let p = String::from_utf8_lossy(pname.as_bytes()).into_owned();
        if e.kind == ClassKind::Enum || e.flags.is_readonly {
            return Err(Thrown::error(
                "Error",
                format!("Cannot create dynamic property {cname}::${p}"),
            ));
        }
        let mut c = Some(cls);
        while let Some(id) = c {
            let ce = self.class(id);
            if ce.flags.allow_dynamic
                || ce.name.as_bytes().eq_ignore_ascii_case(b"stdClass")
                || ce.name.as_bytes() == b"__PHP_Incomplete_Class"
            {
                return Ok(());
            }
            c = ce.parent;
        }
        // Engine-internal (prelude) code manages its own objects.
        if (0..=top)
            .rev()
            .take(1)
            .any(|i| is_prelude_proto(&self.stack[i].proto))
        {
            return Ok(());
        }
        self.diag(
            8192,
            &format!("Creation of dynamic property {cname}::${p} is deprecated"),
        )
    }

    /// Dispatch a method call on `h` by name (generator methods, user method with
    /// visibility check, `__call` fallback, else errors). Shared by `CallMethod`
    /// and `CallMethodDyn`.
    #[allow(clippy::too_many_arguments)] // VM call-dispatch frame setup
    fn invoke_method(
        &mut self,
        top: usize,
        h: ObjectHandle,
        cls: ClassId,
        mname: Vec<u8>,
        dst: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        let gid = h.borrow().id;
        if self.generators.contains_key(&gid) {
            let args: Vec<Value> = (0..argc).map(|i| self.reg_get(top, base + i)).collect();
            if let Some(r) = self.generator_method(gid, &mname.to_ascii_lowercase(), &args) {
                let r = r?;
                self.reg_set(top, dst, r);
                return Ok(());
            }
        }
        let found = self.lookup_method(cls, &mname);
        let accessible = found.as_ref().is_some_and(|m| {
            self.check_access(
                self.stack[top].defining_class,
                m.defining_class,
                m.visibility,
            )
        });
        if let (Some(m), true) = (&found, accessible) {
            let m = m.clone();
            self.enter_method(top, &m, Some(h.clone()), cls, dst, base, argc)?;
        } else if let Some(magic) = self.lookup_method(cls, b"__call") {
            let mut arr = rhp_values::PhpArray::new();
            for i in 0..argc {
                arr.push(self.reg_get(top, base + i));
            }
            let args = vec![Value::str(mname.clone()), Value::Array(arr)];
            let r = self.call_sync(&magic, Some(h.clone()), cls, args)?;
            self.reg_set(top, dst, r);
        } else if let Some(m) = found {
            return Err(self.access_err("method", m.defining_class, &mname, m.visibility));
        } else {
            return Err(Thrown::error(
                "Error",
                format!(
                    "Call to undefined method {}::{}()",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                    String::from_utf8_lossy(&mname)
                ),
            ));
        }
        Ok(())
    }

    /// Shared body for `C::m()` / `C::$m()` static dispatch (resolves the class,
    /// handles enum built-ins, `__callStatic`, LSB, and `$this` forwarding).
    fn do_call_static(
        &mut self,
        top: usize,
        class: ClassRef,
        mname: Vec<u8>,
        dst: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        let cls = self.resolve_class_autoload(top, class)?;
        let found = self.lookup_method(cls, &mname);
        // Enum built-in static methods (cases/from/tryFrom) when not overridden.
        if found.is_none() && self.class(cls).kind == ClassKind::Enum {
            let lc = mname.to_ascii_lowercase();
            if matches!(lc.as_slice(), b"cases" | b"from" | b"tryfrom") {
                let args: Vec<Value> = (0..argc).map(|i| self.reg_get(top, base + i)).collect();
                let r = self.enum_builtin_static(cls, &lc, args)?;
                self.reg_set(top, dst, r);
                return Ok(());
            }
        }
        let accessible = found.as_ref().is_some_and(|m| {
            self.check_access(
                self.stack[top].defining_class,
                m.defining_class,
                m.visibility,
            )
        });
        if !(found.is_some() && accessible) {
            // Undefined / inaccessible static method -> __callStatic.
            if let Some(magic) = self.lookup_method(cls, b"__callstatic") {
                let mut arr = rhp_values::PhpArray::new();
                for i in 0..argc {
                    arr.push(self.reg_get(top, base + i));
                }
                let args = vec![Value::str(mname.clone()), Value::Array(arr)];
                let r = self.call_sync(&magic, None, cls, args)?;
                self.reg_set(top, dst, r);
                return Ok(());
            }
            return match found {
                Some(m) => Err(self.access_err("method", m.defining_class, &mname, m.visibility)),
                None => Err(Thrown::error(
                    "Error",
                    format!(
                        "Call to undefined method {}::{}()",
                        String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                        String::from_utf8_lossy(&mname)
                    ),
                )),
            };
        }
        let m = found.unwrap();
        // LSB: self/parent/static forward the called-class; Name/Dynamic set it.
        let called = match class {
            ClassRef::SelfClass | ClassRef::ParentClass | ClassRef::StaticClass => {
                self.stack[top].called_class.unwrap_or(cls)
            }
            _ => cls,
        };
        // Forward $this to a non-static method iff it is-a the target class.
        let this = if m.is_static {
            None
        } else {
            self.current_this(top)
                .filter(|h| self.class(h.borrow().class).ancestors.contains(&cls))
        };
        self.enter_method(top, &m, this, called, dst, base, argc)
    }

    /// Build the parameter-descriptor array `ReflectionParameter` reads from.
    fn params_to_value(&self, proto: &FunctionProto) -> Value {
        let mut arr = PhpArray::new();
        for p in &proto.params {
            let mut d = PhpArray::new();
            d.insert(
                ArrayKey::from_bytes(b"name"),
                Value::str(proto.names[p.name as usize].as_bytes().to_vec()),
            );
            d.insert(
                ArrayKey::from_bytes(b"optional"),
                Value::Bool(p.default.is_some() || p.variadic),
            );
            d.insert(ArrayKey::from_bytes(b"variadic"), Value::Bool(p.variadic));
            d.insert(ArrayKey::from_bytes(b"byref"), Value::Bool(p.by_ref));
            let (has_def, def) = match &p.default {
                Some(rhp_bytecode::Default::Const(k)) => (true, proto.consts[*k as usize].clone()),
                Some(rhp_bytecode::Default::Init(_)) => (true, Value::Null),
                None => (false, Value::Null),
            };
            d.insert(ArrayKey::from_bytes(b"hasDefault"), Value::Bool(has_def));
            d.insert(ArrayKey::from_bytes(b"default"), def);
            match &p.type_hint {
                Some(ty) => {
                    let name = type_string(ty);
                    let name = name.strip_prefix('?').unwrap_or(&name).to_string();
                    d.insert(ArrayKey::from_bytes(b"hasType"), Value::Bool(true));
                    d.insert(ArrayKey::from_bytes(b"type"), Value::str(name.into_bytes()));
                    d.insert(
                        ArrayKey::from_bytes(b"typeNullable"),
                        Value::Bool(ty.nullable),
                    );
                    d.insert(
                        ArrayKey::from_bytes(b"typeBuiltin"),
                        Value::Bool(is_builtin_type(ty)),
                    );
                }
                None => {
                    d.insert(ArrayKey::from_bytes(b"hasType"), Value::Bool(false));
                    d.insert(ArrayKey::from_bytes(b"type"), Value::Null);
                    d.insert(ArrayKey::from_bytes(b"typeNullable"), Value::Bool(false));
                    d.insert(ArrayKey::from_bytes(b"typeBuiltin"), Value::Bool(false));
                }
            }
            arr.push(Value::Array(d));
        }
        Value::Array(arr)
    }

    fn value_instanceof(&self, v: &Value, name: &[u8]) -> bool {
        match v {
            Value::Object(h) => match self.resolve_class_by_name(name) {
                Ok(cid) => self.classes[h.borrow().class as usize]
                    .ancestors
                    .contains(&cid),
                Err(_) => true,
            },
            Value::Closure(_) => name.eq_ignore_ascii_case(b"Closure"),
            _ => false,
        }
    }

    fn kind_matches(&self, v: &Value, k: &rhp_bytecode::TypeKind, ctx: Option<ClassId>) -> bool {
        use rhp_bytecode::TypeKind as K;
        match k {
            K::Mixed => true,
            K::Int => matches!(v, Value::Int(_)),
            K::Float => matches!(v, Value::Float(_)),
            K::String => matches!(v, Value::Str(_)),
            K::Bool => matches!(v, Value::Bool(_)),
            K::True => matches!(v, Value::Bool(true)),
            K::False => matches!(v, Value::Bool(false)),
            K::Array => matches!(v, Value::Array(_)),
            K::Object => matches!(v, Value::Object(_) | Value::Closure(_)),
            K::Callable => self.is_callable_value(v),
            K::Iterable => matches!(v, Value::Array(_)) || self.value_instanceof(v, b"Traversable"),
            K::Null => matches!(v, Value::Null),
            K::Named(n) => self.value_instanceof(v, n.as_bytes()),
            K::SelfType | K::StaticType => match (v, ctx) {
                (Value::Object(h), Some(c)) => self.classes[h.borrow().class as usize]
                    .ancestors
                    .contains(&c),
                _ => false,
            },
            K::Parent => match (v, ctx.and_then(|c| self.classes[c as usize].parent)) {
                (Value::Object(h), Some(p)) => self.classes[h.borrow().class as usize]
                    .ancestors
                    .contains(&p),
                _ => false,
            },
            K::Void | K::Never => false,
        }
    }

    fn matches_type(&self, v: &Value, ty: &rhp_bytecode::TypeHint, ctx: Option<ClassId>) -> bool {
        if matches!(v, Value::Null) && ty.nullable {
            return true;
        }
        if ty.kinds.is_empty() {
            return true; // only-null / unknown: don't reject
        }
        if ty.intersection {
            ty.kinds.iter().all(|k| self.kind_matches(v, k, ctx))
        } else {
            ty.kinds.iter().any(|k| self.kind_matches(v, k, ctx))
        }
    }

    /// Check/coerce one argument against its declared type. `Ok(None)` = fine as
    /// is, `Ok(Some(v))` = coerced replacement, `Err(())` = type violation.
    fn check_one(
        &self,
        v: &Value,
        ty: &rhp_bytecode::TypeHint,
        strict: bool,
        ctx: Option<ClassId>,
    ) -> Result<Option<Value>, ()> {
        use rhp_bytecode::TypeKind as K;
        let d = deref(v);
        if self.matches_type(&d, ty, ctx) {
            return Ok(None);
        }
        if matches!(d, Value::Null) {
            return Err(()); // null but not nullable
        }
        // int -> float widening is allowed in both strict and coercive modes.
        if let Value::Int(n) = d {
            if ty.kinds.iter().any(|k| matches!(k, K::Float)) {
                return Ok(Some(Value::Float(n as f64)));
            }
        }
        if strict {
            return Err(());
        }
        // Coercive mode: php's zend_verify_weak_scalar_type_hint.
        weak_scalar_coerce(&d, ty).map(Some).ok_or(())
    }

    /// Coercive typing's `__toString` step (zend_verify_weak_scalar_type_hint
    /// for `string`): an object with `__toString` passed where a `string`
    /// (and not its class) is declared becomes its string form.
    fn coerce_stringable(
        &mut self,
        v: Value,
        ty: &rhp_bytecode::TypeHint,
        strict: bool,
        ctx: Option<ClassId>,
    ) -> Result<Value, Thrown> {
        use rhp_bytecode::TypeKind as K;
        if strict || !ty.kinds.contains(&K::String) {
            return Ok(v);
        }
        let Value::Object(h) = deref(&v) else {
            return Ok(v);
        };
        if self.matches_type(&v, ty, ctx) {
            return Ok(v);
        }
        if self
            .lookup_method(h.borrow().class, b"__tostring")
            .is_none()
        {
            return Ok(v);
        }
        Ok(Value::str(self.to_string_vm(&Value::Object(h))?))
    }

    /// [`Vm::coerce_stringable`] over a just-bound frame's passed parameters.
    fn coerce_stringable_args(
        &mut self,
        proto: &FunctionProto,
        frame: &mut Frame,
        caller: Option<bool>,
        ctx: Option<ClassId>,
    ) -> Result<(), Thrown> {
        let strict = caller.unwrap_or(false);
        if strict {
            return Ok(());
        }
        for (i, p) in proto.params.iter().enumerate() {
            let Some(ty) = &p.type_hint else { continue };
            if p.variadic && !p.by_ref {
                // `string ...$xs`: each collected argument.
                let Value::Array(a) = &frame.regs[p.cv as usize] else {
                    continue;
                };
                if !a.iter().any(|(_, v)| matches!(v, Value::Object(_))) {
                    continue;
                }
                let mut a = a.clone();
                let keys: Vec<ArrayKey> = a.iter().map(|(k, _)| k.clone()).collect();
                for k in keys {
                    let cur = a.get(&k).cloned().unwrap_or(Value::Null);
                    if matches!(cur, Value::Object(_)) {
                        let v = self.coerce_stringable(cur, ty, false, ctx)?;
                        a.insert(k, v);
                    }
                }
                frame.regs[p.cv as usize] = Value::Array(a);
                continue;
            }
            if p.by_ref
                || p.variadic
                || i >= frame.argc
                || (i < 64 && frame.holes & (1u64 << i) != 0)
            {
                continue;
            }
            let cur = frame.regs[p.cv as usize].clone();
            if matches!(cur, Value::Object(_)) {
                frame.regs[p.cv as usize] = self.coerce_stringable(cur, ty, false, ctx)?;
            }
        }
        Ok(())
    }

    /// Enforce declared parameter types on a just-bound frame (php's
    /// `strict_types` is the CALLER's; `ctx` resolves `self`/`static`/`parent`).
    /// `caller` is `Some(strict_types)` for a call from user code, `None` for
    /// a call made by a builtin (coercive, and the error has no call site).
    fn check_arg_types(
        &self,
        proto: &FunctionProto,
        frame: &mut Frame,
        caller: Option<bool>,
        ctx: Option<ClassId>,
    ) -> Result<(), (usize, Thrown)> {
        let strict = caller.unwrap_or(false);
        for (i, p) in proto.params.iter().enumerate() {
            let Some(ty) = &p.type_hint else { continue };
            // by-ref / variadic params aren't coerced here; a defaulted (not
            // passed) param is trusted.
            if p.by_ref
                || p.variadic
                || i >= frame.argc
                || (i < 64 && frame.holes & (1u64 << i) != 0)
            {
                continue;
            }
            let cur = frame.regs[p.cv as usize].clone();
            match self.check_one(&cur, ty, strict, ctx) {
                Ok(None) => {}
                Ok(Some(coerced)) => frame.regs[p.cv as usize] = coerced,
                Err(()) => {
                    let fname = self.func_display_name(proto, ctx);
                    let pname = String::from_utf8_lossy(proto.names[p.name as usize].as_bytes())
                        .into_owned();
                    // php appends the call site when the caller is user code (the
                    // new frame has not been pushed yet: the caller is the top);
                    // a callback invoked by a builtin (`caller == None`) has none.
                    let site = caller
                        .and_then(|_| self.user_call_site())
                        .map(|(file, line)| {
                            format!(
                                ", called in {} on line {}",
                                String::from_utf8_lossy(&file),
                                line
                            )
                        });
                    return Err((
                        i,
                        Thrown::error(
                            "TypeError",
                            format!(
                                "{}(): Argument #{} (${}) must be of type {}, {} given{}",
                                fname,
                                i + 1,
                                pname,
                                self.type_string_resolved(ty, ctx, frame.called_class),
                                self.zval_value_name(&deref(&cur)),
                                site.unwrap_or_default(),
                            ),
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Enforce a function's declared return type on `v`, coercing in coercive
    /// mode. `void`/`never`/`mixed` and unknowable types are not checked.
    /// Falling off the end of (or a bare `return;` in) a function with a
    /// declared return type: only `void` allows it (zend_verify_missing_return_type).
    fn check_implicit_return(&self, proto: &FunctionProto, top: usize) -> Result<(), Thrown> {
        use rhp_bytecode::TypeKind as K;
        let Some(ty) = &proto.return_type else {
            return Ok(());
        };
        if ty.kinds.contains(&K::Void) {
            return Ok(());
        }
        let ctx = self.stack[top].defining_class;
        let fname = self.func_display_name(proto, ctx);
        let msg = if ty.kinds.contains(&K::Never) {
            format!("{fname}(): never-returning function must not implicitly return")
        } else {
            let called = self.stack[top].called_class;
            format!(
                "{fname}(): Return value must be of type {}, none returned",
                self.type_string_resolved(ty, ctx, called)
            )
        };
        Err(Thrown::error("TypeError", msg))
    }

    fn check_return_type(
        &self,
        proto: &FunctionProto,
        top: usize,
        v: Value,
    ) -> Result<Value, Thrown> {
        use rhp_bytecode::TypeKind as K;
        let ty = match &proto.return_type {
            Some(t) => t,
            None => return Ok(v),
        };
        if ty
            .kinds
            .iter()
            .any(|k| matches!(k, K::Void | K::Never | K::Mixed))
        {
            return Ok(v);
        }
        let ctx = self.stack[top].defining_class;
        let strict = proto.strict_types;
        match self.check_one(&v, ty, strict, ctx) {
            Ok(None) => Ok(v),
            Ok(Some(coerced)) => Ok(coerced),
            Err(()) => {
                let fname = self.func_display_name(proto, ctx);
                let called = self.stack[top].called_class;
                Err(Thrown::error(
                    "TypeError",
                    format!(
                        "{}(): Return value must be of type {}, {} returned",
                        fname,
                        self.type_string_resolved(ty, ctx, called),
                        self.zval_value_name(&deref(&v)),
                    ),
                ))
            }
        }
    }

    /// Key of a function-static slot: the proto plus, for a method, the class
    /// it runs as. A method declared in a class always runs with that class,
    /// but a trait method is copied into every using class and each copy has
    /// its own statics (php duplicates the op_array per using class).
    fn static_key(&self, top: usize, proto: &Rc<FunctionProto>, sname: Vec<u8>) -> StaticKey {
        let cls = if proto.name.as_bytes().starts_with(b"{closure") {
            None
        } else {
            self.stack[top].defining_class
        };
        (Rc::as_ptr(proto) as usize, cls, sname)
    }

    /// php compiles a statically resolved `call_user_func()` /
    /// `call_user_func_array()` into a direct user call (ZEND_INIT_USER_CALL),
    /// so the callee sees the caller's `strict_types` and the call site; a
    /// namespaced unqualified call (runtime resolution) stays an internal call.
    fn mark_user_call_builtin(&mut self, fname: &[u8], fallback: NIdx) {
        self.cuf_caller = None;
        if fallback == rhp_bytecode::NO_NAME
            && (fname.eq_ignore_ascii_case(b"call_user_func")
                || fname.eq_ignore_ascii_case(b"call_user_func_array"))
        {
            self.cuf_caller = Some(self.caller_strict());
        }
    }

    /// The strict-types mode of the current caller (top of stack), for argument
    /// coercion of a call about to be made.
    fn caller_strict(&self) -> bool {
        self.stack
            .last()
            .map(|f| f.proto.strict_types)
            .unwrap_or(false)
    }

    fn value_debug_type(&self, v: &Value) -> String {
        match v {
            Value::Object(h) => self
                .class_name(h.borrow().class)
                .map(|n| String::from_utf8_lossy(&n).into_owned())
                .unwrap_or_else(|| "object".into()),
            Value::Closure(_) => "Closure".into(),
            Value::Null => "null".into(),
            Value::Bool(_) => "bool".into(),
            Value::Int(_) => "int".into(),
            Value::Float(_) => "float".into(),
            Value::Str(_) => "string".into(),
            Value::Array(_) => "array".into(),
            Value::Resource(_) => "resource".into(),
            Value::Ref(r) => self.value_debug_type(&r.borrow()),
        }
    }

    /// Bind a frame's superglobal CVs to their shared global cells.
    fn bind_superglobals(&mut self, frame: &mut Frame) {
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

    /// `fgets` over any readable stream, optionally capped at `max` bytes.
    fn stream_gets_impl(&mut self, res: &Value, max: Option<usize>) -> Option<Vec<u8>> {
        let id = res_id(res)?;
        match self.streams.get_mut(&id)? {
            StreamState::Mem { buf, pos, eof, .. } => {
                if *pos >= buf.len() {
                    *eof = true;
                    return None;
                }
                let start = *pos;
                let nl = buf[start..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|i| start + i + 1);
                let mut end = nl.unwrap_or(buf.len());
                let capped = max.is_some_and(|m| start + m < end);
                if let Some(m) = max {
                    end = end.min(start + m);
                }
                // A line that runs to the end of the data (no newline) makes
                // php look for more, which sets eof.
                if nl.is_none() && !capped {
                    *eof = true;
                }
                let line = buf[start..end].to_vec();
                *pos = end;
                Some(line)
            }
            StreamState::File(f) => f.gets(max),
            // FdStream::gets takes php's `$length` (it reads `length - 1`).
            StreamState::Fd(f) => f.gets(max.map(|m| m + 1)),
            StreamState::Stdin => self.stdin_stream()?.gets(max.map(|m| m + 1)),
            _ => None,
        }
    }

    /// Free a stream's backing state (fclose / last reference dropped):
    /// closes the descriptor (releasing flock locks) and flushes buffered
    /// wrapper writes.
    fn free_stream(&mut self, id: u32) -> bool {
        self.stream_origins.remove(&id);
        self.stream_modes.remove(&id);
        match self.streams.remove(&id) {
            Some(StreamState::FileWrite { buf, url }) => self
                .wrapper_for(&url)
                .map(|w| w.put_contents(&url, &buf))
                .is_some(),
            Some(_) => true,
            None => false,
        }
    }

    /// Free the streams whose last resource reference has been dropped.
    fn reap_dropped_streams(&mut self) {
        for id in rhp_values::take_dropped_streams() {
            // The CLI std streams live for the whole request.
            if matches!(
                self.streams.get(&id),
                Some(StreamState::Stdin | StreamState::Stdout | StreamState::Stderr)
            ) {
                continue;
            }
            if self.streams.contains_key(&id) {
                self.free_stream(id);
            } else if let Some(data) = self.native_resources.remove(&id) {
                builtins::native_resource_dtor(self, data);
            }
        }
    }

    /// The internal `Directory` class (readonly props, not dynamic, not
    /// cloneable, only created by `dir()`). User code cannot declare a class
    /// of that name, so the name identifies it.
    fn is_directory_class(&self, cls: ClassId) -> bool {
        let n = self.class(cls).name.as_bytes();
        n.len() == 9 && n.eq_ignore_ascii_case(b"Directory")
    }

    /// Allocate a stream resource backed by `state`, returning its resource Value.
    fn make_stream(&mut self, state: StreamState, type_name: &'static str) -> Value {
        let id = self.next_resource_id;
        self.next_resource_id += 1;
        self.streams.insert(id, state);
        Value::Resource(Rc::new(rhp_values::ResourceData { id, type_name }))
    }

    /// Set (or override) an engine constant, e.g. the SAPI's `PHP_BINARY`.
    pub fn set_constant(&mut self, name: &str, value: Value) {
        self.constants.insert(name.as_bytes().to_vec(), value);
    }

    /// Record the top-level script's filesystem path (for `Phar::mapPhar`).
    pub fn set_main_script_path(&mut self, path: impl Into<Vec<u8>>) {
        self.main_script_path = Some(path.into());
    }

    /// The stream wrapper for a URL's scheme (default `file://`).
    fn wrapper_for(&self, url: &[u8]) -> Option<&Rc<dyn builtins::StreamWrapper>> {
        let (scheme, _) = builtins::streams::split_scheme(url);
        self.wrappers.get(scheme)
    }

    /// Install the SAPI's file-inclusion callback (parse + compile a path's
    /// source to a `Program`). Without it, `include`/`require` raise an error.
    pub fn set_includer(
        &mut self,
        f: impl FnMut(&str, &[u8]) -> Result<Program, IncludeError> + 'static,
    ) {
        self.includer = Some(Box::new(f));
    }

    /// Write script output straight to the process stdout (unbuffered, like
    /// the php CLI) instead of collecting it for [`Vm::output`], so it
    /// interleaves correctly with stderr and child-process output.
    pub fn set_stdout_passthrough(&mut self, on: bool) {
        self.stdout_passthrough = on;
    }

    /// Unbuffered-by-default script output (see [`Vm::set_stdout_passthrough`]).
    fn emit_stdout(&mut self, data: &[u8]) {
        if self.stdout_passthrough || self.direct_stdout {
            use std::io::Write;
            let mut so = std::io::stdout().lock();
            let _ = so.write_all(data);
            let _ = so.flush();
        } else {
            self.out.extend_from_slice(data);
        }
    }

    pub fn output(&self) -> &[u8] {
        &self.out
    }

    /// The `STDIN` reader (a dup of fd 0), created on first use.
    fn stdin_stream(&mut self) -> Option<&mut builtins::fdstream::FdStream> {
        if self.stdin_reader.is_none() {
            let fd = builtins::fdstream::dup_raw(0)?;
            self.stdin_reader = Some(builtins::fdstream::FdStream::new(fd));
        }
        self.stdin_reader.as_mut()
    }

    // -- object destruction & request shutdown ---------------------------------

    /// Remember a new object whose class defines `__destruct`.
    pub(crate) fn track_destructible(&mut self, h: &ObjectHandle) {
        let cls = h.borrow().class;
        if self.lookup_method(cls, b"__destruct").is_some() {
            self.destructibles.push(h.clone());
        }
    }

    /// Call `__destruct` on `h` (already removed from the tracking list, so it
    /// runs at most once).
    fn call_destructor(&mut self, h: ObjectHandle) -> Result<(), Thrown> {
        if h.borrow().id < self.destructed_below {
            return Ok(()); // marked destructed by a fatal error's bailout
        }
        let cls = h.borrow().class;
        if let Some(m) = self.lookup_method(cls, b"__destruct") {
            self.call_sync(&m, Some(h), cls, Vec::new())?;
        }
        Ok(())
    }

    /// Destruct every tracked object nothing else references any more (php
    /// destructs on the refcount reaching zero), in creation order.
    fn collect_released(&mut self) -> Result<(), Thrown> {
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
    fn collect_released_now(&mut self) -> Result<(), Thrown> {
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

    /// php's request shutdown after the script finished (normally, through
    /// `exit`, or with an uncaught throwable already dealt with), for hosts
    /// that report uncaught throwables themselves: like [`Vm::run_main`]'s
    /// sequence, but a throwable escaping a shutdown function or destructor
    /// goes to `on_fatal` (which returns the resulting exit code).
    pub fn request_shutdown(&mut self, code: i32, on_fatal: &mut dyn FnMut(Thrown) -> i32) -> i32 {
        let r = self.shutdown_sequence(code, &mut |_vm, t| match t {
            Thrown::Exit(c) => Ok(Some(c)),
            t => Ok(Some(on_fatal(t))),
        });
        r.unwrap_or(255)
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
        // 1. shutdown functions
        let mut i = 0;
        while i < self.shutdown_functions.len() {
            let (cb, args) = self.shutdown_functions[i].clone();
            i += 1;
            self.enter_shutdown_scope();
            if let Err(t) = self.call_callable_value(&cb, args) {
                self.enter_shutdown_scope();
                if let Some(c) = on_throw(self, t)? {
                    status = c;
                    break;
                }
            }
        }
        self.shutdown_functions.clear();
        // 2. destructors
        self.enter_shutdown_scope();
        if let Err(t) = self.shutdown_destructors() {
            self.enter_shutdown_scope();
            if let Some(c) = on_throw(self, t)? {
                status = c;
            }
        }
        self.destructibles.clear();
        // 3. php_output_end_all: flush each buffer into the one below it.
        while let Some(buf) = self.ob_stack.pop() {
            builtins::Host::write_out(self, &buf);
        }
        self.stack.clear();
        Ok(status)
    }

    /// The stack shutdown code runs on: just {main}'s frame (so `global`
    /// still reaches the script's variables), shown in traces as php's
    /// `[internal function]` caller. Frames left above it by an earlier
    /// shutdown step are dropped.
    fn enter_shutdown_scope(&mut self) {
        if self.in_shutdown_scope && !self.stack.is_empty() {
            self.stack.truncate(1);
            return;
        }
        self.stack.clear();
        match self.finished_main.take() {
            Some(mut main) => {
                let mut p = (*main.proto).clone();
                p.debug.file = rhp_values::PhpStr::from("");
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
        // by destructors included).
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
    pub fn run(&mut self, prog: &Program) -> Result<i32, Thrown> {
        // Bind the standard stream resources (CLI SAPI) once.
        if self.streams.is_empty() {
            // Forget drops left over from a previous VM on this thread.
            rhp_values::take_dropped_streams();
            let stdin = self.make_stream(StreamState::Stdin, "stream");
            let stdout = self.make_stream(StreamState::Stdout, "stream");
            let stderr = self.make_stream(StreamState::Stderr, "stream");
            self.constants.insert(b"STDIN".to_vec(), stdin);
            self.constants.insert(b"STDOUT".to_vec(), stdout);
            self.constants.insert(b"STDERR".to_vec(), stderr);
            self.seed_superglobals();
        }
        // Install (hoist) all declared functions, keyed case-insensitively.
        for (idx, proto) in prog.functions.iter().enumerate() {
            self.functions.push(proto.clone());
            self.fn_names
                .insert(proto.name.as_bytes().to_ascii_lowercase(), idx);
        }
        // Register anonymous closure / arrow-function protos (indexed by fnidx).
        self.closures = prog.closures.clone();
        // Install user-defined global constants.
        for (name, value) in &prog.constants {
            if self
                .constants
                .insert(name.as_bytes().to_vec(), value.clone())
                .is_none()
            {
                self.user_const_order.push(name.as_bytes().to_vec());
            }
        }
        // Link the built-in prelude classes, then hoist the user's top-level
        // classes (those whose dependencies are available); the rest, and
        // nested declarations, are declared when execution reaches them.
        self.install_classes(&prog.classes[..prog.num_prelude_classes])?;
        self.decl_classes.clear();
        self.decl_runtime.clear();
        let top = self.register_decl_classes(prog, 0);
        self.hoist_decl_classes(top)?;
        let mut main_frame = Frame::new(prog.main.clone(), 0);
        self.bind_superglobals(&mut main_frame);
        // {main}'s variables are the global symbol table's entries.
        self.bind_global_scope(&mut main_frame);
        self.stack.push(main_frame);
        match self.run_loop() {
            Ok(_) => Ok(0),
            Err(Thrown::Exit(code)) => Ok(code),
            Err(other) => Err(other),
        }
    }

    // -- register access: deref on read, write-through on a Ref slot --

    fn reg_get(&self, top: usize, r: Reg) -> Value {
        match &self.stack[top].regs[r as usize] {
            Value::Ref(rc) => rc.borrow().clone(),
            v => v.clone(),
        }
    }

    /// Plain register write (temporaries / instruction destinations): overwrites
    /// the slot. Never writes through a stale `Ref` left in a reused temp.
    fn reg_set(&mut self, top: usize, r: Reg, v: Value) {
        let slot = &mut self.stack[top].regs[r as usize];
        if !self.destructibles.is_empty() && matches!(slot, Value::Object(_) | Value::Array(_)) {
            self.dtor_check = true;
        }
        *slot = v;
    }

    /// Assign to a PHP *variable* (a CV): if the slot is a reference, write
    /// through it (so by-ref aliases see the change); otherwise overwrite.
    fn cv_assign(&mut self, top: usize, cv: Reg, v: Value) {
        let slot = &mut self.stack[top].regs[cv as usize];
        let old = if let Value::Ref(rc) = slot {
            std::mem::replace(&mut *rc.borrow_mut(), v)
        } else {
            std::mem::replace(slot, v)
        };
        if !self.destructibles.is_empty() && matches!(old, Value::Object(_) | Value::Array(_)) {
            self.dtor_check = true;
        }
    }

    /// Read the value at the current cursor (root + steps) for a compound
    /// assignment / increment: a magic property along the way reads via `__get`.
    fn cursor_read(&mut self, top: usize) -> Result<Value, Thrown> {
        self.cursor_settle_offsets(top)?;
        let steps = std::mem::take(&mut self.cursor_steps);
        let r = self.cursor_read_magic(top, &steps, false);
        self.cursor_steps = steps;
        r
    }

    /// `incdec`: for `++`/`--` (else a compound assignment), which only
    /// changes the error for a string offset.
    fn cursor_read_magic(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        incdec: bool,
    ) -> Result<Value, Thrown> {
        match self.cursor_magic_split(top, steps) {
            MagicSplit::Plain => {
                self.cursor_rw_checks(top, steps, incdec)?;
                Ok(self.cursor_read_steps(top, steps))
            }
            MagicSplit::Leaf(h, name) => self.prop_fetch(top, Value::Object(h), name),
            MagicSplit::Nested(i, h, name) => {
                let got = self.prop_fetch(top, Value::Object(h), name)?;
                let cx = PropCtx {
                    classes: &self.classes,
                    scope: self.stack[top].defining_class,
                    called: self.stack[top].called_class,
                };
                Ok(read_path(&got, &steps[i + 1..], &cx))
            }
            MagicSplit::Offset(i, h) => {
                let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                if i + 1 == steps.len() {
                    return Ok(deref(&self.aa_offset_get(&h, key)?));
                }
                let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::Rw)?;
                let rest = &steps[i + 1..];
                self.with_cursor_root(cell, |vm| vm.cursor_read_magic(top, rest, incdec))
            }
        }
    }

    /// The diagnostics of php's read-for-write fetches along a compound
    /// assignment / `++` path (FETCH_DIM_RW / FETCH_OBJ_RW, then the leaf
    /// read): "Undefined array key" for each key missing from an array (or
    /// indexed under a null about to be autovivified), "Undefined property"
    /// for an absent property, and the "must not be accessed before
    /// initialization" Error for an uninitialized typed one. Containers the
    /// write itself rejects (scalars, strings, ArrayAccess) end the walk.
    fn cursor_rw_checks(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        incdec: bool,
    ) -> Result<(), Thrown> {
        // `None`: the slot does not exist yet (the write creates it as null).
        let mut cur = Some(deref(&self.cursor_root_value(top)));
        let mut warnings: Vec<String> = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            cur = match (cur, step) {
                // A string container: no read-modify-write through its offsets.
                (Some(Value::Str(_)), CursorStep::Append) => {
                    self.flush_rw_warnings(warnings)?;
                    return Err(Thrown::error(
                        "Error",
                        "[] operator not supported for strings",
                    ));
                }
                (Some(Value::Str(_)), CursorStep::Key(k)) => {
                    self.flush_rw_warnings(warnings)?;
                    let cx = PropCtx {
                        classes: &self.classes,
                        scope: self.stack[top].defining_class,
                        called: self.stack[top].called_class,
                    };
                    let checked = string_offset(k, false, &cx);
                    self.flush_path_diags()?;
                    checked?;
                    let rest = &steps[i + 1..];
                    return Err(match rest.first() {
                        None if incdec => {
                            Thrown::error("Error", "Cannot increment/decrement string offsets")
                        }
                        None => Thrown::error(
                            "Error",
                            "Cannot use assign-op operators with string offsets",
                        ),
                        Some(_) => string_offset_misuse(rest, false),
                    });
                }
                (None | Some(Value::Null), CursorStep::Key(k)) => {
                    match ArrayKey::from_value(&deref(k)) {
                        Some(key) => warnings.push(undefined_key_msg(&key)),
                        None => break,
                    }
                    None
                }
                (Some(Value::Array(a)), CursorStep::Key(k)) => {
                    let Some(key) = ArrayKey::from_value(&deref(k)) else {
                        break;
                    };
                    match a.get(&key) {
                        Some(v) => Some(deref(v)),
                        None => {
                            warnings.push(undefined_key_msg(&key));
                            None
                        }
                    }
                }
                (None | Some(Value::Null) | Some(Value::Array(_)), CursorStep::Append) => None,
                (Some(Value::Object(h)), CursorStep::Prop(name)) => {
                    let cls = h.borrow().class;
                    let scope = self.stack[top].defining_class;
                    let key = match classes::resolve_prop(&self.classes, cls, scope, name) {
                        PropRes::Slot(k) => k,
                        PropRes::Dynamic => name.clone(),
                        PropRes::Denied(_) => break,
                    };
                    let present = h.borrow().properties.get(&key).map(deref);
                    match present {
                        Some(v) => Some(v),
                        None => {
                            // Absent and typed: never initialized, or unset
                            // with no `__get` to defer to.
                            if let Some(e) = self.typed_slot(cls, &key) {
                                return Err(e);
                            }
                            warnings.push(format!(
                                "Undefined property: {}::${}",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                String::from_utf8_lossy(name.as_bytes())
                            ));
                            None
                        }
                    }
                }
                _ => break,
            };
        }
        self.flush_rw_warnings(warnings)
    }

    fn flush_rw_warnings(&mut self, warnings: Vec<String>) -> Result<(), Thrown> {
        for w in warnings {
            self.diag(2, &w)?;
        }
        Ok(())
    }

    /// The cursor root's current value.
    fn cursor_root_value(&self, top: usize) -> Value {
        match &self.cursor_root {
            CursorRoot::Cv(r) => self.stack[top].regs[*r as usize].clone(),
            CursorRoot::Cell(rc) => rc.borrow().clone(),
        }
    }

    /// Walk the cursor path to the first property php would route through
    /// `__get` (zend_std_get_property_ptr_ptr returns NULL for a missing or
    /// inaccessible property when the class has an unguarded `__get`).
    fn cursor_magic_split(&self, top: usize, steps: &[CursorStep]) -> MagicSplit {
        let scope = self.stack[top].defining_class;
        let mut cur = self.cursor_root_value(top);
        for (i, step) in steps.iter().enumerate() {
            let next = match (deref(&cur), step) {
                (Value::Array(a), CursorStep::Key(k)) => {
                    match ArrayKey::from_value(k).and_then(|key| a.get(&key).cloned()) {
                        Some(v) => v,
                        None => return MagicSplit::Plain,
                    }
                }
                (Value::Object(h), CursorStep::Prop(name)) => {
                    let cls = h.borrow().class;
                    let present = match classes::resolve_prop(&self.classes, cls, scope, name) {
                        PropRes::Slot(k) => h.borrow().properties.get(&k).cloned(),
                        PropRes::Dynamic => h.borrow().properties.get(name).cloned(),
                        PropRes::Denied(_) => None,
                    };
                    let uninit = match classes::resolve_prop(&self.classes, cls, scope, name) {
                        PropRes::Slot(k) => present.is_none() && self.uninit_typed(&h, &k),
                        _ => false,
                    };
                    match present {
                        Some(v) => v,
                        None if uninit => return MagicSplit::Plain,
                        None if self.magic(&h, cls, b"__get", name).is_some() => {
                            return if i + 1 == steps.len() {
                                MagicSplit::Leaf(h, name.clone())
                            } else {
                                MagicSplit::Nested(i, h, name.clone())
                            };
                        }
                        None => return MagicSplit::Plain,
                    }
                }
                (Value::Object(h), CursorStep::Key(_) | CursorStep::Append) => {
                    let cls = h.borrow().class;
                    return if self.class_implements(cls, b"ArrayAccess") {
                        MagicSplit::Offset(i, h)
                    } else {
                        MagicSplit::Plain
                    };
                }
                _ => return MagicSplit::Plain,
            };
            cur = next;
        }
        MagicSplit::Plain
    }

    /// How many compiler-temporary registers (not CVs) across the live frames
    /// hold a `Ref` to `rc`. Such leftovers are not PHP-visible references.
    fn temp_ref_holders(&self, rc: &Rc<RefCell<Value>>) -> usize {
        self.stack
            .iter()
            .map(|f| {
                let ncv = f.proto.num_cvs as usize;
                f.regs
                    .iter()
                    .skip(ncv)
                    .filter(|v| matches!(v, Value::Ref(r) if Rc::ptr_eq(r, rc)))
                    .count()
            })
            .sum()
    }

    /// Run `f` with the cursor temporarily re-rooted at `cell`.
    fn with_cursor_root<T>(
        &mut self,
        cell: Rc<RefCell<Value>>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let saved = std::mem::replace(&mut self.cursor_root, CursorRoot::Cell(cell));
        let r = f(self);
        self.cursor_root = saved;
        r
    }

    /// The cell a nested write below a magic property lands in: the `__get`
    /// result itself when it is a reference, else a temporary (php: "Indirect
    /// modification of overloaded property has no effect", which still lets
    /// writes reach an object the temporary holds).
    fn magic_write_cell(
        &mut self,
        top: usize,
        h: ObjectHandle,
        name: PhpStr,
    ) -> Result<Rc<RefCell<Value>>, Thrown> {
        let got = self.prop_fetch(top, Value::Object(h), name)?;
        Ok(match got {
            Value::Ref(rc) => rc,
            other => Rc::new(RefCell::new(other)),
        })
    }

    /// Promote the cursor target to a shared cell (`&$o->p[...]`, by-ref
    /// argument passing); a magic property along the way yields its `__get`
    /// result (or a temporary) like php's BP_VAR_W fetch.
    fn cursor_make_ref(
        &mut self,
        top: usize,
        steps: &[CursorStep],
    ) -> Result<Rc<RefCell<Value>>, Thrown> {
        self.cursor_readonly_check(top, steps, true)?;
        let (i, h, name) = match self.cursor_magic_split(top, steps) {
            MagicSplit::Plain => {
                let cx = PropCtx {
                    classes: &self.classes,
                    scope: self.stack[top].defining_class,
                    called: self.stack[top].called_class,
                };
                let r = match &self.cursor_root {
                    CursorRoot::Cv(r) => {
                        let r = *r as usize;
                        make_ref_path(&mut self.stack[top].regs[r], steps, &cx)
                    }
                    CursorRoot::Cell(rc) => {
                        let rc = rc.clone();
                        let c = make_ref_path(&mut rc.borrow_mut(), steps, &cx);
                        c
                    }
                };
                self.flush_path_diags()?;
                return r;
            }
            MagicSplit::Leaf(h, name) => (steps.len() - 1, h, name),
            MagicSplit::Nested(i, h, name) => (i, h, name),
            MagicSplit::Offset(i, h) => {
                let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::W)?;
                let rest = &steps[i + 1..];
                if rest.is_empty() {
                    return Ok(cell);
                }
                return self.with_cursor_root(cell, |vm| vm.cursor_make_ref(top, rest));
            }
        };
        let cell = self.magic_write_cell(top, h, name)?;
        let rest = &steps[i + 1..];
        if rest.is_empty() {
            return Ok(cell);
        }
        self.with_cursor_root(cell, |vm| vm.cursor_make_ref(top, rest))
    }

    fn cursor_read_steps(&self, top: usize, steps: &[CursorStep]) -> Value {
        let cx = PropCtx {
            classes: &self.classes,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
        };
        match &self.cursor_root {
            CursorRoot::Cv(r) => read_path(&self.stack[top].regs[*r as usize], steps, &cx),
            CursorRoot::Cell(rc) => read_path(&rc.borrow(), steps, &cx),
        }
    }

    /// Readonly properties along a cursor path: writing the property itself
    /// follows the readonly rules; writing *through* it (`$o->ro[] = 1`) or
    /// taking a reference is "Cannot indirectly modify readonly property",
    /// unless it holds an object.
    fn cursor_readonly_check(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        make_ref: bool,
    ) -> Result<(), Thrown> {
        let scope = self.stack[top].defining_class;
        let mut cur = self.cursor_root_value(top);
        for (i, step) in steps.iter().enumerate() {
            let next = match (deref(&cur), step) {
                (Value::Array(a), CursorStep::Key(k)) => {
                    match ArrayKey::from_value(k).and_then(|key| a.get(&key).cloned()) {
                        Some(v) => v,
                        None => return Ok(()),
                    }
                }
                (Value::Object(h), CursorStep::Prop(name)) => {
                    let cls = h.borrow().class;
                    let PropRes::Slot(key) = classes::resolve_prop(&self.classes, cls, scope, name)
                    else {
                        return Ok(());
                    };
                    let val = h.borrow().properties.get(&key).cloned();
                    if let Some(pe) = self.class(cls).props.get(&key).filter(|pe| pe.readonly) {
                        let last = i + 1 == steps.len();
                        if last && !make_ref {
                            self.check_readonly_write(&h, &key, val.is_some(), scope)?;
                        } else if !matches!(val.as_ref().map(deref), Some(Value::Object(_))) {
                            return Err(Thrown::error(
                                "Error",
                                format!(
                                    "Cannot indirectly modify readonly property {}::${}",
                                    String::from_utf8_lossy(
                                        self.class(pe.declaring_class).name.as_bytes()
                                    ),
                                    String::from_utf8_lossy(pe.name.as_bytes())
                                ),
                            ));
                        }
                    }
                    match val {
                        Some(v) => v,
                        None => return Ok(()),
                    }
                }
                _ => return Ok(()),
            };
            cur = next;
        }
        Ok(())
    }

    /// Write `v` at the current cursor (root + steps).
    fn cursor_write(&mut self, top: usize, steps: &[CursorStep], v: Value) -> Result<(), Thrown> {
        self.cursor_readonly_check(top, steps, false)?;
        match self.cursor_magic_split(top, steps) {
            MagicSplit::Plain => {}
            MagicSplit::Leaf(h, name) => return self.prop_store(top, Value::Object(h), name, v),
            MagicSplit::Nested(i, h, name) => {
                let cell = self.magic_write_cell(top, h, name)?;
                let rest = &steps[i + 1..];
                return self.with_cursor_root(cell, |vm| vm.cursor_write(top, rest, v));
            }
            // `$obj[$k] = v` / `$obj[] = v` on an ArrayAccess object ->
            // offsetSet; below it, the element is fetched with offsetGet.
            MagicSplit::Offset(i, h) => {
                let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                if i + 1 == steps.len() {
                    let cls = h.borrow().class;
                    if let Some(m) = self.lookup_method(cls, b"offsetset") {
                        self.call_sync(&m, Some(h), cls, vec![key, v])?;
                    }
                    return Ok(());
                }
                let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::W)?;
                let rest = &steps[i + 1..];
                return self.with_cursor_root(cell, |vm| vm.cursor_write(top, rest, v));
            }
        }
        let cx = PropCtx {
            classes: &self.classes,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
        };
        let r = match &self.cursor_root {
            CursorRoot::Cv(r) => {
                let r = *r as usize;
                assign_path(&mut self.stack[top].regs[r], steps, v, &cx)
            }
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                let r = assign_path(&mut rc.borrow_mut(), steps, v, &cx);
                r
            }
        };
        self.flush_path_diags()?;
        // The overwritten element may have held the last reference to an
        // object: its destructor runs before the next statement, as in php.
        if !self.destructibles.is_empty() {
            self.dtor_check = true;
        }
        r
    }

    /// Source line of the instruction currently executing in frame `top`.
    fn current_line(&self, top: usize) -> u32 {
        let f = &self.stack[top];
        let pc = f.pc.saturating_sub(1) as u32;
        let lines = &f.proto.debug.lines;
        // Largest (pc,line) entry whose pc <= the current pc.
        let idx = lines.partition_point(|(p, _)| *p <= pc);
        if idx == 0 {
            0
        } else {
            lines[idx - 1].1
        }
    }

    /// The source file name of frame `top` (for exception getFile()).
    fn current_file(&self, top: usize) -> Vec<u8> {
        self.stack[top].proto.debug.file.as_bytes().to_vec()
    }

    /// Stamp an unlocated refusal with the innermost user frame's file and
    /// line (engine-internal prelude frames have no source to point at).
    fn locate_unsupported(&self, t: Thrown) -> Thrown {
        if !matches!(t, Thrown::Unsupported { location: None, .. }) {
            return t;
        }
        match (0..self.stack.len())
            .rev()
            .find(|&i| !self.is_internal_frame(i))
        {
            Some(i) => t.with_location(
                || String::from_utf8_lossy(&self.current_file(i)).into_owned(),
                self.current_line(i),
            ),
            None => t,
        }
    }

    // -- exceptions: throw site, backtrace, engine-error materialization --

    /// Frame `i` runs engine-internal code (the prelude's PHP-source classes /
    /// helpers, compiled with an empty file name): php has no user frame there.
    fn is_internal_frame(&self, i: usize) -> bool {
        self.stack[i].proto.debug.file.is_empty()
    }

    /// A builtin called from frame `i` is still running (so a frame above `i`
    /// was invoked by it, i.e. from `[internal function]`).
    fn native_active_from(&self, i: usize) -> bool {
        self.native_calls.iter().any(|n| n.caller == i)
    }

    /// Where php's `zend_get_executed_filename/lineno` point: the innermost
    /// user-code frame's current file and line.
    fn throw_site(&self) -> (Vec<u8>, u32) {
        let n = self.stack.len();
        let Some(i) = (0..n)
            .rev()
            .find(|&i| !self.is_internal_frame(i))
            .or(n.checked_sub(1))
        else {
            return (Vec::new(), 0);
        };
        (self.current_file(i), self.current_line(i))
    }

    /// The call site of a frame about to be pushed by the current top frame,
    /// when that caller is user code (php appends ", called in %s on line %d"
    /// / "passed in %s on line %d" only then).
    fn user_call_site(&self) -> Option<(Vec<u8>, u32)> {
        let top = self.stack.len().checked_sub(1)?;
        if self.is_internal_frame(top) || self.native_active_from(top) {
            return None;
        }
        Some((self.current_file(top), self.current_line(top)))
    }

    /// zend_type_to_string_resolved: a declared type as php prints it in
    /// TypeErrors -- class names first (self/parent resolved against `scope`,
    /// static against `called`), then the builtin types in php's fixed order.
    fn type_string_resolved(
        &self,
        ty: &rhp_bytecode::TypeHint,
        scope: Option<ClassId>,
        called: Option<ClassId>,
    ) -> String {
        use rhp_bytecode::TypeKind as K;
        let cname =
            |c: ClassId| String::from_utf8_lossy(self.class(c).name.as_bytes()).into_owned();
        let has = |k: &K| ty.kinds.contains(k);
        if has(&K::Mixed) {
            return "mixed".into();
        }
        let mut parts: Vec<String> = Vec::new();
        for k in &ty.kinds {
            match k {
                K::Named(n) => parts.push(String::from_utf8_lossy(n.as_bytes()).into_owned()),
                K::SelfType => parts.push(scope.map(cname).unwrap_or_else(|| "self".into())),
                K::Parent => parts.push(
                    scope
                        .and_then(|c| self.class(c).parent)
                        .map(cname)
                        .unwrap_or_else(|| "parent".into()),
                ),
                K::Iterable => parts.push("Traversable".into()),
                _ => {}
            }
        }
        if has(&K::StaticType) {
            let name = match (scope, called) {
                (Some(_), Some(c)) => cname(c),
                _ => "static".into(),
            };
            parts.push(name);
        }
        let array = has(&K::Array) || has(&K::Iterable);
        let bool_ = has(&K::Bool) || (has(&K::True) && has(&K::False));
        for (on, name) in [
            (has(&K::Callable), "callable"),
            (has(&K::Object), "object"),
            (array, "array"),
            (has(&K::String), "string"),
            (has(&K::Int), "int"),
            (has(&K::Float), "float"),
            (bool_, "bool"),
            (!bool_ && has(&K::False), "false"),
            (!bool_ && has(&K::True), "true"),
            (has(&K::Void), "void"),
            (has(&K::Never), "never"),
        ] {
            if on {
                parts.push(name.into());
            }
        }
        let sep = if ty.intersection { "&" } else { "|" };
        if ty.nullable || has(&K::Null) {
            if parts.is_empty() {
                return "null".into();
            }
            if parts.len() == 1 {
                return format!("?{}", parts[0]);
            }
            let mut out = parts.join(sep);
            out.push_str("|null");
            return out;
        }
        parts.join(sep)
    }

    /// php's display name for a function in messages: `Class::fn` when scoped.
    fn func_display_name(&self, proto: &FunctionProto, scope: Option<ClassId>) -> String {
        let f = String::from_utf8_lossy(proto.name.as_bytes());
        match scope {
            Some(c) => format!(
                "{}::{}",
                String::from_utf8_lossy(self.class(c).name.as_bytes()),
                f
            ),
            None => f.into_owned(),
        }
    }

    /// The trace `args` of frame `i`: the passed arguments' current values.
    fn frame_args(&self, i: usize) -> PhpArray {
        let f = &self.stack[i];
        let mut args = PhpArray::new();
        for (k, p) in f.proto.params.iter().enumerate() {
            if p.variadic {
                if let Value::Array(a) = deref(&f.regs[p.cv as usize]) {
                    for (key, v) in a.iter() {
                        match key {
                            ArrayKey::Int(_) => args.push(deref(v)),
                            ArrayKey::Str(_) => {
                                args.insert(key.clone(), deref(v));
                            }
                        }
                    }
                }
                break;
            }
            if k >= f.argc {
                break;
            }
            args.push(deref(&f.regs[p.cv as usize]));
        }
        for v in &f.extra_args {
            args.push(deref(v));
        }
        args
    }

    /// php's `zend_fetch_debug_backtrace` as stored in a Throwable's `trace`:
    /// innermost first; each user frame paired with its call site (absent for a
    /// call made by a builtin: `[internal function]`), each running builtin as
    /// its own frame, `include`/`require`/`eval` as pseudo-frames.
    fn build_trace(&self) -> PhpArray {
        self.build_trace_ex(false, false)
    }

    /// [`build_trace`] with debug_backtrace's options: `$trace['object']` for
    /// frames with a `$this`, and no `args`.
    fn build_trace_ex(&self, provide_object: bool, ignore_args: bool) -> PhpArray {
        // Keys and file names are shared (Rc) rather than re-allocated per
        // frame: a trace is built at every Throwable creation.
        let key = |s: &str| ArrayKey::Str(rhp_values::PhpStr::from(s));
        let keys = [
            key("file"),
            key("line"),
            key("function"),
            key("class"),
            key("object"),
            key("type"),
            key("args"),
        ];
        let k = |s: &[u8]| -> ArrayKey {
            let i = match s {
                b"file" => 0,
                b"line" => 1,
                b"function" => 2,
                b"class" => 3,
                b"object" => 4,
                b"type" => 5,
                _ => 6,
            };
            keys[i].clone()
        };
        let file_of = |i: usize| Value::Str(self.stack[i].proto.debug.file.clone());
        let mut out = PhpArray::new();
        for i in (0..self.stack.len()).rev() {
            if !self.is_internal_frame(i) {
                for n in self.native_calls.iter().rev().filter(|n| n.caller == i) {
                    let mut e = PhpArray::new();
                    e.insert(k(b"file"), file_of(i));
                    e.insert(k(b"line"), Value::Int(self.current_line(i) as i64));
                    e.insert(k(b"function"), Value::str(n.name));
                    if !ignore_args {
                        let mut a = PhpArray::new();
                        // SAFETY: see `NativeCall::args` — every entry
                        // still in `native_calls` points at a live slice.
                        let call_args = unsafe { &*n.args };
                        for v in call_args {
                            a.push(deref(v));
                        }
                        e.insert(k(b"args"), Value::Array(a));
                    }
                    out.push(Value::Array(e));
                }
            }
            if i == 0 {
                break;
            }
            let f = &self.stack[i];
            let mut e = PhpArray::new();
            if !self.is_internal_frame(i - 1) && !self.native_active_from(i - 1) {
                e.insert(k(b"file"), file_of(i - 1));
                e.insert(k(b"line"), Value::Int(self.current_line(i - 1) as i64));
            }
            if let Some(kind) = f.include_kind {
                e.insert(k(b"function"), Value::str(kind));
            } else {
                e.insert(k(b"function"), Value::Str(f.proto.name.clone()));
                if let Some(c) = f.defining_class {
                    e.insert(k(b"class"), Value::Str(self.class(c).name.clone()));
                    let this = self.current_this(i);
                    let ty = if this.is_some() { "->" } else { "::" };
                    if let (true, Some(o)) = (provide_object, this) {
                        e.insert(k(b"object"), Value::Object(o));
                    }
                    e.insert(k(b"type"), Value::str(ty));
                }
                if !ignore_args {
                    e.insert(k(b"args"), Value::Array(self.frame_args(i)));
                }
            }
            out.push(Value::Array(e));
        }
        out
    }

    /// The storage key of property `name` on `obj` as its class sees it,
    /// ignoring visibility: the most-derived declaration (a private one is
    /// stored mangled, `"\0Class\0name"`), else the plain (dynamic) name.
    /// Engine-internal accesses such as Exception's private `trace` /
    /// `previous` go through this.
    pub(crate) fn declared_prop_key(&self, obj: &ObjectHandle, name: &[u8]) -> rhp_values::PhpStr {
        let name = rhp_values::PhpStr::from_bytes(name);
        let cls = obj.borrow().class;
        self.class(cls)
            .prop_visible
            .get(&name)
            .cloned()
            .unwrap_or(name)
    }

    /// Stamp a freshly created Throwable with php's creation-site `file`,
    /// `line` and `trace` (zend_default_exception_new).
    fn init_throwable(&self, obj: &ObjectHandle) {
        let (file, line) = self.throw_site();
        let trace = self.build_trace();
        let (kf, kl, kt) = (
            self.declared_prop_key(obj, b"file"),
            self.declared_prop_key(obj, b"line"),
            self.declared_prop_key(obj, b"trace"),
        );
        let mut b = obj.borrow_mut();
        b.properties.insert(kf, Value::str(file));
        b.properties.insert(kl, Value::Int(line as i64));
        b.properties.insert(kt, Value::Array(trace));
    }

    /// Turn an engine-raised `Thrown::Error` into a real Throwable object, created
    /// *now* (so its file/line/trace are the throw point). `at` overrides the
    /// file/line (argument errors are raised at the callee's parameter line).
    fn materialize(&mut self, thrown: Thrown, at: Option<(Vec<u8>, u32)>) -> Thrown {
        let Thrown::Error { class, message } = &thrown else {
            return thrown;
        };
        let Ok(cls) = self.resolve_class_by_name(class.as_bytes()) else {
            return thrown;
        };
        let obj = self.new_object(cls);
        {
            let mut b = obj.borrow_mut();
            b.properties.insert(
                rhp_values::PhpStr::from("message"),
                Value::str(message.clone()),
            );
            if let Some((file, line)) = at {
                b.properties
                    .insert(rhp_values::PhpStr::from("file"), Value::str(file));
                b.properties
                    .insert(rhp_values::PhpStr::from("line"), Value::Int(line as i64));
            }
        }
        Thrown::Object(obj)
    }

    /// Bind `args` into `frame` (which already carries its `$this`/scope) and
    /// enforce declared parameter types. On failure php raises the
    /// ArgumentCountError / TypeError from *inside* the callee: at the failing
    /// parameter's line, with the callee on the trace.
    fn bind_frame(
        &mut self,
        frame: &mut Frame,
        args: &[Value],
        caller: Option<bool>,
    ) -> Result<(), Thrown> {
        let proto = frame.proto.clone();
        frame.argc = args.len();
        // A named-argument call armed `pending_named` for this frame.
        let named = self.pending_named.take();
        let failed = match bind_params(&proto, args, frame) {
            Err(missing) => Some((missing, self.arg_count_error(&proto, frame, args.len()))),
            Ok(()) => {
                if let Some((holes, extra)) = named {
                    frame.holes = holes;
                    if !extra.is_empty() {
                        if let Some(p) = proto.params.iter().find(|p| p.variadic) {
                            if let Value::Array(a) = &mut frame.regs[p.cv as usize] {
                                for (k, v) in extra {
                                    a.insert(ArrayKey::Str(k), deref(&v));
                                }
                            }
                        }
                    }
                }
                let scope = frame.defining_class;
                match self.coerce_stringable_args(&proto, frame, caller, scope) {
                    Err(e) => Some((0, e)),
                    Ok(()) => self.check_arg_types(&proto, frame, caller, scope).err(),
                }
            }
        };
        let Some((idx, err)) = failed else {
            return Ok(());
        };
        let line = proto.params.get(idx).map(|p| p.line).unwrap_or(0);
        let at = (proto.debug.file.as_bytes().to_vec(), line);
        let mut pushed = Frame::new(proto.clone(), Reg::MAX);
        std::mem::swap(&mut pushed, frame);
        self.stack.push(pushed);
        let t = self.materialize(err, Some(at));
        let mut popped = self.stack.pop().expect("pushed frame");
        std::mem::swap(&mut popped, frame);
        Err(t)
    }

    /// "Too few arguments to function f(), N passed[ in F on line L] and
    /// {exactly|at least} M expected" (zend_missing_arg_error).
    fn arg_count_error(&self, proto: &FunctionProto, frame: &Frame, passed: usize) -> Thrown {
        let required = required_params(proto);
        let declared = proto.params.iter().filter(|p| !p.variadic).count();
        let which = if required == declared {
            "exactly"
        } else {
            "at least"
        };
        let name = self.func_display_name(proto, frame.defining_class);
        let msg = match self.user_call_site() {
            Some((file, line)) => format!(
                "Too few arguments to function {name}(), {passed} passed in {} on line {line} and {which} {required} expected",
                String::from_utf8_lossy(&file)
            ),
            None => format!(
                "Too few arguments to function {name}(), {passed} passed and {which} {required} expected"
            ),
        };
        Thrown::error("ArgumentCountError", msg)
    }

    /// Push php's "no user code running" frame: callbacks invoked after the
    /// script (exception handler, shutdown functions) are called from
    /// `[internal function]`.
    fn push_internal_main(&mut self) {
        let mut main = FunctionProto::new(rhp_values::PhpStr::from("{main}"));
        main.debug.file = rhp_values::PhpStr::from("");
        self.stack.push(Frame::new(Rc::new(main), 0));
    }

    /// Give an uncaught throwable to the user exception handler, if one is set
    /// (zend_user_exception_handler). `None`: handled; `Some(t)`: still
    /// uncaught (no handler, or the handler itself threw `t`).
    pub fn handle_uncaught(&mut self, thrown: Thrown) -> Option<Thrown> {
        if !thrown.is_catchable() {
            return Some(thrown);
        }
        let handler = match self.exception_handlers.last() {
            Some(h) if !matches!(h, Value::Null) => h.clone(),
            _ => return Some(thrown),
        };
        let thrown = self.materialize(thrown, None);
        let Thrown::Object(obj) = &thrown else {
            return Some(thrown);
        };
        let arg = Value::Object(obj.clone());
        // The handler runs on the shutdown scope: {main}'s frame stays alive
        // (its globals are still reachable via `global`, and its objects are
        // destructed only after the shutdown functions).
        self.enter_shutdown_scope();
        let r = self.call_callable_value(&handler, vec![arg]);
        self.stack.truncate(1);
        r.err()
    }

    /// The class name of an object (e.g. an uncaught `Thrown::Object`).
    pub fn class_name_of(&self, obj: &ObjectHandle) -> String {
        String::from_utf8_lossy(self.class(obj.borrow().class).name.as_bytes()).into_owned()
    }

    /// CLI SAPI mode: pending unbuffered stdout is handed to the process
    /// before every stderr write, so stdout/stderr interleave as under php.
    pub fn set_direct_stdout(&mut self, on: bool) {
        self.direct_stdout = on;
    }

    /// Write pending unbuffered output to the process's stdout.
    pub fn flush_stdout(&mut self) {
        use std::io::Write;
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(&self.out);
        let _ = o.flush();
        self.out.clear();
    }

    /// Write to the process's stderr (after pending stdout, in CLI mode).
    fn write_stderr(&mut self, data: &[u8]) {
        use std::io::Write;
        if self.direct_stdout {
            self.flush_stdout();
        }
        let _ = std::io::stderr().write_all(data);
    }

    /// php_error_cb's log + display step for an error of type `ty` (an E_*
    /// bit): skipped when masked by error_reporting; logged to stderr as
    /// `PHP <label>:  …` under log_errors; displayed per display_errors
    /// (stdout through the output buffers, or stderr without the leading
    /// newline for "stderr").
    fn display_error(&mut self, ty: i64, label: &str, message: &[u8], file: &[u8], line: i64) {
        self.display_error_trace(ty, label, message, file, line, b"");
    }

    /// [`Vm::display_error`] with php 8.5's `fatal_error_backtraces` block
    /// (`Stack trace:\n#0 …`) after the location.
    fn display_error_trace(
        &mut self,
        ty: i64,
        label: &str,
        message: &[u8],
        file: &[u8],
        line: i64,
        trace: &[u8],
    ) {
        if builtins::php_fns::error_reporting_level(self) & ty == 0 {
            return;
        }
        let ini = |vm: &Self, n: &[u8]| vm.ini_get(n).unwrap_or_default();
        let log = ini_bool(&ini(self, b"log_errors"));
        let mode = display_errors_mode(&ini(self, b"display_errors"));
        let mut tail = b" in ".to_vec();
        tail.extend_from_slice(file);
        tail.extend_from_slice(format!(" on line {line}\n").as_bytes());
        if !trace.is_empty() {
            tail.extend_from_slice(b"Stack trace:\n");
            tail.extend_from_slice(trace);
            tail.push(b'\n');
        }
        if log {
            let mut m = format!("PHP {label}:  ").into_bytes();
            m.extend_from_slice(message);
            m.extend_from_slice(&tail);
            let target = ini(self, b"error_log");
            if target.is_empty() || target == b"syslog" {
                // error_log unset: the CLI SAPI's log_message writes to stderr.
                self.write_stderr(&m);
            } else {
                // php_log_err: append "[d-M-Y H:i:s e] message" to the file.
                let stamp = self
                    .invoke_builtin("date", vec![Value::str("d-M-Y H:i:s e")])
                    .map(|v| conv::to_string_bytes(&v))
                    .unwrap_or_default();
                let mut line = b"[".to_vec();
                line.extend_from_slice(&stamp);
                line.extend_from_slice(b"] ");
                line.extend_from_slice(&m);
                use std::io::Write;
                let path = String::from_utf8_lossy(&target).into_owned();
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    Ok(mut f) => {
                        let _ = f.write_all(&line);
                    }
                    Err(_) => self.write_stderr(&m),
                }
            }
        }
        let html = ini_bool(&ini(self, b"html_errors"));
        if html && matches!(mode, DisplayErrors::Stdout) {
            let mut m = ini(self, b"error_prepend_string");
            m.extend_from_slice(format!("<br />\n<b>{label}</b>:  ").as_bytes());
            m.extend_from_slice(message);
            m.extend_from_slice(b" in <b>");
            m.extend_from_slice(file);
            m.extend_from_slice(format!("</b> on line <b>{line}</b><br />\n").as_bytes());
            m.extend_from_slice(&ini(self, b"error_append_string"));
            self.write_out(&m);
            return;
        }
        match mode {
            DisplayErrors::Stderr => {
                let mut m = format!("{label}: ").into_bytes();
                m.extend_from_slice(message);
                m.extend_from_slice(&tail);
                self.write_stderr(&m);
            }
            DisplayErrors::Stdout => {
                let mut m = ini(self, b"error_prepend_string");
                m.extend_from_slice(format!("\n{label}: ").as_bytes());
                m.extend_from_slice(message);
                m.extend_from_slice(&tail);
                m.extend_from_slice(&ini(self, b"error_append_string"));
                self.write_out(&m);
            }
            DisplayErrors::Off => {}
        }
    }

    /// zend_exception_error(ex, E_ERROR) for an uncaught throwable: a
    /// ParseError reports as `Parse error: <message>`, anything else as
    /// `Fatal error: Uncaught <__toString()>\n  thrown`.
    /// php_error_cb for a fatal error: recorded for `error_get_last()` (even
    /// when not displayed), then logged / displayed.
    fn fatal_error(&mut self, ty: i64, label: &str, message: &[u8], file: &[u8], line: i64) {
        self.last_error = Some((ty, message.to_vec(), file.to_vec(), line));
        self.display_error(ty, label, message, file, line);
    }

    fn report_uncaught(&mut self, thrown: Thrown) {
        let thrown = self.materialize(thrown, None);
        let h = match thrown {
            Thrown::Object(h) => h,
            Thrown::Error { class, message } => {
                let (file, line) = self.throw_site();
                let msg = format!("Uncaught {class}: {message}");
                self.fatal_error(1, "Fatal error", msg.as_bytes(), &file, line as i64);
                return;
            }
            _ => return,
        };
        let cls = h.borrow().class;
        let prop = |n: &str| {
            h.borrow()
                .properties
                .get(&rhp_values::PhpStr::from(n))
                .map(deref)
                .unwrap_or(Value::Null)
        };
        let file = conv::to_string_bytes(&prop("file"));
        let line = conv::to_int(&prop("line"));
        // An emptied `file` makes php report the location as "Unknown".
        let file: &[u8] = if file.is_empty() { b"Unknown" } else { &file };
        let cname = self.class(cls).name.as_bytes().to_ascii_lowercase();
        if cname == b"parseerror" || cname == b"compileerror" {
            let msg = conv::to_string_bytes(&prop("message"));
            let (ty, label) = if cname == b"parseerror" {
                (4, "Parse error")
            } else {
                (64, "Fatal error")
            };
            self.fatal_error(ty, label, &msg, file, line);
            return;
        }
        let obj = Value::Object(h.clone());
        let s = match self.lookup_method(cls, b"__tostring") {
            Some(m) => match self.call_sync(&m, Some(h.clone()), cls, Vec::new()) {
                Ok(Value::Str(s)) => s.as_bytes().to_vec(),
                _ => builtins::php_fns::throwable_to_string(self, &obj),
            },
            None => builtins::php_fns::throwable_to_string(self, &obj),
        };
        let mut msg = b"Uncaught ".to_vec();
        msg.extend_from_slice(&s);
        msg.extend_from_slice(b"\n  thrown");
        self.fatal_error(1, "Fatal error", &msg, file, line);
    }

    /// An exception that escaped to the top level: the user handler gets it
    /// first, else it is reported. `Ok(None)`: handled, carry on; `Ok(Some(c))`:
    /// execution ends with status `c` (255 after a fatal, or an `exit(c)`);
    /// `Err`: an engine refusal the SAPI reports itself.
    fn top_level_throw(&mut self, thrown: Thrown) -> Result<Option<i32>, Thrown> {
        match thrown {
            Thrown::Exit(c) => Ok(Some(c)),
            t if t.is_catchable() => match self.handle_uncaught(t) {
                None => Ok(None),
                Some(Thrown::Exit(c)) => Ok(Some(c)),
                Some(t) if t.is_catchable() => {
                    self.report_uncaught(t);
                    Ok(Some(255))
                }
                Some(t) => Err(t),
            },
            t => Err(t),
        }
    }

    /// The CLI request around [`Vm::run`] (php_execute_script +
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
        self.shutdown_sequence(status, &mut |vm, t| vm.top_level_throw(t))
    }

    /// The `$this` of the current frame, if it is bound to an object.
    fn current_this(&self, top: usize) -> Option<ObjectHandle> {
        let cv = self.stack[top].proto.this_cv?;
        match &self.stack[top].regs[cv as usize] {
            Value::Object(h) => Some(h.clone()),
            // Passed to a by-reference-capable argument slot, the register
            // was promoted to a reference cell.
            Value::Ref(rc) => match &*rc.borrow() {
                Value::Object(h) => Some(h.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    fn cursor_unset(&mut self, top: usize, steps: &[CursorStep]) -> Result<(), Thrown> {
        match self.cursor_magic_split(top, steps) {
            MagicSplit::Plain | MagicSplit::Leaf(..) => {}
            MagicSplit::Nested(i, h, name) => {
                let cell = self.magic_write_cell(top, h, name)?;
                let rest = &steps[i + 1..];
                return self.with_cursor_root(cell, |vm| vm.cursor_unset(top, rest));
            }
            // `unset($obj[$k])` on an ArrayAccess object -> offsetUnset; one
            // deeper in the path is fetched with offsetGet first.
            MagicSplit::Offset(i, h) => {
                let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                if i + 1 == steps.len() {
                    let cls = h.borrow().class;
                    if let Some(m) = self.lookup_method(cls, b"offsetunset") {
                        self.call_sync(&m, Some(h), cls, vec![key])?;
                    }
                    return Ok(());
                }
                let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::Unset)?;
                let rest = &steps[i + 1..];
                return self.with_cursor_root(cell, |vm| vm.cursor_unset(top, rest));
            }
        }
        let cx = PropCtx {
            classes: &self.classes,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
        };
        let r = match &self.cursor_root {
            CursorRoot::Cv(r) => unset_path(&mut self.stack[top].regs[*r as usize], steps, &cx),
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                let r = unset_path(&mut rc.borrow_mut(), steps, &cx);
                r
            }
        };
        self.flush_path_diags()?;
        // The removed element may have held the last reference to an object.
        if !self.destructibles.is_empty() {
            self.dtor_check = true;
        }
        r
    }

    /// Emit the diagnostics a cursor-path walk queued ([`path_diag`]).
    fn flush_path_diags(&mut self) -> Result<(), Thrown> {
        let diags = PATH_DIAGS.with(|d| std::mem::take(&mut *d.borrow_mut()));
        for (level, msg) in diags {
            self.diag(level, &msg)?;
        }
        Ok(())
    }

    /// `offsetGet($key)` on an ArrayAccess object.
    fn aa_offset_get(&mut self, h: &ObjectHandle, key: Value) -> Result<Value, Thrown> {
        let cls = h.borrow().class;
        match self.lookup_method(cls, b"offsetget") {
            Some(m) => self.call_sync(&m, Some(h.clone()), cls, vec![key]),
            None => Ok(Value::Null),
        }
    }

    /// Before a read-modify-write (compound assignment / `++`) through an
    /// ArrayAccess object that is not the leaf: fetch that element once (php's
    /// single FETCH_DIM_RW), and re-root the open cursor at the result so the
    /// read and the following write both go there.
    fn cursor_settle_offsets(&mut self, top: usize) -> Result<(), Thrown> {
        loop {
            let steps = std::mem::take(&mut self.cursor_steps);
            match self.cursor_magic_split(top, &steps) {
                MagicSplit::Offset(i, h) if i + 1 < steps.len() => {
                    let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                    let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::Rw)?;
                    self.cursor_root = CursorRoot::Cell(cell);
                    self.cursor_steps = steps[i + 1..].to_vec();
                }
                _ => {
                    self.cursor_steps = steps;
                    return Ok(());
                }
            }
        }
    }

    /// An ArrayAccess element fetched for writing below it (php's
    /// FETCH_DIM_W/RW/UNSET on an object): `offsetGet`, whose result is the
    /// cell further writes land in — a temporary unless it returned by
    /// reference ("Indirect modification of overloaded element" notice when
    /// that temporary is not an object, which writes could still reach).
    /// ArrayObject / ArrayIterator without a user `offsetGet` hand out their
    /// storage element directly (spl_array_get_dimension_ptr).
    fn aa_fetch_w(
        &mut self,
        h: &ObjectHandle,
        step: &CursorStep,
        key: Value,
        mode: DimFetch,
    ) -> Result<Rc<RefCell<Value>>, Thrown> {
        if let Some(cell) = self.spl_array_dim_ptr(h, step, &key, mode)? {
            return Ok(cell);
        }
        let got = self.aa_offset_get(h, key)?;
        Ok(match got {
            Value::Ref(rc) => rc,
            other => {
                if !matches!(other, Value::Object(_) | Value::Closure(_)) {
                    self.indirect_overloaded_notice(h)?;
                }
                Rc::new(RefCell::new(other))
            }
        })
    }

    fn indirect_overloaded_notice(&mut self, h: &ObjectHandle) -> Result<(), Thrown> {
        let cls = h.borrow().class;
        let msg = format!(
            "Indirect modification of overloaded element of {} has no effect",
            String::from_utf8_lossy(self.class(cls).name.as_bytes())
        );
        self.diag(8, &msg)
    }

    /// The built-in ArrayObject / ArrayIterator class `h` derives from, when
    /// its `method` is still the built-in one (not overridden in user code).
    fn spl_array_owner(&self, h: &ObjectHandle, method: &[u8]) -> Option<ClassId> {
        let cls = h.borrow().class;
        let m = self.lookup_method(cls, method)?;
        let owner = self.class(m.defining_class).name.as_bytes();
        (owner == b"ArrayObject" || owner == b"ArrayIterator").then_some(m.defining_class)
    }

    /// A reference to the backing array of a built-in ArrayObject /
    /// ArrayIterator (`owner` declares the storage property).
    fn spl_array_storage(
        &mut self,
        h: &ObjectHandle,
        owner: ClassId,
    ) -> Result<Rc<RefCell<Value>>, Thrown> {
        let cx = PropCtx {
            classes: &self.classes,
            scope: Some(owner),
            called: Some(owner),
        };
        let mut obj = Value::Object(h.clone());
        make_ref_path(&mut obj, &[CursorStep::Prop(PhpStr::from("__s"))], &cx)
    }

    /// php's spl_array_get_dimension_ptr for a built-in ArrayObject /
    /// ArrayIterator whose `offsetGet` is not overridden: the storage element
    /// itself (created as null for a write; "Undefined array key" first for a
    /// read-modify-write). `None`: not such an object.
    fn spl_array_dim_ptr(
        &mut self,
        h: &ObjectHandle,
        step: &CursorStep,
        key: &Value,
        mode: DimFetch,
    ) -> Result<Option<Rc<RefCell<Value>>>, Thrown> {
        let Some(owner) = self.spl_array_owner(h, b"offsetget") else {
            return Ok(None);
        };
        // `$ao[][...]`: no offset — php's uninitialized_zval.
        if matches!(step, CursorStep::Append) {
            self.indirect_overloaded_notice(h)?;
            return Ok(Some(Rc::new(RefCell::new(Value::Null))));
        }
        let storage = self.spl_array_storage(h, owner)?;
        let akey = match deref(key) {
            Value::Null => {
                self.diag(
                    8192,
                    "Using null as an array offset is deprecated, use an empty string instead",
                )?;
                Some(rhp_values::ArrayKey::from_bytes(b""))
            }
            k => rhp_values::ArrayKey::from_value(&k),
        };
        let Some(akey) = akey else {
            return Err(Thrown::type_error(format!(
                "Cannot access offset of type {} on {}",
                type_lc(key),
                String::from_utf8_lossy(self.class(h.borrow().class).name.as_bytes())
            )));
        };
        let exists = matches!(&*storage.borrow(), Value::Array(a) if a.get(&akey).is_some());
        if !exists {
            match mode {
                DimFetch::Unset => {
                    self.indirect_overloaded_notice(h)?;
                    return Ok(Some(Rc::new(RefCell::new(Value::Null))));
                }
                DimFetch::Rw => {
                    let msg = match array_key_to_value(&akey) {
                        Value::Int(n) => format!("Undefined array key {n}"),
                        k => format!(
                            "Undefined array key \"{}\"",
                            String::from_utf8_lossy(&conv::to_string_bytes(&k))
                        ),
                    };
                    self.diag(2, &msg)?;
                }
                DimFetch::W => {}
            }
        }
        let cx = PropCtx {
            classes: &self.classes,
            scope: Some(owner),
            called: Some(owner),
        };
        let mut slot = storage.borrow_mut();
        make_ref_path(
            &mut slot,
            &[CursorStep::Key(array_key_to_value(&akey))],
            &cx,
        )
        .map(Some)
    }

    /// Promote a CV slot to a shared reference cell, returning the cell. If the
    /// slot already holds a `Ref`, its cell is reused.
    fn promote(&mut self, top: usize, cv: Reg) -> Rc<RefCell<Value>> {
        let f = &self.stack[top];
        if f.global_scope {
            if let Some(n) = f.proto.cv_var_names.get(cv as usize) {
                let n = n.as_bytes().to_vec();
                self.ref_globals.insert(n);
            }
        }
        let slot = &mut self.stack[top].regs[cv as usize];
        match slot {
            Value::Ref(rc) => rc.clone(),
            other => {
                let v = std::mem::replace(other, Value::Null);
                let rc = Rc::new(RefCell::new(v));
                *other = Value::Ref(rc.clone());
                rc
            }
        }
    }

    /// Drive execution, catching thrown exceptions and routing them through the
    /// exception table (per-frame handlers / finally), unwinding frames until a
    /// handler is found or the program's top frame is reached.
    fn run_loop(&mut self) -> Result<Value, Thrown> {
        loop {
            match self.run_frames() {
                Ok(v) => return Ok(v),
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        // Exit / Unsupported: straight to the SAPI (keeping
                        // {main}'s globals for the shutdown sequence). A
                        // refusal raised by a builtin is located first.
                        let thrown = self.locate_unsupported(thrown);
                        self.stack.truncate(1);
                        if let Some(main) = self.stack.pop() {
                            self.finished_main = Some(main);
                        }
                        return Err(thrown);
                    }
                    match self.unwind(thrown, 0)? {
                        UnwindResult::Caught => {}                 // handler installed; re-enter
                        UnwindResult::Escaped(t) => return Err(t), // uncaught
                    }
                }
            }
        }
    }

    fn run_frames(&mut self) -> Result<Value, Thrown> {
        loop {
            let top = self.stack.len() - 1;
            let proto = self.stack[top].proto.clone();
            let pc = self.stack[top].pc;
            if pc >= proto.code.len() {
                // fell off the end == RetNull
                if let Some(done) = self.do_return(Value::Null) {
                    return Ok(done);
                }
                continue;
            }
            let instr = proto.code[pc];
            self.stack[top].pc = pc + 1;

            // Run destructors of objects the program has just released.
            if self.dtor_check {
                self.dtor_check = false;
                if !self.destructibles.is_empty() {
                    self.collect_released()?;
                    self.stack[top].pc = pc;
                    continue;
                }
            }

            // pcntl async signals (php's `EG(vm_interrupt)` check): run queued
            // PHP signal handlers before the next instruction.
            if builtins::pcntl_interrupt_pending() {
                // A throwing handler raises at this (not yet run) instruction;
                // otherwise the instruction then runs as usual.
                builtins::pcntl_dispatch(self)?;
                self.stack[top].pc = pc;
                continue;
            }

            match instr {
                Instr::Nop => {}
                Instr::Mov { dst, src } => {
                    let v = self.reg_get(top, src);
                    self.reg_set(top, dst, v);
                }
                Instr::LoadConst { dst, k } => {
                    let v = proto.consts[k as usize].clone();
                    self.reg_set(top, dst, v);
                }
                Instr::LoadNull { dst } => {
                    // `unset($x)` in the global scope drops the symbol-table
                    // entry's binding (a reference to it keeps the old cell).
                    if self.stack[top].global_scope
                        && (dst as usize) < proto.cv_var_names.len()
                        && matches!(self.stack[top].regs[dst as usize], Value::Ref(_))
                    {
                        self.stack[top].regs[dst as usize] = Value::Null;
                        self.sync_global_cv(top, dst);
                        self.dtor_check = true;
                    } else {
                        self.reg_set(top, dst, Value::Null);
                    }
                }
                Instr::LoadBool { dst, b } => self.reg_set(top, dst, Value::Bool(b)),
                Instr::LoadInt { dst, n } => self.reg_set(top, dst, Value::Int(n)),

                Instr::Assign { cv, src } => {
                    let v = self.reg_get(top, src);
                    self.cv_assign(top, cv, v);
                }

                Instr::Add { dst, a, b } => self.bin(top, dst, a, b, ops::add)?,
                Instr::Sub { dst, a, b } => self.bin(top, dst, a, b, ops::sub)?,
                Instr::Mul { dst, a, b } => self.bin(top, dst, a, b, ops::mul)?,
                Instr::Div { dst, a, b } => self.bin(top, dst, a, b, ops::div)?,
                Instr::Mod { dst, a, b } => self.bin(top, dst, a, b, ops::modulo)?,
                Instr::Pow { dst, a, b } => self.bin(top, dst, a, b, ops::pow)?,
                Instr::BitAnd { dst, a, b } => {
                    self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::And))
                }
                Instr::BitOr { dst, a, b } => {
                    self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Or))
                }
                Instr::BitXor { dst, a, b } => {
                    self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Xor))
                }
                Instr::Shl { dst, a, b } => {
                    self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Shl))
                }
                Instr::Shr { dst, a, b } => {
                    self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Shr))
                }
                Instr::Concat { dst, a, b } => {
                    let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
                    let v = if matches!(x, Value::Object(_) | Value::Array(_))
                        || matches!(y, Value::Object(_) | Value::Array(_))
                    {
                        let mut bytes = self.to_string_vm(&x)?;
                        bytes.extend_from_slice(&self.to_string_vm(&y)?);
                        Value::str(bytes)
                    } else {
                        ops::concat(&x, &y)
                    };
                    self.reg_set(top, dst, v);
                }

                Instr::Neg { dst, a } => {
                    let v = self.reg_get(top, a);
                    let r = ops::negate(&v)?;
                    self.reg_set(top, dst, r);
                }
                Instr::BitNot { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, ops::bitwise_not(&v));
                }
                Instr::Not { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, Value::Bool(!conv::to_bool(&v)));
                }
                Instr::IncDec {
                    dst,
                    cv,
                    pre,
                    delta,
                } => {
                    let cur = self.reg_get(top, cv);
                    let new = inc_dec_value(&cur, delta as i64);
                    self.cv_assign(top, cv, new.clone());
                    self.reg_set(top, dst, if pre { new } else { cur });
                }

                Instr::CastInt { dst, a } => {
                    let v = self.reg_get(top, a);
                    // php's curl_cast_object: (int) $curlHandle is its object id.
                    let n = match &v {
                        Value::Object(h) if rhp_builtins::is_curl_handle(&v) => {
                            h.borrow().id as i64
                        }
                        _ => conv::to_int(&v),
                    };
                    self.reg_set(top, dst, Value::Int(n));
                }
                Instr::CastFloat { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, Value::Float(conv::to_float(&v)));
                }
                Instr::CastString { dst, a } => {
                    let v = self.reg_get(top, a);
                    let s = self.to_string_vm(&v)?;
                    self.reg_set(top, dst, Value::str(s));
                }
                Instr::CastBool { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, Value::Bool(conv::to_bool(&v)));
                }
                Instr::CastArray { dst, a } => {
                    let v = self.reg_get(top, a);
                    let out = match &v {
                        // get_properties_for(ARRAY_CAST): an ArrayObject's storage.
                        Value::Object(h) => match builtins::spl_array_storage(h) {
                            Some((st @ Value::Array(_), _)) => st,
                            _ => self.object_to_array_mangled(h),
                        },
                        _ => cast_to_array(v),
                    };
                    self.reg_set(top, dst, out);
                }
                Instr::CastObject { dst, a } => {
                    let v = self.reg_get(top, a);
                    let obj = self.cast_to_object(v);
                    self.reg_set(top, dst, obj);
                }

                Instr::Cmp { dst, a, b, kind } => {
                    let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
                    let r = self.eval_cmp(kind, &x, &y)?;
                    self.reg_set(top, dst, r);
                }
                Instr::BeginSilence => {
                    self.silence_stack.push((self.stack.len(), self.silence));
                    self.silence += 1;
                }
                Instr::EndSilence => {
                    if let Some((_, lvl)) = self.silence_stack.pop() {
                        self.silence = lvl;
                    }
                }

                Instr::Echo { src } => {
                    let v = self.reg_get(top, src);
                    let bytes = self.to_string_vm(&v)?;
                    self.write_out(&bytes);
                }
                Instr::EchoConst { k } => {
                    let bytes = conv::to_string_bytes(&proto.consts[k as usize]);
                    self.write_out(&bytes);
                }
                Instr::Print { dst, src } => {
                    let v = self.reg_get(top, src);
                    let bytes = self.to_string_vm(&v)?;
                    self.write_out(&bytes);
                    self.reg_set(top, dst, Value::Int(1));
                }

                Instr::Jmp { target } => self.stack[top].pc = target as usize,
                Instr::JmpIfArg { index, target } => {
                    let f = &self.stack[top];
                    if f.argc > index as usize && (index >= 64 || f.holes & (1u64 << index) == 0) {
                        self.stack[top].pc = target as usize;
                    }
                }
                Instr::JmpTrue { cond, target } => {
                    if conv::to_bool(&self.reg_get(top, cond)) {
                        self.stack[top].pc = target as usize;
                    }
                }
                Instr::JmpFalse { cond, target } => {
                    if !conv::to_bool(&self.reg_get(top, cond)) {
                        self.stack[top].pc = target as usize;
                    }
                }
                Instr::JmpNull { src, target } => {
                    if matches!(self.reg_get(top, src), Value::Null) {
                        self.stack[top].pc = target as usize;
                    }
                }
                Instr::JmpNotNull { src, target } => {
                    if !matches!(self.reg_get(top, src), Value::Null) {
                        self.stack[top].pc = target as usize;
                    }
                }

                Instr::NewArray { dst, .. } => {
                    self.reg_set(top, dst, Value::Array(rhp_values::PhpArray::new()));
                }
                Instr::ArrPush { arr, val } => {
                    let v = self.reg_get(top, val);
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        a.push(v);
                    }
                }
                Instr::ArrSet { arr, key, val } => {
                    let k = self.reg_get(top, key);
                    let v = self.reg_get(top, val);
                    let key = ArrayKey::from_value(&k)
                        .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        a.insert(key, v);
                    }
                }
                Instr::ArrPushRef { arr, val } => {
                    let v = self.stack[top].regs[val as usize].clone();
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        a.push(v);
                    }
                }
                Instr::ArrSetRef { arr, key, val } => {
                    let k = self.reg_get(top, key);
                    let v = self.stack[top].regs[val as usize].clone();
                    let key = ArrayKey::from_value(&k)
                        .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
                    if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                        a.insert(key, v);
                    }
                }
                Instr::ArrSpread { arr, src } => {
                    let spread = self.reg_get(top, src);
                    if let Value::Array(inner) = spread {
                        if let Value::Array(a) = &mut self.stack[top].regs[arr as usize] {
                            for (k, iv) in inner.iter() {
                                match k {
                                    ArrayKey::Int(_) => a.push(iv.clone()),
                                    ArrayKey::Str(_) => a.insert(k.clone(), iv.clone()),
                                }
                            }
                        }
                    }
                }

                Instr::Call {
                    dst,
                    name,
                    base,
                    argc,
                    fallback,
                } => {
                    let (user, fname) = self.resolve_fn(name, fallback, &proto.names);
                    if let Some(reason) =
                        builtins::is_refused(std::str::from_utf8(&fname).unwrap_or(""))
                    {
                        let shown = String::from_utf8_lossy(&fname).into_owned();
                        return Err(Thrown::unsupported(format!("{shown}()"), reason));
                    }
                    if let Some(fnidx) = user {
                        // User function: push a frame (args bound to its CVs).
                        let callee = self.functions[fnidx].clone();
                        self.enter_user_call(top, callee, dst, base, argc)?;
                    } else {
                        // Builtin (`fname` is already lowercase).
                        let lc = std::str::from_utf8(&fname).unwrap_or("");
                        let Some(builtin) = builtins::lookup_lc(lc) else {
                            return Err(undefined_function(&proto.names[name as usize]));
                        };
                        // php compiles a few calls to dedicated opcodes (no call
                        // frame): only when resolvable at compile time, i.e. not an
                        // unqualified call inside a namespace.
                        let frameless = fallback == rhp_bytecode::NO_NAME
                            && compiled_inline(&fname, argc as usize);
                        self.mark_user_call_builtin(&fname, fallback);
                        let out = self.call_builtin(top, lc, builtin, base, argc, frameless);
                        self.cuf_caller = None;
                        let out = out?;
                        self.reg_set(top, dst, out);
                    }
                }

                Instr::MakeClosure {
                    dst,
                    fnidx,
                    base,
                    ncaptures,
                } => {
                    // Raw (non-deref) read: a by-reference capture arrives as a
                    // `Value::Ref` (from MakeRef) and must stay a Ref so writes
                    // inside the closure reach the outer variable; by-value
                    // captures were already deref'd into the window by Mov.
                    let captures: Vec<Value> = (0..ncaptures)
                        .map(|i| self.stack[top].regs[(base + i) as usize].clone())
                        .collect();
                    // Bind the defining/static class scope so the closure body can
                    // reach private members and resolve `self::`/`static::`.
                    let cd = Rc::new(rhp_values::ClosureData {
                        // `fnidx` is file-local; resolve into the flat table.
                        fnidx: proto.closure_base + fnidx,
                        captures,
                        bound_this: None,
                        bound_class: self.stack[top].defining_class,
                        bound_static: self.stack[top].called_class,
                        wrapped: None,
                    });
                    self.reg_set(top, dst, Value::Closure(cd));
                }
                Instr::BindGlobal { cv, name } => {
                    let gname = proto.names[name as usize].clone();
                    let cell = self.global_cell(gname.as_bytes());
                    self.ref_globals.insert(gname.as_bytes().to_vec());
                    self.stack[top].regs[cv as usize] = Value::Ref(cell);
                }
                Instr::BindStatic { cv, name, init } => {
                    let sname = proto.names[name as usize].as_bytes().to_vec();
                    let key = self.static_key(top, &proto, sname);
                    let cell = match self.statics.get(&key) {
                        Some(c) => c.clone(),
                        None => {
                            let initv = if init == u32::MAX {
                                Value::Null
                            } else {
                                proto.consts[init as usize].clone()
                            };
                            let c = Rc::new(RefCell::new(initv));
                            self.statics.insert(key, c.clone());
                            c
                        }
                    };
                    self.stack[top].regs[cv as usize] = Value::Ref(cell);
                    self.sync_global_cv(top, cv);
                }
                Instr::BindStaticOrJmp { cv, name, target } => {
                    let sname = proto.names[name as usize].as_bytes().to_vec();
                    let key = self.static_key(top, &proto, sname);
                    if let Some(c) = self.statics.get(&key) {
                        let cell = c.clone();
                        self.stack[top].regs[cv as usize] = Value::Ref(cell);
                        self.sync_global_cv(top, cv);
                        self.stack[top].pc = target as usize;
                    }
                }
                Instr::BindStaticInit { cv, name, src } => {
                    let sname = proto.names[name as usize].as_bytes().to_vec();
                    let key = self.static_key(top, &proto, sname);
                    let v = self.reg_get(top, src);
                    let cell = self
                        .statics
                        .entry(key)
                        .or_insert_with(|| Rc::new(RefCell::new(v)))
                        .clone();
                    self.stack[top].regs[cv as usize] = Value::Ref(cell);
                    self.sync_global_cv(top, cv);
                }
                Instr::DeclareConst { name, src } => {
                    let cname = proto.names[name as usize].as_bytes().to_vec();
                    let v = self.reg_get(top, src);
                    self.constants.entry(cname).or_insert(v);
                }
                Instr::MakeFcc { dst, callee } => {
                    let c = self.reg_get(top, callee);
                    let cd = Rc::new(rhp_values::ClosureData {
                        fnidx: u32::MAX,
                        captures: Vec::new(),
                        bound_this: None,
                        bound_class: self.stack[top].defining_class,
                        bound_static: self.stack[top].called_class,
                        wrapped: Some(Box::new(c)),
                    });
                    self.reg_set(top, dst, Value::Closure(cd));
                }
                Instr::CallDyn {
                    dst,
                    callee,
                    base,
                    argc,
                } => {
                    let c = self.reg_get(top, callee);
                    let args: Vec<Value> = (0..argc).map(|i| self.reg_get(top, base + i)).collect();
                    let strict = self.caller_strict();
                    let out = self.call_callable_value_mode(&c, args, Some(strict))?;
                    self.reg_set(top, dst, out);
                }
                Instr::CallDynV { dst, callee, args } => {
                    let c = self.reg_get(top, callee);
                    let arr = self.reg_get(top, args);
                    let (argvec, named) = self.split_call_args(&arr)?;
                    let strict = self.caller_strict();
                    let out = self.call_named(&c, argvec, named, Some(strict))?;
                    self.reg_set(top, dst, out);
                }

                // -- array element read --
                Instr::DimFetch { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
                    let r = self.dim_read(&a, &k, DimMode::Read)?;
                    self.reg_set(top, dst, r);
                }
                Instr::DimFetchIs { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
                    let r = self.dim_read(&a, &k, DimMode::Is)?;
                    self.reg_set(top, dst, r);
                }
                Instr::DimFetchList { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
                    let r = self.dim_read(&a, &k, DimMode::List)?;
                    self.reg_set(top, dst, r);
                }

                // -- lvalue cursor --
                Instr::LvalCv { cv } => {
                    self.cursor_root = CursorRoot::Cv(cv);
                    self.cursor_global = None;
                    self.cursor_steps.clear();
                }
                Instr::LvalKey { key } => {
                    let k = self.reg_get(top, key);
                    self.cursor_steps.push(CursorStep::Key(k));
                }
                Instr::LvalAppend => self.cursor_steps.push(CursorStep::Append),
                Instr::LvalProp { name } => {
                    let p = proto.names[name as usize].clone();
                    self.cursor_steps.push(CursorStep::Prop(p));
                }
                Instr::LvalPropDyn { name } => {
                    let p = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
                    self.cursor_steps.push(CursorStep::Prop(p));
                }
                Instr::LvalStaticProp { class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = proto.names[name as usize].as_bytes().to_vec();
                    self.check_static_prop_access(top, cls, &pname)?;
                    let cell = self.static_prop_cell(cls, &pname)?;
                    self.cursor_root = CursorRoot::Cell(cell);
                    self.cursor_global = None;
                    self.cursor_steps.clear();
                }
                Instr::LvalGlobal { name } => {
                    let n = conv::to_string_bytes(&self.reg_get(top, name));
                    self.cursor_root = CursorRoot::Cell(self.global_cell(&n));
                    self.cursor_global = Some(n);
                    self.cursor_steps.clear();
                }
                Instr::FetchGlobal { dst, name } => {
                    let n = conv::to_string_bytes(&self.reg_get(top, name));
                    let v = match self.globals.get(&n[..]) {
                        Some(c) => deref(&c.borrow()),
                        None => {
                            let msg = format!(
                                "Undefined global variable ${}",
                                String::from_utf8_lossy(&n)
                            );
                            self.diag(2, &msg)?;
                            Value::Null
                        }
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::FetchGlobals { dst } => {
                    let v = self.globals_array();
                    self.reg_set(top, dst, v);
                }
                Instr::UnsetGlobal { name } => {
                    let n = conv::to_string_bytes(&self.reg_get(top, name));
                    if self.globals.contains_key(&n[..]) {
                        // Live global-scope frames lose the variable too.
                        self.rebind_global(&n, Rc::new(RefCell::new(Value::Null)));
                        self.globals.shift_remove(&n[..]);
                        self.ref_globals.remove(&n[..]);
                        self.dtor_check = true;
                    }
                }
                Instr::FetchLvalRw { dst } => {
                    let v = self.cursor_read(top)?;
                    self.reg_set(top, dst, v); // cursor stays open for the AssignLval
                }
                Instr::AssignLval { src } => {
                    let v = self.reg_get(top, src);
                    let steps = std::mem::take(&mut self.cursor_steps);
                    self.cursor_write(top, &steps, v)?;
                }
                Instr::AssignLvalRef { src } => {
                    // Raw (non-deref) value: `src` holds a `Ref`, so the cursor
                    // target becomes a reference to the same cell.
                    let v = self.stack[top].regs[src as usize].clone();
                    let steps = std::mem::take(&mut self.cursor_steps);
                    // `$GLOBALS['x'] = &$v`: the symbol-table entry itself
                    // becomes the reference (like `$x = &$v` in global scope).
                    if let (true, Some(n), Value::Ref(rc)) =
                        (steps.is_empty(), self.cursor_global.take(), &v)
                    {
                        self.ref_globals.insert(n.clone());
                        self.rebind_global(&n, rc.clone());
                        continue;
                    }
                    self.cursor_write(top, &steps, v)?;
                }
                Instr::MakeRefLval { dst } => {
                    let steps = std::mem::take(&mut self.cursor_steps);
                    let cell = self.cursor_make_ref(top, &steps)?;
                    self.reg_set(top, dst, Value::Ref(cell));
                }
                Instr::UnsetLval => {
                    let steps = std::mem::take(&mut self.cursor_steps);
                    self.cursor_unset(top, &steps)?;
                }
                Instr::IncDecLval { dst, pre, delta } => {
                    self.cursor_settle_offsets(top)?;
                    let steps = std::mem::take(&mut self.cursor_steps);
                    // `$obj[$k]++` on an ArrayAccess object: php fetches the
                    // element for RW (offsetGet) and increments that; there
                    // is no offsetSet.
                    if let MagicSplit::Offset(i, h) = self.cursor_magic_split(top, &steps) {
                        if i + 1 == steps.len() {
                            let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                            let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::Rw)?;
                            let old = deref(&cell.borrow());
                            let new = inc_dec_value(&old, delta as i64);
                            *cell.borrow_mut() = new.clone();
                            self.reg_set(top, dst, if pre { new } else { old });
                            continue;
                        }
                    }
                    let old = self.cursor_read_magic(top, &steps, true)?;
                    let new = inc_dec_value(&old, delta as i64);
                    self.cursor_write(top, &steps, new.clone())?;
                    self.reg_set(top, dst, if pre { new } else { old });
                }

                // -- foreach --
                Instr::FeReset {
                    arr_reg,
                    pos_reg,
                    src,
                    end,
                } => {
                    match self.reg_get(top, src) {
                        Value::Array(a) if !a.is_empty() => {
                            self.reg_set(top, arr_reg, Value::Array(a));
                            self.reg_set(top, pos_reg, Value::Int(0));
                        }
                        Value::Object(h) => {
                            // Resolve the thing we actually iterate: IteratorAggregate
                            // delegates to getIterator(); Generators / Iterators drive
                            // the protocol in FeFetch; a plain object iterates its
                            // accessible properties (snapshotted to an array).
                            let mut obj = h;
                            if self.class_implements(obj.borrow().class, b"IteratorAggregate") {
                                match self.call_iter_method(&obj, b"getiterator")? {
                                    Value::Object(it) => obj = it,
                                    _ => {
                                        self.stack[top].pc = end as usize;
                                        continue;
                                    }
                                }
                            }
                            let oid = obj.borrow().id;
                            let ocls = obj.borrow().class;
                            if self.generators.contains_key(&oid) {
                                self.gen_check_open(oid)?;
                                self.gen_rewind(oid)?;
                                self.reg_set(top, arr_reg, Value::Object(obj));
                                self.reg_set(top, pos_reg, Value::Int(0));
                            } else if self.class_implements(ocls, b"Iterator") {
                                self.call_iter_method(&obj, b"rewind")?;
                                self.reg_set(top, arr_reg, Value::Object(obj));
                                self.reg_set(top, pos_reg, Value::Int(0));
                            } else {
                                let arr = self.object_prop_array(&obj, top);
                                if arr.is_empty() {
                                    self.stack[top].pc = end as usize;
                                } else {
                                    self.reg_set(top, arr_reg, Value::Array(arr));
                                    self.reg_set(top, pos_reg, Value::Int(0));
                                }
                            }
                        }
                        // empty array or non-iterable: skip the loop
                        _ => self.stack[top].pc = end as usize,
                    }
                }
                Instr::FeFetch {
                    arr_reg,
                    pos_reg,
                    key_cv,
                    val_cv,
                    end,
                } => {
                    // Generator mode. The advance happens at the TOP of each
                    // iteration except the first, so the generator's post-yield
                    // code runs *after* the loop body (unlike an array's pos++).
                    // `pos_reg` is 0 on the first fetch, 1 thereafter.
                    if let Value::Object(h) = self.reg_get(top, arr_reg) {
                        let id = h.borrow().id;
                        if self.generators.contains_key(&id) {
                            let first = matches!(self.reg_get(top, pos_reg), Value::Int(0));
                            if !first {
                                self.advance_gen(id, Value::Null)?;
                            }
                            let g = &self.generators[&id];
                            if g.status == GenStatus::Finished {
                                self.stack[top].pc = end as usize;
                            } else {
                                let (k, v) = (g.current_key.clone(), deref(&g.current_val));
                                self.cv_assign(top, val_cv, v);
                                if key_cv != Reg::MAX {
                                    self.cv_assign(top, key_cv, k);
                                }
                                self.reg_set(top, pos_reg, Value::Int(1));
                            }
                            continue;
                        }
                        // User Iterator object: rewind()/valid()/current()/key()/next().
                        if self.class_implements(h.borrow().class, b"Iterator") {
                            let first = matches!(self.reg_get(top, pos_reg), Value::Int(0));
                            if !first {
                                self.call_iter_method(&h, b"next")?;
                            }
                            let valid = conv::to_bool(&self.call_iter_method(&h, b"valid")?);
                            if !valid {
                                self.stack[top].pc = end as usize;
                            } else {
                                let v = self.call_iter_method(&h, b"current")?;
                                self.cv_assign(top, val_cv, v);
                                if key_cv != Reg::MAX {
                                    let k = self.call_iter_method(&h, b"key")?;
                                    self.cv_assign(top, key_cv, k);
                                }
                                self.reg_set(top, pos_reg, Value::Int(1));
                            }
                            continue;
                        }
                    }
                    let pos = match self.reg_get(top, pos_reg) {
                        Value::Int(n) => n as usize,
                        _ => 0,
                    };
                    let snapshot = self.reg_get(top, arr_reg);
                    let Value::Array(a) = snapshot else {
                        self.stack[top].pc = end as usize;
                        continue;
                    };
                    match a.get_index(pos) {
                        Some((k, v)) => {
                            let (k, v) = (k.clone(), deref(v).clone());
                            // Bind as a PHP variable: if the loop var is still a
                            // reference (e.g. left over from a prior by-ref loop),
                            // write through it — matching PHP's foreach gotcha.
                            self.cv_assign(top, val_cv, v);
                            if key_cv != Reg::MAX {
                                self.cv_assign(top, key_cv, array_key_to_value(&k));
                            }
                            self.reg_set(top, pos_reg, Value::Int((pos + 1) as i64));
                        }
                        None => self.stack[top].pc = end as usize,
                    }
                }

                Instr::FeResetRef {
                    arr_cv,
                    pos_reg,
                    end,
                } => {
                    // A Traversable object by reference: only a by-reference
                    // generator (directly or from getIterator()) can do it;
                    // `pos_reg` then holds [generator, fetched-once flag].
                    if let Value::Object(h) = deref(&self.stack[top].regs[arr_cv as usize]) {
                        let mut obj = h;
                        let mut traversable = false;
                        while self.class_implements(obj.borrow().class, b"IteratorAggregate") {
                            // A built-in ArrayObject iterates its storage itself.
                            if self.spl_array_owner(&obj, b"getiterator").is_some() {
                                break;
                            }
                            traversable = true;
                            match self.call_iter_method(&obj, b"getiterator")? {
                                Value::Object(it)
                                    if self.class_implements(it.borrow().class, b"Traversable") =>
                                {
                                    obj = it
                                }
                                _ => {
                                    let cls = obj.borrow().class;
                                    return Err(Thrown::error(
                                        "Exception",
                                        format!(
                                            "Objects returned by {}::getIterator() must be traversable or implement interface Iterator",
                                            String::from_utf8_lossy(self.class(cls).name.as_bytes())
                                        ),
                                    ));
                                }
                            }
                        }
                        let oid = obj.borrow().id;
                        if let Some(g) = self.generators.get(&oid) {
                            let by_value_gen =
                                g.frame.as_ref().is_some_and(|f| !f.proto.returns_ref);
                            self.gen_check_open(oid)?;
                            if by_value_gen {
                                return Err(Thrown::error(
                                    "Exception",
                                    "You can only iterate a generator by-reference if it declared that it yields by-reference",
                                ));
                            }
                            self.gen_rewind(oid)?;
                            let mut st = PhpArray::new();
                            st.push(Value::Object(obj));
                            st.push(Value::Bool(false));
                            self.reg_set(top, pos_reg, Value::Array(st));
                            continue;
                        }
                        // ArrayObject / ArrayIterator (unless `current()` is
                        // overridden): by reference over the backing array.
                        let spl = self
                            .spl_array_owner(&obj, b"getiterator")
                            .or_else(|| self.spl_array_owner(&obj, b"current"));
                        if let Some(owner) = spl {
                            // `pos_reg` holds [storage reference, position].
                            let cell = self.spl_array_storage(&obj, owner)?;
                            let mut st = PhpArray::new();
                            st.push(Value::Ref(cell));
                            st.push(Value::Int(0));
                            self.reg_set(top, pos_reg, Value::Array(st));
                            continue;
                        }
                        if traversable || self.class_implements(obj.borrow().class, b"Iterator") {
                            return Err(Thrown::error(
                                "Error",
                                "An iterator cannot be used with foreach by reference",
                            ));
                        }
                    }
                    let nonempty = match deref(&self.stack[top].regs[arr_cv as usize]) {
                        Value::Array(a) => !a.is_empty(),
                        // A plain object iterates its properties by reference;
                        // `FeFetchRef` ends the loop when none remain accessible.
                        Value::Object(_) => true,
                        _ => false,
                    };
                    if nonempty {
                        self.reg_set(top, pos_reg, Value::Int(0));
                    } else {
                        self.stack[top].pc = end as usize;
                    }
                }
                Instr::FeFetchRef {
                    arr_cv,
                    pos_reg,
                    key_cv,
                    val_cv,
                    end,
                } => {
                    // By-reference generator (see FeResetRef): advance at the
                    // top of each iteration but the first, then alias the loop
                    // variable to the yielded reference.
                    if let Value::Array(st) = self.reg_get(top, pos_reg) {
                        // ArrayObject / ArrayIterator storage (see FeResetRef).
                        if let (Some(Value::Ref(cell)), Some(Value::Int(pos))) = (
                            st.get_index(0).map(|e| e.1.clone()),
                            st.get_index(1).map(|e| e.1.clone()),
                        ) {
                            let fetched = foreach_promote(&mut cell.borrow_mut(), pos as usize);
                            match fetched {
                                Some((k, elem_rc)) => {
                                    self.bind_ref_cv(top, val_cv, elem_rc);
                                    if key_cv != Reg::MAX {
                                        self.cv_assign(top, key_cv, array_key_to_value(&k));
                                    }
                                    let mut st = PhpArray::new();
                                    st.push(Value::Ref(cell));
                                    st.push(Value::Int(pos + 1));
                                    self.reg_set(top, pos_reg, Value::Array(st));
                                }
                                None => self.stack[top].pc = end as usize,
                            }
                            continue;
                        }
                        let (Some(Value::Object(g)), Some(Value::Bool(fetched))) = (
                            st.get_index(0).map(|e| e.1.clone()),
                            st.get_index(1).map(|e| e.1.clone()),
                        ) else {
                            self.stack[top].pc = end as usize;
                            continue;
                        };
                        let id = g.borrow().id;
                        if fetched {
                            self.advance_gen(id, Value::Null)?;
                        }
                        let gs = &self.generators[&id];
                        if gs.status == GenStatus::Finished {
                            self.stack[top].pc = end as usize;
                            continue;
                        }
                        let (k, v) = (gs.current_key.clone(), gs.current_val.clone());
                        let rc = match v {
                            Value::Ref(rc) => rc,
                            other => Rc::new(RefCell::new(other)),
                        };
                        self.bind_ref_cv(top, val_cv, rc);
                        if key_cv != Reg::MAX {
                            self.cv_assign(top, key_cv, k);
                        }
                        let mut st = PhpArray::new();
                        st.push(Value::Object(g));
                        st.push(Value::Bool(true));
                        self.reg_set(top, pos_reg, Value::Array(st));
                        continue;
                    }
                    let pos = match self.reg_get(top, pos_reg) {
                        Value::Int(n) => n as usize,
                        _ => 0,
                    };
                    // By-reference foreach over a plain object: alias each
                    // accessible property slot so writes through the loop var are
                    // visible on the object (matches PHP).
                    if let Value::Object(h) = deref(&self.stack[top].regs[arr_cv as usize]).clone()
                    {
                        let caller = self.stack[top].defining_class;
                        let cls = h.borrow().class;
                        let mut b = h.borrow_mut();
                        let n = b.properties.len();
                        let mut idx = pos;
                        let mut found: Option<PhpStr> = None;
                        while idx < n {
                            let key = b.properties.get_index(idx).unwrap().0;
                            if let Some(name) =
                                classes::slot_accessible(&self.classes, cls, caller, key)
                            {
                                found = Some(name);
                                break;
                            }
                            idx += 1;
                        }
                        match found {
                            Some(name) => {
                                let key = b.properties.get_index(idx).unwrap().0.clone();
                                if let Some(pe) =
                                    self.class(cls).props.get(&key).filter(|pe| pe.readonly)
                                {
                                    return Err(Thrown::error(
                                        "Error",
                                        format!(
                                            "Cannot acquire reference to readonly property {}::${}",
                                            String::from_utf8_lossy(
                                                self.class(pe.declaring_class).name.as_bytes()
                                            ),
                                            String::from_utf8_lossy(pe.name.as_bytes())
                                        ),
                                    ));
                                }
                                let slot = b.properties.get_index_mut(idx).unwrap().1;
                                let rc = match slot {
                                    Value::Ref(rc) => rc.clone(),
                                    other => {
                                        let inner = std::mem::replace(other, Value::Null);
                                        let rc = Rc::new(RefCell::new(inner));
                                        *other = Value::Ref(rc.clone());
                                        rc
                                    }
                                };
                                drop(b);
                                self.bind_ref_cv(top, val_cv, rc);
                                if key_cv != Reg::MAX {
                                    self.cv_assign(
                                        top,
                                        key_cv,
                                        Value::str(name.as_bytes().to_vec()),
                                    );
                                }
                                self.reg_set(top, pos_reg, Value::Int((idx + 1) as i64));
                            }
                            None => {
                                drop(b);
                                self.stack[top].pc = end as usize;
                            }
                        }
                        continue;
                    }
                    // Promote the element at `pos` to a shared cell (through a
                    // Ref if the source variable itself is a reference).
                    let rc_opt = match &self.stack[top].regs[arr_cv as usize] {
                        Value::Ref(rc) => Some(rc.clone()),
                        _ => None,
                    };
                    let fetched = match rc_opt {
                        Some(rc) => foreach_promote(&mut rc.borrow_mut(), pos),
                        None => foreach_promote(&mut self.stack[top].regs[arr_cv as usize], pos),
                    };
                    match fetched {
                        Some((k, elem_rc)) => {
                            self.bind_ref_cv(top, val_cv, elem_rc);
                            if key_cv != Reg::MAX {
                                self.cv_assign(top, key_cv, array_key_to_value(&k));
                            }
                            self.reg_set(top, pos_reg, Value::Int((pos + 1) as i64));
                        }
                        None => self.stack[top].pc = end as usize,
                    }
                }

                Instr::ThrowMatchError { subject } => {
                    let v = self.reg_get(top, subject);
                    return Err(Thrown::error(
                        "UnhandledMatchError",
                        format!("Unhandled match case {}", match_case_desc(&v)),
                    ));
                }
                Instr::DeferredUnsupported { what, why } => {
                    let what = conv::to_string_bytes(&proto.consts[what as usize]);
                    let why = conv::to_string_bytes(&proto.consts[why as usize]);
                    return Err(self.locate_unsupported(Thrown::unsupported(
                        String::from_utf8_lossy(&what).into_owned(),
                        String::from_utf8_lossy(&why).into_owned(),
                    )));
                }
                Instr::Eval { dst, src } => {
                    let v = self.do_eval(top, src)?;
                    self.reg_set(top, dst, v);
                }
                Instr::Throw { src } => {
                    let v = self.reg_get(top, src);
                    match v {
                        Value::Object(h) => {
                            let cls = h.borrow().class;
                            let is_throwable = self
                                .resolve_class_by_name(b"Throwable")
                                .map(|t| self.class(cls).ancestors.contains(&t))
                                .unwrap_or(false);
                            if is_throwable {
                                return Err(Thrown::Object(h));
                            }
                            return Err(Thrown::type_error(
                                "Can only throw objects that implement Throwable",
                            ));
                        }
                        _ => return Err(Thrown::type_error("Can only throw objects")),
                    }
                }
                Instr::FinallyEnd => {
                    let pc = self.stack[top].pc as u32 - 1;
                    let proto = self.stack[top].proto.clone();
                    let fin = proto
                        .exception_table
                        .iter()
                        .find(|r| r.finally.is_some() && r.finally_end == pc)
                        .and_then(|r| r.finally);
                    let cont = fin.and_then(|f| {
                        let pend = &mut self.stack[top].pending;
                        let i = pend.iter().position(|(k, _)| *k == f)?;
                        Some(pend.remove(i).1)
                    });
                    match cont {
                        Some(FinallyCont::Rethrow(t)) => return Err(t),
                        Some(FinallyCont::Return(v)) => {
                            // An outer finally still has to run first.
                            if let Some(f2) = enclosing_finally(&proto, pc, None) {
                                self.push_pending(top, f2, FinallyCont::Return(v));
                                self.stack[top].pc = f2 as usize;
                            } else if let Some(done) = self.do_return(v) {
                                return Ok(done);
                            }
                        }
                        Some(FinallyCont::Jump(target)) => {
                            self.jump_through_finally(top, &proto, pc, target)
                        }
                        None => {}
                    }
                }
                Instr::SetPendingRet { src } => {
                    let mut v = self.reg_get(top, src);
                    if let (Some(ty), false) = (&proto.return_type, proto.is_generator) {
                        let ctx = self.stack[top].defining_class;
                        v = self.coerce_stringable(v, ty, proto.strict_types, ctx)?;
                        v = self.check_return_type(&proto, top, v)?;
                    }
                    let pc = self.stack[top].pc as u32 - 1;
                    let proto = self.stack[top].proto.clone();
                    self.drop_left_finallies(top, &proto, pc);
                    if let Some(f) = enclosing_finally(&proto, pc, None) {
                        self.push_pending(top, f, FinallyCont::Return(v));
                    }
                }
                Instr::JmpFinally { target } => {
                    let pc = self.stack[top].pc as u32 - 1;
                    let proto = self.stack[top].proto.clone();
                    self.drop_left_finallies(top, &proto, pc);
                    self.jump_through_finally(top, &proto, pc, target);
                }

                // -- OOP --
                Instr::New {
                    dst,
                    class,
                    base,
                    argc,
                    ..
                } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let entry = self.class(cls);
                    if entry.kind != ClassKind::Class || entry.flags.is_abstract {
                        let what = if entry.flags.is_abstract {
                            "abstract class"
                        } else {
                            match entry.kind {
                                ClassKind::Enum => "enum",
                                ClassKind::Interface => "interface",
                                ClassKind::Trait => "trait",
                                ClassKind::Class => "class",
                            }
                        };
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Cannot instantiate {} {}",
                                what,
                                String::from_utf8_lossy(entry.name.as_bytes())
                            ),
                        ));
                    }
                    if self.is_directory_class(cls) {
                        // php's Directory get_constructor handler.
                        return Err(Thrown::error(
                            "Error",
                            "Cannot directly construct Directory, use dir() instead",
                        ));
                    }
                    self.update_class_constants(cls)?;
                    let obj = self.new_object(cls);
                    self.reg_set(top, dst, Value::Object(obj.clone()));
                    // Run __construct, discarding its return (into the arg window).
                    if let Some(m) = self.lookup_method(cls, b"__construct") {
                        let ptr = Rc::as_ptr(&obj) as usize;
                        let tracked = !self.destructibles.is_empty()
                            && self
                                .destructibles
                                .last()
                                .is_some_and(|d| Rc::as_ptr(d) as usize == ptr);
                        self.enter_method(top, &m, Some(obj), cls, Reg::MAX, base, argc)?;
                        if tracked {
                            self.ctor_frames.push((self.stack.len() - 1, ptr));
                        }
                    }
                }
                Instr::FetchProp { dst, obj, name, .. } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let v = self.prop_fetch(top, o, pname)?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchPropDyn { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
                    let v = self.prop_fetch(top, o, pname)?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchPropIs { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let v = self.prop_fetch_is(top, o, pname)?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchPropDynIs { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
                    let v = self.prop_fetch_is(top, o, pname)?;
                    self.reg_set(top, dst, v);
                }
                Instr::FetchStaticPropIs { dst, class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = proto.names[name as usize].as_bytes().to_vec();
                    let accessible = match self.class(cls).static_props.get(&pname[..]) {
                        Some(sp) => self.static_prop_accessible(self.stack[top].defining_class, sp),
                        None => false,
                    };
                    // The cell via static_prop_cell: runs the class's
                    // deferred (non-literal) initializers first.
                    let v = if accessible {
                        deref(&self.static_prop_cell(cls, &pname)?.borrow())
                    } else {
                        Value::Null
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::StorePropDyn { obj, name, src } => {
                    let o = self.reg_get(top, obj);
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
                    let val = self.reg_get(top, src);
                    self.prop_store(top, o, pname, val)?;
                    if !self.destructibles.is_empty() {
                        self.dtor_check = true; // the old value may be released
                    }
                }
                Instr::CallMethodDyn {
                    dst,
                    obj,
                    name,
                    base,
                    argc,
                } => {
                    let o = self.reg_get(top, obj);
                    let mname = conv::to_string_bytes(&self.reg_get(top, name));
                    if let Value::Closure(_) = o {
                        let args: Vec<Value> =
                            (0..argc).map(|i| self.reg_get(top, base + i)).collect();
                        let r = self.closure_method(&o, &mname, args)?;
                        self.reg_set(top, dst, r);
                        continue;
                    }
                    let Value::Object(h) = o else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to a member function {}() on {}",
                                String::from_utf8_lossy(&mname),
                                type_lc(&o)
                            ),
                        ));
                    };
                    let cls = h.borrow().class;
                    self.invoke_method(top, h, cls, mname, dst, base, argc)?;
                }
                Instr::StoreProp { obj, name, src } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let val = self.reg_get(top, src);
                    self.prop_store(top, o, pname, val)?;
                    if !self.destructibles.is_empty() {
                        self.dtor_check = true; // the old value may be released
                    }
                }
                Instr::IssetDim { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
                    if let Value::Object(h) = &a {
                        if self.class_implements(h.borrow().class, b"ArrayAccess") {
                            let m = self
                                .lookup_method(h.borrow().class, b"offsetexists")
                                .unwrap();
                            let r =
                                self.call_sync(&m, Some(h.clone()), h.borrow().class, vec![k])?;
                            self.reg_set(top, dst, Value::Bool(conv::to_bool(&r)));
                            continue;
                        }
                    }
                    self.reg_set(top, dst, Value::Bool(isset_dim(&a, &k)));
                }
                Instr::IssetProp { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let r = self.prop_isset(top, o, pname)?;
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::EmptyProp { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let r = self.prop_empty(top, o, pname)?;
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::EmptyPropDyn { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
                    let r = self.prop_empty(top, o, pname)?;
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::IssetPropDyn { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
                    let r = self.prop_isset(top, o, pname)?;
                    self.reg_set(top, dst, Value::Bool(r));
                }
                Instr::UnsetProp { obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    self.prop_unset(top, o, pname)?;
                    if !self.destructibles.is_empty() {
                        self.dtor_check = true;
                    }
                }
                Instr::UnsetPropDyn { obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = PhpStr::from_bytes(conv::to_string_bytes(&self.reg_get(top, name)));
                    self.prop_unset(top, o, pname)?;
                    if !self.destructibles.is_empty() {
                        self.dtor_check = true;
                    }
                }
                Instr::FetchStaticProp {
                    dst, class, name, ..
                } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = proto.names[name as usize].as_bytes().to_vec();
                    self.check_static_prop_access(top, cls, &pname)?;
                    let cell = self.static_prop_cell(cls, &pname)?;
                    let v = cell.borrow().clone();
                    self.reg_set(top, dst, v);
                }
                Instr::FetchClassConst {
                    dst, class, name, ..
                } => {
                    let name_b = proto.names[name as usize].as_bytes().to_vec();
                    let cls = self.resolve_class_autoload(top, class)?;
                    if name_b == b"class" {
                        let v = Value::str(self.class(cls).name.as_bytes().to_vec());
                        self.reg_set(top, dst, v);
                    } else {
                        // php loads every constant of a backed enum on first
                        // fetch (it builds the value -> case table).
                        // (Not from inside a constant expression, which goes
                        // through zend_get_class_constant_ex in php.)
                        let in_const_expr = proto.name.as_bytes() == b"{init}";
                        if self.class(cls).enum_backing.is_some()
                            && !self.class(cls).updated
                            && !in_const_expr
                        {
                            self.update_class_constants(cls)?;
                        }
                        // Inside a constant expression php names the class as
                        // written (`self::X`); the opcode path uses its name.
                        let display: Vec<u8> = match class {
                            ClassRef::Name(n) if in_const_expr => {
                                proto.names[n as usize].as_bytes().to_vec()
                            }
                            ClassRef::SelfClass if in_const_expr => b"self".to_vec(),
                            ClassRef::ParentClass if in_const_expr => b"parent".to_vec(),
                            ClassRef::StaticClass if in_const_expr => b"static".to_vec(),
                            _ => self.class(cls).name.as_bytes().to_vec(),
                        };
                        match self.class(cls).constants.get(&name_b) {
                            Some(c) => {
                                let (vis, decl) = (c.visibility, c.declaring_class);
                                if !self.check_access(self.stack[top].defining_class, decl, vis) {
                                    return Err(self.access_err("constant", decl, &name_b, vis));
                                }
                                let v = if c.init.is_none() {
                                    c.value.clone()
                                } else {
                                    self.class_const_value(cls, &name_b, &display, in_const_expr)?
                                        .unwrap_or(Value::Null)
                                };
                                if let Value::Object(h) = &v {
                                    self.touch_enum_case(h);
                                }
                                self.reg_set(top, dst, v);
                            }
                            None => {
                                return Err(Thrown::error(
                                    "Error",
                                    format!(
                                        "Undefined constant {}::{}",
                                        String::from_utf8_lossy(&display),
                                        String::from_utf8_lossy(&name_b)
                                    ),
                                ))
                            }
                        }
                    }
                }
                Instr::CallMethod {
                    dst,
                    obj,
                    name,
                    base,
                    argc,
                    ..
                } => {
                    let o = self.reg_get(top, obj);
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    if let Value::Closure(_) = o {
                        let args: Vec<Value> =
                            (0..argc).map(|i| self.reg_get(top, base + i)).collect();
                        let r = self.closure_method(&o, &mname, args)?;
                        self.reg_set(top, dst, r);
                        continue;
                    }
                    let Value::Object(h) = o else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to a member function {}() on {}",
                                String::from_utf8_lossy(&mname),
                                type_lc(&o)
                            ),
                        ));
                    };
                    let cls = h.borrow().class;
                    self.invoke_method(top, h, cls, mname, dst, base, argc)?;
                }
                Instr::CallStatic {
                    dst,
                    class,
                    name,
                    base,
                    argc,
                    ..
                } => {
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    self.do_call_static(top, class, mname, dst, base, argc)?;
                }
                Instr::CallStaticDyn {
                    dst,
                    class,
                    name,
                    base,
                    argc,
                } => {
                    let mname = conv::to_string_bytes(&self.reg_get(top, name));
                    self.do_call_static(top, class, mname, dst, base, argc)?;
                }
                Instr::InstanceOf { dst, val, class } => {
                    let v = self.reg_get(top, val);
                    let result = match v {
                        Value::Object(h) => match self.resolve_class_autoload(top, class) {
                            Ok(cid) => self.class(h.borrow().class).ancestors.contains(&cid),
                            Err(_) => false,
                        },
                        // Closures / arrow fns / first-class callables are
                        // instances of the built-in `Closure` class.
                        Value::Closure(_) => match class {
                            ClassRef::Name(nidx) => {
                                let n = proto.names[nidx as usize].as_bytes();
                                let seg = n.rsplit(|&b| b == b'\\').next().unwrap_or(n);
                                seg.eq_ignore_ascii_case(b"Closure")
                            }
                            _ => false,
                        },
                        _ => false,
                    };
                    self.reg_set(top, dst, Value::Bool(result));
                }
                Instr::CloneObj { dst, src } => {
                    let v = self.reg_get(top, src);
                    if let Value::Closure(cd) = &v {
                        // A cloned closure is a new Closure object.
                        let copy = rhp_values::ClosureData {
                            fnidx: cd.fnidx,
                            captures: cd.captures.clone(),
                            bound_this: cd.bound_this.clone(),
                            bound_class: cd.bound_class,
                            bound_static: cd.bound_static,
                            wrapped: cd.wrapped.clone(),
                        };
                        self.reg_set(top, dst, Value::Closure(Rc::new(copy)));
                        continue;
                    }
                    let Value::Object(h) = v else {
                        return Err(Thrown::error(
                            "Error",
                            "__clone method called on non-object",
                        ));
                    };
                    if let Some(class) = rhp_builtins::curl_uncloneable(&h) {
                        return Err(Thrown::error(
                            "Error",
                            format!("Trying to clone an uncloneable object of class {class}"),
                        ));
                    }
                    // php's zval_add_ref: a reference only this object holds is
                    // copied as a plain value, so the clone gets its own slot.
                    // Holders other than the object and our copy that are
                    // compiler temporaries (a dead `&` result, a by-ref
                    // argument already passed) are not PHP references.
                    let (cls, props) = {
                        let od = h.borrow();
                        let mut props = od.properties.clone();
                        for v in props.values_mut() {
                            if let Value::Ref(rc) = v {
                                if Rc::strong_count(rc) == 2 + self.temp_ref_holders(rc) {
                                    let inner = rc.borrow().clone();
                                    *v = inner;
                                }
                            }
                        }
                        (od.class, props)
                    };
                    let uncloneable = self.is_directory_class(cls)
                        || self.class(cls).kind == ClassKind::Enum
                        || self.generators.contains_key(&h.borrow().id)
                        || self
                            .class(cls)
                            .name
                            .as_bytes()
                            .eq_ignore_ascii_case(b"WeakReference");
                    if uncloneable {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Trying to clone an uncloneable object of class {}",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes())
                            ),
                        ));
                    }
                    let id = self.next_object_id;
                    self.next_object_id += 1;
                    // The clone keeps the property slots' unset flags.
                    if let Some(ks) = self.unset_typed_props.get(&h.borrow().id).cloned() {
                        self.unset_typed_props.insert(id, ks);
                    }
                    let new_obj: ObjectHandle = Rc::new(RefCell::new(ObjectData {
                        class: cls,
                        id,
                        properties: props,
                    }));
                    self.track_destructible(&new_obj);
                    rhp_builtins::curl_cloned(&h, &new_obj);
                    self.reg_set(top, dst, Value::Object(new_obj.clone()));
                    if let Some(m) = self.lookup_method(cls, b"__clone") {
                        self.enter_method(top, &m, Some(new_obj), cls, Reg::MAX, src, 0)?;
                    }
                }
                Instr::DeclareClass { class_idx } => {
                    self.declare_class_at((proto.class_base + class_idx) as usize)?;
                }
                Instr::DeclareFunc { idx } => {
                    // The proto is stored in the closures table (file-local index).
                    let f = self.closures[(proto.closure_base + idx) as usize].clone();
                    let lc = f.name.as_bytes().to_ascii_lowercase();
                    if self.fn_names.contains_key(&lc) {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Cannot redeclare function {}()",
                                String::from_utf8_lossy(f.name.as_bytes())
                            ),
                        ));
                    }
                    let i = self.functions.len();
                    self.functions.push(f);
                    self.fn_names.insert(lc, i);
                }

                Instr::BindRef { dst_cv, src_cv } => {
                    let rc = self.promote(top, src_cv);
                    self.stack[top].regs[dst_cv as usize] = Value::Ref(rc);
                    self.sync_global_cv(top, dst_cv);
                }
                Instr::RefAssign { dst_cv, src } => {
                    // `src` already holds a `Ref` (MakeRefLval); alias it raw.
                    let v = self.stack[top].regs[src as usize].clone();
                    self.stack[top].regs[dst_cv as usize] = v;
                    self.sync_global_cv(top, dst_cv);
                }
                Instr::MakeRef { dst, cv } => {
                    let rc = self.promote(top, cv);
                    self.stack[top].regs[dst as usize] = Value::Ref(rc);
                }
                Instr::MakeRefVal { dst, src } => {
                    let v = self.reg_get(top, src);
                    self.reg_set(top, dst, Value::Ref(Rc::new(RefCell::new(v))));
                }

                Instr::CallV {
                    dst,
                    name,
                    args,
                    fallback,
                } => {
                    let (user, fname) = self.resolve_fn(name, fallback, &proto.names);
                    if let Some(reason) =
                        builtins::is_refused(std::str::from_utf8(&fname).unwrap_or(""))
                    {
                        let shown = String::from_utf8_lossy(&fname).into_owned();
                        return Err(Thrown::unsupported(format!("{shown}()"), reason));
                    }
                    let arr = self.reg_get(top, args);
                    let (argv, named) = self.split_call_args(&arr)?;
                    if let Some(fnidx) = user {
                        let callee = self.functions[fnidx].clone();
                        let argv = self.place_named_args(&callee, None, argv, named)?;
                        self.push_user_frame(callee, argv, dst)?;
                    } else if !named.is_empty() {
                        // `call_user_func($f, ...named)` forwards them.
                        if fname.eq_ignore_ascii_case(b"call_user_func") && !argv.is_empty() {
                            let out = self.call_named(
                                &argv[0],
                                argv[1..].to_vec(),
                                named,
                                Some(self.caller_strict()),
                            )?;
                            self.reg_set(top, dst, out);
                        } else {
                            return Err(Thrown::unsupported(
                                "named arguments to a builtin function",
                                "deferred",
                            ));
                        }
                    } else {
                        let name_str = String::from_utf8_lossy(&fname).into_owned();
                        if builtins::lookup(&name_str).is_none() {
                            return Err(undefined_function(&proto.names[name as usize]));
                        }
                        self.mark_user_call_builtin(&fname, fallback);
                        let out = self.invoke_builtin(&name_str, argv);
                        self.cuf_caller = None;
                        let out = out?;
                        self.reg_set(top, dst, out);
                    }
                }
                Instr::NewV { dst, class, args } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let entry = self.class(cls);
                    if entry.kind != ClassKind::Class || entry.flags.is_abstract {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Cannot instantiate {}",
                                String::from_utf8_lossy(entry.name.as_bytes())
                            ),
                        ));
                    }
                    self.update_class_constants(cls)?;
                    let obj = self.new_object(cls);
                    self.reg_set(top, dst, Value::Object(obj.clone()));
                    if let Some(m) = self.lookup_method(cls, b"__construct") {
                        let arr = self.reg_get(top, args);
                        let (argv, named) = self.split_call_args(&arr)?;
                        let argv =
                            self.place_named_args(&m.proto, Some(m.defining_class), argv, named)?;
                        self.push_method_frame(&m, Some(obj), cls, argv, Reg::MAX)?;
                    }
                }
                Instr::CallMethodV {
                    dst,
                    obj,
                    name,
                    args,
                } => {
                    let o = self.reg_get(top, obj);
                    let Value::Object(h) = o else {
                        return Err(Thrown::type_error(
                            "Call to a member function on non-object",
                        ));
                    };
                    let cls = h.borrow().class;
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    let Some(m) = self.lookup_method(cls, &mname) else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to undefined method {}::{}()",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                String::from_utf8_lossy(&mname)
                            ),
                        ));
                    };
                    if !self.check_access(
                        self.stack[top].defining_class,
                        m.defining_class,
                        m.visibility,
                    ) {
                        return Err(self.access_err(
                            "method",
                            m.defining_class,
                            &mname,
                            m.visibility,
                        ));
                    }
                    let arr = self.reg_get(top, args);
                    let (argv, named) = self.split_call_args(&arr)?;
                    let argv =
                        self.place_named_args(&m.proto, Some(m.defining_class), argv, named)?;
                    self.push_method_frame(&m, Some(h.clone()), cls, argv, dst)?;
                }
                Instr::CallStaticV {
                    dst,
                    class,
                    name,
                    args,
                } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    let Some(m) = self.lookup_method(cls, &mname) else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to undefined method {}::{}()",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                String::from_utf8_lossy(&mname)
                            ),
                        ));
                    };
                    let called = match class {
                        ClassRef::SelfClass | ClassRef::ParentClass | ClassRef::StaticClass => {
                            self.stack[top].called_class.unwrap_or(cls)
                        }
                        _ => cls,
                    };
                    let this = if m.is_static {
                        None
                    } else {
                        self.current_this(top)
                            .filter(|h| self.class(h.borrow().class).ancestors.contains(&cls))
                    };
                    let arr = self.reg_get(top, args);
                    let (argv, named) = self.split_call_args(&arr)?;
                    let argv =
                        self.place_named_args(&m.proto, Some(m.defining_class), argv, named)?;
                    self.push_method_frame(&m, this, called, argv, dst)?;
                }

                Instr::FetchConst {
                    dst,
                    name,
                    fallback,
                } => {
                    let cname = proto.names[name as usize].as_bytes().to_vec();
                    let v = self.constants.get(&cname).cloned().or_else(|| {
                        if fallback != rhp_bytecode::NO_NAME {
                            self.constants
                                .get(proto.names[fallback as usize].as_bytes())
                                .cloned()
                        } else {
                            None
                        }
                    });
                    match v {
                        Some(v) => {
                            let bare = cname.rsplit(|&c| c == b'\\').next().unwrap_or(&cname);
                            if bare == b"E_STRICT" && self.constants.contains_key(&b"E_STRICT"[..])
                            {
                                self.diag(
                                    8192,
                                    "Constant E_STRICT is deprecated since 8.4, the error level was removed",
                                )?;
                            }
                            self.reg_set(top, dst, v);
                        }
                        None => {
                            return Err(Thrown::error(
                                "Error",
                                format!(
                                    "Undefined constant \"{}\"",
                                    String::from_utf8_lossy(&cname)
                                ),
                            ))
                        }
                    }
                }

                Instr::Refuse { feature, .. } => {
                    let f = proto.names[feature as usize].as_bytes().to_vec();
                    return Err(self.locate_unsupported(Thrown::unsupported(
                        String::from_utf8_lossy(&f).into_owned(),
                        "refused construct",
                    )));
                }

                Instr::Ret { src } => {
                    let mut v = self.reg_get(top, src);
                    // Enforce a declared return type (the callee's own file decides
                    // strict vs coercive).
                    if let (Some(ty), false) = (&proto.return_type, proto.is_generator) {
                        let ctx = self.stack[top].defining_class;
                        v = self.coerce_stringable(v, ty, proto.strict_types, ctx)?;
                        v = self.check_return_type(&proto, top, v)?;
                    }
                    if let Some(done) = self.do_return(v) {
                        return Ok(done);
                    }
                }
                Instr::RetRef { src } => {
                    // Return-by-reference: deliver the raw slot (a `Value::Ref`
                    // cell) without dereferencing, so the caller's `$x = &f()`
                    // aliases the same cell. No return-type coercion.
                    let v = self.stack[top].regs[src as usize].clone();
                    if let Some(done) = self.do_return(v) {
                        return Ok(done);
                    }
                }
                Instr::ClearTemps { from, to } => {
                    let regs = &mut self.stack[top].regs[from as usize..to as usize];
                    if !self.destructibles.is_empty()
                        && regs
                            .iter()
                            .any(|v| matches!(v, Value::Object(_) | Value::Array(_)))
                    {
                        self.dtor_check = true;
                    }
                    for r in regs {
                        *r = Value::Null;
                    }
                }
                Instr::RetNull | Instr::Halt => {
                    if proto.return_type.is_some() && !proto.is_generator {
                        self.check_implicit_return(&proto, top)?;
                    }
                    if let Some(done) = self.do_return(Value::Null) {
                        return Ok(done);
                    }
                }
                Instr::Include { dst, path, kind } => {
                    let raw = conv::to_string_bytes(&self.reg_get(top, path));
                    let v = self.do_include(top, raw, kind)?;
                    self.reg_set(top, dst, v);
                }
                Instr::Exit { arg } => {
                    // A string message is printed (code 0); an int is the code.
                    let code = match self.reg_get(top, arg) {
                        Value::Int(n) => n as i32,
                        Value::Str(s) => {
                            self.emit_stdout(s.as_bytes());
                            0
                        }
                        Value::Null => 0,
                        other => {
                            let s = conv::to_string_bytes(&other);
                            self.emit_stdout(&s);
                            0
                        }
                    };
                    return Err(Thrown::Exit(code));
                }
                Instr::Yield { dst, key, value } => {
                    // Suspend: hand (key, value, resume-dst) to the generator
                    // driver, leaving this frame on the stack for it to reclaim.
                    // A by-reference generator (`function &gen()`) yields a
                    // variable as a reference to it (php's ZEND_YIELD with
                    // ZEND_ACC_RETURN_REFERENCE), so `foreach (gen() as &$v)`
                    // writes reach it; any other value is yielded as is.
                    let val = if proto.returns_ref && value < proto.num_cvs {
                        Value::Ref(self.promote(top, value))
                    } else if proto.returns_ref {
                        // A reference made by MakeRef/MakeRefLval, or a plain
                        // value (php: "Only variable references…" notice).
                        let v = self.stack[top].regs[value as usize].clone();
                        if !matches!(v, Value::Ref(_)) {
                            self.diag(
                                8,
                                "Only variable references should be yielded by reference",
                            )?;
                        }
                        v
                    } else {
                        self.reg_get(top, value)
                    };
                    let id = *self.gen_stack.last().expect("yield outside a generator");
                    let key_val = if key == Reg::MAX {
                        let g = self.generators.get_mut(&id).unwrap();
                        let k = Value::Int(g.auto_key);
                        g.auto_key += 1;
                        k
                    } else {
                        let kv = self.reg_get(top, key);
                        if let Value::Int(n) = &kv {
                            let g = self.generators.get_mut(&id).unwrap();
                            if *n >= g.auto_key {
                                g.auto_key = n + 1;
                            }
                        }
                        kv
                    };
                    self.yield_out = Some((key_val, val, dst));
                    return Ok(Value::Null);
                }
            }
        }
    }

    /// Unwind a thrown exception through the exception table: find a matching
    /// catch (bind + jump) or finally (re-raise after) in the current frame,
    /// else pop and continue in the caller. Returns `Ok` when a handler is
    /// installed (re-enter `run_frames`), `Err` when it escapes the top frame.
    fn eval_cmp(&mut self, kind: CmpKind, a: &Value, b: &Value) -> Result<Value, Thrown> {
        use std::cmp::Ordering;
        Ok(match kind {
            CmpKind::Eq => Value::Bool(self.compare_values(a, b)? == Ordering::Equal),
            CmpKind::Ne => Value::Bool(self.compare_values(a, b)? != Ordering::Equal),
            CmpKind::Identical => Value::Bool(ops::strict_eq(a, b)),
            CmpKind::NotIdentical => Value::Bool(!ops::strict_eq(a, b)),
            CmpKind::Lt => Value::Bool(self.compare_values(a, b)?.is_lt()),
            CmpKind::Le => Value::Bool(self.compare_values(a, b)?.is_le()),
            // php evaluates `a > b` as `b < a` (ZEND_IS_SMALLER with swapped
            // operands), so uncomparable pairs are false in both directions.
            CmpKind::Gt => Value::Bool(self.compare_values(b, a)?.is_lt()),
            CmpKind::Ge => Value::Bool(self.compare_values(b, a)?.is_le()),
            CmpKind::Spaceship => Value::Int(match self.compare_values(a, b)? {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            }),
        })
    }

    /// An exception was caught at frame depth `depth`: end every `@` region
    /// opened at or above it (php's live-range cleanup of ZEND_END_SILENCE).
    fn unsilence_frames(&mut self, depth: usize) {
        while let Some(&(d, lvl)) = self.silence_stack.last() {
            if d < depth {
                break;
            }
            self.silence = lvl;
            self.silence_stack.pop();
        }
    }

    fn unwind(&mut self, thrown: Thrown, floor: usize) -> Result<UnwindResult, Thrown> {
        // An engine error becomes a real object here, before any frame is popped,
        // so its file/line/trace describe the throw point.
        let thrown = self.materialize(thrown, None);
        loop {
            if self.stack.len() <= floor {
                return Ok(UnwindResult::Escaped(thrown)); // escaped the (sub)run
            }
            let top = self.stack.len() - 1;
            let proto = self.stack[top].proto.clone();
            // The throwing pc has already been advanced past the faulting instr.
            let throw_pc = (self.stack[top].pc as u32).saturating_sub(1);
            let in_try = |r: &&rhp_bytecode::ExRegion| throw_pc >= r.start && throw_pc < r.end;
            let handler = proto.exception_table.iter().find(in_try).cloned();
            // Thrown from inside a running finally block and not handled
            // within it: the exception (or return) that block was finishing
            // is discarded, and a pending exception becomes the innermost
            // `previous` of the new one (zend_exception_set_previous).
            let in_finally = proto.exception_table.iter().filter(|r| {
                r.finally
                    .is_some_and(|f| throw_pc >= f && throw_pc <= r.finally_end)
            });
            for f in in_finally {
                let fin = f.finally.unwrap_or(0);
                let handled_inside = handler
                    .as_ref()
                    .is_some_and(|h| h.start >= fin && h.end <= f.finally_end);
                if !handled_inside {
                    let pend = &mut self.stack[top].pending;
                    if let Some(i) = pend.iter().position(|(k, _)| *k == fin) {
                        if let (_, FinallyCont::Rethrow(prev)) = pend.remove(i) {
                            self.chain_previous(&thrown, prev);
                        }
                    }
                }
            }
            if let Some(region) = handler {
                if let Some(cls) = self.thrown_class_id(&thrown) {
                    for arm in &region.catches {
                        if arm
                            .classes
                            .iter()
                            .any(|cn| self.class_matches(cls, cn.as_bytes()))
                        {
                            let obj = self.thrown_to_object(&thrown)?;
                            if let Some(vr) = arm.var_reg {
                                self.stack[top].regs[vr as usize] = obj;
                            }
                            self.stack[top].pc = arm.target as usize;
                            self.unsilence_frames(top + 1);
                            return Ok(UnwindResult::Caught);
                        }
                    }
                }
                if let Some(fin) = region.finally {
                    self.push_pending(top, fin, FinallyCont::Rethrow(thrown));
                    self.stack[top].pc = fin as usize;
                    self.unsilence_frames(top + 1);
                    return Ok(UnwindResult::Caught);
                }
            }
            let popped = self.stack.pop();
            let depth = self.stack.len();
            if let Some((_, ptr)) = self.ctor_frames.pop_if(|c| c.0 == depth) {
                // The constructor threw: its object will never be destructed.
                self.destructibles.retain(|d| Rc::as_ptr(d) as usize != ptr);
            }
            if self.stack.is_empty() && floor == 0 {
                // An uncaught exception left {main}: keep its globals.
                self.finished_main = popped;
            }
        }
    }

    /// Record what the finally block starting at `fin` does when it ends.
    fn push_pending(&mut self, top: usize, fin: u32, cont: FinallyCont) {
        let pend = &mut self.stack[top].pending;
        pend.retain(|(k, _)| *k != fin);
        pend.push((fin, cont));
    }

    /// A `return`/`break` leaving the finally blocks that contain `pc`
    /// discards whatever those blocks were finishing.
    fn drop_left_finallies(&mut self, top: usize, proto: &FunctionProto, pc: u32) {
        if self.stack[top].pending.is_empty() {
            return;
        }
        for r in &proto.exception_table {
            if let Some(f) = r.finally {
                if pc >= f && pc <= r.finally_end {
                    self.stack[top].pending.retain(|(k, _)| *k != f);
                }
            }
        }
    }

    /// Jump to `target`, first running the innermost enclosing finally that
    /// `target` is outside of (the jump resumes when that block ends).
    fn jump_through_finally(&mut self, top: usize, proto: &FunctionProto, pc: u32, target: u32) {
        if let Some(f) = enclosing_finally(proto, pc, Some(target)) {
            self.push_pending(top, f, FinallyCont::Jump(target));
            self.stack[top].pc = f as usize;
        } else {
            self.stack[top].pc = target as usize;
        }
    }

    /// zend_exception_set_previous: append `prev` at the end of `thrown`'s
    /// `previous` chain, unless it is already part of it (no cycles).
    fn chain_previous(&self, thrown: &Thrown, prev: Thrown) {
        let (Thrown::Object(ex), Thrown::Object(add)) = (thrown, &prev) else {
            return;
        };
        if Rc::ptr_eq(ex, add) {
            return;
        }
        let previous_of = |o: &ObjectHandle| match o
            .borrow()
            .properties
            .get(&self.declared_prop_key(o, b"previous"))
            .map(deref)
        {
            Some(Value::Object(p)) => Some(p),
            _ => None,
        };
        // `add`'s own chain must not already contain `ex`.
        let mut anc = previous_of(add);
        while let Some(a) = anc {
            if Rc::ptr_eq(&a, ex) {
                return;
            }
            anc = previous_of(&a);
        }
        let mut cur = ex.clone();
        loop {
            match previous_of(&cur) {
                None => {
                    let key = self.declared_prop_key(&cur, b"previous");
                    cur.borrow_mut()
                        .properties
                        .insert(key, Value::Object(add.clone()));
                    return;
                }
                Some(p) if Rc::ptr_eq(&p, add) => return,
                Some(p) => cur = p,
            }
        }
    }

    /// Synchronously call a method and return its value (re-entrant: used by
    /// magic methods like `__toString`/`__get` mid-opcode). Exceptions thrown
    /// inside are handled within the sub-run; if uncaught there, they propagate.
    fn call_sync(
        &mut self,
        method: &classes::ResolvedMethod,
        this: Option<ObjectHandle>,
        called_class: ClassId,
        args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        self.call_sync_mode(method, this, called_class, args, None)
    }

    /// [`Self::call_sync`] with the caller's `strict_types` mode for the
    /// argument checks (an internal caller is always coercive).
    fn call_sync_mode(
        &mut self,
        method: &classes::ResolvedMethod,
        this: Option<ObjectHandle>,
        called_class: ClassId,
        args: Vec<Value>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        let proto = method.proto.clone();
        self.run_proto_sync(
            proto,
            this,
            Some(method.defining_class),
            Some(called_class),
            args,
            caller,
        )
    }

    /// Push a capture frame for `proto` and drive it to completion, returning
    /// its value (re-entrant; exceptions handled within the sub-run).
    fn run_proto_sync(
        &mut self,
        proto: Rc<FunctionProto>,
        this: Option<ObjectHandle>,
        defining_class: Option<ClassId>,
        called_class: Option<ClassId>,
        args: Vec<Value>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let mut frame = Frame::new(proto.clone(), 0);
        if let (Some(cv), Some(obj)) = (proto.this_cv, this) {
            frame.regs[cv as usize] = Value::Object(obj);
        }
        frame.defining_class = defining_class;
        frame.called_class = called_class;
        self.bind_frame(&mut frame, &args, caller)?;
        self.bind_superglobals(&mut frame);
        if proto.is_generator {
            return Ok(self.make_generator(frame));
        }
        frame.capture = true;
        let base = self.stack.len();
        self.stack.push(frame);
        loop {
            match self.run_frames() {
                Ok(v) => return Ok(v),
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        return Err(thrown);
                    }
                    match self.unwind(thrown, base)? {
                        UnwindResult::Caught => {}
                        UnwindResult::Escaped(t) => return Err(t),
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // include / require
    // -----------------------------------------------------------------------

    /// Execute one `include`/`require` at runtime: resolve the path, (compile &)
    /// run the file, and return its value (default `1`, or `false` for a missing
    /// `include`). `require` of a missing file is fatal.
    /// `eval($code)`: compile the PHP source at runtime and run it in the current
    /// variable scope (`top`). Returns the script's return value (Null if none).
    fn do_eval(&mut self, top: usize, src: Reg) -> Result<Value, Thrown> {
        let code = conv::to_string_bytes(&self.reg_get(top, src));
        // eval() code is already in PHP mode; prepend an open tag so the parser
        // (which starts in inline/HTML mode) enters PHP.
        let mut source = b"<?php ".to_vec();
        source.extend_from_slice(&code);
        // php names eval'd code "FILE(LINE) : eval()'d code" (its __FILE__, getFile()).
        let file = format!(
            "{}({}) : eval()'d code",
            String::from_utf8_lossy(&self.current_file(top)),
            self.current_line(top)
        );

        let mut cb = self
            .includer
            .take()
            .ok_or_else(|| Thrown::error("Error", "eval is not available"))?;
        let compiled = cb(&file, &source);
        self.includer = Some(cb);
        let prog = self.included_program(compiled, &file)?;

        let main = self.install_included(&prog)?;
        self.run_included(main, top, "eval")
    }

    /// The includer's result: a syntax error becomes a ParseError object
    /// located in the included file (or eval()'d code) at the error's line.
    fn included_program(
        &mut self,
        compiled: Result<Program, IncludeError>,
        file: &str,
    ) -> Result<Program, Thrown> {
        match compiled {
            Ok(p) => Ok(p),
            Err(IncludeError::Thrown(t)) => Err(t),
            Err(IncludeError::Parse { message, line }) => Err(self.materialize(
                Thrown::error("ParseError", message),
                Some((file.as_bytes().to_vec(), line)),
            )),
        }
    }

    /// The main script failed to parse: php reports `Parse error: …` through
    /// the usual display/log path (exit status 255 is the SAPI's job).
    pub fn report_parse_error(&mut self, message: &str, file: &str, line: u32) {
        self.display_error(
            4,
            "Parse error",
            message.as_bytes(),
            file.as_bytes(),
            line as i64,
        );
    }

    fn do_include(
        &mut self,
        top: usize,
        raw_path: Vec<u8>,
        kind: IncludeKind,
    ) -> Result<Value, Thrown> {
        let path_str = String::from_utf8_lossy(&raw_path).into_owned();
        builtins::streams::clear_errno();
        let (realpath, source) = match self.resolve_include_path(top, &path_str) {
            Some(found) => found,
            None => {
                // php_stream_open_for_zend_ex's warning, then the include's.
                let fname = kind.name();
                let ip =
                    String::from_utf8_lossy(&self.ini_get(b"include_path").unwrap_or_default())
                        .into_owned();
                let reason = if path_str.is_empty() {
                    None
                } else {
                    Some(match builtins::streams::last_errno() {
                        Some(c) => builtins::streams::strerror(c),
                        None => builtins::streams::strerror(2), // ENOENT
                    })
                };
                if let Some(reason) = reason {
                    self.diag(
                        2,
                        &format!("{fname}({path_str}): Failed to open stream: {reason}"),
                    )?;
                }
                if kind.is_require() {
                    return Err(Thrown::error(
                        "Error",
                        format!("Failed opening required '{path_str}' (include_path='{ip}')"),
                    ));
                }
                self.diag(
                    2,
                    &format!(
                        "{fname}(): Failed opening '{path_str}' for inclusion (include_path='{ip}')"
                    ),
                )?;
                return Ok(Value::Bool(false));
            }
        };

        // `_once`: skip a file already included (count it as included before
        // running the body, so recursive `_once` includes terminate).
        if kind.is_once() && self.included_files.contains(&realpath) {
            return Ok(Value::Bool(true));
        }
        self.included_files.insert(realpath.clone());
        // php opens each included file as a stream resource (closed again
        // after compiling): it uses up a resource id.
        self.next_resource_id += 1;

        // Compile via the SAPI-provided callback (keeps the VM compiler-agnostic).
        let mut cb = self
            .includer
            .take()
            .ok_or_else(|| Thrown::error("Error", "file inclusion is not available"))?;
        let rp_str = String::from_utf8_lossy(&realpath).into_owned();
        let compiled = cb(&rp_str, &source);
        self.includer = Some(cb);
        let prog = self.included_program(compiled, &rp_str)?;

        let main = self.install_included(&prog)?;
        let ret = self.run_included(main, top, kind.name())?;
        // A file with no `return` yields int(1).
        Ok(match ret {
            Value::Null => Value::Int(1),
            v => v,
        })
    }

    /// Resolve an include path to `(realpath_bytes, source)`: for a schemed URL
    /// (`phar://`, …) read through the stream wrapper directly; otherwise
    /// absolute as-is, else relative to the including file's directory, then CWD.
    fn resolve_include_path(&self, top: usize, path: &str) -> Option<(Vec<u8>, Vec<u8>)> {
        use std::path::{Path, PathBuf};
        // Schemed URLs: no candidate search — the wrapper owns resolution. The
        // returned key (→ `__FILE__`/`__DIR__`/`_once`) is the normalized URL.
        let (scheme, _) = builtins::streams::split_scheme(path.as_bytes());
        if scheme != b"file" {
            let url = path.as_bytes();
            let w = self.wrapper_for(url)?;
            let src = w.get_contents(url)?;
            let key = w.canonical(url).unwrap_or_else(|| normalize_url(url));
            return Some((key, src));
        }
        let p = Path::new(path);
        let mut candidates: Vec<PathBuf> = Vec::new();
        if p.is_absolute() {
            candidates.push(p.to_path_buf());
        } else {
            let cur = self.current_file(top);
            let cur = String::from_utf8_lossy(&cur).into_owned();
            if let Some(dir) = Path::new(&cur).parent() {
                candidates.push(dir.join(p));
            }
            candidates.push(p.to_path_buf()); // relative to CWD
        }
        for cand in candidates {
            if let Ok(canon) = std::fs::canonicalize(&cand) {
                if let Ok(src) = std::fs::read(&canon) {
                    return Some((canon.to_string_lossy().into_owned().into_bytes(), src));
                }
            }
        }
        None
    }

    /// Merge an included program's declarations into the running VM, rebasing its
    /// program-global closure indices by the current closure count. Returns the
    /// rebased `{main}` to execute.
    fn install_included(&mut self, prog: &Program) -> Result<Rc<FunctionProto>, Thrown> {
        let base = self.closures.len() as u32;
        let cbase = self.decl_classes.len();
        // closures: append (rebased).
        for c in &prog.closures {
            self.closures
                .push(Rc::new(rebase_proto_c(c, base, cbase as u32)));
        }
        // functions: skip the re-emitted prelude helpers (the first N); a user
        // function is fatal to redeclare (php semantics).
        for proto in prog.functions.iter().skip(prog.num_prelude_functions) {
            let lc = proto.name.as_bytes().to_ascii_lowercase();
            if self.fn_names.contains_key(&lc) {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot redeclare function {}()",
                        String::from_utf8_lossy(proto.name.as_bytes())
                    ),
                ));
            }
            let idx = self.functions.len();
            self.functions
                .push(Rc::new(rebase_proto_c(proto, base, cbase as u32)));
            self.fn_names.insert(lc, idx);
        }
        // classes: a name already in use is not early-bound; redeclaring it is
        // fatal only when its declaration statement runs (`DeclareClass`).
        let top = self.register_decl_classes(prog, base);
        self.hoist_decl_classes(top)?;
        // constants: define_const skips an already-defined name.
        for (n, v) in &prog.constants {
            self.define_const(n.as_bytes(), v.clone());
        }
        Ok(Rc::new(rebase_proto_c(&prog.main, base, cbase as u32)))
    }

    /// Run an included `{main}` to completion (isolated scope; it shares the VM's
    /// function/class/constant tables) and return its value.
    /// Run an included / eval'd script's `{main}` **in the calling scope**: its
    /// CVs are bound by name to the caller frame's variables (shared cells), so
    /// writes are visible to the caller and reads see the caller's current
    /// values — php's include/eval scope semantics. `$this` / class scope are
    /// forwarded too (an include inside a method can use `$this`).
    fn run_included(
        &mut self,
        main: Rc<FunctionProto>,
        caller_top: usize,
        kind: &'static str,
    ) -> Result<Value, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let mut frame = Frame::new(main.clone(), 0);
        frame.capture = true;
        frame.include_kind = Some(kind);
        frame.defining_class = self.stack[caller_top].defining_class;
        frame.called_class = self.stack[caller_top].called_class;

        // Bind each CV to the caller's same-named variable (shared via a Ref).
        // From the global scope, the included file's variables are the global
        // symbol table's (including the ones it creates).
        let caller_proto = self.stack[caller_top].proto.clone();
        if self.stack[caller_top].global_scope {
            self.bind_global_scope(&mut frame);
        }
        for ci in 0..main.cv_var_names.len() {
            if frame.global_scope {
                break;
            }
            let name = &main.cv_var_names[ci];
            if let Some(caller_ci) = caller_proto.cv_var_names.iter().position(|n| n == name) {
                let rc = self.promote(caller_top, caller_ci as Reg);
                frame.regs[ci] = Value::Ref(rc);
            }
        }
        self.bind_superglobals(&mut frame);
        let base = self.stack.len();
        self.stack.push(frame);
        loop {
            match self.run_frames() {
                Ok(v) => return Ok(v),
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        return Err(thrown);
                    }
                    match self.unwind(thrown, base)? {
                        UnwindResult::Caught => {}
                        UnwindResult::Escaped(t) => return Err(t),
                    }
                }
            }
        }
    }

    /// Push a capture frame for a closure (captures bound to CVs `0..nc`, params
    /// after) and drive it to completion (re-entrant; exceptions handled within).
    fn run_closure_sync(
        &mut self,
        cd: &rhp_values::ClosureData,
        args: Vec<Value>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let proto = self.closures[cd.fnidx as usize].clone();
        let mut frame = Frame::new(proto.clone(), 0);
        for (i, cap) in cd.captures.iter().enumerate() {
            frame.regs[i] = cap.clone();
        }
        // A rebound closure (`Closure::bind`/`bindTo`) overrides the captured
        // `$this` with the new object.
        if let (Some(cv), Some(obj)) = (proto.this_cv, &cd.bound_this) {
            frame.regs[cv as usize] = Value::Object(obj.clone());
        }
        frame.defining_class = cd.bound_class;
        frame.called_class = cd.bound_static;
        self.bind_frame(&mut frame, &args, caller)?;
        self.bind_superglobals(&mut frame);
        if proto.is_generator {
            return Ok(self.make_generator(frame));
        }
        frame.capture = true;
        let base = self.stack.len();
        self.stack.push(frame);
        loop {
            match self.run_frames() {
                Ok(v) => return Ok(v),
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        return Err(thrown);
                    }
                    match self.unwind(thrown, base)? {
                        UnwindResult::Caught => {}
                        UnwindResult::Escaped(t) => return Err(t),
                    }
                }
            }
        }
    }

    /// `(object)` cast: wrap a value in a fresh `stdClass`. Objects pass through;
    /// arrays map each element to a property; null yields an empty object; a
    /// scalar yields `{ scalar: value }`.
    fn cast_to_object(&mut self, v: Value) -> Value {
        let v = deref(&v);
        if let Value::Object(_) | Value::Closure(_) = v {
            return v;
        }
        let cls = match self.resolve_class_by_name(b"stdClass") {
            Ok(c) => c,
            Err(_) => return v, // prelude missing (shouldn't happen)
        };
        let obj = self.new_object(cls);
        {
            let mut b = obj.borrow_mut();
            match v {
                Value::Null => {}
                Value::Array(a) => {
                    for (k, val) in a.iter() {
                        let name = match k {
                            ArrayKey::Int(n) => {
                                rhp_values::PhpStr::from_bytes(n.to_string().into_bytes())
                            }
                            ArrayKey::Str(s) => s.clone(),
                        };
                        b.properties.insert(name, deref(val));
                    }
                }
                scalar => {
                    b.properties
                        .insert(rhp_values::PhpStr::from("scalar"), scalar);
                }
            }
        }
        Value::Object(obj)
    }

    /// Does `class_id`'s ancestor set include the named interface/class?
    fn class_implements(&self, class_id: ClassId, name: &[u8]) -> bool {
        match self.resolve_class_by_name(name) {
            Ok(target) => self.classes[class_id as usize].ancestors.contains(&target),
            Err(_) => false,
        }
    }

    /// Call a zero-argument method on an object (for the iterator protocol).
    fn call_iter_method(&mut self, h: &ObjectHandle, name: &[u8]) -> Result<Value, Thrown> {
        let cls = h.borrow().class;
        let m = self
            .lookup_method(cls, name)
            .ok_or_else(|| Thrown::error("Error", "Iterator method not found"))?;
        self.call_sync(&m, Some(h.clone()), cls, Vec::new())
    }

    /// `(array) $object`: every property, keyed with php's visibility mangling —
    /// public `name`, protected `"\0*\0name"`, private `"\0Class\0name"`.
    fn object_to_array_mangled(&self, h: &ObjectHandle) -> Value {
        let b = h.borrow();
        let cls = b.class;
        let mut a = PhpArray::new();
        let entry = self.class(cls);
        for (key, val) in &b.properties {
            if self.internal_slot(cls, key) {
                continue;
            }
            let key = classes::mangled_prop_key(entry, key);
            a.insert(ArrayKey::from_bytes(key.as_bytes()), deref(val).clone());
        }
        Value::Array(a)
    }

    /// Snapshot an object's accessible (public from `caller` scope) properties as
    /// an array, for `foreach` over a plain object.
    /// Storage key Reflection uses for property `name` of `h` when the
    /// `ReflectionProperty` belongs to class `class`: that class's own private
    /// first, else the visible declaration (visibility is bypassed).
    fn reflect_prop_key(&self, h: &ObjectHandle, class: &[u8], name: &[u8]) -> PhpStr {
        let cls = h.borrow().class;
        let name = PhpStr::from_bytes(name.to_vec());
        let scope = self.resolve_class_by_name(class).ok();
        match classes::resolve_prop(&self.classes, cls, scope, &name) {
            PropRes::Slot(k) => k,
            PropRes::Dynamic => name,
            PropRes::Denied(_) => self.classes[cls as usize]
                .prop_visible
                .get(&name)
                .cloned()
                .unwrap_or(name),
        }
    }

    fn object_prop_array(&self, h: &ObjectHandle, top: usize) -> PhpArray {
        let caller = self.stack[top].defining_class;
        let b = h.borrow();
        let cls = b.class;
        let mut arr = PhpArray::new();
        for (key, val) in &b.properties {
            if self.internal_slot(cls, key) {
                continue;
            }
            if let Some(name) = classes::slot_accessible(&self.classes, cls, caller, key) {
                // Property names iterate as string keys, even numeric ones.
                arr.insert(ArrayKey::Str(name), deref(val).clone());
            }
        }
        arr
    }

    /// Wrap a prepared (but not pushed) frame as a `Generator` object; the body
    /// runs lazily under the generator protocol.
    fn make_generator(&mut self, mut frame: Frame) -> Value {
        frame.ret_dst = Reg::MAX;
        let cls = self
            .resolve_class_by_name(b"Generator")
            .expect("Generator prelude class");
        let id = self.next_object_id;
        self.next_object_id += 1;
        let obj = Rc::new(RefCell::new(ObjectData {
            class: cls,
            id,
            properties: indexmap::IndexMap::with_hasher(rustc_hash::FxBuildHasher),
        }));
        self.generators.insert(
            id,
            GenState {
                frame: Some(frame),
                status: GenStatus::NotStarted,
                current_key: Value::Null,
                current_val: Value::Null,
                return_val: Value::Null,
                auto_key: 0,
                resume_dst: 0,
                at_first_yield: false,
            },
        );
        Value::Object(obj)
    }

    /// Advance generator `id` to its next `yield` (or completion), injecting
    /// `send` as the result of the yield expression it was suspended on.
    fn advance_gen(&mut self, id: u32, send: Value) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let (mut frame, status, resume_dst) = {
            let g = self.generators.get_mut(&id).unwrap();
            if g.status == GenStatus::Finished {
                return Ok(());
            }
            g.at_first_yield = false;
            (
                g.frame.take().expect("suspended generator frame"),
                g.status,
                g.resume_dst,
            )
        };
        if status == GenStatus::Suspended {
            frame.regs[resume_dst as usize] = send;
        }
        frame.capture = true;
        self.generators.get_mut(&id).unwrap().status = GenStatus::Running;
        let base = self.stack.len();
        self.stack.push(frame);
        self.gen_stack.push(id);
        let saved_yield = self.yield_out.take();
        let result = loop {
            match self.run_frames() {
                Ok(v) => {
                    if let Some((k, val, dst)) = self.yield_out.take() {
                        let frame = self.stack.pop().unwrap();
                        let g = self.generators.get_mut(&id).unwrap();
                        g.frame = Some(frame);
                        g.resume_dst = dst;
                        g.current_key = k;
                        g.current_val = val;
                        g.status = GenStatus::Suspended;
                        break Ok(());
                    } else {
                        let g = self.generators.get_mut(&id).unwrap();
                        g.return_val = v;
                        g.current_key = Value::Null;
                        g.current_val = Value::Null;
                        g.status = GenStatus::Finished;
                        break Ok(());
                    }
                }
                Err(thrown) => {
                    if !thrown.is_catchable() {
                        break Err(thrown);
                    }
                    match self.unwind(thrown, base)? {
                        UnwindResult::Caught => {}
                        UnwindResult::Escaped(t) => {
                            self.generators.get_mut(&id).unwrap().status = GenStatus::Finished;
                            break Err(t);
                        }
                    }
                }
            }
        };
        self.gen_stack.pop();
        self.yield_out = saved_yield;
        result
    }

    /// Ensure a generator has started (run to its first yield).
    fn gen_ensure_started(&mut self, id: u32) -> Result<(), Thrown> {
        if self.generators.get(&id).map(|g| g.status) == Some(GenStatus::NotStarted) {
            self.advance_gen(id, Value::Null)?;
            if let Some(g) = self.generators.get_mut(&id) {
                g.at_first_yield = true;
            }
        }
        Ok(())
    }

    /// zend_generator_rewind: start the generator; one resumed past its
    /// first `yield` cannot go back.
    fn gen_rewind(&mut self, id: u32) -> Result<(), Thrown> {
        self.gen_ensure_started(id)?;
        if self.generators.get(&id).is_some_and(|g| !g.at_first_yield) {
            return Err(Thrown::error(
                "Exception",
                "Cannot rewind a generator that was already run",
            ));
        }
        Ok(())
    }

    /// zend_generator_get_iterator's first check: a finished generator
    /// cannot be iterated again.
    fn gen_check_open(&self, id: u32) -> Result<(), Thrown> {
        if self.generators.get(&id).map(|g| g.status) == Some(GenStatus::Finished) {
            return Err(Thrown::error(
                "Exception",
                "Cannot traverse an already closed generator",
            ));
        }
        Ok(())
    }

    /// Dispatch a built-in `Generator` method. Returns `None` if `name` is not a
    /// generator method (so the caller falls through to normal dispatch).
    fn generator_method(
        &mut self,
        id: u32,
        name_lc: &[u8],
        args: &[Value],
    ) -> Option<Result<Value, Thrown>> {
        Some(match name_lc {
            b"current" => (|| {
                self.gen_ensure_started(id)?;
                Ok(deref(&self.generators[&id].current_val))
            })(),
            b"key" => (|| {
                self.gen_ensure_started(id)?;
                Ok(self.generators[&id].current_key.clone())
            })(),
            b"valid" => (|| {
                self.gen_ensure_started(id)?;
                Ok(Value::Bool(
                    self.generators[&id].status != GenStatus::Finished,
                ))
            })(),
            b"next" => (|| {
                self.gen_ensure_started(id)?;
                self.advance_gen(id, Value::Null)?;
                Ok(Value::Null)
            })(),
            b"send" => (|| {
                let v = args.first().cloned().unwrap_or(Value::Null);
                // Sending to a not-yet-started generator first runs to the first yield.
                if self.generators[&id].status == GenStatus::NotStarted {
                    self.advance_gen(id, Value::Null)?;
                }
                self.advance_gen(id, v)?;
                Ok(deref(&self.generators[&id].current_val))
            })(),
            b"rewind" => (|| {
                self.gen_rewind(id)?;
                Ok(Value::Null)
            })(),
            b"getreturn" => Ok(self.generators[&id].return_val.clone()),
            _ => return None,
        })
    }

    /// Built-in enum static methods: `cases()`, `from()`, `tryFrom()`.
    fn enum_builtin_static(
        &mut self,
        cls: ClassId,
        method_lc: &[u8],
        args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        // Case values may be pending constant expressions: cases() evaluates
        // just the pending case constants (php's zend_enum_cases_func), while
        // from()/tryFrom() load the whole class (building the value table).
        if method_lc == b"cases" {
            let pending: Vec<Vec<u8>> = self
                .class(cls)
                .constants
                .iter()
                .filter(|(_, c)| matches!(c.init, Some(classes::ConstInit::EnumCase(_))))
                .map(|(k, _)| k.clone())
                .collect();
            let display = self.class(cls).name.as_bytes().to_vec();
            for name in pending {
                self.class_const_value(cls, &name, &display, false)?;
            }
        } else {
            self.update_class_constants(cls)?;
        }
        self.touch_enum_cases(cls);
        let enum_name = || String::from_utf8_lossy(self.class(cls).name.as_bytes()).into_owned();
        match method_lc {
            b"cases" => {
                let mut arr = rhp_values::PhpArray::new();
                for obj in &self.class(cls).enum_cases {
                    arr.push(Value::Object(obj.clone()));
                }
                Ok(Value::Array(arr))
            }
            b"from" | b"tryfrom" => {
                let is_try = method_lc == b"tryfrom";
                let backing = self.class(cls).enum_backing.ok_or_else(|| {
                    Thrown::error(
                        "Error",
                        format!(
                            "{}::{} is not a backed enum",
                            enum_name(),
                            if is_try { "tryFrom()" } else { "from()" }
                        ),
                    )
                })?;
                let input = match args.first() {
                    Some(Value::Ref(r)) => r.borrow().clone(),
                    Some(v) => v.clone(),
                    None => Value::Null,
                };
                // The declared parameter type is int|string; other types are a TypeError.
                use rhp_bytecode::EnumBacking;
                let matches = |cv: &Value| -> bool {
                    match (backing, &input) {
                        (EnumBacking::Int, Value::Int(n)) => matches!(cv, Value::Int(m) if m == n),
                        (EnumBacking::Str, Value::Str(s)) => {
                            matches!(cv, Value::Str(cs) if cs.as_bytes() == s.as_bytes())
                        }
                        // Cross-type (e.g. int value against a string backing) never matches.
                        _ => false,
                    }
                };
                let cases = self.class(cls).enum_cases.clone();
                let value_key = rhp_values::PhpStr::from("value");
                for obj in &cases {
                    let val = obj.borrow().properties.get(&value_key).cloned();
                    if let Some(v) = val {
                        if matches(&v) {
                            return Ok(Value::Object(obj.clone()));
                        }
                    }
                }
                if is_try {
                    return Ok(Value::Null);
                }
                // from(): no match -> ValueError.
                let shown = match &input {
                    Value::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
                    other => String::from_utf8_lossy(&conv::to_string_bytes(other)).into_owned(),
                };
                Err(Thrown::error(
                    "ValueError",
                    format!(
                        "{} is not a valid backing value for enum {}",
                        shown,
                        enum_name()
                    ),
                ))
            }
            _ => unreachable!("enum_builtin_static called with {:?}", method_lc),
        }
    }

    /// Invoke a PHP callable value synchronously (string function name,
    /// `[obj_or_class, method]`, an `__invoke` object, or a closure).
    /// Instance methods of a `Closure` value: `bindTo`, `call`, `__invoke`.
    fn closure_method(
        &mut self,
        closure: &Value,
        mname: &[u8],
        mut args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        match mname.to_ascii_lowercase().as_slice() {
            b"bindto" => {
                let new_this = args.first().cloned().unwrap_or(Value::Null);
                let scope = args.get(1).cloned().unwrap_or_else(|| Value::str("static"));
                Ok(self
                    .closure_bind(closure, &new_this, &scope)
                    .unwrap_or(Value::Null))
            }
            b"call" => {
                if args.is_empty() {
                    return Err(Thrown::error(
                        "ArgumentCountError",
                        "Closure::call() expects at least 1 argument, 0 given",
                    ));
                }
                // Temporarily binds `$this` AND the scope to the new object.
                let new_this = args.remove(0);
                let bound = self
                    .closure_bind(closure, &new_this, &new_this)
                    .unwrap_or(Value::Null);
                self.call_callable_value(&bound, args)
            }
            b"__invoke" => self.call_callable_value(closure, args),
            _ => Err(Thrown::error(
                "Error",
                format!(
                    "Call to undefined method Closure::{}()",
                    String::from_utf8_lossy(mname)
                ),
            )),
        }
    }

    /// The calling scope a callable resolves against (zend_is_callable's
    /// `scope` / called scope / `$this`): the innermost frame.
    fn callable_scope(&self) -> (Option<ClassId>, Option<ClassId>, Option<ObjectHandle>) {
        if let Some((scope, called)) = self.fcc_scope.get() {
            return (scope, called, None);
        }
        // Prelude (internal) methods act for their caller, as php's internal
        // functions do: the scope is the innermost user frame's.
        let n = self.stack.len();
        let user = (0..n).rev().find(|&i| !self.is_internal_frame(i));
        match user.or(n.checked_sub(1)) {
            Some(top) => (
                self.stack[top].defining_class,
                self.stack[top].called_class,
                self.current_this(top),
            ),
            None => (None, None, None),
        }
    }

    /// The class half of a `Class::method` / `[Class, method]` callable,
    /// with `self`/`parent`/`static` relative to the calling scope. Returns
    /// (class, called class).
    fn callable_class(&self, name: &[u8]) -> Result<(ClassId, ClassId), String> {
        let (scope, called, _) = self.callable_scope();
        let lc = name.to_ascii_lowercase();
        let no_scope = |w: &str| format!("cannot access \"{w}\" when no class scope is active");
        let forward = |cls: ClassId| match called {
            Some(c) if self.class(c).ancestors.contains(&cls) => c,
            _ => cls,
        };
        match lc.as_slice() {
            b"self" => scope
                .map(|c| (c, forward(c)))
                .ok_or_else(|| no_scope("self")),
            b"static" => called
                .or(scope)
                .map(|c| (c, c))
                .ok_or_else(|| no_scope("static")),
            b"parent" => {
                let c = scope.ok_or_else(|| no_scope("parent"))?;
                let p = self.class(c).parent.ok_or_else(|| {
                    "cannot access \"parent\" when current class scope has no parent".to_string()
                })?;
                Ok((p, forward(p)))
            }
            _ => self
                .resolve_class_by_name(name)
                .map(|c| (c, c))
                .map_err(|_| {
                    let n = name.strip_prefix(b"\\").unwrap_or(name);
                    format!("class \"{}\" not found", String::from_utf8_lossy(n))
                }),
        }
    }

    /// The method half of a method callable (zend_is_callable_check_func).
    fn callable_method(
        &self,
        cls: ClassId,
        called: ClassId,
        obj: Option<ObjectHandle>,
        mname: &[u8],
    ) -> Result<Callee, String> {
        let (scope, _, this) = self.callable_scope();
        let cname = || String::from_utf8_lossy(self.class(cls).name.as_bytes()).into_owned();
        let magic = |obj: &Option<ObjectHandle>| -> Option<Callee> {
            let name = mname.to_vec();
            if let Some(o) = obj {
                if let Some(m) = self.lookup_method(cls, b"__call") {
                    return Some(Callee::Magic(m, Some(o.clone()), called, name));
                }
            }
            if let Some(m) = self.lookup_method(cls, b"__callstatic") {
                return Some(Callee::Magic(m, None, called, name));
            }
            None
        };
        // A non-static method called without an object borrows a compatible
        // `$this` from the calling scope.
        let obj =
            obj.or_else(|| this.filter(|h| self.class(h.borrow().class).ancestors.contains(&cls)));
        let Some(m) = self.lookup_method(cls, mname) else {
            if self.class(cls).kind == ClassKind::Enum {
                let lc = mname.to_ascii_lowercase();
                if matches!(lc.as_slice(), b"cases" | b"from" | b"tryfrom") {
                    return Ok(Callee::EnumStatic(cls, lc));
                }
            }
            return magic(&obj).ok_or_else(|| {
                format!(
                    "class {} does not have a method \"{}\"",
                    cname(),
                    String::from_utf8_lossy(mname)
                )
            });
        };
        if !self.check_access(scope, m.defining_class, m.visibility) {
            if let Some(c) = magic(&obj) {
                return Ok(c);
            }
            let v = match m.visibility {
                rhp_values::Visibility::Private => "private",
                rhp_values::Visibility::Protected => "protected",
                rhp_values::Visibility::Public => "public",
            };
            return Err(format!(
                "cannot access {v} method {}::{}()",
                cname(),
                String::from_utf8_lossy(m.proto.name.as_bytes())
            ));
        }
        if m.is_static {
            return Ok(Callee::Method(m, None, called));
        }
        match obj {
            Some(o) => {
                let oc = o.borrow().class;
                Ok(Callee::Method(m, Some(o), oc))
            }
            None => Err(format!(
                "non-static method {}::{}() cannot be called statically",
                cname(),
                String::from_utf8_lossy(m.proto.name.as_bytes())
            )),
        }
    }

    /// Resolve a callable value (zend_is_callable_ex), or say why it is not
    /// one in zend_is_callable_error's words.
    fn resolve_callable(&self, callable: &Value) -> Result<Callee, String> {
        match deref(callable) {
            Value::Closure(cd) => Ok(Callee::Closure(cd)),
            Value::Str(s) => {
                let b = s.as_bytes();
                // `Class::method` splits at the last "::".
                let split = (1..b.len())
                    .rev()
                    .find(|&i| b[i] == b':' && b[i - 1] == b':');
                if let Some(i) = split {
                    if i == 1 {
                        return Err("invalid function name".to_string());
                    }
                    let (cls, called) = self.callable_class(&b[..i - 1])?;
                    return self.callable_method(cls, called, None, &b[i + 1..]);
                }
                let name = b.strip_prefix(b"\\").unwrap_or(b);
                if let Some(&idx) = self.fn_names.get(&name.to_ascii_lowercase()) {
                    return Ok(Callee::Function(self.functions[idx].clone()));
                }
                let n = String::from_utf8_lossy(name).into_owned();
                // Engine-internal `__*` builtins are not user-visible functions.
                if user_visible_builtin(name) {
                    return Ok(Callee::Builtin(n));
                }
                Err(format!(
                    "function \"{}\" not found or invalid function name",
                    String::from_utf8_lossy(b)
                ))
            }
            Value::Array(a) => {
                if a.len() != 2 {
                    return Err("array callback must have exactly two members".to_string());
                }
                let item0 = a.get(&ArrayKey::Int(0)).map(deref);
                let item1 = a.get(&ArrayKey::Int(1)).map(deref);
                let (Some(item0), Some(item1)) = (item0, item1) else {
                    return Err("array callback must have exactly two members".to_string());
                };
                let (cls, called, obj) = match item0 {
                    Value::Object(h) => {
                        let c = h.borrow().class;
                        (c, c, Some(h))
                    }
                    Value::Str(cn) => {
                        let (c, called) = self.callable_class(cn.as_bytes())?;
                        (c, called, None)
                    }
                    _ => {
                        return Err(
                            "first array member is not a valid class name or object".to_string()
                        )
                    }
                };
                let Value::Str(m) = item1 else {
                    return Err("second array member is not a valid method".to_string());
                };
                self.callable_method(cls, called, obj, m.as_bytes())
            }
            Value::Object(h) => {
                let cls = h.borrow().class;
                match self.lookup_method(cls, b"__invoke") {
                    Some(m) => Ok(Callee::Method(m, Some(h), cls)),
                    None => Err("no array or string given".to_string()),
                }
            }
            _ => Err("no array or string given".to_string()),
        }
    }

    /// Autoload the class half of a callable that names an unloaded class
    /// (php's zend_is_callable looks classes up with autoloading).
    fn autoload_callable_class(&mut self, callable: &Value) -> Result<(), Thrown> {
        let class: Option<Vec<u8>> = match deref(callable) {
            Value::Str(s) => {
                let b = s.as_bytes();
                (2..b.len())
                    .rev()
                    .find(|&i| b[i] == b':' && b[i - 1] == b':')
                    .map(|i| b[..i - 1].to_vec())
            }
            Value::Array(a) if a.len() == 2 => match a.get(&ArrayKey::Int(0)).map(deref) {
                Some(Value::Str(s)) => Some(s.as_bytes().to_vec()),
                _ => None,
            },
            _ => None,
        };
        if let Some(c) = class {
            let lc = c.to_ascii_lowercase();
            if !matches!(lc.as_slice(), b"self" | b"parent" | b"static")
                && self.resolve_class_by_name(&c).is_err()
            {
                self.try_autoload(&c)?;
            }
        }
        Ok(())
    }

    /// Invoke a PHP callable value synchronously (string function name,
    /// `Class::method`, `[obj_or_class, method]`, an `__invoke` object, or a
    /// closure). A value that is not callable raises the Error php's dynamic
    /// call (`$f()`) does.
    fn call_callable_value(&mut self, callable: &Value, args: Vec<Value>) -> Result<Value, Thrown> {
        self.call_callable_value_mode(callable, args, None)
    }

    /// [`Vm::call_callable_value`] with the caller's mode for the argument
    /// checks: `Some(strict_types)` for a dynamic call from user code, `None`
    /// for a callback invoked by a builtin (always coercive, like php's
    /// internal callers, and no call site in the error).
    fn call_callable_value_mode(
        &mut self,
        callable: &Value,
        args: Vec<Value>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        let c = deref(callable);
        // First-class callable: forward to the wrapped callable.
        if let Value::Closure(cd) = &c {
            if let Some(w) = &cd.wrapped {
                let w = (**w).clone();
                // Resolve it in the scope the callable was created in.
                let saved = self
                    .fcc_scope
                    .replace(Some((cd.bound_class, cd.bound_static)));
                let r = self.call_callable_value_mode(&w, args, caller);
                self.fcc_scope.set(saved);
                return r;
            }
        }
        let fcc = self.fcc_scope.take();
        self.autoload_callable_class(&c)?;
        self.fcc_scope.set(fcc);
        let resolved = self.resolve_callable(&c);
        // A first-class callable's creation scope applies to resolving it
        // only, not to whatever it then runs.
        self.fcc_scope.set(None);
        let callee = match resolved {
            Ok(callee) => callee,
            Err(_) => return Err(self.dynamic_call_error(&c)),
        };
        match callee {
            Callee::Closure(cd) => self.run_closure_sync(&cd, args, caller),
            Callee::Function(proto) => self.run_proto_sync(proto, None, None, None, args, caller),
            Callee::Builtin(name) => self.invoke_builtin(&name, args),
            Callee::Method(m, this, called) => self.call_sync_mode(&m, this, called, args, caller),
            Callee::Magic(m, this, called, name) => {
                let mut arr = PhpArray::new();
                for a in args {
                    arr.push(a);
                }
                self.call_sync_mode(
                    &m,
                    this,
                    called,
                    vec![Value::str(name), Value::Array(arr)],
                    caller,
                )
            }
            Callee::EnumStatic(cls, lc) => self.enum_builtin_static(cls, &lc, args),
        }
    }

    /// The Error a dynamic call of a non-callable value raises
    /// (ZEND_INIT_DYNAMIC_CALL).
    fn dynamic_call_error(&self, c: &Value) -> Thrown {
        let err = |m: String| Thrown::error("Error", m);
        let method_err = |cls: ClassId, m: &[u8]| -> Thrown {
            match self.lookup_method(cls, m) {
                Some(rm) if !rm.is_static => err(format!(
                    "Non-static method {}::{}() cannot be called statically",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                    String::from_utf8_lossy(rm.proto.name.as_bytes())
                )),
                Some(rm) => {
                    let v = match rm.visibility {
                        rhp_values::Visibility::Private => "private",
                        rhp_values::Visibility::Protected => "protected",
                        rhp_values::Visibility::Public => "public",
                    };
                    let scope = match self.callable_scope().0 {
                        Some(c) => format!(
                            "scope {}",
                            String::from_utf8_lossy(self.class(c).name.as_bytes())
                        ),
                        None => "global scope".to_string(),
                    };
                    err(format!(
                        "Call to {v} method {}::{}() from {scope}",
                        String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                        String::from_utf8_lossy(rm.proto.name.as_bytes())
                    ))
                }
                None => err(format!(
                    "Call to undefined method {}::{}()",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                    String::from_utf8_lossy(m)
                )),
            }
        };
        let class_err = |n: &[u8]| {
            err(format!(
                "Class \"{}\" not found",
                String::from_utf8_lossy(n.strip_prefix(b"\\").unwrap_or(n))
            ))
        };
        match c {
            Value::Str(s) => {
                let b = s.as_bytes();
                match (1..b.len())
                    .rev()
                    .find(|&i| b[i] == b':' && b[i - 1] == b':')
                {
                    Some(i) => match self.callable_class(&b[..i - 1]) {
                        Ok((cls, _)) => method_err(cls, &b[i + 1..]),
                        Err(_) => class_err(&b[..i - 1]),
                    },
                    None => err(format!(
                        "Call to undefined function {}()",
                        String::from_utf8_lossy(b)
                    )),
                }
            }
            Value::Array(a) if a.len() == 2 => {
                let item0 = a.get(&ArrayKey::Int(0)).map(deref);
                let item1 = a.get(&ArrayKey::Int(1)).map(deref);
                match (item0, item1) {
                    (Some(Value::Object(h)), Some(Value::Str(m))) => {
                        method_err(h.borrow().class, m.as_bytes())
                    }
                    (Some(Value::Str(cn)), Some(Value::Str(m))) => {
                        match self.callable_class(cn.as_bytes()) {
                            Ok((cls, _)) => method_err(cls, m.as_bytes()),
                            Err(_) => class_err(cn.as_bytes()),
                        }
                    }
                    (Some(Value::Object(_) | Value::Str(_)), Some(_)) => {
                        err("Second array member is not a valid method".to_string())
                    }
                    (Some(_), Some(_)) => {
                        err("First array member is not a valid class name or object".to_string())
                    }
                    _ => err("Array callback must have exactly two elements".to_string()),
                }
            }
            Value::Array(_) => err("Array callback must have exactly two elements".to_string()),
            Value::Object(_) => err(format!(
                "Object of type {} is not callable",
                self.debug_class(c)
            )),
            v => err(format!(
                "Value of type {} is not callable",
                builtins::php_fns::zend_type_name(v)
            )),
        }
    }

    fn debug_class(&self, v: &Value) -> String {
        match v {
            Value::Object(h) => {
                String::from_utf8_lossy(self.class(h.borrow().class).name.as_bytes()).into_owned()
            }
            _ => String::new(),
        }
    }

    /// Stringify a value, invoking `__toString` on objects that define it.
    // Takes `&mut self` because stringifying an object may run its `__toString`.
    #[allow(clippy::wrong_self_convention)]
    fn to_string_vm(&mut self, v: &Value) -> Result<Vec<u8>, Thrown> {
        let d = match v {
            Value::Ref(r) => r.borrow().clone(),
            other => other.clone(),
        };
        if let Value::Object(h) = &d {
            let cls = h.borrow().class;
            if let Some(m) = self.lookup_method(cls, b"__tostring") {
                let r = self.call_sync(&m, Some(h.clone()), cls, Vec::new())?;
                return Ok(conv::to_string_bytes(&r));
            }
            return Err(Thrown::error(
                "Error",
                format!(
                    "Object of class {} could not be converted to string",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes())
                ),
            ));
        }
        if let Value::Array(_) = d {
            self.diag(2, "Array to string conversion")?;
        }
        Ok(conv::to_string_bytes(&d))
    }

    fn thrown_class_id(&self, thrown: &Thrown) -> Option<ClassId> {
        match thrown {
            Thrown::Object(h) => Some(h.borrow().class),
            Thrown::Error { class, .. } => self.resolve_class_by_name(class.as_bytes()).ok(),
            _ => None,
        }
    }

    fn class_matches(&self, cls: ClassId, catch_name: &[u8]) -> bool {
        match self.resolve_class_by_name(catch_name) {
            Ok(target) => self.class(cls).ancestors.contains(&target),
            Err(_) => false,
        }
    }

    /// Materialize the thrown value as a `Throwable` object (instantiating the
    /// named class for an engine `Error`).
    fn thrown_to_object(&mut self, thrown: &Thrown) -> Result<Value, Thrown> {
        match thrown {
            Thrown::Object(h) => Ok(Value::Object(h.clone())),
            Thrown::Error { class, message } => {
                let cls = self.resolve_class_by_name(class.as_bytes())?;
                self.update_class_constants(cls)?;
                let obj = self.new_object(cls);
                obj.borrow_mut().properties.insert(
                    rhp_values::PhpStr::from("message"),
                    Value::str(message.clone()),
                );
                Ok(Value::Object(obj))
            }
            _ => Err(thrown.clone()),
        }
    }

    /// Pop the current frame and deliver `value`. Returns `Some(value)` when the
    /// outermost frame returns (program done), else delivers to the caller.
    fn do_return(&mut self, value: Value) -> Option<Value> {
        let frame = self.stack.pop().expect("non-empty stack");
        if self
            .ctor_frames
            .last()
            .is_some_and(|c| c.0 == self.stack.len())
        {
            self.ctor_frames.pop();
        }
        if !self.destructibles.is_empty() {
            self.dtor_check = true;
        }
        // A synchronous (re-entrant) call returns its value to the Rust caller.
        if frame.capture {
            return Some(value);
        }
        match self.stack.len().checked_sub(1) {
            Some(caller) => {
                // Reg::MAX means "discard" (constructor / __clone return value).
                if frame.ret_dst != Reg::MAX {
                    self.reg_set(caller, frame.ret_dst, value);
                }
                None
            }
            None => {
                // {main} finished: keep its globals for the shutdown sequence.
                self.finished_main = Some(frame);
                Some(value)
            }
        }
    }

    /// Enter a user function: bind args to the callee's CV registers and push.
    fn enter_user_call(
        &mut self,
        top: usize,
        callee: Rc<FunctionProto>,
        dst: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        // Raw (non-deref) copy: by-ref args arrive as `Value::Ref` and must be
        // kept as references; by-value binding derefs in `bind_params`.
        let args: Vec<Value> = (0..argc)
            .map(|i| self.stack[top].regs[(base + i) as usize].clone())
            .collect();
        self.push_user_frame(callee, args, dst)
    }

    /// Push a frame for a user function with pre-collected args.
    fn push_user_frame(
        &mut self,
        callee: Rc<FunctionProto>,
        args: Vec<Value>,
        dst: Reg,
    ) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let mut frame = Frame::new(callee.clone(), dst);
        self.bind_frame(&mut frame, &args, Some(self.caller_strict()))?;
        self.bind_superglobals(&mut frame);
        if callee.is_generator {
            let gen = self.make_generator(frame);
            let caller = self.stack.len() - 1;
            if dst != Reg::MAX {
                self.reg_set(caller, dst, gen);
            }
            return Ok(());
        }
        self.stack.push(frame);
        Ok(())
    }

    /// Split a call's argument array (a spread / named-argument call) into
    /// positional arguments and named ones (string keys).
    fn split_call_args(&self, arr: &Value) -> Result<(Vec<Value>, NamedArgs), Thrown> {
        let mut pos = Vec::new();
        let mut named: NamedArgs = Vec::new();
        if let Value::Array(a) = deref(arr) {
            for (k, v) in a.iter() {
                match k {
                    ArrayKey::Int(_) => {
                        if !named.is_empty() {
                            return Err(Thrown::error(
                                "Error",
                                "Cannot use positional argument after named argument during unpacking",
                            ));
                        }
                        pos.push(v.clone());
                    }
                    ArrayKey::Str(s) => named.push((s.clone(), v.clone())),
                }
            }
        }
        Ok((pos, named))
    }

    /// php's `zend_handle_named_arg` + `zend_handle_undef_args`: place named
    /// arguments into `proto`'s parameter slots. Skipped parameters take
    /// their default (a constant one directly; a non-constant one through the
    /// prologue, via the hole mask); unknown names go to a variadic
    /// parameter. Arms `pending_named` for the frame push that must follow.
    fn place_named_args(
        &mut self,
        proto: &FunctionProto,
        class: Option<ClassId>,
        mut args: Vec<Value>,
        named: Vec<(PhpStr, Value)>,
    ) -> Result<Vec<Value>, Thrown> {
        if named.is_empty() {
            return Ok(args);
        }
        let positional = args.len();
        let variadic = proto.params.iter().position(|p| p.variadic);
        let mut slots: Vec<Option<Value>> = args.drain(..).map(Some).collect();
        let mut extra: Vec<(PhpStr, Value)> = Vec::new();
        for (name, v) in named {
            let idx = proto.params.iter().position(|p| {
                !p.variadic && proto.names[p.name as usize].as_bytes() == name.as_bytes()
            });
            match idx {
                Some(i) => {
                    if i < slots.len() && slots[i].is_some() {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Named parameter ${} overwrites previous argument",
                                String::from_utf8_lossy(name.as_bytes())
                            ),
                        ));
                    }
                    if slots.len() <= i {
                        slots.resize(i + 1, None);
                    }
                    slots[i] = Some(v);
                }
                None if variadic.is_some() => {
                    if extra.iter().any(|(k, _)| k.as_bytes() == name.as_bytes()) {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Named parameter ${} overwrites previous argument",
                                String::from_utf8_lossy(name.as_bytes())
                            ),
                        ));
                    }
                    extra.push((name, v));
                }
                None => {
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Unknown named parameter ${}",
                            String::from_utf8_lossy(name.as_bytes())
                        ),
                    ))
                }
            }
        }
        let mut holes = 0u64;
        let mut out = Vec::with_capacity(slots.len());
        for (i, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(v) => out.push(v),
                None if i < positional => unreachable!("positional slots are filled"),
                None => {
                    let p = &proto.params[i];
                    match &p.default {
                        Some(ParamDefault::Const(k)) => out.push(proto.consts[*k as usize].clone()),
                        Some(ParamDefault::Init(_)) => out.push(Value::Null),
                        None => {
                            let fname = match class {
                                Some(c) => format!(
                                    "{}::{}",
                                    String::from_utf8_lossy(self.class(c).name.as_bytes()),
                                    String::from_utf8_lossy(proto.name.as_bytes())
                                ),
                                None => String::from_utf8_lossy(proto.name.as_bytes()).into_owned(),
                            };
                            return Err(Thrown::error(
                                "ArgumentCountError",
                                format!(
                                    "{}(): Argument #{} (${}) not passed",
                                    fname,
                                    i + 1,
                                    String::from_utf8_lossy(
                                        proto.names[p.name as usize].as_bytes()
                                    )
                                ),
                            ));
                        }
                    }
                    if i < 64 {
                        holes |= 1u64 << i;
                    }
                }
            }
        }
        self.pending_named = Some((holes, extra));
        Ok(out)
    }

    /// Call a callable value with positional and named arguments.
    fn call_named(
        &mut self,
        c: &Value,
        mut args: Vec<Value>,
        named: Vec<(PhpStr, Value)>,
        caller: Option<bool>,
    ) -> Result<Value, Thrown> {
        if !named.is_empty() {
            self.autoload_callable_class(c)?;
            let Some((p, cls)) = self.callable_proto(c) else {
                return Err(Thrown::unsupported(
                    "named arguments to a builtin function",
                    "deferred",
                ));
            };
            args = self.place_named_args(&p, cls, args, named)?;
        }
        self.call_callable_value_mode(c, args, caller)
    }

    /// The user-code proto a callable value would run (for named-argument
    /// placement before a dynamic call), with its scope class.
    fn callable_proto(&self, c: &Value) -> Option<(Rc<FunctionProto>, Option<ClassId>)> {
        if let Value::Closure(cd) = deref(c) {
            return match &cd.wrapped {
                Some(w) => {
                    // A first-class callable resolves in its creation scope.
                    let saved = self
                        .fcc_scope
                        .replace(Some((cd.bound_class, cd.bound_static)));
                    let r = self.callable_proto(w);
                    self.fcc_scope.set(saved);
                    r
                }
                None => Some((self.closures[cd.fnidx as usize].clone(), None)),
            };
        }
        match self.resolve_callable(c).ok()? {
            Callee::Function(proto) => Some((proto, None)),
            Callee::Method(m, _, _) => Some((m.proto.clone(), Some(m.defining_class))),
            _ => None,
        }
    }

    /// Binary op that may throw (arithmetic).
    fn bin(
        &mut self,
        top: usize,
        dst: Reg,
        a: Reg,
        b: Reg,
        f: fn(&Value, &Value) -> Result<Value, Thrown>,
    ) -> Result<(), Thrown> {
        let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
        let v = f(&x, &y)?;
        self.reg_set(top, dst, v);
        Ok(())
    }

    /// Infallible binary op (bitwise/concat).
    fn bin_inf(&mut self, top: usize, dst: Reg, a: Reg, b: Reg, f: fn(&Value, &Value) -> Value) {
        let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
        let v = f(&x, &y);
        self.reg_set(top, dst, v);
    }

    fn call_builtin(
        &mut self,
        top: usize,
        name_lc: &str,
        builtin: &'static builtins::Builtin,
        base: Reg,
        argc: u16,
        frameless: bool,
    ) -> Result<Value, Thrown> {
        // By-ref builtins (sort/array_push/preg_match/…) need their by-reference
        // positions kept as the `Value::Ref` the call site built, so they can
        // write back to the caller's variable. Other args deref to plain values.
        let ref_mask = rhp_bytecode::builtin_ref_params(name_lc);
        let args: Vec<Value> = (0..argc)
            .map(|i| {
                if ref_mask.get(i as usize).copied().unwrap_or(false) {
                    self.stack[top].regs[(base + i) as usize].clone()
                } else {
                    self.reg_get(top, base + i)
                }
            })
            .collect();
        self.run_builtin(builtin, args, frameless)
    }

    /// Call a builtin by name with a pre-collected argument vector.
    fn invoke_builtin(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Thrown> {
        self.invoke_builtin_ex(name, args, false)
    }

    /// Call a builtin. Unless `frameless` (php compiled the call to an opcode),
    /// the call is tracked as a native frame while it runs, and an engine error
    /// it raises is materialized with that frame on the trace.
    fn invoke_builtin_ex(
        &mut self,
        name: &str,
        args: Vec<Value>,
        frameless: bool,
    ) -> Result<Value, Thrown> {
        let Some(builtin) = builtins::lookup(name) else {
            return Err(Thrown::error(
                "Error",
                format!("Call to undefined function {name}()"),
            ));
        };
        self.run_builtin(builtin, args, frameless)
    }

    /// [`Vm::invoke_builtin_ex`] for an already looked-up builtin.
    /// zpp's checks on a builtin's scalar / array typed parameters (from
    /// php's stubs): a `Stringable` object passed to a `string` parameter is
    /// converted with `__toString()`; other objects, and arrays where no
    /// array is accepted, are TypeErrors; null to a non-nullable scalar is
    /// deprecated (to an array, a TypeError). Builtins that stringify the
    /// elements of a `mixed` / array argument (`implode`, `sprintf`, …) get
    /// those converted too. `user`: called from user code (engine-internal
    /// prelude calls only get the conversions).
    fn coerce_builtin_args(
        &mut self,
        name: &str,
        args: &mut [Value],
        user: bool,
    ) -> Result<(), Thrown> {
        let params = builtins::stub_params::params(name);
        for &(pos, pname, ty, variadic, nullable, has_str, has_arr, by_ref) in params {
            let range = if variadic {
                pos as usize..args.len()
            } else {
                pos as usize..(pos as usize + 1).min(args.len())
            };
            if by_ref {
                // `array &$array` (sort, usort, array_push, …): the referenced
                // value must already be an array.
                if user {
                    for i in range {
                        let cur = deref(&args[i]);
                        if !matches!(cur, Value::Array(_)) {
                            return Err(Thrown::type_error(format!(
                                "{name}(): Argument #{} (${pname}) must be of type {ty}, {} given",
                                i + 1,
                                self.zval_value_name(&cur)
                            )));
                        }
                    }
                }
                continue;
            }
            for i in range {
                let given: Vec<u8> = match &args[i] {
                    Value::Object(h) => {
                        let cls = h.borrow().class;
                        if has_str && self.lookup_method(cls, b"__tostring").is_some() {
                            let s = self.to_string_vm(&args[i])?;
                            args[i] = Value::str(s);
                            continue;
                        }
                        self.class(cls).name.as_bytes().to_vec()
                    }
                    Value::Closure(_) => b"Closure".to_vec(),
                    Value::Array(_) if !has_arr && user => b"array".to_vec(),
                    Value::Null if !nullable && user => {
                        if has_str || ty != "array" {
                            let msg = format!(
                                "{name}(): Passing null to parameter #{} (${pname}) of type {ty} is deprecated",
                                i + 1
                            );
                            self.diag(8192, &msg)?;
                            continue;
                        }
                        b"null".to_vec()
                    }
                    Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_)
                        if (ty == "array" || ty == "?array") && user =>
                    {
                        type_lc(&args[i]).as_bytes().to_vec()
                    }
                    _ => continue,
                };
                return Err(Thrown::type_error(format!(
                    "{name}(): Argument #{} (${pname}) must be of type {ty}, {} given",
                    i + 1,
                    String::from_utf8_lossy(&given)
                )));
            }
        }
        // Elements stringified inside the builtin (zval_get_string).
        let elems: Option<(usize, bool)> = match name {
            "implode" | "join" => Some((0, true)),
            "sprintf" | "printf" => Some((1, false)),
            "fprintf" => Some((2, false)),
            "vsprintf" | "vprintf" => Some((1, true)),
            "vfprintf" => Some((2, true)),
            _ => None,
        };
        if let Some((from, arrays)) = elems {
            for arg in args.iter_mut().skip(from) {
                if arrays {
                    let Value::Array(a) = &*arg else { continue };
                    let hits: Vec<ArrayKey> = a
                        .iter()
                        .filter(|(_, v)| self.is_stringable(&deref(v)))
                        .map(|(k, _)| k.clone())
                        .collect();
                    if hits.is_empty() {
                        continue;
                    }
                    let mut a = a.clone();
                    for k in hits {
                        let v = deref(a.get(&k).unwrap_or(&Value::Null));
                        let s = self.to_string_vm(&v)?;
                        a.insert(k, Value::str(s));
                    }
                    *arg = Value::Array(a);
                } else if self.is_stringable(arg) {
                    let s = self.to_string_vm(arg)?;
                    *arg = Value::str(s);
                }
            }
        }
        Ok(())
    }

    /// An object with `__toString()`.
    fn is_stringable(&self, v: &Value) -> bool {
        match v {
            Value::Object(h) => self
                .lookup_method(h.borrow().class, b"__tostring")
                .is_some(),
            _ => false,
        }
    }

    fn run_builtin(
        &mut self,
        builtin: &'static builtins::Builtin,
        mut args: Vec<Value>,
        frameless: bool,
    ) -> Result<Value, Thrown> {
        // Free streams whose last reference went away (php closes a resource
        // when its refcount drops to zero).
        self.reap_dropped_streams();
        let count_error = builtin_arg_count_error(builtin, args.len());
        if count_error.is_none() {
            let caller = self.stack.len().saturating_sub(1);
            let user = !self.stack.is_empty() && !self.is_internal_frame(caller);
            self.coerce_builtin_args(builtin.name, &mut args, user)?;
        }
        // A builtin called from engine-internal (prelude) code is not a php
        // frame, and php compiles a few calls to opcodes (no frame either).
        let caller = self.stack.len().saturating_sub(1);
        if frameless || self.stack.is_empty() || self.is_internal_frame(caller) {
            let r = match count_error {
                Some(e) => Err(e),
                None => (builtin.func)(self, &args),
            };
            return match self.pending_throw.take() {
                Some(t) => Err(t),
                None => r,
            };
        }
        self.native_calls.push(NativeCall {
            name: builtin.name,
            args: args.as_slice() as *const [Value],
            caller,
        });
        let r = match count_error {
            Some(e) => Err(e),
            None => (builtin.func)(self, &args),
        };
        let r = match r {
            Err(e @ Thrown::Error { .. }) => Err(self.materialize(e, None)),
            other => other,
        };
        // An exception a user error handler threw during the call wins.
        let r = match self.pending_throw.take() {
            Some(t) => Err(t),
            None => r,
        };
        self.native_calls.pop();
        // `args` outlives the entry just popped (see `NativeCall::args`).
        drop(args);
        r
    }
}

/// php 8's `@` lowers error_reporting to these fatal types only
/// (E_ERROR | E_CORE_ERROR | E_COMPILE_ERROR | E_USER_ERROR |
/// E_RECOVERABLE_ERROR | E_PARSE).
pub(crate) const SILENCE_KEEPS: i64 = 4437;

/// "Call to undefined function NAME()" with the name as written at the call
/// site (namespaced for an unqualified call inside a namespace).
fn undefined_function(name: &rhp_values::PhpStr) -> Thrown {
    Thrown::error(
        "Error",
        format!(
            "Call to undefined function {}()",
            String::from_utf8_lossy(name.as_bytes())
        ),
    )
}

/// zend_wrong_parameters_count_error: "f() expects exactly 1 argument, 0 given".
fn builtin_arg_count_error(b: &builtins::Builtin, n: usize) -> Option<Thrown> {
    let max = b.max_args.unwrap_or(usize::MAX);
    if n >= b.min_args && n <= max {
        return None;
    }
    let (which, bound) = if b.min_args == max {
        ("exactly", b.min_args)
    } else if n < b.min_args {
        ("at least", b.min_args)
    } else {
        ("at most", max)
    };
    Some(Thrown::error(
        "ArgumentCountError",
        format!(
            "{}() expects {which} {bound} argument{}, {n} given",
            b.name,
            if bound == 1 { "" } else { "s" }
        ),
    ))
}

/// Calls php compiles to dedicated opcodes instead of a function call
/// (zend_try_compile_special_func): they never appear as a trace frame, and a
/// callback they invoke is called from the caller's own line.
fn compiled_inline(lcname: &[u8], argc: usize) -> bool {
    match lcname {
        b"strlen" | b"is_null" | b"is_bool" | b"is_long" | b"is_int" | b"is_integer"
        | b"is_float" | b"is_double" | b"is_string" | b"is_array" | b"is_object"
        | b"is_resource" | b"is_scalar" | b"boolval" | b"intval" | b"floatval" | b"doubleval"
        | b"strval" | b"gettype" | b"count" | b"sizeof" => argc == 1,
        b"get_class" => argc <= 1,
        b"get_called_class" | b"func_num_args" | b"func_get_args" => argc == 0,
        b"array_key_exists" | b"call_user_func_array" => argc == 2,
        b"call_user_func" => argc >= 1,
        _ => false,
    }
}

/// Describe the unmatched subject for an UnhandledMatchError message.
fn match_case_desc(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Str(_) => "of type string".to_string(),
        Value::Float(_) => "of type double".to_string(),
        Value::Null => "NULL".to_string(),
        other => format!("of type {}", other.type_name()),
    }
}

// --- array element read + lvalue path operations ---------------------------

/// The word php uses in "Cannot instantiate {abstract class|interface|enum} X".
fn kind_word(kind: ClassKind) -> &'static str {
    match kind {
        ClassKind::Interface => "interface",
        ClassKind::Trait => "trait",
        ClassKind::Enum => "enum",
        ClassKind::Class => "abstract class",
    }
}

/// Whether every member of a type is a built-in (non-class) type.
fn is_builtin_type(ty: &rhp_bytecode::TypeHint) -> bool {
    use rhp_bytecode::TypeKind as K;
    ty.kinds
        .iter()
        .all(|k| !matches!(k, K::Named(_) | K::SelfType | K::StaticType | K::Parent))
}

/// php's `zend_verify_weak_scalar_type_hint`: coerce a scalar to the first of
/// int, float, string, bool (in that preference order) the type accepts.
/// `None` if no member accepts it. (The E_DEPRECATED for a fractional float
/// to int is not emitted.)
fn weak_scalar_coerce(d: &Value, ty: &rhp_bytecode::TypeHint) -> Option<Value> {
    use rhp_bytecode::TypeKind as K;
    use rhp_values::conv::Num;
    let has = |want: fn(&K) -> bool| ty.kinds.iter().any(want);
    let (t_int, t_float, t_str) = (
        has(|k| matches!(k, K::Int)),
        has(|k| matches!(k, K::Float)),
        has(|k| matches!(k, K::String)),
    );
    let t_bool = has(|k| matches!(k, K::Bool))
        || (has(|k| matches!(k, K::True)) && has(|k| matches!(k, K::False)));
    // ZEND_DOUBLE_FITS_LONG + NaN check, then truncation.
    let f2i = |f: f64| -> Option<i64> {
        if f.is_nan() || !(-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f) {
            None
        } else {
            Some(f as i64)
        }
    };
    if t_int {
        match d {
            // int|float and a string: is_numeric_string decides.
            Value::Str(s) if t_float => match conv::numeric_string(s.as_bytes()) {
                Some(Num::Int(n)) => return Some(Value::Int(n)),
                Some(Num::Float(f)) => return Some(Value::Float(f)),
                None => {}
            },
            Value::Float(f) => {
                if let Some(n) = f2i(*f) {
                    return Some(Value::Int(n));
                }
            }
            Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
                Some(Num::Int(n)) => return Some(Value::Int(n)),
                Some(Num::Float(f)) => {
                    if let Some(n) = f2i(f) {
                        return Some(Value::Int(n));
                    }
                }
                None => {}
            },
            Value::Bool(b) => return Some(Value::Int(*b as i64)),
            _ => {}
        }
    }
    if t_float {
        match d {
            Value::Int(n) => return Some(Value::Float(*n as f64)),
            Value::Bool(b) => return Some(Value::Float(*b as i64 as f64)),
            Value::Str(s) => {
                if let Some(num) = conv::numeric_string(s.as_bytes()) {
                    return Some(Value::Float(num.as_f64()));
                }
            }
            _ => {}
        }
    }
    if t_str && matches!(d, Value::Int(_) | Value::Float(_) | Value::Bool(_)) {
        return Some(Value::str(conv::to_string_bytes(d)));
    }
    if t_bool && matches!(d, Value::Int(_) | Value::Float(_) | Value::Str(_)) {
        return Some(Value::Bool(conv::to_bool(d)));
    }
    None
}

/// Human-readable form of a declared type, for TypeError messages.
fn type_string(ty: &rhp_bytecode::TypeHint) -> String {
    // php's zend_type_to_string: class names first (declaration order), then
    // the builtin types in a fixed order, `null` last (or `?T` for a single
    // type). `iterable` is `Traversable|array`.
    use rhp_bytecode::TypeKind as K;
    if ty.intersection {
        let parts: Vec<String> = ty
            .kinds
            .iter()
            .filter_map(|k| match k {
                K::Named(n) => Some(String::from_utf8_lossy(n.as_bytes()).into_owned()),
                _ => None,
            })
            .collect();
        return parts.join("&");
    }
    let has = |want: &dyn Fn(&K) -> bool| ty.kinds.iter().any(want);
    let mut parts: Vec<String> = Vec::new();
    for k in &ty.kinds {
        match k {
            K::Named(n) => parts.push(String::from_utf8_lossy(n.as_bytes()).into_owned()),
            K::SelfType => parts.push("self".into()),
            K::Parent => parts.push("parent".into()),
            _ => {}
        }
    }
    if has(&|k| matches!(k, K::Iterable)) && !parts.iter().any(|p| p == "Traversable") {
        parts.push("Traversable".into());
    }
    if has(&|k| matches!(k, K::Mixed)) {
        parts.push("mixed".into());
        return parts.join("|");
    }
    type Pred = fn(&K) -> bool;
    let order: [(Pred, &str); 7] = [
        (|k| matches!(k, K::StaticType), "static"),
        (|k| matches!(k, K::Callable), "callable"),
        (|k| matches!(k, K::Object), "object"),
        (|k| matches!(k, K::Array | K::Iterable), "array"),
        (|k| matches!(k, K::String), "string"),
        (|k| matches!(k, K::Int), "int"),
        (|k| matches!(k, K::Float), "float"),
    ];
    for (pred, name) in order {
        if has(&pred) {
            parts.push(name.into());
        }
    }
    let t = has(&|k| matches!(k, K::True));
    let f = has(&|k| matches!(k, K::False));
    if has(&|k| matches!(k, K::Bool)) || (t && f) {
        parts.push("bool".into());
    } else if f {
        parts.push("false".into());
    } else if t {
        parts.push("true".into());
    }
    if has(&|k| matches!(k, K::Void)) {
        parts.push("void".into());
    }
    if has(&|k| matches!(k, K::Never)) {
        parts.push("never".into());
    }
    if ty.nullable || has(&|k| matches!(k, K::Null)) {
        if parts.len() == 1 {
            return format!("?{}", parts[0]);
        }
        parts.push("null".into());
    }
    parts.join("|")
}

/// The resource id of a (possibly referenced) resource value.
fn res_id(v: &Value) -> Option<u32> {
    match deref(v) {
        Value::Resource(r) => Some(r.id),
        _ => None,
    }
}

/// Normalize a schemed URL (`scheme://…`) by collapsing `.`/`..`/empty path
/// segments after the scheme, preserving the scheme and any leading `/` of the
/// remainder. Used as the canonical `__FILE__`/`_once` key for phar entries.
fn normalize_url(url: &[u8]) -> Vec<u8> {
    let Some(pos) = url.windows(3).position(|w| w == b"://") else {
        return url.to_vec();
    };
    let (pre, rest) = url.split_at(pos + 3);
    let leading_slash = rest.first() == Some(&b'/');
    let mut segs: Vec<&[u8]> = Vec::new();
    for s in rest.split(|&b| b == b'/') {
        match s {
            b"" | b"." => {}
            b".." => {
                segs.pop();
            }
            x => segs.push(x),
        }
    }
    let mut out = pre.to_vec();
    if leading_slash {
        out.push(b'/');
    }
    out.extend_from_slice(&segs.join(&b"/"[..]));
    out
}

/// No instruction rewriting — the bases are applied at run time.
/// Clone a proto, stamping its `closure_base` so its file-local `MakeClosure`
/// indices resolve into the VM's flat closure table (whose tail holds this
/// file's closures), and its `class_base` likewise for `DeclareClass`.
fn rebase_proto_c(proto: &FunctionProto, base: u32, cbase: u32) -> FunctionProto {
    let mut p = proto.clone();
    p.closure_base = base;
    p.class_base = cbase;
    p
}

/// Stamp `closure_base` / `class_base` on a class's method and
/// initializer-thunk protos (their bodies can create closures or declare
/// classes) when merging an included program.
pub(crate) fn rebase_class(c: &ClassProto, base: u32, cbase: u32) -> ClassProto {
    let mut c = c.clone();
    let rb = |p: &FunctionProto| Rc::new(rebase_proto_c(p, base, cbase));
    for m in &mut c.methods {
        m.proto = rb(&m.proto);
    }
    for p in &mut c.props {
        if let PropDefaultProto::Init(t) = &p.default {
            p.default = PropDefaultProto::Init(rb(t));
        }
    }
    for sp in &mut c.static_props {
        if let ConstInitProto::Thunk(t) = &sp.init {
            sp.init = ConstInitProto::Thunk(rb(t));
        }
    }
    for k in &mut c.constants {
        if let ConstInitProto::Thunk(t) = &k.value {
            k.value = ConstInitProto::Thunk(rb(t));
        }
    }
    for ec in &mut c.enum_cases {
        if let Some(ConstInitProto::Thunk(t)) = &ec.value {
            ec.value = Some(ConstInitProto::Thunk(rb(t)));
        }
    }
    c
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

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        other => other.clone(),
    }
}

/// `$container[$key]` read: array element (Null if absent) or string byte.
/// `isset($container[$key])` for arrays and strings (php's
/// `zend_isset_dim_slow` string-offset rules).
fn isset_dim(container: &Value, key: &Value) -> bool {
    match deref(container) {
        Value::Array(_) => !matches!(dim_fetch(container, key), Value::Null),
        Value::Str(s) => {
            let k = deref(key);
            let idx = match &k {
                Value::Int(n) => *n,
                Value::Null | Value::Bool(_) | Value::Float(_) => conv::to_int(&k),
                Value::Str(ks) => match conv::numeric_string(ks.as_bytes()) {
                    Some(conv::Num::Int(n)) => n,
                    _ => return false,
                },
                _ => return false,
            };
            let len = s.as_bytes().len() as i64;
            let i = if idx < 0 { idx + len } else { idx };
            i >= 0 && i < len
        }
        _ => false,
    }
}

/// A public readonly property without an explicit set visibility (php 8.4
/// makes it protected(set)).
fn pe_public_implicit_set(vm: &Vm, cls: ClassId, key: &PhpStr) -> bool {
    vm.class(cls).props.get(key).is_some_and(|pe| {
        pe.readonly
            && pe.visibility == rhp_values::Visibility::Public
            && pe.set_visibility.is_none()
    })
}

/// The fetch kind of a dimension read: php's `BP_VAR_R`, `BP_VAR_IS`
/// (isset / `??`) and `ZEND_FETCH_LIST_R`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DimMode {
    Read,
    Is,
    List,
}

impl Vm {
    /// Raise a diagnostic from inside an instruction; an exception thrown by
    /// the user error handler is returned to be thrown at this point.
    pub(crate) fn diag(&mut self, level: i64, msg: &str) -> Result<(), Thrown> {
        self.raise_error(level, msg);
        match self.pending_throw.take() {
            Some(t) => Err(t),
            None => Ok(()),
        }
    }

    /// php's array-offset key conversion for a read (`zend_fetch_dimension_
    /// address_inner` / `slow_index_convert`), with its diagnostics. `None`:
    /// an illegal offset type (TypeError raised by the caller).
    fn dim_key(&mut self, key: &Value, quiet: bool) -> Result<Option<ArrayKey>, Thrown> {
        match key {
            Value::Null if !quiet => self.diag(
                8192,
                "Using null as an array offset is deprecated, use an empty string instead",
            )?,
            Value::Float(f) if !quiet && (!f.is_finite() || f.fract() != 0.0) => {
                let shown =
                    String::from_utf8_lossy(&rhp_values::fmt::float_serialize(*f)).into_owned();
                self.diag(
                    8192,
                    &format!("Implicit conversion from float {shown} to int loses precision"),
                )?
            }
            Value::Resource(r) if !quiet => self.diag(
                2,
                &format!(
                    "Resource ID#{} used as offset, casting to integer ({})",
                    r.id, r.id
                ),
            )?,
            _ => {}
        }
        Ok(ArrayKey::from_value(key))
    }

    /// A dimension read `$c[$k]` with php's diagnostics: undefined keys,
    /// string offsets, scalar containers and ArrayAccess dispatch.
    fn dim_read(&mut self, container: &Value, key: &Value, mode: DimMode) -> Result<Value, Thrown> {
        let quiet = mode == DimMode::Is;
        match container {
            Value::Array(a) => {
                let Some(k) = self.dim_key(key, quiet)? else {
                    return Err(Thrown::type_error(format!(
                        "Cannot access offset of type {} on array",
                        self.offset_type_name(key)
                    )));
                };
                match a.get(&k) {
                    Some(v) => Ok(deref(v)),
                    None => {
                        if !quiet {
                            let msg = match &k {
                                ArrayKey::Int(n) => format!("Undefined array key {n}"),
                                ArrayKey::Str(s) => format!(
                                    "Undefined array key \"{}\"",
                                    String::from_utf8_lossy(s.as_bytes())
                                ),
                            };
                            self.diag(2, &msg)?;
                        }
                        Ok(Value::Null)
                    }
                }
            }
            Value::Object(h) => {
                let cls = h.borrow().class;
                if self.class_implements(cls, b"ArrayAccess") {
                    if quiet {
                        // zend_std_read_dimension(BP_VAR_IS): offsetExists first.
                        let ex = self.lookup_method(cls, b"offsetexists").unwrap();
                        let r = self.call_sync(&ex, Some(h.clone()), cls, vec![key.clone()])?;
                        if !conv::to_bool(&r) {
                            return Ok(Value::Null);
                        }
                    }
                    let m = self.lookup_method(cls, b"offsetget").unwrap();
                    return self.call_sync(&m, Some(h.clone()), cls, vec![key.clone()]);
                }
                Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot use object of type {} as array",
                        String::from_utf8_lossy(self.class(cls).name.as_bytes())
                    ),
                ))
            }
            Value::Closure(_) => Err(Thrown::error(
                "Error",
                "Cannot use object of type Closure as array",
            )),
            Value::Str(s) if mode != DimMode::List => {
                let bytes = s.as_bytes();
                let offset = match key {
                    Value::Int(n) => *n,
                    Value::Str(ks) => {
                        let kb = ks.as_bytes();
                        match conv::scan_number(kb) {
                            Some((conv::Num::Int(n), end)) => {
                                let trailing =
                                    !kb[end..].iter().all(|b| b" \t\n\r\x0b\x0c".contains(b));
                                if trailing && !quiet {
                                    self.diag(
                                        2,
                                        &format!(
                                            "Illegal string offset \"{}\"",
                                            String::from_utf8_lossy(kb)
                                        ),
                                    )?;
                                }
                                n
                            }
                            _ => {
                                if quiet {
                                    return Ok(Value::Null);
                                }
                                return Err(Thrown::type_error(
                                    "Cannot access offset of type string on string",
                                ));
                            }
                        }
                    }
                    Value::Float(_) | Value::Null | Value::Bool(_) => {
                        if !quiet {
                            self.diag(2, "String offset cast occurred")?;
                        }
                        conv::to_int(key)
                    }
                    _ => {
                        if quiet {
                            return Ok(Value::Null);
                        }
                        return Err(Thrown::type_error(format!(
                            "Cannot access offset of type {} on string",
                            self.offset_type_name(key)
                        )));
                    }
                };
                let len = bytes.len() as i64;
                let i = if offset < 0 { len + offset } else { offset };
                if i >= 0 && i < len {
                    Ok(Value::str(vec![bytes[i as usize]]))
                } else if quiet {
                    Ok(Value::Null)
                } else {
                    self.diag(2, &format!("Uninitialized string offset {offset}"))?;
                    Ok(Value::str(""))
                }
            }
            Value::Null => Ok(Value::Null).and_then(|v| {
                if mode == DimMode::Read {
                    self.diag(2, "Trying to access array offset on null")?;
                }
                Ok(v)
            }),
            other => {
                if !quiet {
                    let ty = match other {
                        Value::Bool(true) => "true",
                        Value::Bool(false) => "false",
                        Value::Int(_) => "int",
                        Value::Float(_) => "float",
                        Value::Str(_) => "string",
                        Value::Resource(_) => "resource",
                        _ => "null",
                    };
                    let msg = if mode == DimMode::List {
                        format!("Cannot use {ty} as array")
                    } else {
                        format!("Trying to access array offset on {ty}")
                    };
                    self.diag(2, &msg)?;
                }
                Ok(Value::Null)
            }
        }
    }

    /// zend_zval_value_name for an illegal offset.
    fn offset_type_name(&self, key: &Value) -> String {
        match key {
            Value::Object(h) => {
                String::from_utf8_lossy(self.class(h.borrow().class).name.as_bytes()).into_owned()
            }
            Value::Closure(_) => "Closure".into(),
            v => v.type_name().to_string(),
        }
    }
}

#[allow(dead_code)]
fn dim_fetch(container: &Value, key: &Value) -> Value {
    match deref(container) {
        Value::Array(a) => match ArrayKey::from_value(key) {
            // A read yields the value, never the element's reference cell.
            Some(k) => a.get(&k).map(deref).unwrap_or(Value::Null),
            None => Value::Null,
        },
        Value::Str(s) => {
            let bytes = s.as_bytes();
            let len = bytes.len() as i64;
            let idx = conv::to_int(key);
            let i = if idx < 0 { len + idx } else { idx };
            if i >= 0 && i < len {
                Value::str(vec![bytes[i as usize]])
            } else {
                Value::str("")
            }
        }
        _ => Value::Null,
    }
}

/// Read the value at a cursor path (Null if any step is absent); non-mutating.
/// Property-name resolution context for cursor paths: the class table and
/// the calling scope, so `$o->p[...]` / `&$o->p` hit the same slot (and the
/// same visibility errors) as a plain `$o->p`.
struct PropCtx<'a> {
    classes: &'a [classes::ClassEntry],
    scope: Option<ClassId>,
    /// The called (LSB) class, named by asymmetric-visibility errors.
    called: Option<ClassId>,
}

impl PropCtx<'_> {
    /// Storage key for `name` on an instance of `cls`, or the access error.
    fn key(&self, cls: ClassId, name: &PhpStr) -> Result<PhpStr, Thrown> {
        classes::prop_key(self.classes, cls, self.scope, name)
    }

    /// Storage key for writing through `$o->name` (with `rest` steps still to
    /// go): also enforces `private(set)` / `protected(set)`. Writing into an
    /// object held by such a property is allowed (handle semantics); anything
    /// else is php's "Cannot (indirectly) modify private(set) property".
    fn write_key(
        &self,
        od: &ObjectData,
        name: &PhpStr,
        leaf_op: &str,
        rest: &[CursorStep],
    ) -> Result<PhpStr, Thrown> {
        let key = self.key(od.class, name)?;
        if let Err(e) = classes::set_access_ok(self.classes, od.class, self.scope, &key) {
            let holds_object = !rest.is_empty()
                && matches!(od.properties.get(&key).map(deref), Some(Value::Object(_)));
            if !holds_object {
                let op = if rest.is_empty() {
                    leaf_op
                } else {
                    "indirectly modify"
                };
                return Err(classes::set_access_err(self.classes, self.called, op, e));
            }
        }
        Ok(key)
    }
}

fn read_path(slot: &Value, steps: &[CursorStep], cx: &PropCtx) -> Value {
    if let Value::Ref(r) = slot {
        return read_path(&r.borrow(), steps, cx);
    }
    let Some((head, rest)) = steps.split_first() else {
        return slot.clone();
    };
    match (slot, head) {
        (Value::Array(a), CursorStep::Key(k)) => match ArrayKey::from_value(k) {
            Some(key) => a
                .get(&key)
                .map(|c| read_path(c, rest, cx))
                .unwrap_or(Value::Null),
            None => Value::Null,
        },
        (Value::Object(h), CursorStep::Prop(name)) => {
            let od = h.borrow();
            match cx.key(od.class, name) {
                Ok(k) => od
                    .properties
                    .get(&k)
                    .map(|c| read_path(c, rest, cx))
                    .unwrap_or(Value::Null),
                Err(_) => Value::Null,
            }
        }
        _ => Value::Null,
    }
}

/// Make `slot` an array (autovivifying null/"" ), or error on a non-empty scalar.
fn autovivify(slot: &mut Value, cx: &PropCtx) -> Result<(), Thrown> {
    match slot {
        Value::Array(_) => Ok(()),
        Value::Null => {
            *slot = Value::Array(PhpArray::new());
            Ok(())
        }
        Value::Bool(false) => {
            path_diag(
                8192,
                "Automatic conversion of false to array is deprecated".into(),
            );
            *slot = Value::Array(PhpArray::new());
            Ok(())
        }
        Value::Object(h) => Err(Thrown::error(
            "Error",
            format!(
                "Cannot use object of type {} as array",
                h.try_borrow()
                    .map(
                        |b| String::from_utf8_lossy(cx.classes[b.class as usize].name.as_bytes())
                            .into_owned()
                    )
                    .unwrap_or_default()
            ),
        )),
        Value::Closure(_) => Err(Thrown::error(
            "Error",
            "Cannot use object of type Closure as array",
        )),
        _ => Err(Thrown::error(
            "Error",
            "Cannot use a scalar value as an array",
        )),
    }
}

thread_local! {
    /// Diagnostics raised inside the pure cursor-path walkers
    /// ([`assign_path`] / [`make_ref_path`] / [`unset_path`]), emitted by
    /// the VM right after the walk ([`Vm::flush_path_diags`]).
    static PATH_DIAGS: RefCell<Vec<(i64, String)>> = const { RefCell::new(Vec::new()) };
}

fn path_diag(level: i64, msg: String) {
    PATH_DIAGS.with(|d| d.borrow_mut().push((level, msg)));
}

/// The array key for a write / fetch-for-write with `k`: php's null-offset
/// and fractional-float deprecations, the TypeError for an unusable key.
fn write_key_of(k: &Value, cx: &PropCtx) -> Result<ArrayKey, Thrown> {
    match deref(k) {
        Value::Null => path_diag(
            8192,
            "Using null as an array offset is deprecated, use an empty string instead".into(),
        ),
        Value::Float(f) if f.is_finite() && f.fract() != 0.0 => path_diag(
            8192,
            format!(
                "Implicit conversion from float {} to int loses precision",
                String::from_utf8_lossy(&conv::to_string_bytes(&Value::Float(f)))
            ),
        ),
        _ => {}
    }
    ArrayKey::from_value(k).ok_or_else(|| illegal_offset_on_array(k, cx))
}

/// php's zend_check_string_offset: the integer offset for a write / unset
/// of string offset `k` ("Illegal string offset" / "String offset cast
/// occurred" warnings; a non-integer key is a TypeError, or for unset php's
/// "Cannot unset string offsets").
fn string_offset(k: &Value, unset: bool, cx: &PropCtx) -> Result<i64, Thrown> {
    let illegal = |ty: String| {
        if unset {
            Thrown::error("Error", "Cannot unset string offsets")
        } else {
            Thrown::type_error(format!("Cannot access offset of type {ty} on string"))
        }
    };
    match deref(k) {
        Value::Int(n) => Ok(n),
        Value::Str(ks) => {
            let kb = ks.as_bytes();
            match conv::scan_number(kb) {
                Some((conv::Num::Int(n), end)) => {
                    let trailing = !kb[end..].iter().all(|b| b" \t\n\r\x0b\x0c".contains(b));
                    if trailing && !unset {
                        path_diag(
                            2,
                            format!("Illegal string offset \"{}\"", String::from_utf8_lossy(kb)),
                        );
                    }
                    Ok(n)
                }
                _ => Err(illegal("string".into())),
            }
        }
        v @ (Value::Float(_) | Value::Null | Value::Bool(_)) => {
            path_diag(2, "String offset cast occurred".into());
            Ok(conv::to_int(&v))
        }
        Value::Object(h) => Err(illegal(
            h.try_borrow()
                .map(|b| {
                    String::from_utf8_lossy(cx.classes[b.class as usize].name.as_bytes())
                        .into_owned()
                })
                .unwrap_or_default(),
        )),
        Value::Closure(_) => Err(illegal("Closure".into())),
        other => Err(illegal(type_lc(&other).to_string())),
    }
}

/// php's zend_wrong_string_offset_error for a fetch below a string offset
/// (`$str[0][…]`, `$str[0]->p`) or a reference to one.
fn string_offset_misuse(rest: &[CursorStep], make_ref: bool) -> Thrown {
    let msg = match rest.first() {
        None if make_ref => "Cannot create references to/from string offsets",
        Some(CursorStep::Prop(_)) => "Cannot use string offset as an object",
        _ => "Cannot use string offset as an array",
    };
    Thrown::error("Error", msg)
}

/// `$str[$k] = $value` (zend_assign_to_string_offset): replace one byte,
/// padding with spaces past the end.
fn assign_string_offset(
    slot: &mut Value,
    k: &Value,
    value: &Value,
    cx: &PropCtx,
) -> Result<(), Thrown> {
    let offset = string_offset(k, false, cx)?;
    let Value::Str(s) = slot else {
        unreachable!("caller checked for a string")
    };
    let len = s.as_bytes().len() as i64;
    if offset < -len {
        path_diag(2, format!("Illegal string offset {offset}"));
        return Ok(());
    }
    let at = if offset < 0 { offset + len } else { offset } as usize;
    let bytes = match deref(value) {
        Value::Array(_) => {
            path_diag(2, "Array to string conversion".into());
            b"Array".to_vec()
        }
        Value::Object(_) | Value::Closure(_) => {
            return Err(Thrown::unsupported(
                "string offset write of an object",
                "converting an object to string here needs __toString dispatch",
            ))
        }
        v => conv::to_string_bytes(&v),
    };
    let Some(&c) = bytes.first() else {
        return Err(Thrown::error(
            "Error",
            "Cannot assign an empty string to a string offset",
        ));
    };
    if bytes.len() != 1 {
        path_diag(
            2,
            "Only the first byte will be assigned to the string offset".into(),
        );
    }
    let mut out = s.as_bytes().to_vec();
    if at >= out.len() {
        out.resize(at, b' ');
        out.push(c);
    } else {
        out[at] = c;
    }
    *slot = Value::str(out);
    Ok(())
}

/// Write `value` at a cursor path, autovivifying + COW-bubbling along the way.
fn assign_path(
    slot: &mut Value,
    steps: &[CursorStep],
    value: Value,
    cx: &PropCtx,
) -> Result<(), Thrown> {
    // `lval = &$x` (a `Ref` value at the leaf) rebinds the slot itself: it
    // does not write through a reference the slot already held.
    if steps.is_empty() && matches!(value, Value::Ref(_)) {
        *slot = value;
        return Ok(());
    }
    // Descend through a reference to its underlying storage.
    if let Value::Ref(rc) = slot {
        let mut inner = rc.borrow_mut();
        return assign_path(&mut inner, steps, value, cx);
    }
    let Some((head, rest)) = steps.split_first() else {
        *slot = value;
        return Ok(());
    };
    if let CursorStep::Prop(name) = head {
        // Descend into an object property (handle semantics: no COW).
        let handle = match slot {
            Value::Object(h) => h.clone(),
            Value::Null => {
                return Err(Thrown::error("Error", "Attempt to assign property on null"))
            }
            other => {
                return Err(Thrown::error(
                    "Error",
                    format!("Attempt to assign property on {}", type_lc(other)),
                ))
            }
        };
        let mut od = handle.borrow_mut();
        let key = cx.write_key(&od, name, "modify", rest)?;
        let child = classes::prop_slot_mut(cx.classes, &mut od, key);
        return assign_path(child, rest, value, cx);
    }
    if let Value::Str(_) = slot {
        let CursorStep::Key(k) = head else {
            return Err(Thrown::error(
                "Error",
                "[] operator not supported for strings",
            ));
        };
        if rest.is_empty() {
            return assign_string_offset(slot, k, &value, cx);
        }
        string_offset(k, false, cx)?;
        return Err(string_offset_misuse(rest, false));
    }
    autovivify(slot, cx)?;
    let Value::Array(arr) = slot else {
        unreachable!("autovivify ensures array")
    };
    let child = match head {
        CursorStep::Key(k) => arr.get_or_insert_mut(write_key_of(k, cx)?),
        CursorStep::Append => arr.append_mut(),
        CursorStep::Prop(_) => unreachable!("handled above"),
    };
    assign_path(child, rest, value, cx)
}

/// The key value for an ArrayAccess cursor step: `Key(k)` -> k, `Append` -> null
/// (`$obj[] = v` calls offsetSet(null, v)); `Prop` is not array access.
fn aa_key(step: &CursorStep) -> Option<Value> {
    match step {
        CursorStep::Key(k) => Some(k.clone()),
        CursorStep::Append => Some(Value::Null),
        CursorStep::Prop(_) => None,
    }
}

/// Navigate the cursor path and promote the target slot (a property or array
/// element) to a shared `Value::Ref` cell, returning the cell — so a by-ref
/// builtin can mutate the caller's property/element in place.
fn make_ref_path(
    slot: &mut Value,
    steps: &[CursorStep],
    cx: &PropCtx,
) -> Result<Rc<RefCell<Value>>, Thrown> {
    // Follow an existing reference: reuse it if this is the target, else descend.
    if let Value::Ref(rc) = slot {
        if steps.is_empty() {
            return Ok(rc.clone());
        }
        let rc = rc.clone();
        let r = make_ref_path(&mut rc.borrow_mut(), steps, cx);
        return r;
    }
    let Some((head, rest)) = steps.split_first() else {
        // Target slot: promote in place to a Ref cell.
        let v = std::mem::replace(slot, Value::Null);
        let rc = Rc::new(RefCell::new(v));
        *slot = Value::Ref(rc.clone());
        return Ok(rc);
    };
    if let CursorStep::Prop(name) = head {
        let handle = match slot {
            Value::Object(h) => h.clone(),
            Value::Null => {
                return Err(Thrown::error(
                    "Error",
                    "Attempt to reference property on null",
                ))
            }
            other => {
                return Err(Thrown::error(
                    "Error",
                    format!("Attempt to reference property on {}", type_lc(other)),
                ))
            }
        };
        let mut od = handle.borrow_mut();
        let key = cx.write_key(&od, name, "indirectly modify", rest)?;
        let child = classes::prop_slot_mut(cx.classes, &mut od, key);
        return make_ref_path(child, rest, cx);
    }
    if let Value::Str(_) = slot {
        let CursorStep::Key(k) = head else {
            return Err(Thrown::error(
                "Error",
                "[] operator not supported for strings",
            ));
        };
        string_offset(k, false, cx)?;
        return Err(string_offset_misuse(rest, true));
    }
    autovivify(slot, cx)?;
    let Value::Array(arr) = slot else {
        unreachable!("autovivify ensures array")
    };
    let child = match head {
        CursorStep::Key(k) => arr.get_or_insert_mut(write_key_of(k, cx)?),
        CursorStep::Append => arr.append_mut(),
        CursorStep::Prop(_) => unreachable!("handled above"),
    };
    make_ref_path(child, rest, cx)
}

/// Remove the leaf key of a cursor path (COW), descending to the parent first.
fn unset_path(slot: &mut Value, steps: &[CursorStep], cx: &PropCtx) -> Result<(), Thrown> {
    if let Value::Ref(rc) = slot {
        let mut inner = rc.borrow_mut();
        return unset_path(&mut inner, steps, cx);
    }
    let Some((head, rest)) = steps.split_first() else {
        return Ok(());
    };
    match head {
        CursorStep::Prop(name) => {
            let Value::Object(h) = slot else {
                return Ok(());
            };
            let h = h.clone();
            let mut od = h.borrow_mut();
            let Ok(key) = cx.key(od.class, name) else {
                return Ok(());
            };
            if rest.is_empty() {
                od.properties.shift_remove(&key);
            } else if let Some(child) = od.properties.get_mut(&key) {
                unset_path(child, rest, cx)?;
            }
        }
        CursorStep::Key(k) => {
            let arr = match slot {
                Value::Array(arr) => arr,
                // ZEND_UNSET_DIM / FETCH_DIM_UNSET on a string.
                Value::Str(_) if rest.is_empty() => {
                    return Err(Thrown::error("Error", "Cannot unset string offsets"))
                }
                Value::Str(_) => {
                    string_offset(k, true, cx)?;
                    return Err(string_offset_misuse(rest, false));
                }
                Value::Bool(false) if rest.is_empty() => {
                    path_diag(
                        8192,
                        "Automatic conversion of false to array is deprecated".into(),
                    );
                    return Ok(());
                }
                Value::Bool(true) | Value::Int(_) | Value::Float(_) | Value::Resource(_)
                    if rest.is_empty() =>
                {
                    return Err(Thrown::error(
                        "Error",
                        "Cannot unset offset in a non-array variable",
                    ))
                }
                _ => return Ok(()),
            };
            let Some(key) = ArrayKey::from_value(k) else {
                if !rest.is_empty() {
                    return Err(illegal_offset_on_array(k, cx));
                }
                let ty = match deref(k) {
                    Value::Object(h) => h
                        .try_borrow()
                        .map(|b| {
                            String::from_utf8_lossy(cx.classes[b.class as usize].name.as_bytes())
                                .into_owned()
                        })
                        .unwrap_or_default(),
                    Value::Closure(_) => "Closure".into(),
                    other => type_lc(&other).to_string(),
                };
                return Err(Thrown::type_error(format!(
                    "Cannot unset offset of type {ty} on array"
                )));
            };
            if rest.is_empty() {
                arr.remove(&key);
            } else if let Some(child) = arr.get_mut(&key) {
                unset_path(child, rest, cx)?;
            }
        }
        CursorStep::Append => {}
    }
    Ok(())
}

/// For by-reference foreach: promote the array element at insertion index `pos`
/// to a shared cell and return `(key, cell)`. The element in the array becomes a
/// `Value::Ref` to that cell, so a later write through the cell is visible in the
/// array. Returns `None` if `slot` is not an array or `pos` is out of range.
fn foreach_promote(slot: &mut Value, pos: usize) -> Option<(ArrayKey, Rc<RefCell<Value>>)> {
    let Value::Array(arr) = slot else { return None };
    if pos >= arr.len() {
        return None;
    }
    let data = arr.make_mut();
    let (k, v) = data.entries.get_index_mut(pos)?;
    let key = k.clone();
    let rc = match v {
        Value::Ref(rc) => rc.clone(),
        other => {
            let inner = std::mem::replace(other, Value::Null);
            let rc = Rc::new(RefCell::new(inner));
            *other = Value::Ref(rc.clone());
            rc
        }
    };
    Some((key, rc))
}

fn type_lc(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) | Value::Closure(_) => "object",
        Value::Resource(_) => "resource",
        Value::Ref(_) => "reference",
    }
}

/// Built-in PHP-source (prelude) code is compiled without a line map.
fn is_prelude_proto(p: &FunctionProto) -> bool {
    p.debug.lines.iter().all(|&(_, l)| l == 0)
}

impl Vm {
    /// Whether class `c` is a built-in class implemented in the PHP prelude.
    fn is_prelude_class(&self, c: ClassId) -> bool {
        self.class(c)
            .methods
            .values()
            .any(|m| m.defining_class == c && is_prelude_proto(&m.proto))
    }

    /// Reading the absent (uninitialized) slot `key` of a typed property:
    /// php's "must not be accessed before initialization" Error.
    fn typed_slot(&self, cls: ClassId, key: &PhpStr) -> Option<Thrown> {
        let pe = self.classes.get(cls as usize)?.props.get(key)?;
        pe.ty.as_ref()?;
        Some(Thrown::error(
            "Error",
            format!(
                "Typed property {}::${} must not be accessed before initialization",
                String::from_utf8_lossy(self.class(pe.declaring_class).name.as_bytes()),
                String::from_utf8_lossy(pe.name.as_bytes())
            ),
        ))
    }

    /// zend_verify_property_type for an assignment of `val` to the declared
    /// slot `key` of class `cls`: coercive (or strict) scalar conversion, the
    /// `__toString` step, else "Cannot assign T to property C::$p of type X".
    pub(crate) fn verify_prop_type(
        &mut self,
        cls: ClassId,
        key: &PhpStr,
        val: Value,
        strict: bool,
    ) -> Result<Value, Thrown> {
        let Some(pe) = self
            .classes
            .get(cls as usize)
            .and_then(|e| e.props.get(key))
        else {
            return Ok(val);
        };
        let Some(ty) = pe.ty.clone() else {
            return Ok(val);
        };
        let decl = pe.declaring_class;
        let name = pe.name.clone();
        let val = self.coerce_stringable(val, &ty, strict, Some(decl))?;
        match self.check_one(&val, &ty, strict, Some(decl)) {
            Ok(None) => Ok(val),
            Ok(Some(c)) => {
                if let (Value::Float(f), Value::Int(_)) = (deref(&val), &c) {
                    if f.fract() != 0.0 {
                        let shown = String::from_utf8_lossy(&rhp_values::fmt::float_serialize(f))
                            .into_owned();
                        self.diag(
                            8192,
                            &format!(
                                "Implicit conversion from float {shown} to int loses precision"
                            ),
                        )?;
                    }
                }
                Ok(c)
            }
            Err(()) => Err(Thrown::type_error(format!(
                "Cannot assign {} to property {}::${} of type {}",
                self.zval_value_name(&deref(&val)),
                String::from_utf8_lossy(self.class(decl).name.as_bytes()),
                String::from_utf8_lossy(name.as_bytes()),
                self.type_string_resolved(&ty, Some(decl), Some(cls))
            ))),
        }
    }

    /// The readonly checks of a write to the declared slot `key` (`present`:
    /// already initialized): php's "Cannot modify readonly property" and,
    /// for an uninitialized one written outside its declaring class,
    /// "Cannot initialize readonly property … from scope/global scope".
    pub(crate) fn check_readonly_write(
        &mut self,
        obj: &ObjectHandle,
        key: &PhpStr,
        present: bool,
        scope: Option<ClassId>,
    ) -> Result<(), Thrown> {
        let cls = obj.borrow().class;
        let Some(pe) = self
            .classes
            .get(cls as usize)
            .and_then(|e| e.props.get(key))
        else {
            return Ok(());
        };
        if !pe.readonly {
            return Ok(());
        }
        let decl = pe.declaring_class;
        let name = pe.name.clone();
        let cname = String::from_utf8_lossy(self.class(decl).name.as_bytes()).into_owned();
        let p = String::from_utf8_lossy(name.as_bytes()).into_owned();
        // Redeclared in a child: the parent that declared it may still
        // initialize it (zend_verify_readonly_initialization_access).
        let parent_scope_ok = scope.is_some_and(|s| {
            s != decl
                && self.class(cls).ancestors.contains(&s)
                && self
                    .class(s)
                    .prop_visible
                    .get(&name)
                    .and_then(|k| self.class(s).props.get(k))
                    .is_some_and(|spe| spe.declaring_class == s)
        });
        if present {
            // php 8.3: during __clone each readonly property may be
            // reinitialized once (from the property's scope).
            let oid = obj.borrow().id;
            let in_clone = self.stack.iter().any(|f| {
                f.proto.name.as_bytes().eq_ignore_ascii_case(b"__clone")
                    && f.proto
                        .this_cv
                        .and_then(|cv| match f.regs.get(cv as usize) {
                            Some(Value::Object(o)) => Some(o.borrow().id == oid),
                            _ => None,
                        })
                        .unwrap_or(false)
            });
            if in_clone
                && (scope == Some(decl) || parent_scope_ok)
                && self.readonly_reinit.insert((oid, key.clone()))
            {
                return Ok(());
            }
            return Err(Thrown::error(
                "Error",
                format!("Cannot modify readonly property {cname}::${p}"),
            ));
        }
        // A public readonly property is implicitly protected(set).
        if pe_public_implicit_set(self, cls, key)
            && !scope.is_some_and(|s| {
                self.class(s).ancestors.contains(&decl) || self.class(decl).ancestors.contains(&s)
            })
        {
            let from = match scope {
                Some(s) => format!(
                    "scope {}",
                    String::from_utf8_lossy(self.class(s).name.as_bytes())
                ),
                None => "global scope".into(),
            };
            return Err(Thrown::error(
                "Error",
                format!("Cannot modify protected(set) readonly property {cname}::${p} from {from}"),
            ));
        }
        if scope != Some(decl) && !parent_scope_ok {
            let from = match scope {
                Some(s) => format!(
                    "scope {}",
                    String::from_utf8_lossy(self.class(s).name.as_bytes())
                ),
                None => "global scope".into(),
            };
            return Err(Thrown::error(
                "Error",
                format!("Cannot initialize readonly property {cname}::${p} from {from}"),
            ));
        }
        Ok(())
    }

    /// A prelude class's `__`-prefixed state slot (see
    /// [`builtins::Host::is_internal_slot`]): declared by a prelude class,
    /// so a user subclass's own `$__x` stays visible.
    pub(crate) fn internal_slot(&self, class: ClassId, key: &PhpStr) -> bool {
        let Some(pe) = self
            .classes
            .get(class as usize)
            .and_then(|e| e.props.get(key))
        else {
            return false;
        };
        pe.name.as_bytes().starts_with(b"__") && self.is_prelude_class(pe.declaring_class)
    }
}

/// A `__`-prefixed method name that is not one of php's magic methods.
fn is_engine_helper_name(name: &[u8]) -> bool {
    const MAGIC: &[&[u8]] = &[
        b"__construct",
        b"__destruct",
        b"__call",
        b"__callstatic",
        b"__get",
        b"__set",
        b"__isset",
        b"__unset",
        b"__sleep",
        b"__wakeup",
        b"__serialize",
        b"__unserialize",
        b"__tostring",
        b"__invoke",
        b"__set_state",
        b"__clone",
        b"__debuginfo",
    ];
    let lc = name.to_ascii_lowercase();
    lc.starts_with(b"__") && !MAGIC.contains(&lc.as_slice())
}

fn array_key_to_value(k: &ArrayKey) -> Value {
    match k {
        ArrayKey::Int(n) => Value::Int(*n),
        ArrayKey::Str(s) => Value::Str(s.clone()),
    }
}

/// php's "Undefined array key" warning text for `k`.
fn undefined_key_msg(k: &ArrayKey) -> String {
    match k {
        ArrayKey::Int(n) => format!("Undefined array key {n}"),
        ArrayKey::Str(s) => format!(
            "Undefined array key \"{}\"",
            String::from_utf8_lossy(s.as_bytes())
        ),
    }
}

/// php's "Cannot access offset of type T on array" TypeError (an array
/// write/fetch-for-write with an unusable key).
fn illegal_offset_on_array(k: &Value, cx: &PropCtx) -> Thrown {
    let ty = match deref(k) {
        Value::Object(h) => h
            .try_borrow()
            .map(|b| {
                String::from_utf8_lossy(cx.classes[b.class as usize].name.as_bytes()).into_owned()
            })
            .unwrap_or_default(),
        Value::Closure(_) => "Closure".into(),
        other => type_lc(&other).to_string(),
    };
    Thrown::type_error(format!("Cannot access offset of type {ty} on array"))
}

/// Bind positional `args` to a callee's parameter CV registers (by value), with
/// constant defaults and variadic collection; surplus args (non-variadic callee)
/// are kept in `extra_args`. `Err(i)`: required parameter `i` was not passed.
fn bind_params(proto: &FunctionProto, args: &[Value], frame: &mut Frame) -> Result<(), usize> {
    let regs = &mut frame.regs;
    for (i, p) in proto.params.iter().enumerate() {
        if p.variadic {
            // Collect the remaining args into an array (by value).
            let mut arr = PhpArray::new();
            if i < args.len() {
                for a in &args[i..] {
                    arr.push(deref(a));
                }
            }
            regs[p.cv as usize] = Value::Array(arr);
            return Ok(()); // variadic is always the last parameter
        } else if i < args.len() {
            // By-ref params keep the incoming Ref (aliasing the caller's var);
            // by-value params receive a dereferenced copy.
            regs[p.cv as usize] = if p.by_ref {
                args[i].clone()
            } else {
                deref(&args[i])
            };
        } else if let Some(def) = &p.default {
            match def {
                ParamDefault::Const(k) => regs[p.cv as usize] = proto.consts[*k as usize].clone(),
                ParamDefault::Init(_) => {} // non-constant defaults: deferred
            }
        } else {
            return Err(i);
        }
    }
    if args.len() > proto.params.len() {
        frame.extra_args = args[proto.params.len()..].iter().map(deref).collect();
    }
    Ok(())
}

fn required_params(proto: &FunctionProto) -> usize {
    proto
        .params
        .iter()
        .filter(|p| p.default.is_none() && !p.variadic)
        .count()
}
