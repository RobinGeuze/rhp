//! The rhp bytecode ISA — **data only, no behavior**.
//!
//! A register-based instruction set (see `docs/design/bytecode.md` §2). Depends
//! only on `rhp-values` so a future `rhp-jit` can lower the same ISA
//! without pulling in the compiler or VM. Every operand is a small `Copy` index
//! or immediate; all heap data (`Value`s, interned names) lives in side tables
//! on [`FunctionProto`], so `Instr: Copy`.
//!
//! This is grown incrementally: the variants below cover language phases 1–4
//! (output, scalars/expressions, casts, builtin calls, array literals,
//! constants). Control-flow, user functions, and full array lvalues add more
//! variants in later phases — the encoding and the side-table design do not
//! change.

use std::rc::Rc;

use rhp_values::{ClassFlags, ClassKind, PhpStr, Value, Visibility};

// --- operand newtypes (kept as aliases for a lean encoding) ---------------
pub type Reg = u16; // register-file index: CVs in [0,num_cvs), temporaries above
pub type CIdx = u32; // index into FunctionProto.consts
pub type NIdx = u32; // index into FunctionProto.names (interned identifier bytes)
/// Sentinel `NIdx` meaning "no name" — used for the optional global-fallback name
/// on namespaced unqualified function/const references (see `Call`/`FetchConst`).
pub const NO_NAME: NIdx = u32::MAX;
pub type Pc = u32; // instruction index into FunctionProto.code (NOT a byte offset)
pub type FnIdx = u32; // index into Program.functions
pub type CacheIdx = u32; // index into a per-proto inline-cache side table (reserved)

/// Comparison kind for the single `Cmp` opcode (all route to `ops::`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpKind {
    Eq,
    Ne,
    Identical,
    NotIdentical,
    Lt,
    Le,
    Gt,
    Ge,
    Spaceship,
}

/// One bytecode instruction. `Copy`, fixed-width, one variant per opcode.
#[derive(Clone, Copy, Debug)]
pub enum Instr {
    // -- move / const / load (phase 1-2) --
    Nop,
    Mov {
        dst: Reg,
        src: Reg,
    },
    LoadConst {
        dst: Reg,
        k: CIdx,
    },
    LoadNull {
        dst: Reg,
    },
    LoadBool {
        dst: Reg,
        b: bool,
    },
    LoadInt {
        dst: Reg,
        n: i64,
    },

    // -- variable store (ref-aware; phase 2/6) --
    Assign {
        cv: Reg,
        src: Reg,
    },

