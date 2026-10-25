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

use rhp_bytecode::{
    ClassProto, ClassRef, CmpKind, ConstInitProto, Default as ParamDefault, FunctionProto,
    IncludeKind, Instr, NIdx, PropDefaultProto, Program, Reg,
};
use rhp_builtins::{self as builtins, Host};
use rhp_semantics::{cast_to_array, inc_dec_value, ops};
use rhp_values::{
    conv, ArrayKey, ClassId, ClassKind, ObjectData, ObjectHandle, PhpArray, PhpStr, Thrown, Value,
};

/// A pending lvalue step (an evaluated key, an append, or an object property).
enum CursorStep {
    Key(Value),
    Append,
    Prop(rhp_values::PhpStr),
}

/// The virtual machine: output sink + call stack + global tables.
pub struct Vm {
    out: Vec<u8>,
    stack: Vec<Frame>,
    functions: Vec<Rc<FunctionProto>>,
    fn_names: HashMap<Vec<u8>, usize>, // lowercase name -> index into `functions`
    closures: Vec<Rc<FunctionProto>>, // anonymous closure protos, indexed by `fnidx`
    constants: HashMap<Vec<u8>, Value>,
    max_depth: usize,
    // lvalue cursor: a root in the current frame (CV or a static-prop cell) +
    // pending path steps. Cursor opcodes are emitted contiguously, so a single
    // cursor never nests.
    cursor_root: CursorRoot,
    cursor_steps: Vec<CursorStep>,
    // OOP class table (see `classes` module).
    classes: Vec<classes::ClassEntry>,
    class_ids: HashMap<Vec<u8>, ClassId>, // lowercase name -> id
    next_object_id: u32,
    // `global $x`: {main}'s CV register for each top-level variable name, plus
    // an overflow table for globals first created inside a function.
    main_cv_names: HashMap<Vec<u8>, Reg>,
    globals: HashMap<Vec<u8>, Rc<RefCell<Value>>>,
    // `static $x`: persistent cells keyed by (function proto pointer, var name).
    statics: HashMap<(usize, Vec<u8>), Rc<RefCell<Value>>>,
    // Non-constant class const / static-prop / property-default initializers,
    // evaluated after all classes link (so cross-class/enum refs resolve).
    pending_inits: Vec<classes::PendingInit>,
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
    includer: Option<Box<dyn FnMut(&str, &[u8]) -> Result<Program, Thrown>>>,
    // Resolved real paths already included (for the `_once` variants).
    included_files: std::collections::HashSet<Vec<u8>>,
    // Registered `spl_autoload_register` callables, in call order.
    autoloaders: Vec<Value>,
    // Stream wrappers by scheme (`file`, `phar`, …), installed at startup from
    // the module list; runtime-mutable later via `stream_wrapper_register`.
    wrappers: HashMap<Vec<u8>, Rc<dyn builtins::StreamWrapper>>,
    // Filesystem path of the top-level script (for `Phar::mapPhar`).
    main_script_path: Option<Vec<u8>>,
    // Open stream resources (fopen family), keyed by resource id.
    streams: HashMap<u32, StreamState>,
    next_resource_id: u32,
    // CLI arguments ($argv / $_SERVER['argv']); argv[0] is the script name.
    argv: Vec<Vec<u8>>,
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
    Mem { buf: Vec<u8>, pos: usize },
    /// A file opened for writing: buffered, flushed to the wrapper on
    /// flush/close. `append` keeps existing content.
    FileWrite { buf: Vec<u8>, url: Vec<u8> },
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
}

mod classes;

/// Where an lvalue cursor is rooted.
enum CursorRoot {
    /// A register (compiled variable) in the current frame.
    Cv(Reg),
    /// A shared static-property cell.
    Cell(Rc<RefCell<Value>>),
}

