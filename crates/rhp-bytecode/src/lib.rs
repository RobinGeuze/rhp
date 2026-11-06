//! The rhp bytecode ISA — **data only, no behavior** — and php's arginfo for
//! the builtins ([`signatures`]), which the compiler and the VM share.
//!
//! A register-based instruction set (see `docs/design/bytecode.md` §2). Depends
//! only on `rhp-values` so a future `rhp-jit` can lower the same ISA
//! without pulling in the compiler or VM. Every operand is a small `Copy` index
//! or immediate; all heap data (`Value`s, interned names) lives in side tables
//! on [`FunctionProto`], so `Instr: Copy`.

use std::rc::Rc;

use rhp_values::{ClassFlags, ClassKind, PhpStr, Value, Visibility};

pub mod signatures;

// --- operand newtypes (kept as aliases for a lean encoding) ---------------
pub type Reg = u16; // register-file index: CVs in [0,num_cvs), temporaries above
pub type CIdx = u32; // index into FunctionProto.consts
pub type NIdx = u32; // index into FunctionProto.names (interned identifier bytes)
/// Sentinel `NIdx` meaning "no name" — used for the optional global-fallback name
/// on namespaced unqualified function/const references (see `Call`/`FetchConst`).
pub const NO_NAME: NIdx = u32::MAX;
pub type Pc = u32; // instruction index into FunctionProto.code (NOT a byte offset)

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

/// What php's compiler decides about a call by name (`Call`) that reaches a
/// builtin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallFlags(u8);

impl CallFlags {
    /// php compiles the call to an opcode of its own
    /// (zend_try_compile_special_func: `strlen`, the `is_*` checks, `count`,
    /// …): it never shows in a trace, and a callback it runs is called from
    /// the caller's line.
    pub const INLINE: CallFlags = CallFlags(1);
    /// A php 8.4 frameless internal call (ZEND_FRAMELESS_ICALL_n): the
    /// builtin gets no frame of its own, so what it runs sees its caller's.
    pub const ICALL: CallFlags = CallFlags(2);
    /// `call_user_func()` / `call_user_func_array()` compiled to a user call
    /// (ZEND_INIT_USER_CALL): the callback sees the caller's `strict_types`.
    pub const USER_CALL: CallFlags = CallFlags(4);
    /// The call is a whole expression statement: its result is unused
    /// (php's USED_RET() is false).
    pub const RESULT_UNUSED: CallFlags = CallFlags(8);

    pub const fn contains(self, other: CallFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: CallFlags) {
        self.0 |= other.0;
    }
}

