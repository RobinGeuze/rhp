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

mod attributes;
mod autoload;
mod callables;
mod calls;
mod classes;
mod closures;
mod compare;
mod const_link;
mod dispatch;
mod errors;
mod fibers;
mod frame_stack;
mod gc;
mod generators;
mod hooks;
mod host;
mod include;
mod inherit;
mod lazy;
mod lifecycle;
mod link_cache;
mod link_checks;
mod lookup_cache;
mod lvalue;
mod memory;
mod opcache;
mod output;
mod props;
mod reflect;
mod refsrc;
mod request;
mod scope;
mod stream_filters;
mod streams;
mod stub_default;
mod traits;
mod types;
mod user_streams;
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
pub use link_cache::link_cache_stats;

/// The virtual machine: call stack + global tables, running one request
/// under its SAPI.
pub struct Vm {
    /// Typed references and their type sources (see `refsrc`).
    typed_refs: refsrc::TypedRefs,
    /// Active magic-accessor guards: (object id, accessor, property name).
    prop_guards: rustc_hash::FxHashSet<(u32, &'static [u8], PhpStr)>,
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
    stack: frame_stack::FrameStack,
    /// Internal functions' signature argument counts, by defining class
    /// and name (see `Vm::internal_arg_count_error`).
    arg_counts: rustc_hash::FxHashMap<(ClassId, PhpStr), Option<calls::ArgRange>>,
    /// Register files of returned frames, for the next calls.
    reg_pool: frame_stack::RegPool,
    /// The programs installed so far: the main script is unit 0, then each
    /// included file and eval()'d string, in load order. A program installed
    /// right after one that added no closures and no class declarations
    /// shares that unit, since their bases are equal.
    units: Vec<UnitBase>,
    functions: Vec<UnitFn>,
    fn_names: rustc_hash::FxHashMap<Vec<u8>, usize>, // lowercase name -> index into `functions`
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
    /// `class_alias()` entries of the class table: (lowercased alias, class,
    /// `decl_classes.len()` when added), for `get_declared_classes()`.
    class_aliases: Vec<(Vec<u8>, ClassId, usize)>,
    constants: rustc_hash::FxHashMap<Vec<u8>, Value>,
    /// User-defined constant names in definition order (`get_defined_constants`).
    user_const_order: Vec<Vec<u8>>,
    /// Attributed global constants' attributes, by constant name.
    const_attrs: HashMap<Vec<u8>, attributes::ConstAttrs>,
    /// The `#[\Deprecated]` global constants whose deprecation is being
    /// raised (php's CONST_PROTECT_RECURSION).
    const_guard: Vec<Vec<u8>>,
    /// After max_execution_time ran out: the (frame, pc) of the last user
    /// instruction run. php raises the timeout where a jump lands or a
    /// function starts (its VM interrupt checks).
    timeout_prev: Option<(usize, usize)>,
    /// EG(ticks_count): statements run under `declare(ticks)` since the
    /// tick functions last ran.
    ticks_count: u32,
    /// A jump landed on a loop head: php's layout tests the condition at
    /// the bottom, so its landing is the body start, here (frame, pc).
    timeout_wait: Option<(usize, usize)>,
    /// Builtin calls in progress (innermost last), for exception traces.
    native_calls: Vec<NativeCall>,
    /// `set_exception_handler` stack (innermost last; null = none).
    exception_handlers: Vec<Value>,
    /// `register_shutdown_function` callbacks with their arguments, in order.
    shutdown_functions: Vec<(Value, Vec<Value>)>,
    /// The entries of `shutdown_functions` registered under a name
    /// (register_user_shutdown_function), with their index.
    shutdown_names: Vec<(Vec<u8>, usize)>,
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
    /// What class names in protos resolved to (`lookup_cache`).
    class_names: lookup_cache::ClassNameCache,
    class_bytes: lookup_cache::ClassBytesCache,
    // The global symbol table (php's EG(symbol_table)), in insertion order:
    // one shared cell per global variable. {main}'s CVs, top-level included
    // files, `global $x` and `$GLOBALS[...]` all alias these cells.
    globals: indexmap::IndexMap<Vec<u8>, Rc<RefCell<Value>>, rustc_hash::FxBuildHasher>,
    // Auto-globals php creates on first use (auto_globals_jit): their initial
    // value, moved into `globals` when a script first references the name.
    jit_globals: HashMap<Vec<u8>, Value>,
    // Typed property slots that were `unset()` (object id -> storage keys):
    // php clears a slot's IS_PROP_UNINIT flag on unset, after which an absent
    // slot defers to the magic accessors; a never-initialized one does not.
    unset_typed_props: HashMap<u32, Vec<PhpStr>>,
    // Globals that became PHP references (`global $x`, `=&`, by-ref passing):
    // php's shutdown sweep only destroys plain (non-reference) object globals.
    ref_globals: rustc_hash::FxHashSet<Vec<u8>>,
    // `static $x`: persistent cells keyed by (function proto pointer, unit,
    // class, var name).
    statics: rustc_hash::FxHashMap<StaticKey, Rc<RefCell<Value>>>,
    // The statics of the running include and eval `{main}` frames, keyed by
    // stack index; each frame's are dropped when it returns.
    include_statics: rustc_hash::FxHashMap<StaticKey, Rc<RefCell<Value>>>,
    // Non-constant class const / static-prop / property-default initializers,
    // evaluated after all classes link (so cross-class/enum refs resolve).
    // Generator runtime state, keyed by the Generator object's id.
    generators: rustc_hash::FxHashMap<u32, GenState>,
    // Set by `Yield` / `YieldFrom` to tell the generator driver why the
    // frame stopped.
    yield_out: Option<generators::YieldOut>,
    // Stack of generator ids currently being advanced (innermost last).
    gen_stack: Vec<u32>,
    // The running generator frames: (stack index, the generator, the leaf
    // generator it runs for), so traces show the `yield from` chain.
    gen_resumes: Vec<(usize, u32, u32)>,
    // A force-closed generator ended without a return value.
    gen_abort: bool,
    // `include`/`require`: a callback (injected by the SAPI) that parses+compiles
    // an included file's source into a Program, keeping the VM compiler-agnostic.
    includer: Option<Includer>,
    /// The scope a first-class callable was created in, while its wrapped
    /// callable is resolved (php resolves `$this->priv(...)` at creation).
    fcc_scope: std::cell::Cell<Option<(Option<ClassId>, Option<ClassId>)>>,
    /// The `$this` that goes with `fcc_scope` (a callable bound when it was
    /// checked, such as a shutdown function's).
    fcc_this: std::cell::RefCell<Option<ObjectHandle>>,
    // Resolved real paths already included (for the `_once` variants), in
    // inclusion order (`get_included_files`).
    included_files: indexmap::IndexSet<Vec<u8>, rustc_hash::FxBuildHasher>,
    /// Classes `[0, internal_classes)` are the built-in (prelude) ones.
    internal_classes: usize,
    /// The linked-class cache's per-request side (`link_cache`).
    link_cache: link_cache::LinkCacheState,
    /// Diagnostics raised so far (a link that raised one is not cached).
    diag_seq: u64,
    // Registered `spl_autoload_register` callables, in call order.
    autoloaders: Vec<autoload::Autoloader>,
    /// Lowercased class names whose autoload is in progress (php's
    /// `EG(in_autoload)`): a nested lookup of the same name does not re-enter
    /// the autoloaders.
    in_autoload: rustc_hash::FxHashSet<Vec<u8>>,
    /// Stack depth at each running autoload: php's zend_lookup_class clears
    /// the constant-expression file/line override for the loader.
    autoload_depths: Vec<usize>,
    /// Inheritance-check state while classes link (see `inherit`).
    inherit: inherit::LinkState,
    /// Named-argument binding for the frame about to be pushed: the skipped
    /// parameter mask and the extra named arguments collected by a variadic
    /// parameter. Set by a call instruction right before the push.
    pending_named: Option<(u64, Vec<(PhpStr, Value)>)>,
    /// Set while a statically resolved `call_user_func[_array]()` builtin runs:
    /// the caller's `strict_types`, consumed by its single callback dispatch.
    cuf_caller: Option<bool>,
    /// Set with `cuf_caller` when php binds the callback at compile time
    /// (`CallFlags::CT_BOUND`): its call is a direct one.
    cuf_direct: bool,
    /// The stack depth of the builtin call whose result is unused
    /// (`CallFlags::RESULT_UNUSED`), else `usize::MAX`.
    unused_call_depth: usize,
    /// Set by `$f(...)` right before its callable dispatch, which then
    /// resolves the value as a dynamic call (zend_init_dynamic_call_*).
    dyn_call: bool,
    /// The builtin about to run got a dynamic call's raw arguments: its
    /// by-value positions are dereferenced first.
    dyn_raw_args: bool,
    /// Set by a direct call by name (`f(...)`) right before its builtin
    /// runs; any other way to reach a builtin is php's ZEND_CALL_DYNAMIC.
    call_direct: bool,
    /// The running builtin was called dynamically (a callback, `$f()`):
    /// the scope functions refuse that (zend_forbid_dynamic_call).
    builtin_dynamic: bool,
    /// A callable check's deprecation threw (see `Vm::check_callable_ex`).
    callable_dep_thrown: bool,
    /// The callable argument the running builtin's zpp resolved, by its
    /// address in the builtin's argument vector (alive and unmodified for
    /// the call), so calling it needs no second lookup. Cleared before
    /// each builtin's zpp and after it returns.
    zpp_fcc: Option<(*const Value, calls::Callee)>,
    // Stream wrappers by scheme (`file`, `phar`, …), installed at startup from
    // the module list; runtime-mutable later via `stream_wrapper_register`.
    wrappers: HashMap<Vec<u8>, Rc<dyn builtins::StreamWrapper>>,
    // Built-in wrappers `stream_wrapper_unregister` took out, kept for
    // `stream_wrapper_restore` (None: one served outside `wrappers`).
    wrappers_off: HashMap<Vec<u8>, Option<Rc<dyn builtins::StreamWrapper>>>,
    // User wrappers in `wrappers`, in registration order (php's hash order
    // for `stream_get_wrappers`).
    user_wrappers: Vec<(Vec<u8>, Rc<builtins::main::streams::userspace::UserWrapper>)>,
    // Filesystem path of the top-level script (for `Phar::mapPhar`).
    main_script_path: Option<Vec<u8>>,
    /// Whether auto_prepend_file runs before the main script (not for
    /// `-r` code, which php evaluates without php_execute_script).
    auto_prepend: bool,
    /// Open stream resources (fopen family), one record per resource id.
    streams: StreamTable,
    /// Clones whose `__clone` runs (object id -> the frame's stack index
    /// and the readonly slots it may still modify once).
    readonly_reinit: HashMap<u32, (usize, Vec<PhpStr>)>,
    /// Inside php's parent-hook trampoline (`parent::$p::get()` without a
    /// parent hook): property access uses the backing value.
    hook_bypass: bool,
    next_resource_id: u32,
    /// The resource values made, by id, for `get_resources()` (php's
    /// EG(regular_list)): a resource is listed while a value holds it.
    resource_list: std::collections::BTreeMap<u32, Vec<std::rc::Weak<rhp_values::ResourceData>>>,
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
    // `@` nesting depth (the error_reporting mask itself is in the ini
    // value, as php's EG(error_reporting)).
    silence: u32,
    // Active `@` regions: (frame depth, silence level to restore, the
    // error_reporting value before it).
    silence_stack: Vec<(usize, u32, i64)>,
    // Engine-internal quiet regions (preloading): nothing is displayed.
    quiet: u32,
    // Null-key deprecations a read-modify-write path already raised (its
    // write converts those keys quietly).
    rw_null_keys: u32,
    // An error handler destroyed the container of a read-modify-write
    // fetch (zend_undefined_index_write failed): the operation is dropped.
    rw_aborted: bool,
    // The value a cursor write stored in a typed property after coercion
    // (the assignment expression's value).
    typed_write_result: Option<Value>,
    // `StoreProp` wants `typed_write_result` from the write it runs.
    want_typed_result: bool,
    // The running cursor write is a `++`/`--`.
    incdec_write: bool,
    // The static property an open cursor is rooted at: its cell, the class
    // it was reached through and its name (valid while the root is that cell).
    cursor_static: Option<lvalue::StaticRoot>,
    // An undefined variable assigned below a `__get` property: php reads
    // it (and warns) after fetching the container.
    deferred_undef_src: Option<Reg>,
    // An undefined variable as the last key of an assignment's path: php
    // reads it after checking the container (see `assign_undef_key`).
    deferred_undef_key: Option<Reg>,
    // False-to-array deprecations already raised for the open cursor's
    // write (its walk then converts quietly).
    skip_false_dep: u32,
    // A compound assignment's operator left its operands for the
    // `AssignLval` to free once the store is done (`free_bin_operands`).
    compound_free: Option<(Reg, Reg)>,
    // The typed property the open cursor's leaf is, as the last
    // `cursor_through_check` found it.
    cursor_leaf: Option<(ClassId, ClassId, PhpStr, Rc<rhp_bytecode::TypeHint>)>,
    // The open cursor has a null key (set by `LvalKey`).
    cursor_null_key: bool,
    /// Steps (bit per index) of the open cursor whose key is a reference
    /// holding null.
    cursor_ref_null: u64,
    // The last raised diagnostic (`error_get_last`): type, message, file, line.
    last_error: Option<(i64, Vec<u8>, Vec<u8>, i64)>,
    /// EG(last_fatal_error_backtrace): `error_get_last()['trace']`.
    last_fatal_trace: Option<rhp_values::PhpArray>,
    /// `set_error_handler` stack (innermost last): (callback or null, mask).
    error_handlers: Vec<(Value, i64)>,
    /// A user error handler is running (php unsets the handler meanwhile).
    /// An exception a user error handler threw while a builtin (or the
    /// engine) raised a diagnostic: php's pending `EG(exception)`, thrown
    /// once the builtin returns.
    pending_throw: Option<Thrown>,
    /// The include kind while an include opens its file (php's docref
    /// names the include then).
    include_docref: Option<&'static str>,
    /// The frame depth a user-wrapper include runs its stream at: php's
    /// docref names the include there.
    include_docref_depth: usize,
    /// Streams of includes a fatal error cut short: php closes them with
    /// the resource list at shutdown.
    held_streams: Vec<Value>,
    /// The end-of-request resource list close is running: filters see a
    /// null stream.
    closing_resources: bool,
    /// Open streams with a filter layer, plus user wrapper streams (their
    /// release is checked at once: php frees a resource on its last
    /// reference, and their close runs PHP code).
    filtered_streams: u32,
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
    // `__PHP_Incomplete_Class`, once linked: its objects take
    // incomplete_class.c's property and method handlers.
    incomplete_class: Option<ClassId>,
    // `Throwable`, once the prelude is linked (new objects check it).
    throwable_class: Option<ClassId>,
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
    // The primary script's return value, for the SAPI's `script_done`.
    main_ret: Option<Value>,
    // The next include runs as the request's next primary script.
    primary_next: bool,
    // php -a: the readline shell runs instead of a script.
    interactive: bool,
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
    /// This thread's cycle-collector state (frames release into it).
    gc: &'static rhp_values::gc::State,
    /// This thread's memory accounting (checked between instructions).
    mem: &'static rhp_values::mem::MemState,
    /// A memory_limit crossing waiting for an interrupt point.
    delayed_oom: Option<Box<memory::DelayedOom>>,
    /// EG(active_fiber): the running Fiber object.
    active_fiber: Option<ObjectHandle>,
    /// The cycle collector's destructor fiber and the state of a
    /// collection's destructor phase.
    gc_dtor: gc::DtorState,
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
    /// `include`/`require`/`eval` frame: the construct name shown in
    /// traces, and the frame whose variable scope it runs in.
    include: Option<(&'static str, usize)>,
    /// Variables created by name (`$$n = v`) that have no compiled slot.
    dyn_vars: DynVars,
    /// Runs in the global scope ({main}, or a file included from it): its
    /// CVs are the global symbol table's cells.
    global_scope: bool,
    /// The Closure object this frame runs (`Closure::getCurrent()`, the
    /// closure's own static variables).
    closure: Option<Rc<rhp_values::ClosureData>>,
    /// HashTable iterators of the running foreach loops over an object or
    /// by reference. A loop left without its `FeFree` (an exception, a jump
    /// through a finally) gives its iterator up when a catch frees its
    /// temporaries or the frame goes. Boxed to keep `Frame` small (calls
    /// move it): most frames have none.
    #[allow(clippy::box_collection)]
    fe_live: Option<Box<Vec<FeLive>>>,
    /// Called by a prelude function: php has no such call, so its `$this`
    /// is no possible root when the frame ends.
    internal_caller: bool,
    /// What php's VM stack holds for this frame, in bytes, and whether it
    /// took a page of its own (set by [`frame_stack::FrameStack::push`]).
    vm_size: u32,
    vm_page: bool,
}

type DynVarMap = indexmap::IndexMap<Vec<u8>, Rc<RefCell<Value>>, rustc_hash::FxBuildHasher>;

/// A frame's variables created by name, boxed on first use (keeps `Frame`
/// small: calls move it).
#[derive(Default)]
struct DynVars(Option<Box<DynVarMap>>);

impl DynVars {
    fn get(&self, name: &[u8]) -> Option<&Rc<RefCell<Value>>> {
        self.0.as_ref()?.get(name)
    }

    fn insert(&mut self, name: Vec<u8>, cell: Rc<RefCell<Value>>) {
        self.0.get_or_insert_default().insert(name, cell);
    }

    fn shift_remove(&mut self, name: &[u8]) -> Option<Rc<RefCell<Value>>> {
        self.0.as_mut()?.shift_remove(name)
    }

    /// The variable `name`, created undefined when missing.
    fn cell(&mut self, name: &[u8]) -> Rc<RefCell<Value>> {
        self.0
            .get_or_insert_default()
            .entry(name.to_vec())
            .or_insert_with(|| Rc::new(RefCell::new(Value::Undef)))
            .clone()
    }

    fn iter(&self) -> impl Iterator<Item = (&Vec<u8>, &Rc<RefCell<Value>>)> {
        self.0.iter().flat_map(|m| m.iter())
    }
}

/// A running foreach's iterator: `(pos_reg, arr, by_ref, id)`.
type FeLive = (Reg, Reg, bool, u32);

impl Drop for Frame {
    fn drop(&mut self) {
        if self.regs.capacity() > 0 || self.closure.is_some() || self.fe_live.is_some() {
            self.release(rhp_values::gc::state());
        }
        if self.regs.capacity() > 0 {
            // Temporaries go with the register file; its buffer was not
            // counted (`frame_stack`).
            self.regs.clear();
            let regs = std::mem::take(&mut self.regs);
            rhp_values::mem::uncounted(|| drop(regs));
        }
    }
}

impl Frame {
    /// Free the frame's variables, `$this` and Closure as php's
    /// leave helper does (what dropping it starts with; done again, it
    /// does nothing).
    fn release(&mut self, gc: &rhp_values::gc::State) {
        if let Some(live) = self.fe_live.take() {
            for (pos, arr, by_ref, id) in *live {
                dispatch::release_fe_iter(&mut self.regs, pos, arr, by_ref, id);
            }
        }
        // php frees a frame's variables, then its `$this`: the handles come
        // back in that order. Each variable may become a possible root of
        // a cycle as it goes (a prelude function is internal in php: it
        // has none). Scalars and temporaries go with the register file.
        let proto = &*self.proto;
        let this = match proto.this_cv {
            Some(cv) => self
                .regs
                .get_mut(cv as usize)
                .map(|s| std::mem::replace(s, Value::Null)),
            None => None,
        };
        if !proto.is_internal() {
            let n = (proto.num_cvs as usize).min(self.regs.len());
            for slot in &mut self.regs[..n] {
                if let Value::Object(_) | Value::Array(_) | Value::Ref(_) | Value::Closure(_) = slot
                {
                    let v = std::mem::replace(slot, Value::Undef);
                    rhp_values::gc::possible_root_in(gc, &v);
                    drop(v);
                }
            }
        }
        if let Some(this) = this {
            // A prelude method's variables go before its `$this` too.
            self.regs.clear();
            if self.closure.is_none() && !self.internal_caller {
                rhp_values::gc::possible_root_in(gc, &this);
            }
            drop(this);
        }
        // A closure call releases the Closure (not its bound `$this`).
        if let Some(c) = self.closure.take() {
            let c = Value::Closure(c);
            rhp_values::gc::possible_root_in(gc, &c);
            drop(c);
        }
    }
}

impl Frame {
    /// See `Frame::include`.
    fn include_kind(&self) -> Option<&'static str> {
        self.include.map(|i| i.0)
    }

    /// See `Frame::include`.
    fn include_caller(&self) -> Option<usize> {
        self.include.map(|i| i.1)
    }

    fn new(proto: Rc<FunctionProto>, unit: UnitId, ret_dst: Reg) -> Self {
        // Every register starts undefined (a zero tag, so this is a fill
        // of zeroes): a variable must be, and a temporary is written before
        // it is read (a copy reads `Undef` as null, a move turns it into
        // null, and `undef_cv` never warns for one).
        let n = proto.num_registers as usize;
        let regs = frame_stack::alloc_regs(n, rhp_values::mem::state());
        Self::with_regs(proto, unit, ret_dst, regs)
    }

    /// [`Frame::new`] with a register file of undefined registers.
    #[inline]
    fn with_regs(proto: Rc<FunctionProto>, unit: UnitId, ret_dst: Reg, regs: Vec<Value>) -> Self {
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
            include: None,
            dyn_vars: DynVars::default(),
            global_scope: false,
            closure: None,
            fe_live: None,
            internal_caller: false,
            vm_size: 0,
            vm_page: false,
        }
    }
}