    // -- arithmetic / bitwise / unary (phase 2) — route to ops:: --
    Add {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Sub {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Mul {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Div {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Mod {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Pow {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    BitAnd {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    BitOr {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    BitXor {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Shl {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Shr {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Concat {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Neg {
        dst: Reg,
        a: Reg,
    },
    BitNot {
        dst: Reg,
        a: Reg,
    },
    Not {
        dst: Reg,
        a: Reg,
    },
    /// `++`/`--`; ref-aware write-through. `pre`: value is new(true)/old(false).
    IncDec {
        dst: Reg,
        cv: Reg,
        pre: bool,
        delta: i8,
    },

    // -- casts (phase 2) --
    CastInt {
        dst: Reg,
        a: Reg,
    },
    CastFloat {
        dst: Reg,
        a: Reg,
    },
    CastString {
        dst: Reg,
        a: Reg,
    },
    CastBool {
        dst: Reg,
        a: Reg,
    },
    CastArray {
        dst: Reg,
        a: Reg,
    },
    /// `(object)` cast: wrap into a `stdClass` (arrays map elements to properties).
    CastObject {
        dst: Reg,
        a: Reg,
    },

    /// `@expr` begin: diagnostics raised until the matching `EndSilence` are
    /// recorded but not displayed (ZEND_BEGIN_SILENCE).
    BeginSilence,
    /// `@expr` end: restore the display state saved by `BeginSilence`.
    EndSilence,

    // -- compare (phase 2) — route to ops:: --
    Cmp {
        dst: Reg,
        a: Reg,
        b: Reg,
        kind: CmpKind,
    },

    // -- output (phase 1) --
    Echo {
        src: Reg,
    },
    EchoConst {
        k: CIdx,
    },
    Print {
        dst: Reg,
        src: Reg,
    },

    // -- jumps / branches (phase 3) --
    Jmp {
        target: Pc,
    },
    /// Prologue guard for a non-constant parameter default: jump to `target`
    /// (skipping the default-init code) if argument `index` was actually passed.
    JmpIfArg {
        index: u16,
        target: Pc,
    },
    JmpTrue {
        cond: Reg,
        target: Pc,
    },
    JmpFalse {
        cond: Reg,
        target: Pc,
    },
    JmpNull {
        src: Reg,
        target: Pc,
    },
    JmpNotNull {
        src: Reg,
        target: Pc,
    },

    // -- arrays (phase 4; literals first) --
    NewArray {
        dst: Reg,
        cap: u32,
    },
    ArrPush {
        arr: Reg,
        val: Reg,
    },
    ArrSet {
        arr: Reg,
        key: Reg,
        val: Reg,
    },
    ArrSpread {
        arr: Reg,
        src: Reg,
    },
    /// `[&$x]` / `[k => &$x]`: push/insert the raw `Value::Ref` in `val` so the
    /// element aliases the source variable (no deref).
    ArrPushRef {
        arr: Reg,
        val: Reg,
    },
    ArrSetRef {
        arr: Reg,
        key: Reg,
        val: Reg,
    },

    // -- array element read (value context): element or Null; string offset --
    DimFetch {
        dst: Reg,
        arr: Reg,
        key: Reg,
    },

    // -- lvalue cursor: descend a root CV by keys/appends, then commit --
    // Emitted contiguously (no interleaving); the VM holds the cursor between them.
    LvalCv {
        cv: Reg,
    }, // begin a cursor rooted at CV `cv`
    LvalKey {
        key: Reg,
    }, // descend into element `key` (autovivify on write)
    LvalAppend, // descend into a freshly appended element
    FetchLvalRw {
        dst: Reg,
    }, // read the current cursor location (null if absent), keep cursor open
    AssignLval {
        src: Reg,
    }, // write `src` at the cursor location (COW-bubbling), close cursor
    UnsetLval,  // remove the cursor's leaf key, close cursor
    /// `++`/`--` on the cursor location (ref-correct inc_dec); dst = new/old. Closes cursor.
    IncDecLval {
        dst: Reg,
        pre: bool,
        delta: i8,
    },

    // -- foreach (by value): iterate a COW snapshot --
    FeReset {
        arr_reg: Reg,
        pos_reg: Reg,
        src: Reg,
        end: Pc,
    }, // snapshot src; pos=0; if not-array/empty -> jmp end
    FeFetch {
        arr_reg: Reg,
        pos_reg: Reg,
        key_cv: Reg,
        val_cv: Reg,
        end: Pc,
    }, // bind key?/value; pos++; if done jmp end
    /// By-reference foreach setup: `arr_cv` is the source variable's CV (iterated
    /// live, not snapshotted). Skips to `end` if it is not a non-empty array.
    FeResetRef {
        arr_cv: Reg,
        pos_reg: Reg,
        end: Pc,
    },
    /// By-reference foreach step: promote element at `pos` to a shared cell and
    /// bind `val_cv` to it (so body writes flow back into the array).
    FeFetchRef {
        arr_cv: Reg,
        pos_reg: Reg,
        key_cv: Reg,
        val_cv: Reg,
        end: Pc,
    },

    // -- calls / return (phase 1 builtins; user functions later) --
    /// Resolve `name` (case-insensitive): refused → Unsupported; builtin →
    /// call with `regs[base..base+argc]`; user function (later) → push frame.
    /// `fallback` is a second name to try if `name` misses (`NO_NAME` if none):
    /// an unqualified call inside a namespace resolves `Ns\name` first, then the
    /// global `name` (PHP's fall-back-to-global rule).
    Call {
        dst: Reg,
        name: NIdx,
        base: Reg,
        argc: u16,
        fallback: NIdx,
    },
    /// Variadic call: `args` holds a `PhpArray` of the evaluated arguments
    /// (used when a call site contains argument spread `...$x`).
    CallV {
        dst: Reg,
        name: NIdx,
        args: Reg,
        fallback: NIdx,
    },
    /// Dynamic call `$f(args)`: `callee` holds a closure / string name / callable
    /// array / `__invoke` object; call it with `regs[base..base+argc]`.
    CallDyn {
        dst: Reg,
        callee: Reg,
        base: Reg,
        argc: u16,
    },
    /// Variadic dynamic call `$f(...$a)`: `args` holds a `PhpArray`.
    CallDynV {
        dst: Reg,
        callee: Reg,
        args: Reg,
    },
    /// Build a closure value: read `ncaptures` capture values from
    /// `regs[base..base+ncaptures]` and pair them with closure proto `fnidx`.
    MakeClosure {
        dst: Reg,
        fnidx: u32,
        base: Reg,
        ncaptures: u16,
    },
    /// First-class callable (`f(...)`, `$o->m(...)`, `C::m(...)`): wrap the
    /// callable value in `callee` as a `Closure`.
    MakeFcc {
        dst: Reg,
        callee: Reg,
    },
    Ret {
        src: Reg,
    },
    /// `return` from a by-reference function (`function &f()`): `src` holds a
    /// reference to the returned lvalue; it is delivered raw (not dereferenced).
    RetRef {
        src: Reg,
    },
    RetNull,
    /// `yield` / `yield $v` / `yield $k => $v` (only inside a generator). Suspends
    /// the frame, handing `(key, value)` to the driver; `dst` receives the value
    /// passed to `->send()` when the generator resumes. `key == Reg::MAX` means
    /// an auto-incrementing integer key.
    Yield {
        dst: Reg,
        key: Reg,
        value: Reg,
    },

    /// `match` with no matching arm and no default: throw `\UnhandledMatchError`.
    ThrowMatchError {
        subject: Reg,
    },
    /// A construct the compiler accepts syntactically but refuses to run (goto,
    /// …): the refusal is deferred to runtime so a file that merely *contains*
    /// it — down an unexecuted branch — still compiles and loads. `what`/`why`
    /// are interned message strings.
    DeferredUnsupported {
        what: CIdx,
        why: CIdx,
    },
    /// `eval($code)`: compile the PHP source in `src` at runtime and run it in
    /// the current variable scope; the script's return value lands in `dst`.
    Eval {
        dst: Reg,
        src: Reg,
    },
    /// `throw $e`: begin unwinding with the Throwable in `src`.
    Throw {
        src: Reg,
    },
    /// End of a `finally` block: perform the pending continuation (re-raise a
    /// pending exception, perform a pending return, or fall through).
    FinallyEnd,
    /// A `return` that must cross a `finally`: stash the value as a pending
    /// return, then `Jmp` to the finally block.
    SetPendingRet {
        src: Reg,
    },
    /// A `break`/`continue` that leaves a `try` with a `finally`: run the
    /// enclosing finally blocks (innermost first) that do not contain
    /// `target`, then jump to `target`.
    JmpFinally {
        target: Pc,
    },

    // -- OOP (phase 7) --
    /// `new C(args)`: allocate into `dst`, then run `__construct` via the window.
    New {
        dst: Reg,
        class: ClassRef,
        base: Reg,
        argc: u16,
        cache: CacheIdx,
    },
    /// Variadic `new` / method / static calls: `args` holds a `PhpArray`.
    NewV {
        dst: Reg,
        class: ClassRef,
        args: Reg,
    },
    CallMethodV {
        dst: Reg,
        obj: Reg,
        name: NIdx,
        args: Reg,
    },
    CallStaticV {
        dst: Reg,
        class: ClassRef,
        name: NIdx,
        args: Reg,
    },
    /// `$o->p` (value context).
    FetchProp {
        dst: Reg,
        obj: Reg,
        name: NIdx,
        cache: CacheIdx,
    },
    /// `$o->$p` / `$o->{expr}`: the property name is a runtime string in `name`.
    FetchPropDyn {
        dst: Reg,
        obj: Reg,
        name: Reg,
    },
    /// `$o->p` in isset/empty/`??` context (php's BP_VAR_IS): an inaccessible
    /// or missing property is null without an error, via `__isset` then
    /// `__get` when defined.
    FetchPropIs {
        dst: Reg,
        obj: Reg,
        name: NIdx,
    },
    /// `FetchPropIs` with the property name in a register.
    FetchPropDynIs {
        dst: Reg,
        obj: Reg,
        name: Reg,
    },
    /// `C::$p` in isset/empty/`??` context: null (no error) when the static
    /// property is undeclared or inaccessible.
    FetchStaticPropIs {
        dst: Reg,
        class: ClassRef,
        name: NIdx,
    },
    /// `$o->$p = src`: dynamic-name property write.
    StorePropDyn {
        obj: Reg,
        name: Reg,
        src: Reg,
    },
    /// `$o->$m(args)`: dynamic-name method call.
    CallMethodDyn {
        dst: Reg,
        obj: Reg,
        name: Reg,
        base: Reg,
        argc: u16,
    },
    /// `C::$p` (value context).
    FetchStaticProp {
        dst: Reg,
        class: ClassRef,
        name: NIdx,
        cache: CacheIdx,
    },
    /// `C::K` / `C::class`.
    FetchClassConst {
        dst: Reg,
        class: ClassRef,
        name: NIdx,
        cache: CacheIdx,
    },
    /// `$o->m(args)`.
    CallMethod {
        dst: Reg,
        obj: Reg,
        name: NIdx,
        base: Reg,
        argc: u16,
        cache: CacheIdx,
    },
    /// `C::m(args)` / `self::` / `static::` / `parent::`.
    CallStatic {
        dst: Reg,
        class: ClassRef,
        name: NIdx,
        base: Reg,
        argc: u16,
        cache: CacheIdx,
    },
    /// `C::$m(args)` / `$class::$m(args)`: static call whose method name is a
    /// runtime string in register `name`.
    CallStaticDyn {
        dst: Reg,
        class: ClassRef,
        name: Reg,
        base: Reg,
        argc: u16,
    },
    /// `$o->name = src` (single-level): direct write to a declared/accessible
    /// property, else `__set`, else a dynamic property.
    StoreProp {
        obj: Reg,
        name: NIdx,
        src: Reg,
    },
    /// Promote the current lvalue cursor target (a property or array element) to
    /// a shared cell and put a `Ref` to it in `dst` — for passing `$o->p` /
    /// `$a[$k]` to a by-reference builtin (sort/array_push/array_shift/…).
    MakeRefLval {
        dst: Reg,
    },
    /// `isset($o->name)`: declared non-null property, else `__isset`, else false.
    IssetProp {
        dst: Reg,
        obj: Reg,
        name: NIdx,
    },
    /// `unset($o->name)`: remove a declared property, else `__unset`.
    UnsetProp {
        obj: Reg,
        name: NIdx,
    },
    /// `empty($o->name)`: a reachable set property's falsiness, else (php's
    /// ZEND_PROPERTY_NOT_EMPTY) `__isset` and, if it says yes, `__get`.
    EmptyProp {
        dst: Reg,
        obj: Reg,
        name: NIdx,
    },
    /// `EmptyProp` with the name in a register.
    EmptyPropDyn {
        dst: Reg,
        obj: Reg,
        name: Reg,
    },
    /// `isset($o->$name)`: `IssetProp` with the name in a register.
    IssetPropDyn {
        dst: Reg,
        obj: Reg,
        name: Reg,
    },
    /// `unset($o->$name)`: `UnsetProp` with the name in a register.
    UnsetPropDyn {
        obj: Reg,
        name: Reg,
    },
    /// `global $x`: bind CV `cv` to the global variable `name` (a shared cell).
    BindGlobal {
        cv: Reg,
        name: NIdx,
    },
    /// `static $x = <const>`: bind CV `cv` to a per-function persistent cell,
    /// initialized to const `init` (`u32::MAX` = null) on first execution.
    BindStatic {
        cv: Reg,
        name: NIdx,
        init: u32,
    },
    /// `static $x = <non-constant expr>;` (php 8.3+), part 1: if the static
    /// cell `name` already exists, bind CV `cv` to it and jump to `target`
    /// (skipping the initializer); otherwise fall through to evaluate it.
    BindStaticOrJmp {
        cv: Reg,
        name: NIdx,
        target: Pc,
    },
    /// Part 2: create the static cell `name` holding the value in `src` (unless
    /// the initializer re-entered and already created it) and bind CV `cv`.
    BindStaticInit {
        cv: Reg,
        name: NIdx,
        src: Reg,
    },
    /// Top-level `const NAME = <expr>;`: define global constant `name` with the
    /// value in `src` (a no-op if it is already defined), in statement order.
    DeclareConst {
        name: NIdx,
        src: Reg,
    },
    /// `$val instanceof C` (never throws).
    InstanceOf {
        dst: Reg,
        val: Reg,
        class: ClassRef,
    },
    /// `clone $src`: shallow copy + new id + `__clone`.
    CloneObj {
        dst: Reg,
        src: Reg,
    },
    /// Cursor step: descend into object property `name`.
    LvalProp {
        name: NIdx,
    },
    /// Cursor step: descend into object property named by `name` (a runtime
    /// string register), e.g. `$o->$p = ...`.
    LvalPropDyn {
        name: Reg,
    },
    /// Cursor root: a static-property cell `C::$name`.
    LvalStaticProp {
        class: ClassRef,
        name: NIdx,
    },
    /// Link + register a conditionally-declared class at this point.
    DeclareClass {
        class_idx: u32,
    },
    /// Register a conditionally-declared function (one nested in control flow, so
    /// not hoisted): its proto is stored in the closures table at file-local
    /// index `idx` (resolved via the running proto's `closure_base`).
    DeclareFunc {
        idx: u32,
    },

    // -- references --
    /// `$dst =& $src`: promote `src`'s slot to a shared Ref; `dst` aliases it.
    BindRef {
        dst_cv: Reg,
        src_cv: Reg,
    },
    /// `$dst =& <lvalue>`: `src` already holds a `Ref` (from `MakeRefLval`); make
    /// CV `dst_cv` alias it (raw copy, no deref).
    RefAssign {
        dst_cv: Reg,
        src: Reg,
    },
    /// `<lvalue> =& <src>`: write the raw `Ref` in `src` (not deref'd) to the
    /// current lvalue-cursor target, so that element/property becomes a reference.
    AssignLvalRef {
        src: Reg,
    },
    /// Materialize CV `cv` as a Ref and place `Value::Ref` in `dst` (by-ref arg).
    MakeRef {
        dst: Reg,
        cv: Reg,
    },
    /// Wrap the *value* in `src` in a fresh `Value::Ref` cell (no aliasing of any
    /// variable slot). Used for `&$this`, which php protects from being modified
    /// through the reference.
    MakeRefVal {
        dst: Reg,
        src: Reg,
    },

    // -- misc --
    /// `name` (resolved constant). `fallback` is tried if `name` misses
    /// (`NO_NAME` if none): an unqualified constant inside a namespace resolves
    /// `Ns\NAME` first, then the global `NAME`.
    FetchConst {
        dst: Reg,
        name: NIdx,
        fallback: NIdx,
    },
    /// Emitted only for a structurally-refused construct on a reachable path;
    /// raises the uncatchable `Thrown::Unsupported`.
    Refuse {
        feature: NIdx,
        reason: NIdx,
    },
    /// `__halt_compiler()`: stop this proto as a top-level return.
    Halt,
    /// `exit`/`die`: a string `arg` is printed (code 0), an int `arg` is the exit
    /// code, null/absent is code 0. Terminates the script via the SAPI exit path.
    Exit {
        arg: Reg,
    },
    /// `include` / `require` (and the `_once` variants): evaluate `path` to a
    /// string, compile+run that file, and leave its return value in `dst`.
    Include {
        dst: Reg,
        path: Reg,
        kind: IncludeKind,
    },
}

/// Which file-inclusion construct an [`Instr::Include`] came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncludeKind {
    Include,
    IncludeOnce,
    Require,
    RequireOnce,
}

impl IncludeKind {
    /// require / require_once are fatal when the file can't be opened.
    pub fn is_require(self) -> bool {
        matches!(self, IncludeKind::Require | IncludeKind::RequireOnce)
    }
    /// the `_once` variants skip a file already included.
    pub fn is_once(self) -> bool {
        matches!(self, IncludeKind::IncludeOnce | IncludeKind::RequireOnce)
    }
    /// the construct name, for error messages.
    pub fn name(self) -> &'static str {
        match self {
            IncludeKind::Include => "include",
            IncludeKind::IncludeOnce => "include_once",
            IncludeKind::Require => "require",
            IncludeKind::RequireOnce => "require_once",
        }
    }
}

/// A by-value/by-ref/variadic parameter (used from phase 5).
#[derive(Clone)]
pub struct ParamInfo {
    pub name: NIdx,
    pub cv: Reg,
    pub by_ref: bool,
    pub variadic: bool,
    pub default: Option<Default>,
    /// Declared type (for `strict_types` enforcement / coercion); None = untyped.
    pub type_hint: Option<TypeHint>,
    /// Source line of the parameter declaration (php raises argument
    /// TypeError / ArgumentCountError at the failing parameter's line).
    pub line: u32,
}

/// A declared parameter / return type: a (possibly nullable) union or
/// intersection of type kinds. A single plain type is one `kinds` element with
/// `intersection = false`.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeHint {
    pub nullable: bool,
    pub intersection: bool,
    pub kinds: Vec<TypeKind>,
}

/// One member of a declared type.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    Int,
    Float,
    String,
    Bool,
    Array,
    Object,
    Callable,
    Iterable,
    Mixed,
    Void,
    Never,
    Null,
    True,
    False,
    SelfType,
    StaticType,
    Parent,
    /// A class / interface / enum name (resolved FQN, no leading backslash).
    Named(PhpStr),
}

#[derive(Clone, Copy)]
pub enum Default {
    Const(CIdx),
    Init(Pc),
}

/// A protected region in the exception table (used from phase 3+). Protects the
/// half-open instruction range `[start, end)`.
#[derive(Clone)]
pub struct ExRegion {
    pub start: Pc,
    pub end: Pc,
    pub catches: Vec<CatchArm>,
    pub finally: Option<Pc>,
    /// The pc of the finally block's `FinallyEnd` (the block is
    /// `[finally, finally_end]`); meaningless without `finally`.
    pub finally_end: Pc,
}

#[derive(Clone)]
pub struct CatchArm {
    pub classes: Vec<PhpStr>,
    pub var_reg: Option<Reg>,
    pub target: Pc,
}

/// Per-proto inline-cache slot (reserved; `Cell`-based ICs land in phase 5/7).
#[derive(Clone, Default)]
pub struct InlineCache {
    pub class_id: u32,
    pub payload: u32,
}

#[derive(Clone, Default)]
pub struct ProtoFlags {
    pub has_finally: bool,
    pub uses_ref: bool,
    pub generator: bool,
}

/// Debug side-table: never consulted for semantics.
#[derive(Clone, Default)]
pub struct DebugInfo {
    pub file: PhpStr,
    pub lines: Vec<(Pc, u32)>,
    pub cv_names: Vec<NIdx>,
}

/// One compiled PHP function/method/closure, or the script body (`"{main}"`).
/// Carries no `'arena` borrow — the AST is consumed at compile time.
/// `Clone` is used by the VM to stamp an included file's protos with a
/// `closure_base` offset before merging them (see `Vm::install_included`).
#[derive(Clone)]
pub struct FunctionProto {
    pub name: PhpStr,
    pub num_params: u16,
    pub params: Vec<ParamInfo>,
    /// Named-local registers occupy `[0, num_cvs)`, params first.
    pub num_cvs: u16,
    /// Total frame register-file size (CVs + temporaries high-water mark).
    pub num_registers: u16,
    pub code: Vec<Instr>,
    pub consts: Vec<Value>,
    pub names: Vec<PhpStr>,
    pub exception_table: Vec<ExRegion>,
    pub caches: Vec<InlineCache>,
    pub debug: DebugInfo,
    pub flags: ProtoFlags,
    /// CV holding `$this` for an instance method; `None` otherwise.
    pub this_cv: Option<Reg>,
    /// True for a `static function` (no `$this` binding).
    pub is_static: bool,
    /// Number of captured variables for a closure proto. Captures occupy CVs
    /// `[0, num_captures)`, params follow at `[num_captures, num_captures+num_params)`.
    pub num_captures: u16,
    /// True if the body contains `yield`: calling it builds a `Generator` object
    /// (the body runs lazily, driven by the generator protocol) instead of
    /// running immediately.
    pub is_generator: bool,
    /// Offset of this proto's file into the VM's flat closure table. A
    /// `MakeClosure` operand is a *file-local* closure index; the running
    /// closure index is `closure_base + fnidx`. The base program is 0; an
    /// included file is stamped with the closure count at merge time, so no
    /// instruction rewriting is needed (see `Vm::install_included`).
    pub closure_base: u32,
    /// CVs that name a superglobal (`$_SERVER`, `$GLOBALS`, …), paired with the
    /// name; the VM binds each to the shared global cell at frame entry so they
    /// are visible in every scope without an explicit `global`.
    pub superglobals: Vec<(Reg, PhpStr)>,
    /// Declared return type (for `strict_types` enforcement); None = untyped.
    pub return_type: Option<TypeHint>,
    /// `declare(strict_types=1)` in this proto's file: argument/return coercion is
    /// disabled for calls made *from* this proto (php's caller-decides rule).
    pub strict_types: bool,
    /// `function &f()` — returns by reference: `return $lval` yields a reference to
    /// the lvalue, which `$x =& f()` aliases.
    pub returns_ref: bool,
    /// CV register -> variable name (index = Reg). Used to bind an `include`d /
    /// `eval`'d script's `{main}` to the *calling* scope's variables by name
    /// (php executes include/eval in the enclosing variable scope).
    pub cv_var_names: Vec<PhpStr>,
}

impl FunctionProto {
    /// An empty proto (builder starting point).
    pub fn new(name: PhpStr) -> Self {
        FunctionProto {
            name,
            num_params: 0,
            params: Vec::new(),
            num_cvs: 0,
            num_registers: 0,
            code: Vec::new(),
            consts: Vec::new(),
            names: Vec::new(),
            exception_table: Vec::new(),
            caches: Vec::new(),
            debug: DebugInfo::default(),
            flags: ProtoFlags::default(),
            this_cv: None,
            is_static: false,
            num_captures: 0,
            is_generator: false,
            closure_base: 0,
            superglobals: Vec::new(),
            return_type: None,
            strict_types: false,
            returns_ref: false,
            cv_var_names: Vec::new(),
        }
    }
}

impl std::fmt::Debug for FunctionProto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionProto")
            .field("name", &self.name)
            .field("num_cvs", &self.num_cvs)
            .field("num_registers", &self.num_registers)
            .field("code_len", &self.code.len())
            .field("consts", &self.consts.len())
            .finish()
    }
}

/// A fully compiled program: the script body, functions, and class declarations.
pub struct Program {
    pub main: Rc<FunctionProto>,
    pub functions: Vec<Rc<FunctionProto>>,
    /// Compiled, UN-LINKED class declarations; the VM links them (`Vm::install`),
    /// resolving `extends`/`implements`/`use` by name and assigning `ClassId`s.
    pub classes: Vec<ClassProto>,
    /// Anonymous closure / arrow-function protos, indexed by the `fnidx` operand
    /// of `MakeClosure` and by `ClosureData::fnidx`.
    pub closures: Vec<Rc<FunctionProto>>,
    /// User-defined global constants (`const NAME = <const-expr>;`), folded at
    /// compile time; the VM installs them into its constant table at startup.
    pub constants: Vec<(PhpStr, Value)>,
    /// {main}'s variable names indexed by CV register, so the VM can bind a
    /// function's `global $x` to the top-level variable's slot.
    pub main_var_names: Vec<PhpStr>,
    /// How many leading entries of `classes` are the built-in prelude (always
    /// re-emitted per compile). An included file's prelude classes are skipped on
    /// merge; user classes past this count fatally redeclare if already defined.
    pub num_prelude_classes: usize,
    /// How many leading entries of `functions` are prelude helper functions
    /// (`iterator_*`, …), re-emitted per compile and skipped on include merge.
    pub num_prelude_functions: usize,
}

// ---------------------------------------------------------------------------
// OOP: class operand + compiled (un-linked) class declarations
// ---------------------------------------------------------------------------

/// How a `New`/`CallStatic`/`FetchStaticProp`/`FetchClassConst`/`InstanceOf`
/// names its class. `Copy`, owns no heap.
#[derive(Clone, Copy, Debug)]
pub enum ClassRef {
    /// `Foo` — interned name; resolved to a `ClassId` at run time (IC-cacheable).
    Name(NIdx),
    /// `self` — the current frame's defining class.
    SelfClass,
    /// `parent` — the parent of the frame's defining class.
    ParentClass,
    /// `static` — the current frame's called class (late static binding).
    StaticClass,
    /// `new $cls()` / `$cls::m()` — a runtime string or object in a register.
    Dynamic(Reg),
}

/// One compiled class/interface/trait/enum declaration (parents referenced by
/// name; resolved to `ClassId` at link time).
#[derive(Clone)]
pub struct ClassProto {
    pub name: PhpStr,
    pub kind: ClassKind,
    pub flags: ClassFlags,
    pub parent: Option<PhpStr>,
    pub interfaces: Vec<PhpStr>,
    pub traits: Vec<PhpStr>,
    pub methods: Vec<MethodProto>,
    pub props: Vec<PropProto>,
    pub static_props: Vec<StaticPropProto>,
    pub constants: Vec<ConstProto>,
    /// Enum cases, in declaration order: `(name, backing value)`. The backing
    /// value is `None` for a pure enum, `Some(Int|Str)` for a backed enum.
    pub enum_cases: Vec<EnumCaseProto>,
    /// Backing type of a backed enum (`Int`/`Str`); `None` for a pure enum or a
    /// non-enum class.
    pub enum_backing: Option<EnumBacking>,
}

/// One compiled enum case.
#[derive(Clone)]
pub struct EnumCaseProto {
    pub name: PhpStr,
    /// The backing value — folded, or a thunk evaluating a constant expression
    /// on first access — or `None` for a pure enum.
    pub value: Option<ConstInitProto>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnumBacking {
    Int,
    Str,
}

/// By-ref mask for `sscanf`: positions 0 (`$string`) and 1 (`$format`) are by
/// value; every assignment target after them is by reference.
static SSCANF_REF_MASK: [bool; 64] = {
    let mut m = [true; 64];
    m[0] = false;
    m[1] = false;
    m
};

/// Per-parameter by-reference flags for builtins whose arguments are passed by
/// reference (the array-mutating sort/stack family, and the `preg_*` out-params
/// `&$matches` / `&$count`). The call site emits `MakeRef` for each flagged
/// position, and the VM passes those positions to the builtin as `Value::Ref`
/// rather than a deref'd copy. Empty slice = no by-ref parameters. This is the
/// single source of truth shared by the compiler and the VM.
pub fn builtin_ref_params(name_lc: &str) -> &'static [bool] {
    match name_lc {
        // arg 0 (the array) by reference.
        "sort" | "rsort" | "asort" | "arsort" | "ksort" | "krsort" | "usort" | "uasort"
        | "uksort" | "array_push" | "array_pop" | "array_shift" | "array_unshift" | "shuffle"
        | "array_splice" | "natsort" | "natcasesort" | "array_walk"
        | "array_walk_recursive"
        // internal-pointer functions mutate the array's pointer.
        | "end" | "reset" | "next" | "prev" | "current" | "key" | "each" => &[true],
        // `sscanf($string, $format, &...$vars)`: every argument from position 2
        // on is an output passed by reference.
        "sscanf" => &SSCANF_REF_MASK,
        // `mb_convert_variables($to, $from, &$var, &...$vars)`.
        "mb_convert_variables" => &SSCANF_REF_MASK,
        // `&$idna_info` at position 3.
        "idn_to_ascii" | "idn_to_utf8" => &[false, false, false, true],
        // `&$next` at position 4.
        "grapheme_extract" => &[false, false, false, false, true],
        // `&$matches` at position 2.
        "preg_match" | "preg_match_all" => &[false, false, true],
        // `&$count` at position 4.
        "preg_replace" | "preg_replace_callback" | "preg_replace_callback_array" => {
            &[false, false, false, false, true]
        }
        _ => &[],
    }
}

#[derive(Clone)]
pub struct MethodProto {
    pub name: PhpStr,
    pub proto: Rc<FunctionProto>,
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
}

#[derive(Clone)]
pub struct PropProto {
    pub name: PhpStr,
    pub visibility: Visibility,
    /// Asymmetric write visibility (`private(set)` / `protected(set)`), when
    /// narrower than `visibility`.
    pub set_visibility: Option<Visibility>,
    pub is_readonly: bool,
    pub default: PropDefaultProto,
    /// Declared type (`None` = untyped); a lazily evaluated default is
    /// checked against it (php's `update_property`).
    pub type_hint: Option<TypeHint>,
}

#[derive(Clone)]
pub struct StaticPropProto {
    pub name: PhpStr,
    pub visibility: Visibility,
    pub is_readonly: bool,
    pub init: ConstInitProto,
    /// Declared type (`None` = untyped).
    pub type_hint: Option<TypeHint>,
}

#[derive(Clone)]
pub struct ConstProto {
    pub name: PhpStr,
    pub visibility: Visibility,
    pub value: ConstInitProto,
    /// Declared type of a typed class constant (`None` = untyped).
    pub type_hint: Option<TypeHint>,
}

/// A property's per-instance default, as compiled.
#[derive(Clone)]
pub enum PropDefaultProto {
    /// Constant default folded at compile time.
    Const(Value),
    /// Non-constant initializer: a zero-param thunk run per-instance at `new`.
    Init(Rc<FunctionProto>),
    /// Declared without a default.
    None,
}

/// A class-constant / static-prop initializer, as compiled.
#[derive(Clone)]
pub enum ConstInitProto {
    Const(Value),
    Thunk(Rc<FunctionProto>),
}
