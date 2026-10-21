//! The phrust bytecode ISA — **data only, no behavior**.
//!
//! A register-based instruction set (see `docs/design/bytecode.md` §2). Depends
//! only on `phrust-values` so a future `phrust-jit` can lower the same ISA
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

use phrust_values::{ClassFlags, ClassKind, PhpStr, Value, Visibility};

// --- operand newtypes (kept as aliases for a lean encoding) ---------------
pub type Reg = u16; // register-file index: CVs in [0,num_cvs), temporaries above
pub type CIdx = u32; // index into FunctionProto.consts
pub type NIdx = u32; // index into FunctionProto.names (interned identifier bytes)
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
    Mov { dst: Reg, src: Reg },
    LoadConst { dst: Reg, k: CIdx },
    LoadNull { dst: Reg },
    LoadBool { dst: Reg, b: bool },
    LoadInt { dst: Reg, n: i64 },

    // -- variable store (ref-aware; phase 2/6) --
    Assign { cv: Reg, src: Reg },

    // -- arithmetic / bitwise / unary (phase 2) — route to ops:: --
    Add { dst: Reg, a: Reg, b: Reg },
    Sub { dst: Reg, a: Reg, b: Reg },
    Mul { dst: Reg, a: Reg, b: Reg },
    Div { dst: Reg, a: Reg, b: Reg },
    Mod { dst: Reg, a: Reg, b: Reg },
    Pow { dst: Reg, a: Reg, b: Reg },
    BitAnd { dst: Reg, a: Reg, b: Reg },
    BitOr { dst: Reg, a: Reg, b: Reg },
    BitXor { dst: Reg, a: Reg, b: Reg },
    Shl { dst: Reg, a: Reg, b: Reg },
    Shr { dst: Reg, a: Reg, b: Reg },
    Concat { dst: Reg, a: Reg, b: Reg },
    Neg { dst: Reg, a: Reg },
    BitNot { dst: Reg, a: Reg },
    Not { dst: Reg, a: Reg },
    /// `++`/`--`; ref-aware write-through. `pre`: value is new(true)/old(false).
    IncDec { dst: Reg, cv: Reg, pre: bool, delta: i8 },

    // -- casts (phase 2) --
    CastInt { dst: Reg, a: Reg },
    CastFloat { dst: Reg, a: Reg },
    CastString { dst: Reg, a: Reg },
    CastBool { dst: Reg, a: Reg },
    CastArray { dst: Reg, a: Reg },

    // -- compare (phase 2) — route to ops:: --
    Cmp { dst: Reg, a: Reg, b: Reg, kind: CmpKind },

    // -- output (phase 1) --
    Echo { src: Reg },
    EchoConst { k: CIdx },
    Print { dst: Reg, src: Reg },

    // -- jumps / branches (phase 3) --
    Jmp { target: Pc },
    JmpTrue { cond: Reg, target: Pc },
    JmpFalse { cond: Reg, target: Pc },
    JmpNull { src: Reg, target: Pc },
    JmpNotNull { src: Reg, target: Pc },

    // -- arrays (phase 4; literals first) --
    NewArray { dst: Reg, cap: u32 },
    ArrPush { arr: Reg, val: Reg },
    ArrSet { arr: Reg, key: Reg, val: Reg },
    ArrSpread { arr: Reg, src: Reg },

    // -- array element read (value context): element or Null; string offset --
    DimFetch { dst: Reg, arr: Reg, key: Reg },

    // -- lvalue cursor: descend a root CV by keys/appends, then commit --
    // Emitted contiguously (no interleaving); the VM holds the cursor between them.
    LvalCv { cv: Reg },        // begin a cursor rooted at CV `cv`
    LvalKey { key: Reg },      // descend into element `key` (autovivify on write)
    LvalAppend,                // descend into a freshly appended element
    FetchLvalRw { dst: Reg },  // read the current cursor location (null if absent), keep cursor open
    AssignLval { src: Reg },   // write `src` at the cursor location (COW-bubbling), close cursor
    UnsetLval,                 // remove the cursor's leaf key, close cursor
    /// `++`/`--` on the cursor location (ref-correct inc_dec); dst = new/old. Closes cursor.
    IncDecLval { dst: Reg, pre: bool, delta: i8 },

    // -- foreach (by value): iterate a COW snapshot --
    FeReset { arr_reg: Reg, pos_reg: Reg, src: Reg, end: Pc }, // snapshot src; pos=0; if not-array/empty -> jmp end
    FeFetch { arr_reg: Reg, pos_reg: Reg, key_cv: Reg, val_cv: Reg, end: Pc }, // bind key?/value; pos++; if done jmp end

    // -- calls / return (phase 1 builtins; user functions later) --
    /// Resolve `name` (case-insensitive): refused → Unsupported; builtin →
    /// call with `regs[base..base+argc]`; user function (later) → push frame.
    Call { dst: Reg, name: NIdx, base: Reg, argc: u16 },
    Ret { src: Reg },
    RetNull,

    /// `match` with no matching arm and no default: throw `\UnhandledMatchError`.
    ThrowMatchError { subject: Reg },
    /// `throw $e`: begin unwinding with the Throwable in `src`.
    Throw { src: Reg },
    /// End of a `finally` block: perform the pending continuation (re-raise a
    /// pending exception, perform a pending return, or fall through).
    FinallyEnd,
    /// A `return` that must cross a `finally`: stash the value as a pending
    /// return, then `Jmp` to the finally block.
    SetPendingRet { src: Reg },

    // -- OOP (phase 7) --
    /// `new C(args)`: allocate into `dst`, then run `__construct` via the window.
    New { dst: Reg, class: ClassRef, base: Reg, argc: u16, cache: CacheIdx },
    /// `$o->p` (value context).
    FetchProp { dst: Reg, obj: Reg, name: NIdx, cache: CacheIdx },
    /// `C::$p` (value context).
    FetchStaticProp { dst: Reg, class: ClassRef, name: NIdx, cache: CacheIdx },
    /// `C::K` / `C::class`.
    FetchClassConst { dst: Reg, class: ClassRef, name: NIdx, cache: CacheIdx },
    /// `$o->m(args)`.
    CallMethod { dst: Reg, obj: Reg, name: NIdx, base: Reg, argc: u16, cache: CacheIdx },
    /// `C::m(args)` / `self::` / `static::` / `parent::`.
    CallStatic { dst: Reg, class: ClassRef, name: NIdx, base: Reg, argc: u16, cache: CacheIdx },
    /// `$val instanceof C` (never throws).
    InstanceOf { dst: Reg, val: Reg, class: ClassRef },
    /// `clone $src`: shallow copy + new id + `__clone`.
    CloneObj { dst: Reg, src: Reg },
    /// Cursor step: descend into object property `name`.
    LvalProp { name: NIdx },
    /// Cursor root: a static-property cell `C::$name`.
    LvalStaticProp { class: ClassRef, name: NIdx },
    /// Link + register a conditionally-declared class at this point.
    DeclareClass { class_idx: u32 },

    // -- references --
    /// `$dst =& $src`: promote `src`'s slot to a shared Ref; `dst` aliases it.
    BindRef { dst_cv: Reg, src_cv: Reg },
    /// Materialize CV `cv` as a Ref and place `Value::Ref` in `dst` (by-ref arg).
    MakeRef { dst: Reg, cv: Reg },

    // -- misc --
    FetchConst { dst: Reg, name: NIdx },
    /// Emitted only for a structurally-refused construct on a reachable path;
    /// raises the uncatchable `Thrown::Unsupported`.
    Refuse { feature: NIdx, reason: NIdx },
    /// `__halt_compiler()`: stop this proto as a top-level return.
    Halt,
}

