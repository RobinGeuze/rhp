//! The register bytecode VM.
//!
//! Executes a `rhp_bytecode::Program`. Every arithmetic/compare/concat/cast
//! opcode is a thin wrapper over the verified `rhp_values::ops` / value
//! `conv` functions, and calls go through `rhp_builtins`, so operator and
//! builtin semantics live in one place shared with the rest of the engine.
//!
//! `dispatch` runs the instruction loop. `calls`, `props`, `lvalue`, `types`,
//! `generators` and `include` hold what its handlers call; `errors` raises
//! diagnostics and unwinds exceptions; `lifecycle` builds the engine and runs
//! the shutdown sequence; `host` and `streams` serve the builtins.

mod calls;
mod classes;
mod closures;
mod compare;
mod const_link;
mod dispatch;
mod errors;
mod generators;
mod hooks;
mod host;
mod include;
mod inherit;
mod lazy;
mod lifecycle;
mod link_checks;
mod lvalue;
mod memory;
mod opcache;
mod output;
mod props;
mod reflect;
mod request;
mod streams;
mod stub_default;
mod types;
mod zpp;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rhp_builtins as builtins;
use rhp_builtins::ini_parse_bool as ini_bool;
use rhp_bytecode::{ClassProto, FunctionProto, Reg};
use rhp_values::ops::deref;
use rhp_values::{ClassId, ObjectHandle, PhpStr, Thrown, Value};

use calls::NativeCall;
use errors::FinallyCont;
use generators::GenState;
use include::Includer;
use lvalue::{CursorRoot, CursorSteps};
use streams::StreamTable;

pub use include::IncludeError;

