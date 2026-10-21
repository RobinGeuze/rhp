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

use phrust_values::{PhpStr, Value};

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

/// A fully compiled program: the script body plus all declared functions.
pub struct Program {
    pub main: Rc<FunctionProto>,
    pub functions: Vec<Rc<FunctionProto>>,
    // classes: Vec<ClassEntry>  — phase 7
}