/// One bytecode instruction. `Copy`, fixed-width, one variant per opcode.
#[derive(Clone, Copy, Debug)]
#[repr(u16)]
pub enum Instr {
    // -- move / const / load --
    Nop,
    /// ZEND_TICKS: after each statement under `declare(ticks=n)`; every
    /// `n`th one runs the registered tick functions.
    Ticks {
        n: u32,
    },
    Mov {
        dst: Reg,
        src: Reg,
    },
    /// `Mov` out of a temporary that is dead afterwards (an argument moved
    /// into the call window): `src` becomes null, so a freed argument gives
    /// its object handle back when the call returns, as in php.
    MovTake {
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
    /// `unset($x)`: CV `cv` becomes undefined (a reference it held is
    /// dropped, not written through).
    UnsetCv {
        cv: Reg,
    },
    /// Warn when CV `cv` is undefined ("Undefined variable $x"), once for a
    /// read that several instructions make quietly (a `list()` source).
    CheckCv {
        cv: Reg,
    },
    /// `Mov` for a quiet read (`??`, `??=`, `?->` under isset): an undefined
    /// CV reads as null without the warning (php's BP_VAR_IS).
    MovIs {
        dst: Reg,
        src: Reg,
    },
    LoadBool {
        dst: Reg,
        b: bool,
    },
    LoadInt {
        dst: Reg,
        n: i64,
    },

    // -- variable store (ref-aware) --
    Assign {
        cv: Reg,
        src: Reg,
    },

    // -- arithmetic / bitwise / unary — route to ops:: --
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

    // -- casts --
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

    /// `@expr` begin (ZEND_BEGIN_SILENCE): error_reporting drops to the
    /// fatal types; the old value is kept for `EndSilence`.
    BeginSilence,
    /// `@expr` end (ZEND_END_SILENCE): the old error_reporting comes back,
    /// unless the code under `@` set one that shows more than fatal errors.
    EndSilence,

    // -- compare — route to ops:: --
    Cmp {
        dst: Reg,
        a: Reg,
        b: Reg,
        kind: CmpKind,
    },

    // -- output --
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

    // -- jumps / branches --
    Jmp {
        target: Pc,
    },
    /// Prologue guard for a non-constant parameter default: jump to `target`
    /// (skipping the default-init code) if argument `index` was actually passed.
    JmpIfArg {
        index: u16,
        target: Pc,
    },
    /// After a non-constant default was stored into parameter `index`:
    /// check it against the parameter's type (php's RECV_INIT).
    RecvInitCheck {
        index: u16,
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
    /// ZEND_ASSERT_CHECK: with zend.assertions <= 0, store true in `dst`
    /// and jump to `target`, past the `assert()` call.
    AssertCheck {
        dst: Reg,
        target: Pc,
    },

    // -- arrays --
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
    /// `f(...$src)`: unpack an array or Traversable into the argument array
    /// `arr` (int keys positional, string keys named).
    ArgSpread {
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
    /// `DimFetch` as the object of a property write (`$a[k]->p = v`): an
    /// ArrayAccess element is fetched for writing (`BP_VAR_W`), anything
    /// else is read.
    DimFetchW {
        dst: Reg,
        arr: Reg,
        key: Reg,
    },
    /// An element on the way to an `unset($a[$k]->p)` (FETCH_DIM_UNSET):
    /// a missing key is silent, a string offset or a scalar container an
    /// Error.
    DimFetchUnset {
        dst: Reg,
        arr: Reg,
        key: Reg,
    },
    /// `DimFetch` in isset/`??` context (`BP_VAR_IS`): no diagnostics, and an
    /// absent string offset reads as Null.
    DimFetchIs {
        dst: Reg,
        arr: Reg,
        key: Reg,
    },
    /// `DimFetch` for a `list()`/`[...] =` destructuring element
    /// (`ZEND_FETCH_LIST_R`).
    DimFetchList {
        dst: Reg,
        arr: Reg,
        key: Reg,
    },

    // -- lvalue cursor: descend a root CV by keys/appends, then commit --
    // Emitted contiguously (no interleaving); the VM holds the cursor between them.
    /// Begin a cursor rooted at CV `cv` for a write (php's BP_VAR_W fetch:
    /// an undefined variable becomes null, silently).
    LvalCv {
        cv: Reg,
    },
    /// [`Instr::LvalCv`] for a read-modify-write (`$a[$k] .= v`, `$a[0]++`):
    /// an undefined variable warns, then becomes null.
    LvalCvRw {
        cv: Reg,
    },
    /// [`Instr::LvalCv`] for `unset($a[...])`: an undefined variable warns
    /// and stays undefined (the unset does nothing).
    LvalCvUnset {
        cv: Reg,
    },
    LvalKey {
        key: Reg,
    }, // descend into element `key` (autovivify on write)
    LvalAppend, // descend into a freshly appended element
    FetchLvalRw {
        dst: Reg,
    }, // read the current cursor location (null if absent), keep cursor open
    /// Write `src` at the cursor location (COW-bubbling), close the cursor.
    /// The assignment's value goes to `dst` when that is another register:
    /// `src`, or for a string offset the byte written (null if none was).
    AssignLval {
        src: Reg,
        dst: Reg,
    },
    UnsetLval, // remove the cursor's leaf key, close cursor
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
        /// `yield from` delegation: a generator source is attached where it
        /// stands (started, not rewound) instead of being rewound.
        delegate: bool,
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

    // -- calls / return --
    /// Resolve `name` (case-insensitive): a user function pushes a frame, a
    /// builtin is called with `regs[base..base+argc]`.
    /// `fallback` is a second name to try if `name` misses (`NO_NAME` if none):
    /// an unqualified call inside a namespace resolves `Ns\name` first, then the
    /// global `name` (PHP's fall-back-to-global rule). `flags` apply when the
    /// call reaches a builtin.
    Call {
        dst: Reg,
        name: NIdx,
        base: Reg,
        argc: u16,
        fallback: NIdx,
        flags: CallFlags,
    },
    /// Variadic call: `args` holds a `PhpArray` of the evaluated arguments
    /// (used when a call site contains argument spread `...$x` or a named
    /// argument). php compiles such a call as a plain one, so it has no
    /// `CallFlags`.
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
    /// callable value in `callee` as a `Closure`. `ns_name` is the
    /// namespaced name as written when `callee` holds the global name an
    /// unqualified `f(...)` in a namespace fell back to, else `NO_NAME`.
    MakeFcc {
        dst: Reg,
        callee: Reg,
        ns_name: NIdx,
        /// In a constant expression (no `__callStatic` callables there).
        const_expr: bool,
        /// Static-call syntax (`C::m(...)`, `'C::m'(...)`): `self`/`parent`/
        /// `static` mean the scope's classes and `$this` is kept
        /// (ZEND_INIT_STATIC_METHOD_CALL), unlike an array callable.
        static_call: bool,
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
    /// A read of `$this` where it may be missing: throw php's "Using $this
    /// when not in object context" unless `cv` holds an object.
    CheckThis {
        cv: Reg,
    },
    /// A property fetch in a constant expression (zend_ast_evaluate's
    /// ZEND_AST_PROP): the runtime name (`Reg::MAX` for none) must convert
    /// to a string, and an object must be an enum. Replaces `name` with
    /// its string.
    ConstExprProp {
        obj: Reg,
        name: Reg,
    },
    /// A dimension read in a constant expression: not on an object.
    ConstExprDim {
        arr: Reg,
    },
    /// Throw an `Error` whose message is the string constant `k`: a form
    /// php compiles but always rejects when it runs.
    ThrowError {
        k: CIdx,
    },
    /// A call result passed for a by-reference parameter: raise php's notice
    /// "Only variables should be passed by reference" and pass the value.
    SendNoRef {
        dst: Reg,
        src: Reg,
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

    // -- OOP --
    /// `new C(args)`: allocate into `dst`, then run `__construct` via the window.
    New {
        dst: Reg,
        class: ClassRef,
        base: Reg,
        argc: u16,
    },
    /// ZEND_NEW for a `new` with arguments: create the object in `dst`;
    /// the arguments are evaluated next, then `NewCall` / `NewCallV`
    /// runs the constructor.
    NewObj {
        dst: Reg,
        class: ClassRef,
    },
    /// Run the constructor of the object `NewObj` put in `dst`.
    NewCall {
        dst: Reg,
        base: Reg,
        argc: u16,
    },
    /// `NewCall` with the arguments in a `PhpArray` (spread / named).
    NewCallV {
        dst: Reg,
        args: Reg,
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
    /// `$o->$m(...$args)` / `$o->$m(name: v)`: `CallMethodV` with the method
    /// name a runtime string in register `name`.
    CallMethodDynV {
        dst: Reg,
        obj: Reg,
        name: Reg,
        args: Reg,
    },
    /// `C::$m(...$args)` / `C::$m(name: v)`: `CallStaticV` with the method
    /// name a runtime string in register `name`.
    CallStaticDynV {
        dst: Reg,
        class: ClassRef,
        name: Reg,
        args: Reg,
    },
    /// `$o->p` (value context).
    FetchProp {
        dst: Reg,
        obj: Reg,
        name: NIdx,
    },
    /// `$o->$p` / `$o->{expr}`: the property name is a runtime string in `name`.
    FetchPropDyn {
        dst: Reg,
        obj: Reg,
        name: Reg,
    },
    /// `$o->p` as the object of a property write (`$o->p->q = v`): php's
    /// BP_VAR_W fetch, which a magic `__get` serves as a write
    /// (`Host::magic_fetch`).
    FetchPropW {
        dst: Reg,
        obj: Reg,
        name: NIdx,
    },
    /// `FetchPropW` with the property name in a register.
    FetchPropDynW {
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
    },
    /// `self::class` / `parent::class` / `static::class` / `$obj::class`
    /// (ZEND_FETCH_CLASS_NAME; with `const_expr`, zend_ast_evaluate's
    /// ZEND_AST_CLASS_NAME).
    FetchClassName {
        dst: Reg,
        class: ClassRef,
        const_expr: bool,
    },
    /// `C::K` / `C::class`.
    FetchClassConst {
        dst: Reg,
        class: ClassRef,
        name: NIdx,
    },
    /// `C::{expr}`: `FetchClassConst` with a runtime name, which must be a
    /// string.
    FetchClassConstDyn {
        dst: Reg,
        class: ClassRef,
        name: Reg,
    },
    /// SEND_FUNC_ARG's question for a call of a callable value (`$f(...)`):
    /// whether parameter `idx` of what `callee` calls is by reference.
    ValueArgIsRef {
        dst: Reg,
        callee: Reg,
        idx: u16,
    },
    /// SEND_FUNC_ARG's question for a call resolved at run time: whether
    /// parameter `idx` of method `name` on the object in `obj` is by
    /// reference (false for a non-object / unknown / magic method).
    MethodArgIsRef {
        dst: Reg,
        obj: Reg,
        name: NIdx,
        idx: u16,
    },
    /// The user function a call the compiler could not resolve runs
    /// (another file's, or declared later), looked up once before its
    /// arguments as php's INIT_FCALL_BY_NAME: `name`, then `fallback`, both
    /// lowercase. `dst` gets its index, or -1 (none, or a builtin).
    ResolveFunc {
        dst: Reg,
        name: NIdx,
        fallback: NIdx,
    },
    /// The same question for the function [`Instr::ResolveFunc`] put in
    /// `func`.
    FuncArgIsRef {
        dst: Reg,
        func: Reg,
        idx: u16,
    },
    /// php's SEND_VAR_EX: variable `cv` as argument `idx` of the function
    /// in `func` (see [`Instr::ResolveFunc`]): a reference when that
    /// parameter is by reference, else its value.
    SendVarEx {
        dst: Reg,
        cv: Reg,
        func: Reg,
        idx: u16,
    },
    /// Copy `src` to `dst` as is: a reference stays a reference.
    PassArg {
        dst: Reg,
        src: Reg,
    },
    /// The same for static method (or constructor) `name` of `class`; the
    /// class is resolved (autoloaded) first, as php's INIT_STATIC_METHOD_CALL
    /// / NEW do before the arguments.
    StaticArgIsRef {
        dst: Reg,
        class: ClassRef,
        name: NIdx,
        idx: u16,
    },
    /// `$o->m(args)`.
    CallMethod {
        dst: Reg,
        obj: Reg,
        name: NIdx,
        base: Reg,
        argc: u16,
    },
    /// `C::m(args)` / `self::` / `static::` / `parent::`.
    CallStatic {
        dst: Reg,
        class: ClassRef,
        name: NIdx,
        base: Reg,
        argc: u16,
    },
    /// `parent::$prop::get()` / `parent::$prop::set($v)` inside a hook of
    /// `$prop` (php's INIT_PARENT_PROPERTY_HOOK_CALL): the parent class's
    /// hook, or a plain read / write of the backing value when it has none.
    /// `argc` [`PACKED_ARGS`]: `base` holds the packed arguments array (a
    /// call with named arguments).
    CallParentHook {
        dst: Reg,
        /// The property name.
        name: NIdx,
        kind: HookKind,
        base: Reg,
        argc: u16,
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
    /// `isset($c[$k])`: array element present and non-null, string offset in
    /// range, or `ArrayAccess::offsetExists`; else false.
    IssetDim {
        dst: Reg,
        arr: Reg,
        key: Reg,
    },
    /// `empty($c[$k])`: `!DimFetchIs`, except that an ArrayAccess object is
    /// asked php's `has_dimension(check_empty = 1)`: `offsetExists`, which
    /// sees an `empty()` fetch (`Host::magic_fetch`), then `offsetGet`.
    EmptyDim {
        dst: Reg,
        arr: Reg,
        key: Reg,
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
    /// `global $$n` / `global ${expr}`: bind the current scope's variable
    /// named by register `name` to that global.
    BindGlobalDyn {
        name: Reg,
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
    /// `#[A] const NAME = <expr>;` (ZEND_DECLARE_ATTRIBUTED_CONST):
    /// `DeclareConst` that also attaches `const_attrs[attrs]` to the new
    /// constant.
    DeclareAttrConst {
        name: NIdx,
        src: Reg,
        attrs: u32,
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
    /// Cursor root: `C::${expr}` / `C::$$x` (static-property name in a register).
    LvalStaticPropDyn {
        class: ClassRef,
        name: Reg,
    },
    /// `C::${expr}` (value context): `FetchStaticProp` with a runtime name.
    FetchStaticPropDyn {
        dst: Reg,
        class: ClassRef,
        name: Reg,
    },
    /// `unset(C::$p)`: resolve the class (autoloading it), then throw
    /// "Attempt to unset static property".
    UnsetStaticProp {
        class: ClassRef,
        name: Reg,
    },
    /// `FetchStaticPropIs` with a runtime name.
    FetchStaticPropDynIs {
        dst: Reg,
        class: ClassRef,
        name: Reg,
    },
    /// `$$name` / `${expr}` read: the current scope's variable of that name
    /// ("Undefined variable" warning and null when it does not exist).
    FetchVarVar {
        dst: Reg,
        name: Reg,
    },
    /// [`Instr::FetchVarVar`] for isset/empty/`??`: no warning.
    FetchVarVarIs {
        dst: Reg,
        name: Reg,
    },
    /// Cursor root: the current scope's variable named by register `name`
    /// (created on write), for `$$name = v`, `${expr}[] = v`, ….
    LvalVarVar {
        name: Reg,
    },
    /// [`Instr::LvalVarVar`] for a read-modify-write: an undefined variable
    /// warns first.
    LvalVarVarRw {
        name: Reg,
    },
    /// `$$name = &<src>`: make the current scope's variable named by register
    /// `name` an alias of the reference in `src`.
    BindVarVarRef {
        name: Reg,
        src: Reg,
    },
    /// `unset($$name)`: remove the current scope's variable of that name.
    UnsetVarVar {
        name: Reg,
    },
    /// Cursor root: the global variable named by `name` (a runtime string
    /// register), i.e. `$GLOBALS[$name]...` as an lvalue (php 8.1+: element
    /// writes through `$GLOBALS` go to the global symbol table).
    LvalGlobal {
        name: Reg,
    },
    /// [`Instr::LvalGlobal`] for a read-modify-write (`$GLOBALS['x'] .= v`):
    /// an undefined global warns first ("Undefined global variable $x").
    LvalGlobalRw {
        name: Reg,
    },
    /// `$GLOBALS[$name]` read: the global variable's value ("Undefined global
    /// variable" warning when absent).
    FetchGlobal {
        dst: Reg,
        name: Reg,
    },
    /// Bare `$GLOBALS` read: a by-value copy of the global symbol table.
    FetchGlobals {
        dst: Reg,
    },
    /// `unset($GLOBALS[$name])`: remove the global variable.
    UnsetGlobal {
        name: Reg,
    },
    /// Declare a class at this statement. `class_idx` is file-local: indices
    /// below the file's user top-level class count name `Program::classes`
    /// (after the prelude) — a no-op when that class was already hoisted at
    /// load; the rest name `Program::cond_classes` (classes nested in control
    /// flow or function bodies), linked only when reached.
    DeclareClass {
        class_idx: u32,
    },
    /// Register a conditionally-declared function (one nested in control flow, so
    /// not hoisted): its proto is stored in the closures table at file-local
    /// index `idx` (resolved against the running frame's unit).
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
    /// [`Instr::MakeRef`] for an argument whose by-reference-ness the callee
    /// decides when it binds it (a method's window): an undefined variable
    /// stays undefined until then (php's SEND_VAR_EX): a by-reference
    /// parameter creates it, a by-value one warns.
    MakeArgRef {
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
    /// `__halt_compiler()`: stop this proto as a top-level return.
    Halt,
    /// End of an expression statement: the temporaries `[from, to)` it used are
    /// dead — release them (so an object only they still held is freed, and
    /// its destructor runs, at the statement boundary as in php).
    ClearTemps {
        from: Reg,
        to: Reg,
    },
    /// `exit`/`die`: `arg` is php 8.4's `int|string $status` (`Reg::MAX`: no
    /// argument, status 0); a string is printed (status 0). Terminates the
    /// script via the SAPI exit path.
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

/// A by-value/by-ref/variadic parameter.
#[derive(Clone)]
pub struct ParamInfo {
    pub name: NIdx,
    pub cv: Reg,
    pub by_ref: bool,
    pub variadic: bool,
    pub default: Option<ParamDefault>,
    /// Declared type (for `strict_types` enforcement / coercion); None = untyped.
    pub type_hint: Option<TypeHint>,
    /// Source line of the parameter declaration (php raises argument
    /// TypeError / ArgumentCountError at the failing parameter's line).
    pub line: u32,
    /// `#[...]` attributes on the parameter (Reflection metadata only).
    pub attributes: Vec<AttributeProto>,
    /// A constructor-promoted parameter (`public function __construct(private $x)`).
    pub promoted: bool,
    /// A non-constant default compiled as a standalone thunk, for
    /// `ReflectionParameter::getDefaultValue()` (the call path evaluates it
    /// inline in the prologue instead).
    pub default_thunk: Option<Rc<FunctionProto>>,
    /// The default is a constant reference (`= PHP_EOL`, `= self::X`): its
    /// name, for `getDefaultValueConstantName()`.
    pub default_const: Option<PhpStr>,
}

/// A compiled `#[Name(args)]` attribute (Reflection metadata only).
#[derive(Clone)]
pub struct AttributeProto {
    /// The attribute class name, namespace/use-resolved (no leading `\`).
    pub name: PhpStr,
    /// The argument list as an array (int keys for positional arguments,
    /// string keys for named ones) — folded, or a thunk evaluated lazily in
    /// the declaring scope (`ReflectionAttribute::getArguments()`); `None`
    /// when the attribute has no argument list.
    pub args: Option<ConstInitProto>,
    /// An internal attribute's validator error that
    /// `#[\DelayedTargetValidation]` deferred to `newInstance()`.
    pub validation_error: Option<PhpStr>,
    /// Where it is written and its file's `strict_types`: `newInstance()`
    /// runs the constructor as if called from there.
    pub file: PhpStr,
    pub line: u32,
    pub strict_types: bool,
}

/// Declaration metadata surfaced by Reflection: attributes, the doc comment
/// and the declaration's source line span.
#[derive(Clone, Default)]
pub struct DeclMeta {
    pub attributes: Vec<AttributeProto>,
    /// The `/** ... */` comment php's scanner attaches to the declaration.
    pub doc_comment: Option<PhpStr>,
    pub start_line: u32,
    pub end_line: u32,
    /// Byte offset of the declaration (orders members declared on one line).
    pub offset: u32,
}

/// A declared parameter / return type: a (possibly nullable) union or
/// intersection of type kinds. A single plain type is one `kinds` element with
/// `intersection = false`.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeHint {
    pub nullable: bool,
    pub intersection: bool,
    pub kinds: Vec<TypeKind>,
    /// DNF intersection groups of a union (`(A&B)|C|null`): each inner list is
    /// one parenthesized intersection; `kinds` holds the union's other members.
    pub groups: Vec<Vec<PhpStr>>,
    /// Where each group sits among the union's class members (the count of
    /// class members declared before it), for php's declaration-order type
    /// strings. Missing entries mean "first".
    pub group_pos: Vec<u16>,
    /// `null` was spelled as a union member (`int|null`), not `?int`; only
    /// matters for how Reflection prints a multi-member union.
    pub explicit_null: bool,
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

/// A parameter's default value.
#[derive(Clone, Copy)]
pub enum ParamDefault {
    /// A constant default folded at compile time: an index into `consts`.
    Const(CIdx),
    /// A non-constant default, evaluated by the prologue code `JmpIfArg`
    /// skips when the argument is passed.
    Init,
}

/// A protected region in the exception table. Protects the
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
    /// The first temporary the try body allocates: an exception caught
    /// here frees every temporary from it up (php's live ranges).
    pub temps: Reg,
}

#[derive(Clone)]
pub struct CatchArm {
    pub classes: Vec<PhpStr>,
    pub var_reg: Option<Reg>,
    pub target: Pc,
}

/// Debug side-table: never consulted for semantics.
#[derive(Clone, Default)]
pub struct DebugInfo {
    pub file: PhpStr,
    pub lines: Vec<(Pc, u32)>,
}

/// What the compiler knows about a function that the VM checks while
/// running it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProtoFlags(u16);

impl ProtoFlags {
    /// Built-in code, compiled from the prelude (the PHP source behind some
    /// of php's internal functions and classes). php runs internal code
    /// without a user frame: its errors are reported at the calling user
    /// code, it takes its caller's `strict_types`, and its argument errors
    /// read like an internal function's.
    pub const INTERNAL: ProtoFlags = ProtoFlags(1);
    /// Not set by the compiler: the VM's copy of the script's `{main}` that
    /// holds the globals while shutdown callbacks run. php runs them with
    /// no frame at all, so this one, too, is no user frame.
    pub const SHUTDOWN_SCOPE: ProtoFlags = ProtoFlags(2);
    /// A property hook, named `$prop::get` / `$prop::set` (php's
    /// `func->common.prop_info`).
    pub const HOOK: ProtoFlags = ProtoFlags(4);
    /// The body refers to `$this` (ZEND_ACC_USES_THIS): `Closure::bind`
    /// refuses to unbind it.
    pub const USES_THIS: ProtoFlags = ProtoFlags(8);
    /// `#[\Deprecated]` (ZEND_ACC_DEPRECATED): calling it raises
    /// E_USER_DEPRECATED first.
    pub const DEPRECATED: ProtoFlags = ProtoFlags(16);
    /// `#[\NoDiscard]` (ZEND_ACC_NODISCARD): a call whose result is unused
    /// warns first.
    pub const NODISCARD: ProtoFlags = ProtoFlags(32);
    /// A constant-expression thunk (`{init}`: a class constant, property
    /// default or attribute argument). php evaluates those with no frame
    /// (zend_ast_evaluate), reporting at the expression's line.
    pub const CONST_EXPR: ProtoFlags = ProtoFlags(64);
    /// A [`ProtoFlags::CONST_EXPR`] thunk that sets no file/line override:
    /// just a global constant name (php looks it up directly), or one with
    /// no class scope (zend_ast_evaluate_ex overrides only with a scope).
    pub const CONST_EXPR_PLAIN: ProtoFlags = ProtoFlags(128);
    /// Not set by the compiler: a frame that runs no code and only stands
    /// for a call site (the dummy frame php pushes for an attribute
    /// constructor). Traces show it as the caller, never as a frame.
    pub const CALL_SITE: ProtoFlags = ProtoFlags(256);

    pub const fn contains(self, other: ProtoFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: ProtoFlags) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: ProtoFlags) -> ProtoFlags {
        ProtoFlags(self.0 | other.0)
    }

    pub fn insert(&mut self, other: ProtoFlags) {
        self.0 |= other.0;
    }
}

/// One compiled PHP function/method/closure, or the script body (`"{main}"`).
/// Carries no `'arena` borrow — the AST is consumed at compile time. The VM
/// runs it as compiled and shares it (`Rc`) between every install of its
/// program: file-local operands (`MakeClosure`, `DeclareFunc`,
/// `DeclareClass`) are resolved against the running frame's unit.
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
    pub debug: DebugInfo,
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
    /// Reflection metadata (attributes, doc comment, line span).
    pub meta: DeclMeta,
    pub flags: ProtoFlags,
    /// The attribute lists of the attributed global constants this body
    /// declares (`DeclareAttrConst`).
    pub const_attrs: Vec<Rc<Vec<AttributeProto>>>,
    /// The calls whose result is unused (php's IS_UNUSED result after
    /// zend_do_free), ascending: a `#[\NoDiscard]` callee warns when called
    /// from one.
    pub discard_calls: Vec<Pc>,
    /// Each `NewObj` pc, its `NewCall`/`NewCallV` pc and register: an
    /// exception thrown in between releases the object without its
    /// destructor (php's live range of a NEW result).
    pub new_ranges: Vec<(Pc, Pc, Reg)>,
}

impl FunctionProto {
    /// Built-in (prelude) code: see [`ProtoFlags::INTERNAL`].
    pub fn is_internal(&self) -> bool {
        self.flags.contains(ProtoFlags::INTERNAL)
    }

    /// For a property hook (`$prop::get`), the property's name.
    pub fn hook_prop(&self) -> Option<&[u8]> {
        if !self.flags.contains(ProtoFlags::HOOK) {
            return None;
        }
        let n = self.name.as_bytes();
        n.get(1..n.len().checked_sub(5)?)
    }

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
            debug: DebugInfo::default(),
            this_cv: None,
            is_static: false,
            num_captures: 0,
            is_generator: false,
            superglobals: Vec::new(),
            return_type: None,
            strict_types: false,
            returns_ref: false,
            cv_var_names: Vec::new(),
            meta: DeclMeta::default(),
            flags: ProtoFlags::default(),
            const_attrs: Vec::new(),
            discard_calls: Vec::new(),
            new_ranges: Vec::new(),
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
    /// Shared: the VM links a declaration without copying it.
    pub classes: Vec<Rc<ClassProto>>,
    /// Anonymous closure / arrow-function protos (and conditional functions),
    /// indexed by the file-local `fnidx` of `MakeClosure` (`idx` of
    /// `DeclareFunc`).
    pub closures: Vec<Rc<FunctionProto>>,
    /// How many leading entries of `classes` are the built-in prelude (always
    /// re-emitted per compile). An included file's prelude classes are skipped on
    /// merge; user classes past this count fatally redeclare if already defined.
    pub num_prelude_classes: usize,
    /// How many leading entries of `functions` are prelude helper functions
    /// (`iterator_*`, …), re-emitted per compile and skipped on include merge.
    pub num_prelude_functions: usize,
    /// Class declarations nested in control flow / function bodies; never
    /// hoisted, declared by `DeclareClass` when execution reaches them.
    pub cond_classes: Vec<Rc<ClassProto>>,
    /// Diagnostics php's compiler raises for this file: `(level, message,
    /// line)`, reported when the file is loaded.
    pub compile_diags: Vec<(i64, String, u32)>,
    /// Whether `compile_diags` were reported: a program opcache hands out
    /// again reports them only with `opcache.record_warnings`.
    pub compile_diags_reported: std::cell::Cell<bool>,
}

// ---------------------------------------------------------------------------
// OOP: class operand + compiled (un-linked) class declarations
// ---------------------------------------------------------------------------

/// How a `New`/`CallStatic`/`FetchStaticProp`/`FetchClassConst`/`InstanceOf`
/// names its class. `Copy`, owns no heap.
#[derive(Clone, Copy, Debug)]
pub enum ClassRef {
    /// `Foo` — interned name; resolved to a `ClassId` at run time.
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
    /// The `use T { ... }` adaptation rules, in source order (`None` when
    /// there are none).
    pub trait_rules: Option<Box<TraitRules>>,
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
    /// Reflection metadata (attributes, doc comment, line span).
    pub meta: DeclMeta,
    /// Where the declaration starts (file, line): php's
    /// `info.user.filename` / `line_start`. The file is empty for a
    /// built-in prelude class.
    pub decl_file: PhpStr,
    pub decl_line: u32,
}

/// A class's trait adaptations: php's `trait_precedences` and
/// `trait_aliases`. Trait names are resolved, as written otherwise.
#[derive(Clone, Default)]
pub struct TraitRules {
    pub precedences: Vec<TraitPrecedence>,
    pub aliases: Vec<TraitAlias>,
}

/// `T::m insteadof A, B;`
#[derive(Clone)]
pub struct TraitPrecedence {
    pub trait_name: PhpStr,
    pub method: PhpStr,
    pub excludes: Vec<PhpStr>,
}

/// `[T::]m as [modifier] [alias];`
#[derive(Clone)]
pub struct TraitAlias {
    pub trait_name: Option<PhpStr>,
    pub method: PhpStr,
    pub alias: Option<PhpStr>,
    /// A new visibility (`as protected`).
    pub visibility: Option<Visibility>,
    /// `as final`.
    pub is_final: bool,
}

/// One compiled enum case.
#[derive(Clone)]
pub struct EnumCaseProto {
    pub name: PhpStr,
    /// The backing value — folded, or a thunk evaluating a constant expression
    /// on first access — or `None` for a pure enum.
    pub value: Option<ConstInitProto>,
    pub meta: DeclMeta,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnumBacking {
    Int,
    Str,
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

/// Which property hook.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HookKind {
    Get = 0,
    Set = 1,
}

impl HookKind {
    pub fn name(self) -> &'static str {
        match self {
            HookKind::Get => "get",
            HookKind::Set => "set",
        }
    }
}

/// One compiled property hook: a method named `$prop::get` / `$prop::set`.
#[derive(Clone)]
pub struct HookProto {
    pub proto: Rc<FunctionProto>,
    pub is_final: bool,
    pub is_abstract: bool,
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
    pub meta: DeclMeta,
    /// Declared by constructor promotion.
    pub promoted: bool,
    /// The `get` and `set` hooks (index: [`HookKind`]).
    pub hooks: [Option<HookProto>; 2],
    /// A hooked property no hook body of which uses `$this->prop`
    /// (zend_property_is_virtual): it has no backing value.
    pub is_virtual: bool,
    pub is_abstract: bool,
    pub is_final: bool,
}

impl PropProto {
    pub fn has_hooks(&self) -> bool {
        self.hooks.iter().any(Option::is_some)
    }
}

#[derive(Clone)]
pub struct StaticPropProto {
    pub name: PhpStr,
    pub visibility: Visibility,
    pub is_readonly: bool,
    pub init: ConstInitProto,
    /// Declared type (`None` = untyped).
    pub type_hint: Option<TypeHint>,
    pub meta: DeclMeta,
    /// Declared with an explicit default (`static $x = 1;`).
    pub has_default: bool,
}

#[derive(Clone)]
pub struct ConstProto {
    pub name: PhpStr,
    pub visibility: Visibility,
    pub value: ConstInitProto,
    /// Declared type of a typed class constant (`None` = untyped).
    pub type_hint: Option<TypeHint>,
    pub meta: DeclMeta,
    pub is_final: bool,
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

/// [`Instr::CallParentHook`]'s `argc` for a packed arguments array.
pub const PACKED_ARGS: u16 = u16::MAX;

// The plain u16 tag (`repr(u16)`) costs no space: the dispatch loop reads
// it without decoding a niche.
const _: () = assert!(std::mem::size_of::<Instr>() == 24);