/// The virtual machine: call stack + global tables, running one request
/// under its SAPI.
pub struct Vm {
    /// Active magic-accessor guards: (object id, accessor, property name).
    prop_guards: std::collections::HashSet<(u32, &'static [u8], PhpStr)>,
    /// The fetch the next magic accessor or ArrayAccess call serves (php's
    /// `BP_VAR_W` below a write, `check_empty` for `empty()`); the next
    /// synchronous call takes it.
    pending_magic_fetch: builtins::MagicFetch,
    /// The fetch the running accessor serves ([`builtins::Host::magic_fetch`]), and
    /// the stack index of that accessor's frame: code it calls runs in
    /// frames above and sees a plain read.
    magic_fetch: (usize, builtins::MagicFetch),
    /// The output layer (main/output.c): the `ob_start` handler stack every
    /// write passes through on its way to [`Vm::emit_stdout`].
    output: output::Output,
    stack: Vec<Frame>,
    /// The programs installed so far: the main script is unit 0, then each
    /// included file and eval()'d string, in load order. A program installed
    /// right after one that added no closures and no class declarations
    /// shares that unit, since their bases are equal.
    units: Vec<UnitBase>,
    functions: Vec<UnitFn>,
    fn_names: HashMap<Vec<u8>, usize>, // lowercase name -> index into `functions`
    /// Bumped by every function declaration (see [`Vm::add_fn_name`]).
    fn_gen: u32,
    /// Call sites by name already resolved, keyed by (proto address, pc):
    /// php's run-time cache slots. See [`Vm::call_target`].
    call_sites: rustc_hash::FxHashMap<(usize, u32), calls::CallSite>,
    /// Every unit's closure protos (and conditional functions), indexed by
    /// `ClosureData::fnidx`: a unit's `MakeClosure`/`DeclareFunc` operand
    /// plus its [`UnitBase::closures`].
    closures: Vec<UnitFn>,
    /// Flat table of user class declarations (a unit's `DeclareClass`
    /// operand plus its [`UnitBase::classes`]): each file's top-level
    /// class-likes then its nested ones.
    decl_classes: Vec<DeclSlot>,
    /// `decl_classes` slots declared by an executed `DeclareClass` (not
    /// early-bound): running that declaration again is a redeclaration.
    decl_runtime: std::collections::HashSet<usize>,
    /// Per class id: its position in php's class table — the `decl_classes`
    /// slot it was compiled into (php adds every declaration to the table
    /// at compile time and renames it in place when it is declared), so
    /// `get_declared_classes()` lists classes in compile order.
    class_order: Vec<usize>,
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
    // lvalue cursor: a root in the current frame (CV or a static-prop cell) +
    // pending path steps. Cursor opcodes are emitted contiguously, so a single
    // cursor never nests.
    cursor_root: CursorRoot,
    cursor_steps: CursorSteps,
    // The global's name when the open cursor is rooted at `$GLOBALS[name]`
    // (`$GLOBALS[name] = &$v` rebinds the symbol-table entry itself).
    cursor_global: Option<Vec<u8>>,
    // OOP class table (see `classes` module).
    classes: Vec<classes::ClassEntry>,
    class_ids: rustc_hash::FxHashMap<Vec<u8>, ClassId>, // lowercase name -> id
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
    // `static $x`: persistent cells keyed by (function proto pointer, unit,
    // class, var name).
    statics: HashMap<StaticKey, Rc<RefCell<Value>>>,
    // The statics of the running include and eval `{main}` frames, keyed by
    // stack index; each frame's are dropped when it returns.
    include_statics: HashMap<StaticKey, Rc<RefCell<Value>>>,
    // Non-constant class const / static-prop / property-default initializers,
    // evaluated after all classes link (so cross-class/enum refs resolve).
    // Generator runtime state, keyed by the Generator object's id.
    generators: HashMap<u32, GenState>,
    // Set by the `Yield` opcode to hand (key, value, resume-dst) to the driver.
    yield_out: Option<(Value, Value, Reg)>,
    // Stack of generator ids currently being advanced (innermost last).
    gen_stack: Vec<u32>,
    // `include`/`require`: a callback (injected by the SAPI) that parses+compiles
    // an included file's source into a Program, keeping the VM compiler-agnostic.
    includer: Option<Includer>,
    /// The scope a first-class callable was created in, while its wrapped
    /// callable is resolved (php resolves `$this->priv(...)` at creation).
    fcc_scope: std::cell::Cell<Option<(Option<ClassId>, Option<ClassId>)>>,
    // Resolved real paths already included (for the `_once` variants), in
    // inclusion order (`get_included_files`).
    included_files: indexmap::IndexSet<Vec<u8>>,
    /// Classes `[0, internal_classes)` are the built-in (prelude) ones.
    internal_classes: usize,
    // Registered `spl_autoload_register` callables, in call order.
    autoloaders: Vec<Value>,
    /// Lowercased class names whose autoload is in progress (php's
    /// `EG(in_autoload)`): a nested lookup of the same name does not re-enter
    /// the autoloaders.
    in_autoload: std::collections::HashSet<Vec<u8>>,
    /// Inheritance-check state while classes link (see `inherit`).
    inherit: inherit::LinkState,
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
    /// Open stream resources (fopen family), one record per resource id.
    streams: StreamTable,
    /// Readonly properties already reinitialized inside a `__clone`
    /// (object id, storage key): once each.
    readonly_reinit: HashMap<u32, Vec<PhpStr>>,
    /// Inside php's parent-hook trampoline (`parent::$p::get()` without a
    /// parent hook): property access uses the backing value.
    hook_bypass: bool,
    next_resource_id: u32,
    // The most recently opened directory stream (`readdir()` with no handle).
    // The last opened directory handle (php's DIRG(default_dir), which holds
    // a reference so the handle stays open).
    last_dir: Option<Value>,
    // The lazily-allocated default stream context resource id.
    default_context: Option<u32>,
    // Stream-context resources (`stream_context_create`), id -> options array.
    stream_contexts: HashMap<u32, Value>,
    // The `notification` callback of each stream context, by context id.
    stream_notifiers: HashMap<u32, Value>,
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
    stdin_reader: Option<builtins::main::streams::plain_wrapper::FdStream>,
    // Live objects whose class has a `__destruct` (strong handles, creation
    // order). An entry whose only owner is this list has been released by the
    // program: its destructor runs at the next check (`dtor_check`), as php's
    // refcount release would. Whatever is left runs at request shutdown.
    destructibles: Vec<ObjectHandle>,
    // Objects a fatal error's bailout marked destructed
    // (zend_objects_store_mark_destructed): kept, never destructed, until
    // shutdown ends.
    marked_destructed: Vec<ObjectHandle>,
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
    // Per-run extension state (`Host::module_state`), by module key.
    module_states: HashMap<&'static str, Box<dyn std::any::Any>>,
    /// The server API layer: sink, response headers, request input.
    sapi: request::SapiState,
    /// This request's view of the compiled-script cache, once used.
    opcache: Option<opcache::Request>,
    /// While running `opcache.preload`: the files compiled so far and where
    /// they were installed.
    preload_rec: Option<Vec<opcache::PreloadRec>>,
    /// What preloading declared, once installed in this request.
    preloaded: Option<Rc<opcache::Preload>>,
    /// This thread's memory accounting (checked between instructions).
    mem: &'static rhp_values::mem::MemState,
    /// A memory_limit crossing waiting for an interrupt point.
    delayed_oom: Option<Box<memory::DelayedOom>>,
    /// Last, so it drops after every value above: the request state kept
    /// outside the Vm is reset when the engine goes.
    _request_state: lifecycle::RequestStateReset,
}

/// A function-static slot: (proto address, unit, class it runs as,
/// variable name).
type StaticKey = (usize, UnitId, Option<ClassId>, Vec<u8>);

/// Index into [`Vm::units`].
type UnitId = u32;

/// An installed program (the main script, an included file, eval()'d
/// code): where its file-local closure and class-declaration indices start
/// in the VM's flat tables. Bytecode stays as compiled and is shared (with
/// opcache, across requests); a frame carries its unit, and the VM adds
/// these bases when it runs `MakeClosure`, `DeclareFunc` or `DeclareClass`.
#[derive(Clone, Copy, PartialEq)]
struct UnitBase {
    closures: u32,
    classes: u32,
}

/// A function proto and the unit it was installed with.
#[derive(Clone)]
struct UnitFn {
    proto: Rc<FunctionProto>,
    unit: UnitId,
}

/// One entry of [`Vm::decl_classes`]: a class declaration, its unit, and
/// whether it has been declared (hoisted or executed).
struct DeclSlot {
    proto: Rc<ClassProto>,
    unit: UnitId,
    done: bool,
}

/// One activation record.
struct Frame {
    proto: Rc<FunctionProto>,
    /// The unit `proto` belongs to.
    unit: UnitId,
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
    /// `include`/`require`/`eval` frame: the frame whose variable scope it
    /// runs in.
    include_caller: Option<usize>,
    /// Variables created by name (`$$n = v`) that have no compiled slot.
    dyn_vars: HashMap<Vec<u8>, Rc<RefCell<Value>>>,
    /// Runs in the global scope ({main}, or a file included from it): its
    /// CVs are the global symbol table's cells.
    global_scope: bool,
    /// The Closure object this frame runs (`Closure::getCurrent()`, the
    /// closure's own static variables).
    closure: Option<Rc<rhp_values::ClosureData>>,
}

impl Drop for Frame {
    fn drop(&mut self) {
        // php frees a frame's variables, then its `$this`: the handles come
        // back in that order.
        if let Some(cv) = self.proto.this_cv {
            if let Some(slot) = self.regs.get_mut(cv as usize) {
                let this = std::mem::replace(slot, Value::Null);
                self.regs.clear();
                drop(this);
            }
        }
    }
}

impl Frame {
    fn new(proto: Rc<FunctionProto>, unit: UnitId, ret_dst: Reg) -> Self {
        let regs = vec![Value::Null; proto.num_registers as usize];
        Frame {
            proto,
            unit,
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
            include_caller: None,
            dyn_vars: HashMap::new(),
            global_scope: false,
            closure: None,
        }
    }
}