/// A by-value/by-ref/variadic parameter (used from phase 5).
#[derive(Clone)]
pub struct ParamInfo {
    pub name: NIdx,
    pub cv: Reg,
    pub by_ref: bool,
    pub variadic: bool,
    pub default: Option<Default>,
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
}

pub struct MethodProto {
    pub name: PhpStr,
    pub proto: Rc<FunctionProto>,
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
}

pub struct PropProto {
    pub name: PhpStr,
    pub visibility: Visibility,
    pub is_readonly: bool,
    pub default: PropDefaultProto,
}

pub struct StaticPropProto {
    pub name: PhpStr,
    pub visibility: Visibility,
    pub is_readonly: bool,
    pub init: ConstInitProto,
}

pub struct ConstProto {
    pub name: PhpStr,
    pub visibility: Visibility,
    pub value: ConstInitProto,
}

/// A property's per-instance default, as compiled.
pub enum PropDefaultProto {
    /// Constant default folded at compile time.
    Const(Value),
    /// Non-constant initializer: a zero-param thunk run per-instance at `new`.
    Init(Rc<FunctionProto>),
    /// Declared without a default.
    None,
}

/// A class-constant / static-prop initializer, as compiled.
pub enum ConstInitProto {
    Const(Value),
    Thunk(Rc<FunctionProto>),
}