impl Host for Vm {
    fn write_out(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    fn class_name(&self, class_id: u32) -> Option<Vec<u8>> {
        self.classes.get(class_id as usize).map(|c| c.name.as_bytes().to_vec())
    }

    fn class_parent_name(&self, class_id: u32) -> Option<Vec<u8>> {
        let parent = self.classes.get(class_id as usize)?.parent?;
        Some(self.classes[parent as usize].name.as_bytes().to_vec())
    }

    fn is_enum_class(&self, class_id: u32) -> bool {
        self.classes.get(class_id as usize).is_some_and(|c| c.kind == ClassKind::Enum)
    }

    fn define_const(&mut self, name: &[u8], value: Value) -> bool {
        if self.constants.contains_key(name) {
            return false;
        }
        self.constants.insert(name.to_vec(), value);
        true
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
        self.wrapper_for(url)?.get_contents(url)
    }

    fn stream_exists(&self, url: &[u8]) -> bool {
        self.wrapper_for(url).map(|w| w.exists(url)).unwrap_or(false)
    }

    fn stream_stat(&self, url: &[u8]) -> Option<builtins::StatInfo> {
        self.wrapper_for(url)?.stat(url)
    }

    fn stream_put_contents(&mut self, url: &[u8], data: &[u8]) -> Option<usize> {
        self.wrapper_for(url)?.put_contents(url, data)
    }

    fn stream_wrapper(&self, scheme: &[u8]) -> Option<Rc<dyn builtins::StreamWrapper>> {
        self.wrappers.get(scheme).cloned()
    }

    fn running_main_path(&self) -> Option<Vec<u8>> {
        self.main_script_path.clone()
    }

    fn stream_open(&mut self, path: &[u8], mode: &[u8]) -> Option<Value> {
        let writable = mode.iter().any(|&b| matches!(b, b'w' | b'a' | b'x' | b'c' | b'+'));
        let append = mode.first() == Some(&b'a');
        let state = if path == b"php://stdout" || path == b"php://output" {
            StreamState::Stdout
        } else if path == b"php://stderr" {
            StreamState::Stderr
        } else if path == b"php://stdin" {
            StreamState::Stdin
        } else if path == b"php://memory" || path.starts_with(b"php://temp") {
            StreamState::Mem { buf: Vec::new(), pos: 0 }
        } else if path.starts_with(b"php://") {
            return None; // other php:// streams unsupported
        } else if writable {
            let buf = if append {
                self.wrapper_for(path).and_then(|w| w.get_contents(path)).unwrap_or_default()
            } else {
                Vec::new()
            };
            StreamState::FileWrite { buf, url: path.to_vec() }
        } else {
            let data = self.wrapper_for(path).and_then(|w| w.get_contents(path))?;
            StreamState::Mem { buf: data, pos: 0 }
        };
        Some(self.make_stream(state, "stream"))
    }

    fn stream_write(&mut self, res: &Value, data: &[u8]) -> Option<usize> {
        let id = res_id(res)?;
        enum Act {
            Out,
            Err,
            Buf,
            No,
        }
        let act = match self.streams.get(&id)? {
            StreamState::Stdout => Act::Out,
            StreamState::Stderr => Act::Err,
            StreamState::Mem { .. } | StreamState::FileWrite { .. } => Act::Buf,
            StreamState::Stdin => Act::No,
        };
        match act {
            Act::Out => self.out.extend_from_slice(data),
            Act::Err => {
                use std::io::Write;
                let _ = std::io::stderr().write_all(data);
            }
            Act::Buf => match self.streams.get_mut(&id)? {
                StreamState::Mem { buf, .. } | StreamState::FileWrite { buf, .. } => {
                    buf.extend_from_slice(data)
                }
                _ => {}
            },
            Act::No => return None,
        }
        Some(data.len())
    }

    fn stream_read(&mut self, res: &Value, len: i64) -> Option<Vec<u8>> {
        let id = res_id(res)?;
        match self.streams.get_mut(&id)? {
            StreamState::Mem { buf, pos } => {
                let take = if len < 0 { buf.len() - *pos } else { (len as usize).min(buf.len() - *pos) };
                let out = buf[*pos..*pos + take].to_vec();
                *pos += take;
                Some(out)
            }
            _ => Some(Vec::new()),
        }
    }

    fn stream_gets(&mut self, res: &Value) -> Option<Vec<u8>> {
        let id = res_id(res)?;
        match self.streams.get_mut(&id)? {
            StreamState::Mem { buf, pos } => {
                if *pos >= buf.len() {
                    return None;
                }
                let start = *pos;
                let nl = buf[start..].iter().position(|&b| b == b'\n').map(|i| start + i + 1);
                let end = nl.unwrap_or(buf.len());
                let line = buf[start..end].to_vec();
                *pos = end;
                Some(line)
            }
            _ => None,
        }
    }

    fn stream_eof(&self, res: &Value) -> bool {
        match res_id(res).and_then(|id| self.streams.get(&id)) {
            Some(StreamState::Mem { buf, pos }) => *pos >= buf.len(),
            _ => true,
        }
    }

    fn stream_close(&mut self, res: &Value) -> bool {
        let Some(id) = res_id(res) else { return false };
        match self.streams.remove(&id) {
            Some(StreamState::FileWrite { buf, url }) => {
                self.wrapper_for(&url).map(|w| w.put_contents(&url, &buf)).is_some()
            }
            Some(_) => true,
            None => false,
        }
    }

    fn stream_flush(&mut self, res: &Value) -> bool {
        let Some(id) = res_id(res) else { return false };
        if let Some(StreamState::FileWrite { buf, url }) = self.streams.get(&id) {
            let (buf, url) = (buf.clone(), url.clone());
            return self.wrapper_for(&url).map(|w| w.put_contents(&url, &buf)).is_some();
        }
        true
    }

    fn stream_rewind(&mut self, res: &Value) -> bool {
        if let Some(StreamState::Mem { pos, .. }) =
            res_id(res).and_then(|id| self.streams.get_mut(&id))
        {
            *pos = 0;
        }
        true
    }

    fn call_callable(&mut self, callable: &Value, args: &[Value]) -> Result<Value, Thrown> {
        self.call_callable_value(callable, args.to_vec())
    }

    fn resolve_class(&self, name: &[u8]) -> Option<u32> {
        self.resolve_class_by_name(name).ok()
    }

    fn function_exists(&self, name: &[u8]) -> bool {
        self.fn_names.contains_key(&name.to_ascii_lowercase())
            || builtins::lookup(&String::from_utf8_lossy(name)).is_some()
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
        // Scope: an explicit class name / object, else the new $this's class,
        // else keep the closure's original scope.
        let scope_class = match deref(scope) {
            Value::Null => new_this_h.as_ref().map(|h| h.borrow().class).or(cd.bound_class),
            Value::Object(h) => Some(h.borrow().class),
            v => self.resolve_class_by_name(&conv::to_string_bytes(&v)).ok().or(cd.bound_class),
        };
        Some(Value::Closure(Rc::new(rhp_values::ClosureData {
            fnidx: cd.fnidx,
            captures: cd.captures.clone(),
            bound_this: new_this_h,
            bound_class: scope_class,
            bound_static: scope_class,
            wrapped: cd.wrapped.clone(),
        })))
    }

    fn class_kind(&self, class_id: u32) -> Option<ClassKind> {
        self.classes.get(class_id as usize).map(|c| c.kind)
    }

    fn class_has_method(&self, class_id: u32, method: &[u8]) -> bool {
        self.lookup_method(class_id, method).is_some()
    }

    fn class_has_property(&self, class_id: u32, prop: &[u8]) -> bool {
        match self.classes.get(class_id as usize) {
            Some(entry) => {
                entry.props.keys().any(|k| k.as_bytes() == prop)
                    || entry.static_props.contains_key(prop)
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
        let obj = self.new_object(cls);
        {
            let mut b = obj.borrow_mut();
            for (k, v) in props {
                b.properties.insert(rhp_values::PhpStr::from_bytes(k), v);
            }
        }
        Some(Value::Object(obj))
    }

    fn object_prop(&self, obj: &Value, name: &[u8]) -> Option<Value> {
        let key = rhp_values::PhpStr::from_bytes(name);
        let h = match obj {
            Value::Object(h) => h,
            Value::Ref(r) => match &*r.borrow() {
                Value::Object(h) => return h.borrow().properties.get(&key).cloned(),
                _ => return None,
            },
            _ => return None,
        };
        h.borrow().properties.get(&key).cloned()
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
        let c = match callable {
            Value::Ref(r) => r.borrow().clone(),
            other => other.clone(),
        };
        match c {
            Value::Closure(_) => true,
            Value::Str(s) => {
                let lower = s.as_bytes().to_ascii_lowercase();
                self.fn_names.contains_key(&lower)
                    || builtins::lookup(&String::from_utf8_lossy(s.as_bytes())).is_some()
            }
            Value::Array(a) if a.len() == 2 => {
                let item0 = a.get(&ArrayKey::Int(0)).cloned().unwrap_or(Value::Null);
                let method = a.get(&ArrayKey::Int(1)).cloned().unwrap_or(Value::Null);
                let mname = conv::to_string_bytes(&method);
                let cls = match item0 {
                    Value::Object(h) => Some(h.borrow().class),
                    Value::Str(cn) => self.resolve_class_by_name(cn.as_bytes()).ok(),
                    _ => None,
                };
                cls.map(|c| self.lookup_method(c, &mname).is_some()).unwrap_or(false)
            }
            Value::Object(h) => {
                let cls = h.borrow().class;
                self.lookup_method(cls, b"__invoke").is_some()
            }
            _ => false,
        }
    }
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
    /// What a `finally` block must do when it finishes (set by the unwinder).
    pending: Option<FinallyCont>,
    /// A synchronous (re-entrant) call: when this frame returns, `do_return`
    /// hands the value back to the Rust caller instead of delivering to a reg.
    capture: bool,
    /// Number of positional arguments actually passed (for non-constant parameter
    /// default-init guards in the prologue).
    argc: usize,
}

/// Result of unwinding: a handler was installed, or the exception escaped the
/// current (sub)run and should propagate to the Rust caller.
enum UnwindResult {
    Caught,
    Escaped(Thrown),
}

/// What to do after a `finally` block completes.
enum FinallyCont {
    /// An exception was in flight (no catch matched); re-raise after finally.
    Rethrow(Thrown),
    /// A `return` crossed out of the try/catch; complete it after finally.
    Return(Value),
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
            pending: None,
            capture: false,
            argc: 0,
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
        Vm {
            out: Vec::new(),
            stack: Vec::new(),
            functions: Vec::new(),
            fn_names: HashMap::new(),
            closures: Vec::new(),
            constants,
            max_depth: 1 << 16,
            cursor_root: CursorRoot::Cv(0),
            cursor_steps: Vec::new(),
            classes: Vec::new(),
            class_ids: HashMap::new(),
            next_object_id: 1,
            main_cv_names: HashMap::new(),
            globals: HashMap::new(),
            statics: HashMap::new(),
            pending_inits: Vec::new(),
            generators: HashMap::new(),
            yield_out: None,
            gen_stack: Vec::new(),
            bc_scale: 0,
            includer: None,
            included_files: std::collections::HashSet::new(),
            autoloaders: Vec::new(),
            wrappers,
            main_script_path: None,
            streams: HashMap::new(),
            next_resource_id: 1,
            argv: Vec::new(),
        }
    }

    /// Set the CLI argument vector (`$argv`; `argv[0]` is the script name).
    pub fn set_argv(&mut self, argv: Vec<Vec<u8>>) {
        self.argv = argv;
    }

    /// Get or create the shared global cell for `name` (the overflow globals
    /// table), used for superglobals and `global`.
    fn global_cell(&mut self, name: &[u8]) -> Rc<RefCell<Value>> {
        self.globals
            .entry(name.to_vec())
            .or_insert_with(|| Rc::new(RefCell::new(Value::Null)))
            .clone()
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
            server.insert(ArrayKey::from_bytes(k.as_bytes()), Value::str(v.into_bytes()));
        }
        server.insert(ArrayKey::from_bytes(b"argv"), Value::Array(argv_arr.clone()));
        server.insert(ArrayKey::from_bytes(b"argc"), Value::Int(argc));
        server.insert(ArrayKey::from_bytes(b"PHP_SELF"), Value::str(script.clone()));
        server.insert(ArrayKey::from_bytes(b"SCRIPT_NAME"), Value::str(script.clone()));
        let script_fn = self.main_script_path.clone().unwrap_or(script.clone());
        server.insert(ArrayKey::from_bytes(b"SCRIPT_FILENAME"), Value::str(script_fn));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        server.insert(ArrayKey::from_bytes(b"REQUEST_TIME"), Value::Int(now));
        server.insert(ArrayKey::from_bytes(b"REQUEST_TIME_FLOAT"), Value::Float(now as f64));

        let set = |vm: &mut Vm, name: &[u8], v: Value| {
            *vm.global_cell(name).borrow_mut() = v;
        };
        set(self, b"_SERVER", Value::Array(server));
        set(self, b"_ENV", {
            let mut a = PhpArray::new();
            for (k, v) in std::env::vars() {
                a.insert(ArrayKey::from_bytes(k.as_bytes()), Value::str(v.into_bytes()));
            }
            Value::Array(a)
        });
        for empty in [&b"_GET"[..], b"_POST", b"_COOKIE", b"_FILES", b"_REQUEST", b"_SESSION"] {
            set(self, empty, Value::Array(PhpArray::new()));
        }
        // $argv / $argc as plain globals (available at top level / via `global`).
        set(self, b"argv", Value::Array(argv_arr));
        set(self, b"argc", Value::Int(argc));
    }

    /// Whether `v` is an instance of the named class/interface (or a Closure for
    /// the `Closure` type). Unresolvable target class → lenient `true` (the class
    /// may be autoloadable; we don't reject on a name we can't see).
    fn value_instanceof(&self, v: &Value, name: &[u8]) -> bool {
        match v {
            Value::Object(h) => match self.resolve_class_by_name(name) {
                Ok(cid) => self.classes[h.borrow().class as usize].ancestors.contains(&cid),
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
            K::Iterable => {
                matches!(v, Value::Array(_)) || self.value_instanceof(v, b"Traversable")
            }
            K::Null => matches!(v, Value::Null),
            K::Named(n) => self.value_instanceof(v, n.as_bytes()),
            K::SelfType | K::StaticType => match (v, ctx) {
                (Value::Object(h), Some(c)) => {
                    self.classes[h.borrow().class as usize].ancestors.contains(&c)
                }
                _ => false,
            },
            K::Parent => match (v, ctx.and_then(|c| self.classes[c as usize].parent)) {
                (Value::Object(h), Some(p)) => {
                    self.classes[h.borrow().class as usize].ancestors.contains(&p)
                }
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
        // Coercive mode: coerce to a lone scalar target kind.
        if let Some(k) = scalar_target(ty) {
            if let Some(cv) = coerce_scalar(&d, k) {
                return Ok(Some(cv));
            }
        }
        Err(())
    }

    /// Enforce declared parameter types on a just-bound frame (php's
    /// `strict_types` is the CALLER's; `ctx` resolves `self`/`static`/`parent`).
    fn check_arg_types(
        &self,
        proto: &FunctionProto,
        frame: &mut Frame,
        strict: bool,
        ctx: Option<ClassId>,
    ) -> Result<(), Thrown> {
        for (i, p) in proto.params.iter().enumerate() {
            let Some(ty) = &p.type_hint else { continue };
            // by-ref / variadic params aren't coerced here; a defaulted (not
            // passed) param is trusted.
            if p.by_ref || p.variadic || i >= frame.argc {
                continue;
            }
            let cur = frame.regs[p.cv as usize].clone();
            match self.check_one(&cur, ty, strict, ctx) {
                Ok(None) => {}
                Ok(Some(coerced)) => frame.regs[p.cv as usize] = coerced,
                Err(()) => {
                    let fname = String::from_utf8_lossy(proto.name.as_bytes()).into_owned();
                    let pname =
                        String::from_utf8_lossy(proto.names[p.name as usize].as_bytes()).into_owned();
                    // php appends the call site (the caller is the current top of
                    // stack — the new frame has not been pushed yet).
                    let site = self.stack.last().map(|f| {
                        let pc = f.pc.saturating_sub(1) as u32;
                        let idx = f.proto.debug.lines.partition_point(|(p, _)| (*p as u32) <= pc);
                        let line = if idx == 0 { 0 } else { f.proto.debug.lines[idx - 1].1 };
                        format!(
                            ", called in {} on line {}",
                            String::from_utf8_lossy(f.proto.debug.file.as_bytes()),
                            line
                        )
                    });
                    return Err(Thrown::error(
                        "TypeError",
                        format!(
                            "{}(): Argument #{} (${}) must be of type {}, {} given{}",
                            fname,
                            i + 1,
                            pname,
                            type_string(ty),
                            self.value_debug_type(&deref(&cur)),
                            site.unwrap_or_default(),
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Enforce a function's declared return type on `v`, coercing in coercive
    /// mode. `void`/`never`/`mixed` and unknowable types are not checked.
    fn check_return_type(&self, proto: &FunctionProto, top: usize, v: Value) -> Result<Value, Thrown> {
        use rhp_bytecode::TypeKind as K;
        let ty = match &proto.return_type {
            Some(t) => t,
            None => return Ok(v),
        };
        if ty.kinds.iter().any(|k| matches!(k, K::Void | K::Never | K::Mixed)) {
            return Ok(v);
        }
        let ctx = self.stack[top].defining_class;
        let strict = proto.strict_types;
        match self.check_one(&v, ty, strict, ctx) {
            Ok(None) => Ok(v),
            Ok(Some(coerced)) => Ok(coerced),
            Err(()) => {
                let fname = String::from_utf8_lossy(proto.name.as_bytes()).into_owned();
                Err(Thrown::error(
                    "TypeError",
                    format!(
                        "{}(): Return value must be of type {}, {} returned",
                        fname,
                        type_string(ty),
                        self.value_debug_type(&deref(&v)),
                    ),
                ))
            }
        }
    }

    /// The strict-types mode of the current caller (top of stack), for argument
    /// coercion of a call about to be made.
    fn caller_strict(&self) -> bool {
        self.stack.last().map(|f| f.proto.strict_types).unwrap_or(false)
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
            let cell = self.global_cell(name.as_bytes());
            frame.regs[cv as usize] = Value::Ref(cell);
        }
    }

    /// Allocate a stream resource backed by `state`, returning its resource Value.
    fn make_stream(&mut self, state: StreamState, type_name: &'static str) -> Value {
        let id = self.next_resource_id;
        self.next_resource_id += 1;
        self.streams.insert(id, state);
        Value::Resource(Rc::new(rhp_values::ResourceData { id, type_name }))
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
        f: impl FnMut(&str, &[u8]) -> Result<Program, Thrown> + 'static,
    ) {
        self.includer = Some(Box::new(f));
    }

    pub fn output(&self) -> &[u8] {
        &self.out
    }

    /// Execute a compiled program; returns the process exit code.
    pub fn run(&mut self, prog: &Program) -> Result<i32, Thrown> {
        // Bind the standard stream resources (CLI SAPI) once.
        if self.streams.is_empty() {
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
            self.fn_names.insert(proto.name.as_bytes().to_ascii_lowercase(), idx);
        }
        // Register anonymous closure / arrow-function protos (indexed by fnidx).
        self.closures = prog.closures.clone();
        // Install user-defined global constants.
        for (name, value) in &prog.constants {
            self.constants.insert(name.as_bytes().to_vec(), value.clone());
        }
        // Record {main}'s variable -> register map for `global $x`.
        for (reg, name) in prog.main_var_names.iter().enumerate() {
            self.main_cv_names.insert(name.as_bytes().to_vec(), reg as Reg);
        }
        // Link all declared classes (flatten inheritance, resolve forward refs).
        self.install_classes(&prog.classes)?;
        let mut main_frame = Frame::new(prog.main.clone(), 0);
        self.bind_superglobals(&mut main_frame);
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
        self.stack[top].regs[r as usize] = v;
    }

    /// Assign to a PHP *variable* (a CV): if the slot is a reference, write
    /// through it (so by-ref aliases see the change); otherwise overwrite.
    fn cv_assign(&mut self, top: usize, cv: Reg, v: Value) {
        let slot = &mut self.stack[top].regs[cv as usize];
        if let Value::Ref(rc) = slot {
            *rc.borrow_mut() = v;
        } else {
            *slot = v;
        }
    }

    /// Read the value at the current cursor (root + steps), non-mutating.
    fn cursor_read(&self, top: usize) -> Value {
        self.cursor_read_steps(top, &self.cursor_steps)
    }

    fn cursor_read_steps(&self, top: usize, steps: &[CursorStep]) -> Value {
        match &self.cursor_root {
            CursorRoot::Cv(r) => read_path(&self.stack[top].regs[*r as usize], steps),
            CursorRoot::Cell(rc) => read_path(&rc.borrow(), steps),
        }
    }

    /// Write `v` at the current cursor (root + steps).
    fn cursor_write(&mut self, top: usize, steps: &[CursorStep], v: Value) -> Result<(), Thrown> {
        // `$obj[$k] = v` / `$obj[] = v` on an ArrayAccess object -> offsetSet.
        if steps.len() == 1 {
            if let (Some(h), Some(key)) = (self.cursor_root_obj(top), aa_key(&steps[0])) {
                let cls = h.borrow().class;
                if self.class_implements(cls, b"ArrayAccess") {
                    let m = self.lookup_method(cls, b"offsetset").unwrap();
                    self.call_sync(&m, Some(h), cls, vec![key, v])?;
                    return Ok(());
                }
            }
        }
        match &self.cursor_root {
            CursorRoot::Cv(r) => {
                let r = *r as usize;
                assign_path(&mut self.stack[top].regs[r], steps, v)
            }
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                let r = assign_path(&mut rc.borrow_mut(), steps, v);
                r
            }
        }
    }

    /// The deref'd object at the cursor root, if any (for ArrayAccess dispatch).
    fn cursor_root_obj(&self, top: usize) -> Option<ObjectHandle> {
        let v = match &self.cursor_root {
            CursorRoot::Cv(r) => self.stack[top].regs[*r as usize].clone(),
            CursorRoot::Cell(rc) => rc.borrow().clone(),
        };
        match deref(&v) {
            Value::Object(h) => Some(h),
            _ => None,
        }
    }

    /// Source line of the instruction currently executing in frame `top`.
    fn current_line(&self, top: usize) -> u32 {
        let f = &self.stack[top];
        let pc = f.pc.saturating_sub(1) as u32;
        let lines = &f.proto.debug.lines;
        // Largest (pc,line) entry whose pc <= the current pc.
        let idx = lines.partition_point(|(p, _)| (*p as u32) <= pc);
        if idx == 0 { 0 } else { lines[idx - 1].1 }
    }

    /// The source file name of frame `top` (for exception getFile()).
    fn current_file(&self, top: usize) -> Vec<u8> {
        self.stack[top].proto.debug.file.as_bytes().to_vec()
    }

    /// The `$this` of the current frame, if it is bound to an object.
    fn current_this(&self, top: usize) -> Option<ObjectHandle> {
        let cv = self.stack[top].proto.this_cv?;
        match &self.stack[top].regs[cv as usize] {
            Value::Object(h) => Some(h.clone()),
            _ => None,
        }
    }

    fn cursor_unset(&mut self, top: usize, steps: &[CursorStep]) {
        // `unset($obj[$k])` on an ArrayAccess object -> offsetUnset.
        if steps.len() == 1 {
            if let (Some(h), Some(CursorStep::Key(k))) = (self.cursor_root_obj(top), steps.first()) {
                let cls = h.borrow().class;
                if self.class_implements(cls, b"ArrayAccess") {
                    if let Some(m) = self.lookup_method(cls, b"offsetunset") {
                        let _ = self.call_sync(&m, Some(h), cls, vec![k.clone()]);
                    }
                    return;
                }
            }
        }
        match &self.cursor_root {
            CursorRoot::Cv(r) => unset_path(&mut self.stack[top].regs[*r as usize], steps),
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                unset_path(&mut rc.borrow_mut(), steps);
            }
        }
    }

    /// Promote a CV slot to a shared reference cell, returning the cell. If the
    /// slot already holds a `Ref`, its cell is reused.
    fn promote(&mut self, top: usize, cv: Reg) -> Rc<RefCell<Value>> {
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
                        self.stack.clear();
                        return Err(thrown); // Exit / Unsupported: straight to the SAPI
                    }
                    match self.unwind(thrown, 0)? {
                        UnwindResult::Caught => {} // handler installed; re-enter
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
                Instr::LoadNull { dst } => self.reg_set(top, dst, Value::Null),
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
                Instr::BitAnd { dst, a, b } => self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::And)),
                Instr::BitOr { dst, a, b } => self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Or)),
                Instr::BitXor { dst, a, b } => self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Xor)),
                Instr::Shl { dst, a, b } => self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Shl)),
                Instr::Shr { dst, a, b } => self.bin_inf(top, dst, a, b, |x, y| ops::bitwise(x, y, ops::BitOp::Shr)),
                Instr::Concat { dst, a, b } => {
                    let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
                    let v = if matches!(x, Value::Object(_)) || matches!(y, Value::Object(_)) {
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
                Instr::IncDec { dst, cv, pre, delta } => {
                    let cur = self.reg_get(top, cv);
                    let new = inc_dec_value(&cur, delta as i64);
                    self.cv_assign(top, cv, new.clone());
                    self.reg_set(top, dst, if pre { new } else { cur });
                }

                Instr::CastInt { dst, a } => {
                    let v = self.reg_get(top, a);
                    self.reg_set(top, dst, Value::Int(conv::to_int(&v)));
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
                    self.reg_set(top, dst, cast_to_array(v));
                }
                Instr::CastObject { dst, a } => {
                    let v = self.reg_get(top, a);
                    let obj = self.cast_to_object(v);
                    self.reg_set(top, dst, obj);
                }

                Instr::Cmp { dst, a, b, kind } => {
                    let (x, y) = (self.reg_get(top, a), self.reg_get(top, b));
                    self.reg_set(top, dst, eval_cmp(kind, &x, &y));
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
                    if self.stack[top].argc > index as usize {
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

                Instr::Call { dst, name, base, argc, fallback } => {
                    let (user, fname) = self.resolve_fn(name, fallback, &proto.names);
                    if let Some(reason) = builtins::is_refused(
                        std::str::from_utf8(&fname).unwrap_or(""),
                    ) {
                        let shown = String::from_utf8_lossy(&fname).into_owned();
                        return Err(Thrown::unsupported(format!("{shown}()"), reason));
                    }
                    if let Some(fnidx) = user {
                        // User function: push a frame (args bound to its CVs).
                        let callee = self.functions[fnidx].clone();
                        self.enter_user_call(top, callee, dst, base, argc)?;
                    } else {
                        // Builtin.
                        let name_str = String::from_utf8_lossy(&fname).into_owned();
                        let out = self.call_builtin(top, &name_str, base, argc)?;
                        self.reg_set(top, dst, out);
                    }
                }

                Instr::MakeClosure { dst, fnidx, base, ncaptures } => {
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
                    let gname = proto.names[name as usize].as_bytes().to_vec();
                    // Share the global's cell: {main}'s slot if it is a top-level
                    // variable, else an entry in the overflow table.
                    let cell = match self.main_cv_names.get(&gname).copied() {
                        Some(mreg) => self.promote(0, mreg),
                        None => self
                            .globals
                            .entry(gname)
                            .or_insert_with(|| Rc::new(RefCell::new(Value::Null)))
                            .clone(),
                    };
                    self.stack[top].regs[cv as usize] = Value::Ref(cell);
                }
                Instr::BindStatic { cv, name, init } => {
                    let sname = proto.names[name as usize].as_bytes().to_vec();
                    let key = (Rc::as_ptr(&proto) as usize, sname);
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
                }
                Instr::MakeFcc { dst, callee } => {
                    let c = self.reg_get(top, callee);
                    let cd = Rc::new(rhp_values::ClosureData {
                        fnidx: u32::MAX,
                        captures: Vec::new(),
                        bound_this: None,
                        bound_class: None,
                        bound_static: None,
                        wrapped: Some(Box::new(c)),
                    });
                    self.reg_set(top, dst, Value::Closure(cd));
                }
                Instr::CallDyn { dst, callee, base, argc } => {
                    let c = self.reg_get(top, callee);
                    let args: Vec<Value> = (0..argc).map(|i| self.reg_get(top, base + i)).collect();
                    let out = self.call_callable_value(&c, args)?;
                    self.reg_set(top, dst, out);
                }
                Instr::CallDynV { dst, callee, args } => {
                    let c = self.reg_get(top, callee);
                    let arr = self.reg_get(top, args);
                    let argvec = self.spread_args(&arr);
                    let out = self.call_callable_value(&c, argvec)?;
                    self.reg_set(top, dst, out);
                }

                // -- array element read --
                Instr::DimFetch { dst, arr, key } => {
                    let a = self.reg_get(top, arr);
                    let k = self.reg_get(top, key);
                    // `$obj[$k]` on an ArrayAccess object dispatches offsetGet.
                    if let Value::Object(h) = &a {
                        if self.class_implements(h.borrow().class, b"ArrayAccess") {
                            let m = self.lookup_method(h.borrow().class, b"offsetget").unwrap();
                            let r = self.call_sync(&m, Some(h.clone()), h.borrow().class, vec![k])?;
                            self.reg_set(top, dst, r);
                            continue;
                        }
                    }
                    self.reg_set(top, dst, dim_fetch(&a, &k));
                }

                // -- lvalue cursor --
                Instr::LvalCv { cv } => {
                    self.cursor_root = CursorRoot::Cv(cv);
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
                Instr::LvalStaticProp { class, name } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = proto.names[name as usize].as_bytes().to_vec();
                    let cell = self.static_prop_cell(cls, &pname)?;
                    self.cursor_root = CursorRoot::Cell(cell);
                    self.cursor_steps.clear();
                }
                Instr::FetchLvalRw { dst } => {
                    let v = self.cursor_read(top);
                    self.reg_set(top, dst, v); // cursor stays open for the AssignLval
                }
                Instr::AssignLval { src } => {
                    let v = self.reg_get(top, src);
                    let steps = std::mem::take(&mut self.cursor_steps);
                    self.cursor_write(top, &steps, v)?;
                }
                Instr::MakeRefLval { dst } => {
                    let steps = std::mem::take(&mut self.cursor_steps);
                    let cell = match &self.cursor_root {
                        CursorRoot::Cv(r) => {
                            let r = *r as usize;
                            make_ref_path(&mut self.stack[top].regs[r], &steps)?
                        }
                        CursorRoot::Cell(rc) => {
                            let rc = rc.clone();
                            let c = make_ref_path(&mut rc.borrow_mut(), &steps)?;
                            c
                        }
                    };
                    self.reg_set(top, dst, Value::Ref(cell));
                }
                Instr::UnsetLval => {
                    let steps = std::mem::take(&mut self.cursor_steps);
                    self.cursor_unset(top, &steps);
                }
                Instr::IncDecLval { dst, pre, delta } => {
                    let steps = std::mem::take(&mut self.cursor_steps);
                    let old = self.cursor_read_steps(top, &steps);
                    let new = inc_dec_value(&old, delta as i64);
                    self.cursor_write(top, &steps, new.clone())?;
                    self.reg_set(top, dst, if pre { new } else { old });
                }

                // -- foreach --
                Instr::FeReset { arr_reg, pos_reg, src, end } => {
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
                                self.gen_ensure_started(oid)?;
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
                Instr::FeFetch { arr_reg, pos_reg, key_cv, val_cv, end } => {
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
                                let (k, v) = (g.current_key.clone(), g.current_val.clone());
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
                            let (k, v) = (k.clone(), deref(&v).clone());
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

                Instr::FeResetRef { arr_cv, pos_reg, end } => {
                    let nonempty = match &self.stack[top].regs[arr_cv as usize] {
                        Value::Array(a) => !a.is_empty(),
                        Value::Ref(rc) => matches!(&*rc.borrow(), Value::Array(a) if !a.is_empty()),
                        _ => false,
                    };
                    if nonempty {
                        self.reg_set(top, pos_reg, Value::Int(0));
                    } else {
                        self.stack[top].pc = end as usize;
                    }
                }
                Instr::FeFetchRef { arr_cv, pos_reg, key_cv, val_cv, end } => {
                    let pos = match self.reg_get(top, pos_reg) {
                        Value::Int(n) => n as usize,
                        _ => 0,
                    };
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
                            self.reg_set(top, val_cv, Value::Ref(elem_rc));
                            if key_cv != Reg::MAX {
                                self.reg_set(top, key_cv, array_key_to_value(&k));
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
                Instr::FinallyEnd => match self.stack[top].pending.take() {
                    Some(FinallyCont::Rethrow(t)) => return Err(t),
                    Some(FinallyCont::Return(v)) => {
                        if let Some(done) = self.do_return(v) {
                            return Ok(done);
                        }
                    }
                    None => {}
                },
                Instr::SetPendingRet { src } => {
                    let v = self.reg_get(top, src);
                    self.stack[top].pending = Some(FinallyCont::Return(v));
                }

                // -- OOP --
                Instr::New { dst, class, base, argc, .. } => {
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
                    let obj = self.new_object(cls);
                    // Throwables record their creation site for getLine()/getFile().
                    if self.class_implements(cls, b"Throwable") {
                        let line = self.current_line(top);
                        let file = self.current_file(top);
                        let mut b = obj.borrow_mut();
                        b.properties.insert(rhp_values::PhpStr::from("line"), Value::Int(line as i64));
                        b.properties.insert(rhp_values::PhpStr::from("file"), Value::str(file));
                    }
                    self.reg_set(top, dst, Value::Object(obj.clone()));
                    // Run __construct, discarding its return (into the arg window).
                    if let Some(m) = self.lookup_method(cls, b"__construct") {
                        self.enter_method(top, &m, Some(obj), cls, Reg::MAX, base, argc)?;
                    }
                }
                Instr::FetchProp { dst, obj, name, .. } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let v = match o {
                        Value::Object(h) => {
                            let cls = h.borrow().class;
                            if let Some(pe) = self.class(cls).props.get(&pname) {
                                let (vis, decl) = (pe.visibility, pe.declaring_class);
                                if !self.check_access(self.stack[top].defining_class, decl, vis) {
                                    return Err(self.access_err("property", decl, pname.as_bytes(), vis));
                                }
                            }
                            match h.borrow().properties.get(&pname).cloned() {
                                Some(v) => v,
                                // Undefined property: __get fallback, else Null.
                                None => match self.lookup_method(cls, b"__get") {
                                    Some(getter) => {
                                        let arg = Value::str(pname.as_bytes().to_vec());
                                        self.call_sync(&getter, Some(h.clone()), cls, vec![arg])?
                                    }
                                    None => Value::Null,
                                },
                            }
                        }
                        _ => Value::Null, // php warns "property on <type>"
                    };
                    self.reg_set(top, dst, v);
                }
                Instr::StoreProp { obj, name, src } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let val = self.reg_get(top, src);
                    if let Value::Object(h) = o {
                        let cls = h.borrow().class;
                        let declared = self.class(cls).props.get(&pname).map(|pe| (pe.visibility, pe.declaring_class));
                        let accessible = declared.map_or(true, |(vis, decl)| {
                            self.check_access(self.stack[top].defining_class, decl, vis)
                        });
                        if declared.is_none() || !accessible {
                            // Undefined or inaccessible: route to __set if present.
                            if let Some(setter) = self.lookup_method(cls, b"__set") {
                                let nm = Value::str(pname.as_bytes().to_vec());
                                self.call_sync(&setter, Some(h), cls, vec![nm, val])?;
                                continue;
                            }
                            if !accessible {
                                let (vis, decl) = declared.unwrap();
                                return Err(self.access_err("property", decl, pname.as_bytes(), vis));
                            }
                        }
                        // Direct write, honouring an existing by-ref property slot.
                        let existing_ref = match h.borrow().properties.get(&pname) {
                            Some(Value::Ref(rc)) => Some(rc.clone()),
                            _ => None,
                        };
                        match existing_ref {
                            Some(rc) => *rc.borrow_mut() = val,
                            None => {
                                h.borrow_mut().properties.insert(pname, val);
                            }
                        }
                    }
                }
                Instr::IssetProp { dst, obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    let result = match o {
                        Value::Object(h) => {
                            let present = h.borrow().properties.get(&pname).cloned();
                            match present {
                                // A declared/dynamic property: set iff non-null.
                                Some(v) => !matches!(deref(&v), Value::Null),
                                None => {
                                    let cls = h.borrow().class;
                                    match self.lookup_method(cls, b"__isset") {
                                        Some(m) => {
                                            let nm = Value::str(pname.as_bytes().to_vec());
                                            let r = self.call_sync(&m, Some(h.clone()), cls, vec![nm])?;
                                            conv::to_bool(&r)
                                        }
                                        None => false,
                                    }
                                }
                            }
                        }
                        _ => false,
                    };
                    self.reg_set(top, dst, Value::Bool(result));
                }
                Instr::UnsetProp { obj, name } => {
                    let o = self.reg_get(top, obj);
                    let pname = proto.names[name as usize].clone();
                    if let Value::Object(h) = o {
                        let present = h.borrow().properties.contains_key(&pname);
                        if present {
                            h.borrow_mut().properties.shift_remove(&pname);
                        } else {
                            let cls = h.borrow().class;
                            if let Some(m) = self.lookup_method(cls, b"__unset") {
                                let nm = Value::str(pname.as_bytes().to_vec());
                                self.call_sync(&m, Some(h), cls, vec![nm])?;
                            }
                        }
                    }
                }
                Instr::FetchStaticProp { dst, class, name, .. } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let pname = proto.names[name as usize].as_bytes().to_vec();
                    if let Some(sp) = self.class(cls).static_props.get(&pname) {
                        let (vis, decl) = (sp.visibility, sp.declaring_class);
                        if !self.check_access(self.stack[top].defining_class, decl, vis) {
                            return Err(self.access_err("property", decl, &pname, vis));
                        }
                    }
                    let cell = self.static_prop_cell(cls, &pname)?;
                    let v = cell.borrow().clone();
                    self.reg_set(top, dst, v);
                }
                Instr::FetchClassConst { dst, class, name, .. } => {
                    let name_b = proto.names[name as usize].as_bytes().to_vec();
                    let cls = self.resolve_class_autoload(top, class)?;
                    if name_b == b"class" {
                        let v = Value::str(self.class(cls).name.as_bytes().to_vec());
                        self.reg_set(top, dst, v);
                    } else {
                        match self.class(cls).constants.get(&name_b) {
                            Some(c) => {
                                let (vis, decl) = (c.visibility, c.declaring_class);
                                if !self.check_access(self.stack[top].defining_class, decl, vis) {
                                    return Err(self.access_err("constant", decl, &name_b, vis));
                                }
                                let v = c.value.clone();
                                self.reg_set(top, dst, v);
                            }
                            None => {
                                return Err(Thrown::error(
                                    "Error",
                                    format!(
                                        "Undefined constant {}::{}",
                                        String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                        String::from_utf8_lossy(&name_b)
                                    ),
                                ))
                            }
                        }
                    }
                }
                Instr::CallMethod { dst, obj, name, base, argc, .. } => {
                    let o = self.reg_get(top, obj);
                    let Value::Object(h) = o else {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Call to a member function {}() on {}",
                                String::from_utf8_lossy(proto.names[name as usize].as_bytes()),
                                type_lc(&o)
                            ),
                        ));
                    };
                    let cls = h.borrow().class;
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    // Built-in Generator methods (current/next/send/valid/…).
                    let gid = h.borrow().id;
                    if self.generators.contains_key(&gid) {
                        let args: Vec<Value> = (0..argc).map(|i| self.reg_get(top, base + i)).collect();
                        if let Some(r) = self.generator_method(gid, &mname.to_ascii_lowercase(), &args) {
                            let r = r?;
                            self.reg_set(top, dst, r);
                            continue;
                        }
                    }
                    let found = self.lookup_method(cls, &mname);
                    let accessible = found.as_ref().is_some_and(|m| {
                        self.check_access(self.stack[top].defining_class, m.defining_class, m.visibility)
                    });
                    if let (Some(m), true) = (&found, accessible) {
                        let m = m.clone();
                        self.enter_method(top, &m, Some(h.clone()), cls, dst, base, argc)?;
                    } else if let Some(magic) = self.lookup_method(cls, b"__call") {
                        // Undefined or inaccessible method -> __call($name, $args).
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
                }
                Instr::CallStatic { dst, class, name, base, argc, .. } => {
                    let cls = self.resolve_class_autoload(top, class)?;
                    let mname = proto.names[name as usize].as_bytes().to_vec();
                    let found = self.lookup_method(cls, &mname);
                    // Enum built-in static methods (cases/from/tryFrom) when not
                    // overridden by a user-defined method.
                    if found.is_none() && self.class(cls).kind == ClassKind::Enum {
                        let lc = mname.to_ascii_lowercase();
                        if matches!(lc.as_slice(), b"cases" | b"from" | b"tryfrom") {
                            let args: Vec<Value> =
                                (0..argc).map(|i| self.reg_get(top, base + i)).collect();
                            let r = self.enum_builtin_static(cls, &lc, args)?;
                            self.reg_set(top, dst, r);
                            continue;
                        }
                    }
                    let accessible = found.as_ref().is_some_and(|m| {
                        self.check_access(self.stack[top].defining_class, m.defining_class, m.visibility)
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
                            continue;
                        }
                        return match found {
                            Some(m) => {
                                Err(self.access_err("method", m.defining_class, &mname, m.visibility))
                            }
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
                    self.enter_method(top, &m, this, called, dst, base, argc)?;
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
                    let Value::Object(h) = v else {
                        return Err(Thrown::error("Error", "__clone method called on non-object"));
                    };
                    let (cls, props) = {
                        let od = h.borrow();
                        (od.class, od.properties.clone())
                    };
                    let id = self.next_object_id;
                    self.next_object_id += 1;
                    let new_obj: ObjectHandle =
                        Rc::new(RefCell::new(ObjectData { class: cls, id, properties: props }));
                    self.reg_set(top, dst, Value::Object(new_obj.clone()));
                    if let Some(m) = self.lookup_method(cls, b"__clone") {
                        self.enter_method(top, &m, Some(new_obj), cls, Reg::MAX, src, 0)?;
                    }
                }
                Instr::DeclareClass { .. } => {
                    return Err(Thrown::unsupported(
                        "conditional class declaration",
                        "only top-level class declarations for now",
                    ))
                }

                Instr::BindRef { dst_cv, src_cv } => {
                    let rc = self.promote(top, src_cv);
                    self.stack[top].regs[dst_cv as usize] = Value::Ref(rc);
                }
                Instr::MakeRef { dst, cv } => {
                    let rc = self.promote(top, cv);
                    self.stack[top].regs[dst as usize] = Value::Ref(rc);
                }

                Instr::CallV { dst, name, args, fallback } => {
                    let (user, fname) = self.resolve_fn(name, fallback, &proto.names);
                    if let Some(reason) =
                        builtins::is_refused(std::str::from_utf8(&fname).unwrap_or(""))
                    {
                        let shown = String::from_utf8_lossy(&fname).into_owned();
                        return Err(Thrown::unsupported(format!("{shown}()"), reason));
                    }
                    let arr = self.reg_get(top, args);
                    let argv = self.spread_args(&arr);
                    if let Some(fnidx) = user {
                        let callee = self.functions[fnidx].clone();
                        self.push_user_frame(callee, argv, dst)?;
                    } else {
                        let name_str = String::from_utf8_lossy(&fname).into_owned();
                        let out = self.invoke_builtin(&name_str, argv)?;
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
                    let obj = self.new_object(cls);
                    self.reg_set(top, dst, Value::Object(obj.clone()));
                    if let Some(m) = self.lookup_method(cls, b"__construct") {
                        let arr = self.reg_get(top, args);
                        let argv = self.spread_args(&arr);
                        self.push_method_frame(&m, Some(obj), cls, argv, Reg::MAX)?;
                    }
                }
                Instr::CallMethodV { dst, obj, name, args } => {
                    let o = self.reg_get(top, obj);
                    let Value::Object(h) = o else {
                        return Err(Thrown::type_error("Call to a member function on non-object"));
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
                    if !self.check_access(self.stack[top].defining_class, m.defining_class, m.visibility) {
                        return Err(self.access_err("method", m.defining_class, &mname, m.visibility));
                    }
                    let arr = self.reg_get(top, args);
                    let argv = self.spread_args(&arr);
                    self.push_method_frame(&m, Some(h.clone()), cls, argv, dst)?;
                }
                Instr::CallStaticV { dst, class, name, args } => {
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
                    let argv = self.spread_args(&arr);
                    self.push_method_frame(&m, this, called, argv, dst)?;
                }

                Instr::FetchConst { dst, name, fallback } => {
                    let cname = proto.names[name as usize].as_bytes().to_vec();
                    let v = self.constants.get(&cname).cloned().or_else(|| {
                        if fallback != rhp_bytecode::NO_NAME {
                            self.constants.get(proto.names[fallback as usize].as_bytes()).cloned()
                        } else {
                            None
                        }
                    });
                    match v {
                        Some(v) => {
                            self.reg_set(top, dst, v);
                        }
                        None => {
                            return Err(Thrown::error(
                                "Error",
                                format!("Undefined constant \"{}\"", String::from_utf8_lossy(&cname)),
                            ))
                        }
                    }
                }

                Instr::Refuse { feature, .. } => {
                    let f = proto.names[feature as usize].as_bytes().to_vec();
                    return Err(Thrown::unsupported(
                        String::from_utf8_lossy(&f).into_owned(),
                        "refused construct",
                    ));
                }

                Instr::Ret { src } => {
                    let mut v = self.reg_get(top, src);
                    // Enforce a declared return type (the callee's own file decides
                    // strict vs coercive).
                    if proto.return_type.is_some() {
                        v = self.check_return_type(&proto, top, v)?;
                    }
                    if let Some(done) = self.do_return(v) {
                        return Ok(done);
                    }
                }
                Instr::RetNull | Instr::Halt => {
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
                            self.out.extend_from_slice(s.as_bytes());
                            0
                        }
                        Value::Null => 0,
                        other => {
                            let s = conv::to_string_bytes(&other);
                            self.out.extend_from_slice(&s);
                            0
                        }
                    };
                    return Err(Thrown::Exit(code));
                }
                Instr::Yield { dst, key, value } => {
                    // Suspend: hand (key, value, resume-dst) to the generator
                    // driver, leaving this frame on the stack for it to reclaim.
                    let val = self.reg_get(top, value);
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
    fn unwind(&mut self, thrown: Thrown, floor: usize) -> Result<UnwindResult, Thrown> {
        loop {
            if self.stack.len() <= floor {
                return Ok(UnwindResult::Escaped(thrown)); // escaped the (sub)run
            }
            let top = self.stack.len() - 1;
            let proto = self.stack[top].proto.clone();
            // The throwing pc has already been advanced past the faulting instr.
            let throw_pc = (self.stack[top].pc as u32).saturating_sub(1);
            if let Some(region) =
                proto.exception_table.iter().find(|r| throw_pc >= r.start && throw_pc < r.end)
            {
                let region = region.clone();
                if let Some(cls) = self.thrown_class_id(&thrown) {
                    for arm in &region.catches {
                        if arm.classes.iter().any(|cn| self.class_matches(cls, cn.as_bytes())) {
                            let obj = self.thrown_to_object(&thrown)?;
                            if let Some(vr) = arm.var_reg {
                                self.stack[top].regs[vr as usize] = obj;
                            }
                            self.stack[top].pc = arm.target as usize;
                            return Ok(UnwindResult::Caught);
                        }
                    }
                }
                if let Some(fin) = region.finally {
                    self.stack[top].pending = Some(FinallyCont::Rethrow(thrown));
                    self.stack[top].pc = fin as usize;
                    return Ok(UnwindResult::Caught);
                }
            }
            self.stack.pop();
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
        let proto = method.proto.clone();
        self.run_proto_sync(proto, this, Some(method.defining_class), Some(called_class), args)
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
    ) -> Result<Value, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let mut frame = Frame::new(proto.clone(), 0);
        frame.argc = args.len();
        bind_params(&proto, &args, &mut frame.regs)?;
        self.bind_superglobals(&mut frame);
        if let (Some(cv), Some(obj)) = (proto.this_cv, this) {
            frame.regs[cv as usize] = Value::Object(obj);
        }
        frame.defining_class = defining_class;
        frame.called_class = called_class;
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
    fn do_include(&mut self, top: usize, raw_path: Vec<u8>, kind: IncludeKind) -> Result<Value, Thrown> {
        let path_str = String::from_utf8_lossy(&raw_path).into_owned();
        let (realpath, source) = match self.resolve_include_path(top, &path_str) {
            Some(found) => found,
            None => {
                if kind.is_require() {
                    return Err(Thrown::error(
                        "Error",
                        &format!("Failed opening required '{path_str}'"),
                    ));
                }
                // include of a missing file: php warns (to stderr) and returns false
                return Ok(Value::Bool(false));
            }
        };

        // `_once`: skip a file already included (count it as included before
        // running the body, so recursive `_once` includes terminate).
        if kind.is_once() && self.included_files.contains(&realpath) {
            return Ok(Value::Bool(true));
        }
        self.included_files.insert(realpath.clone());

        // Compile via the SAPI-provided callback (keeps the VM compiler-agnostic).
        let mut cb = self
            .includer
            .take()
            .ok_or_else(|| Thrown::error("Error", "file inclusion is not available"))?;
        let rp_str = String::from_utf8_lossy(&realpath).into_owned();
        let compiled = cb(&rp_str, &source);
        self.includer = Some(cb);
        let prog = compiled?;

        let main = self.install_included(&prog)?;
        let ret = self.run_included(main)?;
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
        // closures: append (rebased).
        for c in &prog.closures {
            self.closures.push(Rc::new(rebase_proto(c, base)));
        }
        // functions: a user function is fatal to redeclare (php semantics).
        for proto in &prog.functions {
            let lc = proto.name.as_bytes().to_ascii_lowercase();
            if self.fn_names.contains_key(&lc) {
                return Err(Thrown::error(
                    "Error",
                    &format!("Cannot redeclare function {}()", String::from_utf8_lossy(proto.name.as_bytes())),
                ));
            }
            let idx = self.functions.len();
            self.functions.push(Rc::new(rebase_proto(proto, base)));
            self.fn_names.insert(lc, idx);
        }
        // classes: skip the re-emitted built-in prelude (always the first N); a
        // user class past that is fatal to redeclare.
        let mut new_classes: Vec<ClassProto> = Vec::new();
        for c in prog.classes.iter().skip(prog.num_prelude_classes) {
            if self.class_ids.contains_key(&c.name.as_bytes().to_ascii_lowercase()) {
                return Err(Thrown::error(
                    "Error",
                    &format!("Cannot declare class {}, because the name is already in use", String::from_utf8_lossy(c.name.as_bytes())),
                ));
            }
            new_classes.push(rebase_class(c, base));
        }
        self.install_classes(&new_classes)?;
        // constants: define_const skips an already-defined name.
        for (n, v) in &prog.constants {
            self.define_const(n.as_bytes(), v.clone());
        }
        Ok(Rc::new(rebase_proto(&prog.main, base)))
    }

    /// Run an included `{main}` to completion (isolated scope; it shares the VM's
    /// function/class/constant tables) and return its value.
    fn run_included(&mut self, main: Rc<FunctionProto>) -> Result<Value, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let mut frame = Frame::new(main, 0);
        frame.capture = true;
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
    ) -> Result<Value, Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let proto = self.closures[cd.fnidx as usize].clone();
        let mut frame = Frame::new(proto.clone(), 0);
        frame.argc = args.len();
        bind_params(&proto, &args, &mut frame.regs)?;
        self.bind_superglobals(&mut frame);
        let strict = self.caller_strict();
        self.check_arg_types(&proto, &mut frame, strict, cd.bound_class)?;
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
                            ArrayKey::Int(n) => rhp_values::PhpStr::from_bytes(n.to_string().into_bytes()),
                            ArrayKey::Str(s) => s.clone(),
                        };
                        b.properties.insert(name, deref(val));
                    }
                }
                scalar => {
                    b.properties.insert(rhp_values::PhpStr::from("scalar"), scalar);
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

    /// Snapshot an object's accessible (public from `caller` scope) properties as
    /// an array, for `foreach` over a plain object.
    fn object_prop_array(&self, h: &ObjectHandle, top: usize) -> PhpArray {
        let caller = self.stack[top].defining_class;
        let b = h.borrow();
        let cls = b.class;
        let mut arr = PhpArray::new();
        for (name, val) in &b.properties {
            let visible = match self.class(cls).props.get(name) {
                Some(pe) => self.check_access(caller, pe.declaring_class, pe.visibility),
                None => true, // dynamic properties are public
            };
            if visible {
                arr.insert(ArrayKey::from_bytes(name.as_bytes()), deref(val).clone());
            }
        }
        arr
    }

    /// Wrap a prepared (but not pushed) frame as a `Generator` object; the body
    /// runs lazily under the generator protocol.
    fn make_generator(&mut self, mut frame: Frame) -> Value {
        frame.ret_dst = Reg::MAX;
        let cls = self.resolve_class_by_name(b"Generator").expect("Generator prelude class");
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
            (g.frame.take().expect("suspended generator frame"), g.status, g.resume_dst)
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
                Ok(self.generators[&id].current_val.clone())
            })(),
            b"key" => (|| {
                self.gen_ensure_started(id)?;
                Ok(self.generators[&id].current_key.clone())
            })(),
            b"valid" => (|| {
                self.gen_ensure_started(id)?;
                Ok(Value::Bool(self.generators[&id].status != GenStatus::Finished))
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
                Ok(self.generators[&id].current_val.clone())
            })(),
            b"rewind" => (|| {
                self.gen_ensure_started(id)?;
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
                    Thrown::error("Error", format!("{}::{} is not a backed enum", enum_name(), if is_try { "tryFrom()" } else { "from()" }))
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
                    format!("{} is not a valid backing value for enum {}", shown, enum_name()),
                ))
            }
            _ => unreachable!("enum_builtin_static called with {:?}", method_lc),
        }
    }

    /// Invoke a PHP callable value synchronously (string function name,
    /// `[obj_or_class, method]`, an `__invoke` object, or a closure).
    fn call_callable_value(&mut self, callable: &Value, args: Vec<Value>) -> Result<Value, Thrown> {
        let c = match callable {
            Value::Ref(r) => r.borrow().clone(),
            other => other.clone(),
        };
        match c {
            // First-class callable: forward to the wrapped callable.
            Value::Closure(cd) if cd.wrapped.is_some() => {
                let w = (**cd.wrapped.as_ref().unwrap()).clone();
                self.call_callable_value(&w, args)
            }
            Value::Closure(cd) => self.run_closure_sync(&cd, args),
            Value::Str(s) => {
                let lower = s.as_bytes().to_ascii_lowercase();
                if let Some(&fnidx) = self.fn_names.get(&lower) {
                    let proto = self.functions[fnidx].clone();
                    self.run_proto_sync(proto, None, None, None, args)
                } else {
                    let name = String::from_utf8_lossy(s.as_bytes()).into_owned();
                    self.invoke_builtin(&name, args)
                }
            }
            Value::Array(a) if a.len() == 2 => {
                let item0 = a.get(&ArrayKey::Int(0)).cloned().unwrap_or(Value::Null);
                let method = a.get(&ArrayKey::Int(1)).cloned().unwrap_or(Value::Null);
                let mname = conv::to_string_bytes(&method);
                match item0 {
                    Value::Object(h) => {
                        let cls = h.borrow().class;
                        let m = self.lookup_method(cls, &mname).ok_or_else(|| {
                            Thrown::error("Error", "Call to undefined method in callable")
                        })?;
                        self.call_sync(&m, Some(h), cls, args)
                    }
                    Value::Str(cn) => {
                        let cls = self.resolve_class_by_name(cn.as_bytes())?;
                        let m = self.lookup_method(cls, &mname).ok_or_else(|| {
                            Thrown::error("Error", "Call to undefined method in callable")
                        })?;
                        self.call_sync(&m, None, cls, args)
                    }
                    _ => Err(Thrown::type_error("Invalid callable array")),
                }
            }
            Value::Object(h) => {
                let cls = h.borrow().class;
                match self.lookup_method(cls, b"__invoke") {
                    Some(m) => self.call_sync(&m, Some(h), cls, args),
                    None => Err(Thrown::type_error("Object is not callable (no __invoke)")),
                }
            }
            _ => Err(Thrown::type_error("Value is not callable")),
        }
    }

    /// Stringify a value, invoking `__toString` on objects that define it.
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
                let obj = self.new_object(cls);
                obj.borrow_mut()
                    .properties
                    .insert(rhp_values::PhpStr::from("message"), Value::str(message.clone()));
                Ok(Value::Object(obj))
            }
            _ => Err(thrown.clone()),
        }
    }

    /// Pop the current frame and deliver `value`. Returns `Some(value)` when the
    /// outermost frame returns (program done), else delivers to the caller.
    fn do_return(&mut self, value: Value) -> Option<Value> {
        let frame = self.stack.pop().expect("non-empty stack");
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
            None => Some(value),
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
            return Err(Thrown::error(
                "Error",
                "Maximum call stack depth exceeded",
            ));
        }
        // Raw (non-deref) copy: by-ref args arrive as `Value::Ref` and must be
        // kept as references; by-value binding derefs in `bind_params`.
        let args: Vec<Value> =
            (0..argc).map(|i| self.stack[top].regs[(base + i) as usize].clone()).collect();
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
        frame.argc = args.len();
        bind_params(&callee, &args, &mut frame.regs)?;
        self.bind_superglobals(&mut frame);
        let strict = self.caller_strict();
        self.check_arg_types(&callee, &mut frame, strict, None)?;
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

    /// Flatten an array value into a positional argument vector (for spread).
    fn spread_args(&self, arr: &Value) -> Vec<Value> {
        match arr {
            Value::Array(a) => a.iter().map(|(_, v)| v.clone()).collect(),
            Value::Ref(r) => self.spread_args(&r.borrow()),
            _ => Vec::new(),
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
        name: &str,
        base: Reg,
        argc: u16,
    ) -> Result<Value, Thrown> {
        // By-ref builtins (sort/array_push/preg_match/…) need their by-reference
        // positions kept as the `Value::Ref` the call site built, so they can
        // write back to the caller's variable. Other args deref to plain values.
        let ref_mask = rhp_bytecode::builtin_ref_params(&name.to_ascii_lowercase());
        let args: Vec<Value> = (0..argc)
            .map(|i| {
                if ref_mask.get(i as usize).copied().unwrap_or(false) {
                    self.stack[top].regs[(base + i) as usize].clone()
                } else {
                    self.reg_get(top, base + i)
                }
            })
            .collect();
        self.invoke_builtin(name, args)
    }

    /// Call a builtin by name with a pre-collected argument vector.
    fn invoke_builtin(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Thrown> {
        let Some(builtin) = builtins::lookup(name) else {
            return Err(Thrown::error("Error", format!("Call to undefined function {name}()")));
        };
        if args.len() < builtin.min_args {
            return Err(Thrown::error(
                "ArgumentCountError",
                format!(
                    "Too few arguments to function {name}(), {} passed and at least {} expected",
                    args.len(),
                    builtin.min_args
                ),
            ));
        }
        if let Some(max) = builtin.max_args {
            if args.len() > max {
                return Err(Thrown::error(
                    "ArgumentCountError",
                    format!("Too many arguments to function {name}()"),
                ));
            }
        }
        (builtin.func)(self, &args)
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

/// The lone scalar kind of a type (for coercive coercion), or None if there is
/// not exactly one scalar member.
fn scalar_target(ty: &rhp_bytecode::TypeHint) -> Option<&rhp_bytecode::TypeKind> {
    use rhp_bytecode::TypeKind as K;
    let mut found = None;
    for k in &ty.kinds {
        if matches!(k, K::Int | K::Float | K::String | K::Bool) {
            if found.is_some() {
                return None; // more than one scalar → ambiguous
            }
            found = Some(k);
        }
    }
    found
}

/// Coerce a scalar value to a scalar kind (php coercive-mode rules); None if not
/// coercible (e.g. array/object/non-numeric-string to int).
fn coerce_scalar(d: &Value, k: &rhp_bytecode::TypeKind) -> Option<Value> {
    use rhp_bytecode::TypeKind as K;
    use rhp_values::conv::Num;
    match k {
        K::Int => match d {
            Value::Bool(b) => Some(Value::Int(*b as i64)),
            Value::Float(f) => Some(Value::Int(*f as i64)),
            Value::Str(s) => conv::scan_number(s.as_bytes()).map(|(num, _)| {
                Value::Int(match num {
                    Num::Int(n) => n,
                    Num::Float(f) => f as i64,
                })
            }),
            _ => None,
        },
        K::Float => match d {
            Value::Bool(b) => Some(Value::Float(*b as i64 as f64)),
            Value::Int(n) => Some(Value::Float(*n as f64)),
            Value::Str(s) => conv::scan_number(s.as_bytes()).map(|(num, _)| Value::Float(num.as_f64())),
            _ => None,
        },
        K::String => match d {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) => Some(Value::str(conv::to_string_bytes(d))),
            _ => None,
        },
        K::Bool => match d {
            Value::Int(_) | Value::Float(_) | Value::Str(_) => Some(Value::Bool(conv::to_bool(d))),
            _ => None,
        },
        _ => None,
    }
}

/// Human-readable form of a declared type, for TypeError messages.
fn type_string(ty: &rhp_bytecode::TypeHint) -> String {
    use rhp_bytecode::TypeKind as K;
    let sep = if ty.intersection { "&" } else { "|" };
    let mut parts: Vec<String> = ty
        .kinds
        .iter()
        .map(|k| match k {
            K::Int => "int".into(),
            K::Float => "float".into(),
            K::String => "string".into(),
            K::Bool => "bool".into(),
            K::Array => "array".into(),
            K::Object => "object".into(),
            K::Callable => "callable".into(),
            K::Iterable => "iterable".into(),
            K::Mixed => "mixed".into(),
            K::Void => "void".into(),
            K::Never => "never".into(),
            K::Null => "null".into(),
            K::True => "true".into(),
            K::False => "false".into(),
            K::SelfType => "self".into(),
            K::StaticType => "static".into(),
            K::Parent => "parent".into(),
            K::Named(n) => String::from_utf8_lossy(n.as_bytes()).into_owned(),
        })
        .collect();
    if ty.nullable && ty.kinds.len() == 1 && !matches!(ty.kinds[0], K::Null) {
        return format!("?{}", parts[0]);
    }
    if ty.nullable {
        parts.push("null".into());
    }
    parts.join(sep)
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

/// Clone a proto, stamping its `closure_base` so its file-local `MakeClosure`
/// indices resolve into the VM's flat closure table (whose tail holds this
/// file's closures). No instruction rewriting — the base is applied at run time.
fn rebase_proto(proto: &FunctionProto, base: u32) -> FunctionProto {
    let mut p = proto.clone();
    p.closure_base = base;
    p
}

/// Stamp `closure_base` on a class's method and initializer-thunk protos (their
/// bodies can create closures) when merging an included program.
fn rebase_class(c: &ClassProto, base: u32) -> ClassProto {
    let mut c = c.clone();
    for m in &mut c.methods {
        m.proto = Rc::new(rebase_proto(&m.proto, base));
    }
    for p in &mut c.props {
        if let PropDefaultProto::Init(t) = &p.default {
            p.default = PropDefaultProto::Init(Rc::new(rebase_proto(t, base)));
        }
    }
    for sp in &mut c.static_props {
        if let ConstInitProto::Thunk(t) = &sp.init {
            sp.init = ConstInitProto::Thunk(Rc::new(rebase_proto(t, base)));
        }
    }
    for k in &mut c.constants {
        if let ConstInitProto::Thunk(t) = &k.value {
            k.value = ConstInitProto::Thunk(Rc::new(rebase_proto(t, base)));
        }
    }
    c
}

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        other => other.clone(),
    }
}

/// `$container[$key]` read: array element (Null if absent) or string byte.
fn dim_fetch(container: &Value, key: &Value) -> Value {
    match deref(container) {
        Value::Array(a) => match ArrayKey::from_value(key) {
            Some(k) => a.get(&k).cloned().unwrap_or(Value::Null),
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
fn read_path(slot: &Value, steps: &[CursorStep]) -> Value {
    if let Value::Ref(r) = slot {
        return read_path(&r.borrow(), steps);
    }
    let Some((head, rest)) = steps.split_first() else {
        return slot.clone();
    };
    match (slot, head) {
        (Value::Array(a), CursorStep::Key(k)) => match ArrayKey::from_value(k) {
            Some(key) => a.get(&key).map(|c| read_path(c, rest)).unwrap_or(Value::Null),
            None => Value::Null,
        },
        (Value::Object(h), CursorStep::Prop(name)) => {
            let od = h.borrow();
            od.properties.get(name).map(|c| read_path(c, rest)).unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

/// Make `slot` an array (autovivifying null/"" ), or error on a non-empty scalar.
fn autovivify(slot: &mut Value) -> Result<(), Thrown> {
    match slot {
        Value::Array(_) => Ok(()),
        Value::Null => {
            *slot = Value::Array(PhpArray::new());
            Ok(())
        }
        Value::Str(s) if s.is_empty() => {
            *slot = Value::Array(PhpArray::new());
            Ok(())
        }
        _ => Err(Thrown::type_error("Cannot use a scalar value as an array")),
    }
}

/// Write `value` at a cursor path, autovivifying + COW-bubbling along the way.
fn assign_path(slot: &mut Value, steps: &[CursorStep], value: Value) -> Result<(), Thrown> {
    // Descend through a reference to its underlying storage.
    if let Value::Ref(rc) = slot {
        let mut inner = rc.borrow_mut();
        return assign_path(&mut inner, steps, value);
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
        let child = od.properties.entry(name.clone()).or_insert(Value::Null);
        return assign_path(child, rest, value);
    }
    autovivify(slot)?;
    let Value::Array(arr) = slot else { unreachable!("autovivify ensures array") };
    let child = match head {
        CursorStep::Key(k) => {
            let key = ArrayKey::from_value(k)
                .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
            arr.get_or_insert_mut(key)
        }
        CursorStep::Append => arr.append_mut(),
        CursorStep::Prop(_) => unreachable!("handled above"),
    };
    assign_path(child, rest, value)
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
fn make_ref_path(slot: &mut Value, steps: &[CursorStep]) -> Result<Rc<RefCell<Value>>, Thrown> {
    // Follow an existing reference: reuse it if this is the target, else descend.
    if let Value::Ref(rc) = slot {
        if steps.is_empty() {
            return Ok(rc.clone());
        }
        let rc = rc.clone();
        let r = make_ref_path(&mut rc.borrow_mut(), steps);
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
            Value::Null => return Err(Thrown::error("Error", "Attempt to reference property on null")),
            other => {
                return Err(Thrown::error(
                    "Error",
                    format!("Attempt to reference property on {}", type_lc(other)),
                ))
            }
        };
        let mut od = handle.borrow_mut();
        let child = od.properties.entry(name.clone()).or_insert(Value::Null);
        return make_ref_path(child, rest);
    }
    autovivify(slot)?;
    let Value::Array(arr) = slot else { unreachable!("autovivify ensures array") };
    let child = match head {
        CursorStep::Key(k) => {
            let key = ArrayKey::from_value(k)
                .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
            arr.get_or_insert_mut(key)
        }
        CursorStep::Append => arr.append_mut(),
        CursorStep::Prop(_) => unreachable!("handled above"),
    };
    make_ref_path(child, rest)
}

/// Remove the leaf key of a cursor path (COW), descending to the parent first.
fn unset_path(slot: &mut Value, steps: &[CursorStep]) {
    if let Value::Ref(rc) = slot {
        let mut inner = rc.borrow_mut();
        return unset_path(&mut inner, steps);
    }
    let Some((head, rest)) = steps.split_first() else { return };
    match head {
        CursorStep::Prop(name) => {
            let Value::Object(h) = slot else { return };
            let h = h.clone();
            let mut od = h.borrow_mut();
            if rest.is_empty() {
                od.properties.shift_remove(name);
            } else if let Some(child) = od.properties.get_mut(name) {
                unset_path(child, rest);
            }
        }
        CursorStep::Key(k) => {
            let Value::Array(arr) = slot else { return };
            let Some(key) = ArrayKey::from_value(k) else { return };
            if rest.is_empty() {
                arr.remove(&key);
            } else if let Some(child) = arr.get_mut(&key) {
                unset_path(child, rest);
            }
        }
        CursorStep::Append => {}
    }
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

fn array_key_to_value(k: &ArrayKey) -> Value {
    match k {
        ArrayKey::Int(n) => Value::Int(*n),
        ArrayKey::Str(s) => Value::Str(s.clone()),
    }
}

/// Bind positional `args` to a callee's parameter CV registers (by value), with
/// constant defaults and variadic collection. By-ref params and named args are
/// not yet supported.
fn bind_params(proto: &FunctionProto, args: &[Value], regs: &mut [Value]) -> Result<(), Thrown> {
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
            regs[p.cv as usize] = if p.by_ref { args[i].clone() } else { deref(&args[i]) };
        } else if let Some(def) = &p.default {
            match def {
                ParamDefault::Const(k) => regs[p.cv as usize] = proto.consts[*k as usize].clone(),
                ParamDefault::Init(_) => {} // non-constant defaults: deferred
            }
        } else {
            return Err(Thrown::error(
                "ArgumentCountError",
                format!(
                    "Too few arguments to function {}(), {} passed and at least {} expected",
                    String::from_utf8_lossy(proto.name.as_bytes()),
                    args.len(),
                    required_params(proto),
                ),
            ));
        }
    }
    Ok(())
}

fn required_params(proto: &FunctionProto) -> usize {
    proto.params.iter().filter(|p| p.default.is_none() && !p.variadic).count()
}

fn eval_cmp(kind: CmpKind, a: &Value, b: &Value) -> Value {
    match kind {
        CmpKind::Eq => Value::Bool(ops::loose_eq(a, b)),
        CmpKind::Ne => Value::Bool(!ops::loose_eq(a, b)),
        CmpKind::Identical => Value::Bool(ops::strict_eq(a, b)),
        CmpKind::NotIdentical => Value::Bool(!ops::strict_eq(a, b)),
        CmpKind::Lt => Value::Bool(ops::loose_compare(a, b).is_lt()),
        CmpKind::Le => Value::Bool(ops::loose_compare(a, b).is_le()),
        CmpKind::Gt => Value::Bool(ops::loose_compare(a, b).is_gt()),
        CmpKind::Ge => Value::Bool(ops::loose_compare(a, b).is_ge()),
        CmpKind::Spaceship => ops::spaceship(a, b),
    }
}
